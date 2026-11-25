const api=globalThis.browser??chrome;
const el=id=>document.getElementById(id);
const local="http://127.0.0.1:47832";
function status(text){el("status").textContent=text}
let currentTab;
let enabledSites=[];
async function detect(){
  const {server,sites=[]}=await api.storage.local.get(["server","sites"]);
  el("server").value=server??(local+"/v1");
  const [tab]=await api.tabs.query({active:true,currentWindow:true});
  currentTab=tab; enabledSites=sites;
  if(tab?.url && sites.includes(new URL(tab.url).origin))el("enable").textContent="Disable on this site";
  if((await api.storage.session.get("pair")).pair){status("Connected for this browser session.");return}
  try{
    const r=await fetch(local+"/.well-known/psms",{redirect:"error",signal:AbortSignal.timeout(1200)});
    if((await r.json()).service==="psms"){status("Local pSMS detected. Get a Browser autofill token in Settings.");return}
  }catch{}
  status("Enter the address shown by pSMS and unlock this session.");
}
el("connect").onclick=async()=>{
  try{
    const u=new URL(el("server").value);
    if(u.username||u.password||u.search||u.hash||!(u.protocol==="https:" || (u.protocol==="http:" && u.hostname==="127.0.0.1")))throw Error();
    const key=el("key").value,token=el("token").value.trim();
    if(key.length!==32 || !/^[a-f\d]{64}$/i.test(token)){status("Enter your 32-character recovery key and Browser autofill token.");return}
    if(!await api.permissions.request({origins:[u.protocol+"//"+u.hostname+"/*"]}))return;
    const server=u.href.replace(/\/+$/,"");
    const r=await fetch(server+"/otp/recent",{headers:{Authorization:"Bearer "+token},redirect:"error",credentials:"omit",cache:"no-store",signal:AbortSignal.timeout(5000)});
    if(!r.ok){status("Connection denied. Check the server and the Browser autofill token.");return}
    await api.storage.local.set({server});await api.storage.session.set({pair:{key,token,server}});
    el("key").value=el("token").value="";status("Connected. Enable autofill on the sign-in site.");
  }catch{status("Could not connect. Check the server address and try again.")}
};
el("lock").onclick=async()=>{await api.storage.session.remove("pair");status("Locked.")};
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
el("server").disabled=el("connect").disabled=el("enable").disabled=true;
detect().catch(()=>status("Open Settings in pSMS to connect.")).finally(()=>{
  el("server").disabled=el("connect").disabled=el("enable").disabled=false;
});
