const api=globalThis.browser??chrome;
const el=id=>document.getElementById(id);
const local="http://127.0.0.1:47832";
function status(text){el("status").textContent=text}
const hosted="https://na.sms.papamo.dev/free/v1";
let customServer=local+"/v1";
function showMode(mode){
  el("mode").value=mode;
  el("server-label").hidden=mode==="hosted";
  el("server-label").style.display=mode==="hosted"?"none":"grid";
  for(const item of document.querySelectorAll(".manual"))item.hidden=mode==="hosted";
  el("account").hidden=mode!=="hosted";
  el("server").value=mode==="hosted"?hosted:customServer;
}
el("mode").onchange=()=>{
  if(el("mode").value==="hosted")customServer=el("server").value;
  el("token").value=el("key").value="";
  showMode(el("mode").value);
  status(el("mode").value==="hosted"?"Use your Papamo account.":"Enter your server credentials.");
};
let currentTab;
let enabledSites=[];
async function detect(){
  const {server,sites=[]}=await api.storage.local.get(["server","sites"]);
  if(server && server!==hosted)customServer=server; showMode(server && server!==hosted?"custom":"hosted");
  const [tab]=await api.tabs.query({active:true,currentWindow:true});
  currentTab=tab; enabledSites=sites;
  if(tab?.url && sites.includes(new URL(tab.url).origin))el("enable").textContent="Disable on this site";
  const {pair}=await api.storage.session.get("pair");
  if(pair){el("account").textContent="Reconnect to Papamo";status(pair.username ? "Connected as "+pair.username+"." : "Connected for this browser session.");return}
  try{
    const r=await fetch(local+"/.well-known/psms",{redirect:"error",signal:AbortSignal.timeout(1200)});
    if((await r.json()).service==="psms" && !server){status("A local archive is also available under Local or custom server.");return}
  }catch{}
  status(el("mode").value==="hosted"?"Sign in to use your hosted inbox.":"Enter the address shown by pSMS.");
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
    const u=new URL(el("mode").value==="hosted"?hosted:el("server").value);
    if(u.username||u.password||u.search||u.hash||!(u.protocol==="https:" || (u.protocol==="http:" && u.hostname==="127.0.0.1")))throw Error();
    const key=await customKey(el("key").value),token=el("token").value.trim();
    if(!key || !/^[a-f\d]{64}$/i.test(token)){status("Enter your 12-word backup or original key and a browser autofill token.");return}
    if(!await api.permissions.request({origins:[u.protocol+"//"+u.hostname+"/*"]}))return;
    const server=u.href.replace(/\/+$/,"");
    const r=await fetch(server+"/otp/recent",{headers:{Authorization:"Bearer "+token},redirect:"error",credentials:"omit",cache:"no-store",signal:AbortSignal.timeout(5000)});
    if(!r.ok){status("Connection denied. Check the server and the Browser autofill token.");return}
    await api.storage.local.set({server});await api.storage.session.set({pair:{key,token,server}});
    el("key").value=el("token").value="";status("Connected. Enable autofill on the sign-in site.");
  }catch{status("Could not connect. Check the server address and try again.")}
};
el("lock").onclick=async()=>{await api.storage.session.remove(["pair","signin"]);status("Locked.")};
el("enable").onclick=async()=>{
  try{
    const tab=currentTab;
    const site=new URL(tab.url), origin=site.origin, pattern=site.protocol+"//"+site.hostname+"/*";
    if(!origin.startsWith("https://")){status("Open an HTTPS sign-in page first.");return}
    const sites=enabledSites;
    const id="psms-"+Array.from(new TextEncoder().encode(origin),x=>x.toString(16).padStart(2,"0")).join("");
    if(sites.includes(origin)){
      enabledSites=sites.filter(s=>s!==origin);await api.storage.local.set({sites:enabledSites});
      await api.scripting.unregisterContentScripts({ids:[id]});el("enable").textContent="Enable on this site";status("Disabled on this site.");return
    }
    if(!await api.permissions.request({origins:[pattern]}))return;
    await api.scripting.registerContentScripts([{id,matches:[pattern],js:["field.js"],allFrames:false}]);
    enabledSites=[...sites,origin];await api.storage.local.set({sites:enabledSites});
    await api.scripting.executeScript({target:{tabId:tab.id},files:["field.js"]});
    el("enable").textContent="Disable on this site";status("Enabled. Focus a verification code field.");
  }catch{status("Could not enable autofill here. Open a normal HTTPS tab and try again.")}
};
el("mode").disabled=el("server").disabled=el("connect").disabled=el("enable").disabled=true;
detect().catch(()=>status("Open Settings in pSMS to connect.")).finally(()=>{
  el("mode").disabled=el("server").disabled=el("connect").disabled=el("enable").disabled=false;
});
