const api=globalThis.browser??chrome;
const el=id=>document.getElementById(id);
const local="http://127.0.0.1:47832";
function status(text){el("status").textContent=text}
const hosted="https://na.sms.papamo.dev/free/v1";
let customServer=local+"/v1";
let custom=false;
// Two states: connected (enable sites, disconnect), or not (one Sign in button). A self-hosted
// archive is a link away.
function show(pair){
  const connected=!!pair;
  el("signed-out").hidden=connected||custom;
  el("custom").hidden=connected||!custom;
  el("signed-in").hidden=!connected;
  el("site").hidden=!connected;
  el("lock").hidden=!connected;
  // Back from the self-hosted form to the two choices.
  el("mode").hidden=connected||!custom;
  if(connected)el("who").textContent=pair.username||(pair.server===hosted?"your Papamo account":new URL(pair.server).host);
  el("server").value=customServer;
}
el("mode").onclick=()=>{custom=false;status("");show(null)};
el("self").onclick=()=>{custom=true;status("Enter the address and Browser autofill token from pSMS Settings, and your recovery words.");show(null)};
let currentTab;
let enabledSites=[];
let unboundSites=[];
function showUnbound(){
  const origin=currentTab?.url?new URL(currentTab.url).origin:"";
  el("unbound-label").hidden=!enabledSites.includes(origin);
  el("unbound").checked=unboundSites.includes(origin);
}
el("unbound").onchange=async()=>{
  const origin=new URL(currentTab.url).origin;
  unboundSites=unboundSites.filter(s=>s!==origin);
  if(el("unbound").checked)unboundSites.push(origin);
  await api.storage.local.set({unboundSites});
  status(el("unbound").checked?"Untagged codes will be suggested on this site.":"Only codes tagged for this site will be suggested.");
};
async function detect(){
  const {server,sites=[],unboundSites:loose=[]}=await api.storage.local.get(["server","sites","unboundSites"]);
  unboundSites=loose;
  if(server && server!==hosted){customServer=server;custom=true}
  const [tab]=await api.tabs.query({active:true,currentWindow:true});
  currentTab=tab; enabledSites=sites;
  if(tab?.url && sites.includes(new URL(tab.url).origin))el("enable").textContent="Disable on this site";
  showUnbound();
  const {pair}=await api.storage.session.get("pair");
  show(pair);
  if(pair){status(tab?.url?.startsWith("https://")?"":"Open a sign-in page to enable autofill there.");return}
  // pSMS running on this computer: the quickest way in, approved in the app.
  try{
    const r=await fetch(local+"/.well-known/psms",{redirect:"error",signal:AbortSignal.timeout(1200)});
    if((await r.json()).service==="psms")el("here").hidden=false;
  }catch{}
  const {localAsk}=await api.storage.session.get("localAsk");
  status(localAsk?.state==="waiting"?"Waiting for you to click Allow in pSMS…":"");
}
el("account").onclick=async()=>{
  try {
    if(!await api.permissions.request({origins:["https://www.papamo.dev/*","https://na.sms.papamo.dev/*"]}))return;
    if(!await api.runtime.sendMessage({type:"psms:signin"}))throw Error();
    window.close();
  } catch { status("Could not start sign-in. Try again."); }
};
async function customKey(input) {
  const value=input.trim();
  if(value.length===32 && !/\s/.test(value))return value;
  const words=value.toLowerCase().split(/\s+/);
  if(words.length!==12)return null;
  const list=await (await fetch(api.runtime.getURL("english.json"))).json();
  const indexes=words.map(word=>list.indexOf(word));
  if(indexes.includes(-1))return null;
  const bits=indexes.map(i=>i.toString(2).padStart(11,"0")).join("");
  const entropy=Uint8Array.from(bits.slice(0,128).match(/.{8}/g),b=>parseInt(b,2));
  const digest=new Uint8Array(await crypto.subtle.digest("SHA-256",entropy));
  if(parseInt(bits.slice(128),2)!==(digest[0]>>4))return null;
  return Array.from(entropy,b=>b.toString(16).padStart(2,"0")).join("");
}
el("connect").onclick=async()=>{
  try{
    const u=new URL(el("server").value);
    if(u.username||u.password||u.search||u.hash||!(u.protocol==="https:" || (u.protocol==="http:" && u.hostname==="127.0.0.1")))throw Error();
    const key=await customKey(el("key").value),token=el("token").value.trim();
    if(!key || !/^[a-f\d]{64}$/i.test(token)){status("Enter your 12-word backup or original key and a browser autofill token.");return}
    if(!await api.permissions.request({origins:[u.protocol+"//"+u.hostname+"/*"]}))return;
    const server=u.href.replace(/\/+$/,"");
    const r=await fetch(server+"/otp/recent",{headers:{Authorization:"Bearer "+token},redirect:"error",credentials:"omit",cache:"no-store",signal:AbortSignal.timeout(5000)});
    if(!r.ok){status("Connection denied. Check the server and the Browser autofill token.");return}
    await api.storage.local.set({server});await api.storage.session.set({pair:{key,token,server}});
    el("key").value=el("token").value="";show({server});status("Connected. Enable autofill on the sign-in site.");
  }catch{status("Could not connect. Check the server address and try again.")}
};
el("lock").onclick=async()=>{await api.storage.session.remove(["pair","signin"]);show(null);status("Disconnected. Sign in again to use autofill.")};
el("enable").onclick=async()=>{
  try{
    const tab=currentTab;
    const site=new URL(tab.url), origin=site.origin, pattern=site.protocol+"//"+site.hostname+"/*";
    if(!origin.startsWith("https://")){status("Open an HTTPS sign-in page first.");return}
    const sites=enabledSites;
    const id="psms-"+Array.from(new TextEncoder().encode(origin),x=>x.toString(16).padStart(2,"0")).join("");
    if(sites.includes(origin)){
      enabledSites=sites.filter(s=>s!==origin);unboundSites=unboundSites.filter(s=>s!==origin);
      await api.storage.local.set({sites:enabledSites,unboundSites});showUnbound();
      await api.scripting.unregisterContentScripts({ids:[id]});el("enable").textContent="Enable on this site";status("Disabled on this site.");return
    }
    if(!await api.permissions.request({origins:[pattern]}))return;
    await api.scripting.registerContentScripts([{id,matches:[pattern],js:["field.js"],allFrames:false}]);
    enabledSites=[...sites,origin];await api.storage.local.set({sites:enabledSites});
    await api.scripting.executeScript({target:{tabId:tab.id},files:["field.js"]});
    el("enable").textContent="Disable on this site";showUnbound();status("Enabled. Focus a verification code field.");
  }catch{status("Could not enable autofill here. Open a normal HTTPS tab and try again.")}
};
el("mode").disabled=el("server").disabled=el("connect").disabled=el("enable").disabled=true;
detect().catch(()=>status("Open Settings in pSMS to connect.")).finally(()=>{
  el("mode").disabled=el("server").disabled=el("connect").disabled=el("enable").disabled=false;
});

el("local").onclick=async()=>{
  try{
    // The app may hand over a papamo.dev inbox, so ask for it now while there's a click.
    await api.permissions.request({origins:["https://na.sms.papamo.dev/*"]}).catch(()=>false);
    if(!await api.runtime.sendMessage({type:"psms:local-connect"}))throw Error();
    status("Click Allow in pSMS to finish.");
  }catch{status("Could not reach pSMS on this computer. Open it and try again.")}
};
// Connected (or refused) while the popup is open: show it straight away.
api.storage.onChanged.addListener(async(changes,area)=>{
  if(area!=="session")return;
  if(changes.pair?.newValue){show(changes.pair.newValue);status("Connected. Enable autofill on the sign-in site.")}
  const ask=changes.localAsk?.newValue;
  if(ask?.state==="denied")status("pSMS didn't allow it. Choose Connect again to retry.");
  if(ask?.state==="expired")status("No answer from pSMS. Choose Connect again.");
});
