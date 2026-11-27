(() => {
  if (window.top !== window || globalThis.psmsSuggestionInstalled) return;
  globalThis.psmsSuggestionInstalled=true;
  const api=globalThis.browser ?? chrome;
  let field, host, timer, retry, stream, expiresTimer, generation=0;
  function clear(){
    generation++; clearInterval(timer); clearTimeout(retry); clearTimeout(expiresTimer); stream?.disconnect(); stream=null;
    host?.remove(); host=null; field=null;
  }
  const TEXTLIKE=["text","number","tel","password"];
  // One box per digit: the 4-8 single-character boxes around this one.
  function boxes(el){
    if(!(el instanceof HTMLInputElement) || el.maxLength!==1 || !TEXTLIKE.includes(el.type)) return null;
    let scope=el.parentElement;
    for(let up=0; scope && up<4; up++, scope=scope.parentElement){
      const group=[...scope.querySelectorAll("input")].filter(i=>i.maxLength===1 && TEXTLIKE.includes(i.type) && !i.disabled);
      if(group.length>=4) return group.length<=8 ? group : null;
    }
    return null;
  }
  // A box asking for a texted code: marked as one (autocomplete="one-time-code"), labelled like
  // one, a short numeric box, or a row of one-digit boxes. Promo, gift and postal codes are not.
  function eligible(el){
    if (!(el instanceof HTMLInputElement) || el.disabled || el.readOnly || el.value || !TEXTLIKE.includes(el.type)) return false;
    if (el.autocomplete.split(/\s+/).includes("one-time-code")) return true;
    const label=[el.name,el.id,el.placeholder,el.getAttribute("aria-label"),el.getAttribute("aria-labelledby")&&document.getElementById(el.getAttribute("aria-labelledby"))?.textContent,...[...(el.labels??[])].map(l=>l.textContent)].join(" ");
    if (/promo|coupon|discount|gift|voucher|referral|invite|zip|postal|postcode|country|area|captcha|search|card|cvv|cvc/i.test(label)) return false;
    if (boxes(el)) return true;
    if (/\botp\b|one[-_ ]?time|verif\w*|security[-_ ]?code|auth\w*[-_ ]?code|passcode|\b2fa\b|\bmfa\b|\bpin\b|sms[-_ ]?code|\bcode\b/i.test(label)) return true;
    return el.inputMode==="numeric" && el.maxLength>=4 && el.maxLength<=8;
  }
  function position(){
    if(!host || !field) return;
    const r=field.getBoundingClientRect();
    if(!r.width || !r.height || r.bottom<0 || r.top>innerHeight){clear();return}
    host.style.setProperty("left",Math.max(4,Math.min(r.left,innerWidth-178))+"px","important");
    host.style.setProperty("top",(r.bottom+54<=innerHeight?r.bottom+5:Math.max(4,r.top-51))+"px","important");
  }
  function display(el,data){
    host?.remove(); clearTimeout(expiresTimer);
    host=document.createElement("span");
    host.style.cssText="all:initial!important;position:fixed!important;z-index:2147483647!important";
    const root=host.attachShadow({mode:"closed"});
    const button=document.createElement("button");
    button.type="button";
    button.style.cssText="display:grid;gap:2px;min-width:144px;max-width:250px;padding:6px 14px;border:1px solid #c4c5cb;border-radius:9px;background:#f7f6fb;color:#202126;box-shadow:0 3px 12px #1414222b;font:500 12px system-ui;cursor:pointer";
    if(matchMedia("(prefers-color-scheme: dark)").matches){button.style.background="#34363d";button.style.color="#f5f5f7";button.style.borderColor="#555861"}
    const caption=document.createElement("span");
    caption.textContent=(data.verified?"Autofill from ":"Code from ")+data.sender+(data.verified?"":" · not site-checked");
    const code=document.createElement("strong");
    code.textContent=data.code.length===6?data.code.slice(0,3)+" "+data.code.slice(3):data.code;
    code.style.cssText="font:600 17px system-ui;font-variant-numeric:tabular-nums";
    button.append(caption,code);
    button.onpointerdown=e=>e.preventDefault();
    button.onclick=e=>{
      if(!e.isTrusted || Date.now()>=data.expires || !el.isConnected || !eligible(el) || document.visibilityState!=="visible"){clear();return}
      const set=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value").set;
      const group=boxes(el);
      const targets=group && group.length===data.code.length ? group.map((box,i)=>[box,data.code[i]]) : [[el,data.code]];
      for(const [box,value] of targets){
        set.call(box,value);
        box.dispatchEvent(new Event("input",{bubbles:true}));box.dispatchEvent(new Event("change",{bubbles:true}));
      }
      clear();
    };
    root.append(button);document.documentElement.append(host);position();
    expiresTimer=setTimeout(clear,Math.max(0,data.expires-Date.now()));
  }
  function focus(el){
    if(el===host)return;
    clear();
    if(!eligible(el)||document.visibilityState!=="visible")return;
    field=el; const current=generation;
    let fetching=false, dirty=false;
    async function check(){
      if (current !== generation) return;
      if (fetching) { dirty=true; return; }
      if(!el.isConnected || !eligible(el) || document.visibilityState!=="visible"){if(current===generation)clear();return}
      try{
        fetching=true;
        const data=await api.runtime.sendMessage({type:"psms:suggest"});
        if(current!==generation)return;
        if(data && data.expires>Date.now())display(el,data);else{host?.remove();host=null}
      }catch{if(current===generation)clear()} finally {fetching=false; if(dirty && current===generation){dirty=false;void check()}}
    }
    function poll() {
      clearInterval(timer);
      timer=setInterval(check,4000);
    }
    function subscribe() {
      if(current!==generation)return;
      try {
        const port=api.runtime.connect({name:"psms:arrivals"});
        stream=port;
        port.onMessage.addListener(event=>{
          if(current!==generation)return;
          if(event.type==="ready")clearInterval(timer);
          if(event.type==="ready" || event.type==="arrival")void check();
        });
        port.onDisconnect.addListener(()=>{
          void api.runtime.lastError;
          if(current!==generation)return;
          stream=null; poll(); void check();
          retry=setTimeout(subscribe,60000);
        });
      } catch { if(current===generation)retry=setTimeout(subscribe,60000); }
    }
    void check(); poll(); subscribe();
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
