import test from "node:test";
import assert from "node:assert/strict";
import {queueFill} from "../dist/handoff.js";

function port() {
  let receive,closed;
  const value={messages:[],disconnections:0,
    onMessage:{addListener:callback=>{receive=callback;}},
    onDisconnect:{addListener:callback=>{closed=callback;}},
    postMessage:message=>value.messages.push(message),
    disconnect:()=>{value.disconnections++;closed();}};
  return {value,reply:message=>receive(message)};
}
test("accepted selection leaves the port alive until the popup actually closes",async()=>{
  const p=port(); const request={action:"fill_account",id:"synthetic"};
  const accepted=queueFill(()=>p.value,request);
  assert.deepEqual(p.value.messages,[request]);
  p.reply({ok:true}); assert.equal(await accepted,true);
  assert.equal(p.value.disconnections,0);
  p.value.disconnect();
});
test("rejected or malformed acknowledgements do not accept a fill",async()=>{
  for(const reply of [{ok:false},{ok:"true"},null,{}]) {
    const p=port(); const accepted=queueFill(()=>p.value,{});
    p.reply(reply); assert.equal(await accepted,false);
    assert.equal(p.value.disconnections,1);
  }
});
test("worker disconnection or send failure rejects the handoff",async()=>{
  const p=port(); const accepted=queueFill(()=>p.value,{});
  p.value.disconnect(); await assert.rejects(accepted,/connection closed/);
  const broken=port();broken.value.postMessage=()=>{throw Error("synthetic send failure");};
  await assert.rejects(queueFill(()=>broken.value,{}),/synthetic send failure/);
  assert.equal(broken.value.disconnections,1);
});
