if (typeof importScripts === "function") importScripts("match.js");
const api = globalThis.browser ?? chrome;
async function authorizedPair(sender) {
  if (sender.frameId !== 0 || !sender.tab?.active || !sender.url) return null;
  const site = new URL(sender.url);
  if (site.protocol !== "https:" || !await api.permissions.contains({origins:[site.protocol+"//"+site.hostname+"/*"]})) return null;
  const {sites=[],unboundSites=[],server} = await api.storage.local.get(["sites","unboundSites","server"]);
  if (!sites.includes(site.origin) || !server) return null;
  const {pair} = await api.storage.session.get("pair");
  if (pair?.server !== server || !pair?.key || !pair?.token) return null;
  return {site, server, pair, unbound: unboundSites.includes(site.origin)};
}
async function suggest(sender) {
  const allowed = await authorizedPair(sender);
  if (!allowed) return null;
  const {site, server, pair, unbound} = allowed;
  const response = await fetch(server+"/otp/recent",{headers:{Authorization:"Bearer "+pair.token},redirect:"error",cache:"no-store",credentials:"omit",signal:AbortSignal.timeout(4000)});
  if (!response.ok) return null;
  const key = await crypto.subtle.importKey("raw",new TextEncoder().encode(pair.key),"AES-GCM",false,["decrypt"]);
  // A site-tagged code always wins. Otherwise use the newest untagged code, if the site allows them.
  let best = null;
  for (const row of await response.json()) {
    try {
      const bytes=Uint8Array.from(atob(row.body),ch=>ch.charCodeAt(0));
      const clear=await crypto.subtle.decrypt({name:"AES-GCM",iv:bytes.slice(0,12)},key,bytes.slice(12));
      const result=extractSuggestion(JSON.parse(new TextDecoder().decode(clear)),site.hostname,row.archived_at,Date.now(),unbound);
      if(result && (!best || (result.verified && !best.verified) || (result.verified === best.verified && result.expires > best.expires))) best = result;
    } catch { /* Other archive keys may occur in the same inbox. */ }
  }
  if (!best) return null;
  const current = await authorizedPair(sender);
  return current?.pair.token === pair.token && current.server === server && (best.verified || current.unbound) ? best : null;
}
api.runtime.onMessage.addListener((msg,sender,respond)=>{
  if(msg?.type !== "psms:suggest") return false;
  suggest(sender).then(respond,()=>respond(null));
  return true;
});

api.runtime.onConnect.addListener(port => {
  if (port.name !== "psms:arrivals") return;
  const controller = new AbortController();
  let watchdog;
  const resetTimeout = () => {
    clearTimeout(watchdog);
    watchdog = setTimeout(() => controller.abort(), 35000);
  };
  const changed = (changes, area) => {
    if ((area === "session" && changes.pair) ||
        (area === "local" && (changes.sites || changes.unboundSites || changes.server))) controller.abort();
  };
  port.onDisconnect.addListener(() => controller.abort());
  api.storage.onChanged.addListener(changed);
  async function listen() {
    const allowed = await authorizedPair(port.sender);
    if (!allowed || controller.signal.aborted) return;
    resetTimeout();
    const response = await fetch(allowed.server + "/otp/events", {
      headers: {Authorization: "Bearer " + allowed.pair.token, Accept: "text/event-stream"},
      credentials: "omit", cache: "no-store", redirect: "error", signal: controller.signal,
    });
    if (!response.ok || !response.headers.get("content-type")?.startsWith("text/event-stream")) return;
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    while (!controller.signal.aborted) {
      const {done, value} = await reader.read();
      if (done) return;
      resetTimeout();
      buffer += decoder.decode(value, {stream:true});
      let boundary;
      while ((boundary = /\r?\n\r?\n/.exec(buffer))) {
        const frame = buffer.slice(0, boundary.index);
        buffer = buffer.slice(boundary.index + boundary[0].length);
        const type = /^event: *(ready|arrival)\r?$/m.exec(frame)?.[1];
        if (type) port.postMessage({type});
      }
      if (buffer.length > 4096) return;
    }
  }
  listen().catch(() => {}).finally(() => {
    controller.abort();
    clearTimeout(watchdog);
    api.storage.onChanged.removeListener(changed);
    port.disconnect();
  });
});

const hostedInbox = "https://www.papamo.dev/sms/inbox";
const hostedApi = "https://na.sms.papamo.dev/free/v1";
api.runtime.onMessage.addListener((msg, sender, reply) => {
  if (msg?.type !== "psms:signin" || sender.url !== api.runtime.getURL("popup.html")) return false;
  (async () => {
    const tab = await api.tabs.create({url:"about:blank"});
    await api.storage.session.set({signin:{id:tab.id, expires:Date.now()+600000}});
    await api.tabs.update(tab.id,{url:hostedInbox});
  })().then(()=>reply(true),()=>reply(false));
  return true;
});
const connecting = new Set();
api.tabs.onUpdated.addListener(async (id, change, tab) => {
  if(change.status !== "complete" || tab.url?.split(/[?#]/)[0] !== hostedInbox || connecting.has(id))return;
  const {signin} = await api.storage.session.get("signin");
  if(signin?.id !== id || signin.expires < Date.now())return;
  connecting.add(id);
  try {
    const [response] = await api.scripting.executeScript({target:{tabId:id},world:"MAIN",func:async (deadline)=>{
      if(location.origin!=="https://www.papamo.dev" || location.pathname!=="/sms/inbox")return;
      const status=document.getElementById("status");
      while(Date.now()<deadline) {
        const result=await window.connectPapamoAutofill?.();
        if(result?.state==="connected")return result;
        if(result?.state==="needs-key" && status)status.textContent="Restore your archive from its backup, or complete phone setup to connect autofill.";
        await new Promise(resolve=>setTimeout(resolve,1200));
      }
    },args:[signin.expires]});
    const result=response?.result;
    const pending=(await api.storage.session.get("signin")).signin;
    if(pending?.id!==id || pending.expires!==signin.expires || pending.expires<Date.now() || !/^[0-9a-f]{64}$/.test(result?.token??"") || result?.key?.length!==32)return;
    await api.storage.local.set({server:hostedApi});
    await api.storage.session.set({pair:{key:result.key,token:result.token,server:hostedApi,username:result.username}});
    await api.storage.session.remove("signin");
    await api.scripting.executeScript({target:{tabId:id},func:()=>{
      if(location.origin==="https://www.papamo.dev" && location.pathname==="/sms/inbox"){
        const status=document.getElementById("status");
        if(status)status.textContent="Autofill connected. Open the extension on a sign-in site to enable it.";
      }
    }});
  } catch { /* A closed or navigated tab cancels the handoff. */ }
  finally { connecting.delete(id); }
});
