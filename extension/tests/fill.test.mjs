import test from "node:test";
import assert from "node:assert/strict";
import {fillLogin} from "../dist/fill.js";
import {createCoordinator} from "../dist/background.js";

function dom({passwords=1, usernames=1, newPassword=false, url="https://example.com/login", frame=false}={}) {
  class Input {
    constructor(type) { this.type=type; this.name=type === "password" ? "password" : "username"; this.id=""; this.autocomplete=""; this.disabled=false; this.readOnly=false; this.events=[]; this.stored=""; }
    get value() { return this.stored; } set value(v) {this.stored=v;}
    getClientRects() { return this.hidden ? [] : [{}]; }
    getBoundingClientRect() {return {width:200,height:32};}
    closest() { return null; }
    dispatchEvent(event) {this.events.push(event.type);}
  }
  const users=Array.from({length:usernames},()=>new Input("text"));
  const secrets=Array.from({length:passwords},()=>new Input("password"));
  const form={elements:[...users,...secrets]};
  for(const input of form.elements) input.form=form;
  if(newPassword && secrets[0]) secrets[0].autocomplete="section-signup new-password";
  globalThis.HTMLInputElement=Input;
  globalThis.window={}; window.top=frame ? {} : window;
  globalThis.location=new URL(url);
  globalThis.document={querySelectorAll:()=>secrets};
  globalThis.getComputedStyle=()=>({visibility:"visible",display:"block",opacity:"1"});
  return {users,secrets};
}

test("fills one unambiguous login with native setters and input/change, without submitting",()=>{
  const {users,secrets}=dom();
  assert.equal(fillLogin(location.href,location.origin,"alice","secret"),"filled");
  assert.equal(users[0].value,"alice"); assert.equal(secrets[0].value,"secret");
  assert.deepEqual(users[0].events,["input","change"]); assert.deepEqual(secrets[0].events,["input","change"]);
});
test("rejects changed URL, iframes, signup and ambiguous fields without writing",()=>{
  for(const options of [{passwords:2},{usernames:2},{newPassword:true},{frame:true}]) {
    const {users,secrets}=dom(options);
    assert.notEqual(fillLogin("https://example.com/login","https://example.com","alice","secret"),"filled");
    for(const field of [...users,...secrets]) assert.equal(field.value,"");
  }
  dom(); assert.equal(fillLogin("https://example.com/other",location.origin,"alice","secret"),"page_changed");
});
test("ignores hidden, readonly or disabled password controls and refuses unowned fields",()=>{
  for(const property of ["hidden","disabled","readOnly"]) {
    const {secrets}=dom(); secrets[0][property]=true;
    assert.equal(fillLogin(location.href,location.origin,"a","s"),"ambiguous_form");
  }
  const {secrets}=dom(); secrets[0].form=null;
  assert.equal(fillLogin(location.href,location.origin,"a","s"),"unsupported_form");
});

function fixture({pollStatus="ok", changed=false, badCredential=false, beginCredential=false}={}) {
  let tab={id:3,windowId:4,url:"https://example.com/login"};
  const calls=[],injections=[],badges=[];
  let resolveDone;
  const done=new Promise(r=>resolveDone=r);
  const api={
    tabs:{query:async()=>[tab],get:async()=>tab,update:async()=>{},onUpdated:{},onRemoved:{}}, windows:{update:async()=>{}},
    action:{setBadgeText:async value=>{badges.push(value.text); if(value.text!=="…") resolveDone();},setTitle:async()=>{}},
    runtime:{sendNativeMessage:async(_host,message)=>{
      calls.push(message);
      if(message.action==="begin_fill") return {version:1,status:beginCredential?"ok":"pending",matches:[],credential:{username:"do not fill",password:"do not fill"}};
      if(message.action==="poll_fill") return {version:1,status:pollStatus,matches:[],credential:badCredential?{}:{username:"alice",password:"secret"}};
      resolveDone(); return {version:1,status:"cancelled",matches:[]};
    }},
    scripting:{executeScript:async options=>{
      if(options.target.frameIds) return [{frameId:0,documentId:"initial-document",result:true}];
      injections.push(options); return [{frameId:0,documentId:"initial-document",result:"filled"}];
    }},
  };
  const coordinator=createCoordinator(api,async()=>{if(changed) {tab={...tab,url:"https://example.com/changed"}; coordinator.invalidate(3);}});
  return {coordinator,calls,injections,badges,done};
}
const message={action:"fill_account",id:"entry-id",tabId:3,url:"https://example.com/login"};
test("background polls approval after popup handoff and injects only the original document",async()=>{
  const f=fixture(); assert.deepEqual(await f.coordinator.start(message),{ok:true}); await f.done;
  assert.equal(f.injections.length,1);
  assert.deepEqual(f.injections[0].target,{tabId:3,documentIds:["initial-document"]});
  assert.deepEqual(f.injections[0].args,[message.url,"https://example.com","alice","secret"]);
  assert.match(f.calls[0].request_id,/^[a-f0-9]{32}$/);
  assert.equal(f.calls[1].action,"poll_fill");
});
test("navigation revokes pending fill; cancellation and malformed credentials never inject",async()=>{
  for(const options of [{changed:true},{pollStatus:"cancelled"},{badCredential:true},{beginCredential:true}]) {
    const f=fixture(options); await f.coordinator.start(message); await f.done;
    assert.equal(f.injections.length,0);
  }
});
test("background rejects page-supplied origin and changed or non-HTTPS active tabs",async()=>{
  const f=fixture();
  for(const bad of [{...message,tabId:7},{...message,url:"https://phishing.example/login"},{...message,id:""}])
    assert.deepEqual(await f.coordinator.start(bad),{ok:false});
  assert.equal(f.calls.length,0);
});
