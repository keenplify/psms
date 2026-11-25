if (typeof importScripts === "function") importScripts("match.js");
const api = globalThis.browser ?? chrome;
async function suggest(sender) {
  if (sender.frameId !== 0 || !sender.tab?.active || !sender.url) return null;
  const site = new URL(sender.url);
  if (site.protocol !== "https:" || !await api.permissions.contains({origins:[site.protocol+"//"+site.hostname+"/*"]})) return null;
  const {sites=[],server} = await api.storage.local.get(["sites","server"]);
  if (!sites.includes(site.origin) || !server) return null;
  const {pair} = await api.storage.session.get("pair");
  if (pair?.server !== server || !pair?.key || !pair?.token) return null;
  const response = await fetch(server+"/otp/recent",{headers:{Authorization:"Bearer "+pair.token},redirect:"error",cache:"no-store",credentials:"omit",signal:AbortSignal.timeout(4000)});
  if (!response.ok) return null;
  const key = await crypto.subtle.importKey("raw",new TextEncoder().encode(pair.key),"AES-GCM",false,["decrypt"]);
  for (const row of await response.json()) {
    try {
      const bytes=Uint8Array.from(atob(row.body),ch=>ch.charCodeAt(0));
      const clear=await crypto.subtle.decrypt({name:"AES-GCM",iv:bytes.slice(0,12)},key,bytes.slice(12));
      const result=extractSuggestion(JSON.parse(new TextDecoder().decode(clear)),site.hostname,row.archived_at);
      if(result) return result;
    } catch { /* Other archive keys may occur in the same inbox. */ }
  }
  return null;
}
api.runtime.onMessage.addListener((msg,sender,respond)=>{
  if(msg?.type !== "psms:suggest") return false;
  suggest(sender).then(respond,()=>respond(null));
  return true;
});
