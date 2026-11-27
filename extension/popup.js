const api=globalThis.browser??chrome;
const el=id=>document.getElementById(id);
const local="http://127.0.0.1:47832";
function status(text){el("status").textContent=text}
const hosted="https://na.sms.papamo.dev/free/v1";
let customServer=local+"/v1";
let custom=false;
// Two states: connected (turn sites off, disconnect), or not (one Sign in button). A self-hosted
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
let blockedSites=[];
let everywhere=false;
const tabOrigin=()=>currentTab?.url?.startsWith("https://")?new URL(currentTab.url).origin:"";
// Autofill is on for every site unless you turn it off here.
function showSite(){
  const origin=tabOrigin();
  const on=everywhere && origin && !blockedSites.includes(origin);
  el("enable").hidden=everywhere && !origin;
  el("enable").textContent=!everywhere?"Turn on autofill":on?"Turn off on this site":"Turn on for this site";
}
async function detect(){
  const {server,blockedSites:blocked=[]}=await api.storage.local.get(["server","blockedSites"]);
  blockedSites=blocked;
  if(server && server!==hosted){customServer=server;custom=true}
  const [tab]=await api.tabs.query({active:true,currentWindow:true});
  currentTab=tab;
  // Firefox asks for site access separately from installing.
  everywhere=await api.permissions.contains({origins:["https://*/*"]});
  showSite();
  const {pair}=await api.storage.session.get("pair");
  show(pair);
  if(pair){status(everywhere?"Codes appear under verification code fields.":"Turn on autofill to fill codes on sign-in pages.");return}
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
    el("key").value=el("token").value="";show({server});status("Connected. Codes appear under verification code fields.");
  }catch{status("Could not connect. Check the server address and try again.")}
};
el("lock").onclick=async()=>{await api.storage.session.remove(["pair","signin"]);show(null);status("Disconnected. Sign in again to use autofill.")};
el("enable").onclick=async()=>{
  try{
    if(!everywhere){
      if(!await api.permissions.request({origins:["https://*/*"]}))return;
      everywhere=true;showSite();status("On. Codes appear under verification code fields.");return
    }
    const origin=tabOrigin();
    if(!origin)return;
    const off=!blockedSites.includes(origin);
    blockedSites=blockedSites.filter(s=>s!==origin);
    if(off)blockedSites.push(origin);
    await api.storage.local.set({blockedSites});
    showSite();
    status(off?"Off on this site.":"On for this site again.");
  }catch{status("Could not change this site. Try again.")}
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
  if(changes.pair?.newValue){show(changes.pair.newValue);status("Connected. Codes appear under verification code fields.")}
  const ask=changes.localAsk?.newValue;
  if(ask?.state==="denied")status("pSMS didn't allow it. Choose Connect again to retry.");
  if(ask?.state==="expired")status("No answer from pSMS. Choose Connect again.");
});
