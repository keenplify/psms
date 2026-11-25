(() => {
  if (window.top !== window || globalThis.psmsSuggestionInstalled) return;
  globalThis.psmsSuggestionInstalled=true;
  const api=globalThis.browser ?? chrome;
  let field, host, timer, expiresTimer, generation=0;
  function clear(){
    generation++; clearInterval(timer); clearTimeout(expiresTimer);
    host?.remove(); host=null; field=null;
  }
  function eligible(el){
    if (!(el instanceof HTMLInputElement) || el.disabled || el.readOnly || el.value || !["text","number","tel","password"].includes(el.type)) return false;
    const label=[el.name,el.id,el.placeholder,el.getAttribute("aria-label"),...[...(el.labels??[])].map(l=>l.textContent)].join(" ");
    return el.autocomplete.split(/\s+/).includes("one-time-code") || /\botp\b|one[-_ ]?time[-_ ]?(code|password)|verification[-_ ]?code|security[-_ ]?code/i.test(label);
  }
  function position(){
    if(!host || !field) return;
    const r=field.getBoundingClientRect();
    if(!r.width || !r.height || r.bottom<0 || r.top>innerHeight){clear();return}
    host.style.left=Math.max(4,Math.min(r.left,innerWidth-178))+"px";
    host.style.top=(r.bottom+54<=innerHeight?r.bottom+5:Math.max(4,r.top-51))+"px";
  }
  function display(el,data){
    host?.remove(); clearTimeout(expiresTimer);
    host=document.createElement("span");
    host.style.cssText="all:initial!important;position:fixed!important;z-index:2147483647!important";
    const root=host.attachShadow({mode:"closed"});
    const button=document.createElement("button");
    button.type="button";
    button.style.cssText="display:grid;gap:2px;min-width:144px;max-width:250px;padding:6px 14px;border:1px solid #c4c5cb;border-radius:9px;background:#f7f6fb;color:#202126;box-shadow:0 3px 12px #1414222b;font:500 12px system-ui;cursor:pointer";
    const caption=document.createElement("span");
    caption.textContent="Autofill from "+data.sender;
    const code=document.createElement("strong");
    code.textContent=data.code.length===6?data.code.slice(0,3)+" "+data.code.slice(3):data.code;
    code.style.cssText="font:600 17px system-ui;font-variant-numeric:tabular-nums";
    button.append(caption,code);
    button.onpointerdown=e=>e.preventDefault();
    button.onclick=e=>{
      if(!e.isTrusted || Date.now()>=data.expires || !el.isConnected || !eligible(el) || document.visibilityState!=="visible"){clear();return}
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value").set.call(el,data.code);
      el.dispatchEvent(new Event("input",{bubbles:true}));el.dispatchEvent(new Event("change",{bubbles:true}));clear();
    };
    root.append(button);document.documentElement.append(host);position();
    expiresTimer=setTimeout(clear,Math.max(0,data.expires-Date.now()));
  }
  function focus(el){
    if(el===host)return;
    clear();
    if(!eligible(el)||document.visibilityState!=="visible")return;
    field=el; const current=generation;
    async function check(){
      if(!el.isConnected || !eligible(el) || document.visibilityState!=="visible"){if(current===generation)clear();return}
      try{
        const data=await api.runtime.sendMessage({type:"psms:suggest"});
        if(current!==generation)return;
        if(data && data.expires>Date.now())display(el,data);else{host?.remove();host=null}
      }catch{if(current===generation)clear()}
    }
    check(); timer=setInterval(check,4000);
  }
  document.addEventListener("focusin",e=>focus(e.target),true);
  document.addEventListener("focusout",()=>setTimeout(()=>{if(field && document.activeElement!==field && document.activeElement!==host)clear()},0),true);
  document.addEventListener("visibilitychange",()=>focus(document.activeElement));
  document.addEventListener("input",e=>{if(e.target===field && field.value)clear()},true);
  document.addEventListener("keydown",e=>{if(e.key==="Escape")clear()},true);
  window.addEventListener("pagehide",clear);
  window.addEventListener("resize",position);
  window.addEventListener("scroll",position,true);
  focus(document.activeElement);
})();
