import test from 'node:test';
import assert from 'node:assert/strict';
import { contextualRoomTransport, boundedMessages, visibleRoomQuestion, CONTEXT_BYTES } from './pi/room-context.mjs';
const entry = (sequence, text, name = null) => ({sequence, text, author:name ? {name,pane_id:'w1:p2'} : null,reply_to:sequence === 2 ? 1 : null});
function fixture(messages) {
  const sent=[], requests=[];
  const d={delivery_id:'d',workspace_id:'w1',request_sequence:4,text:'What do you think of their choice?'};
  const transport=contextualRoomTransport({sendMessage:(...a)=>sent.push(a)}, {call:async(s,m,p,o)=>{
    requests.push([s,m,p,o]);
    if(m==='room.delivery.register')return {receiver_id:'r',server_epoch:'e'};
    if(m==='room.delivery.claim')return {delivery:d};
    if(m==='room.read')return {room_id:'room:w1',next_sequence:7,messages};
    return {};
  }});
  const message={customType:'room-question',content:'CURRENT QUESTION',display:true,details:{delivery_id:'d',request_sequence:4,workspace_id:'w1'}};
  return {transport,sent,requests,message};
}
test('new human delivery includes attributed prior replies but not later/current messages',async()=>{
  const f=fixture([entry(1,'Your favourite?'),entry(2,'Noether','Aporia'),entry(3,'Euler','Phronesis'),entry(4,'current'),entry(5,'later')]);
  await f.transport.call('/explicit.sock','room.delivery.register',{workspace_id:'w1'});
  assert.equal(f.requests.length,1,'joining never reads history or wakes agents');
  await f.transport.call('/explicit.sock','room.delivery.claim',{});
  f.transport.sendMessage(f.message,{triggerTurn:true,deliverAs:'followUp'});
  const [m,options]=f.sent[0];
  assert.match(m.content, /Noether/);assert.match(m.content,/Phronesis/);assert.match(m.content,/Euler/);
  assert.doesNotMatch(m.content,/"text":"later"|"text":"current"/);
  assert.equal(visibleRoomQuestion(m),'What do you think of their choice?');
  assert.deepEqual(options,{triggerTurn:true,deliverAs:'followUp'});
  assert.equal(f.requests.at(-1)[0],'/explicit.sock');
  assert.throws(()=>f.transport.sendMessage(f.message,{}),/context unavailable/);
});
test('arrival and unsolicited agent posts are attributed history, never fresh deliveries',async()=>{
  const f=fixture([{...entry(1,'Joined the room.','Ada'),arrival:true,reply_to:null},
    {...entry(2,'An unsolicited observation','Bob'),reply_to:null},entry(3,'prior human')]);
  await f.transport.call('/s','room.delivery.register',{workspace_id:'w1'});
  assert.equal(f.sent.length,0);
  await f.transport.call('/s','room.delivery.claim',{});
  f.transport.sendMessage(f.message,{triggerTurn:true,deliverAs:'followUp'});
  const body=f.sent[0][0].content;
  assert.match(body,/"speaker":"agent","name":"Ada"/);
  assert.match(body,/"speaker":"agent","name":"Bob"/);
  assert.match(body,/Joined the room\./);assert.match(body,/An unsolicited observation/);
  assert.equal(f.sent.length,1);
});
test('different claim times produce identical historical snapshot',async()=>{
  const shared=[entry(1,'question'),entry(2,'answer','Ada'),entry(3,'answer','Bob')];
  const one=fixture(shared),two=fixture([...shared,entry(4,'current'),entry(5,'new reply','Ada')]);
  for(const f of [one,two]){await f.transport.call('/s','room.delivery.register',{workspace_id:'w1'});await f.transport.call('/s','room.delivery.claim',{});f.transport.sendMessage(f.message,{});}
  assert.equal(one.sent[0][0].content,two.sent[0][0].content);
});
test('byte bound keeps contiguous suffix and rejects malformed attribution/order',()=>{
  const messages=Array.from({length:40},(_,i)=>entry(i+1,'界'.repeat(2000),'Ada'));
  const selected=boundedMessages(messages,{newest:true});
  assert.ok(Buffer.byteLength(selected.map(m=>JSON.stringify(m)).join('\n'))<=CONTEXT_BYTES);
  assert.equal(selected.at(-1).sequence,40);assert.ok(selected[0].sequence>1);
  assert.throws(()=>boundedMessages([entry(2,'a'),entry(1,'b')]),/out of order/);
  assert.throws(()=>boundedMessages([{...entry(1,'a'),author:{name:'x'}}]),/Invalid room author/);
});
test('history failure prevents context-free model invocation',async()=>{
  const sent=[]; const t=contextualRoomTransport({sendMessage:m=>sent.push(m)},{call:async(s,m)=>{
    if(m==='room.delivery.register')return {};
    if(m==='room.delivery.claim')return {delivery:{delivery_id:'d',workspace_id:'w1',request_sequence:1,text:'q'}};
    throw new Error('read failed');
  }});
  await t.call('/s','room.delivery.register',{workspace_id:'w1'});await t.call('/s','room.delivery.claim',{});
  assert.throws(()=>t.sendMessage({customType:'room-question',details:{delivery_id:'d',request_sequence:1,workspace_id:'w1'}},{}),/context unavailable/);
  assert.equal(sent.length,0);
});
test('visual renderer extracts only question from new and historical envelopes',()=>{
  const q='hello\n"quoted"';
  const content=`BEGIN ROOM QUESTION {"secret":"routing"}\n${JSON.stringify(q)}\nEND ROOM QUESTION {}\ninternal instructions`;
  assert.equal(visibleRoomQuestion({content}),q);
  assert.equal(visibleRoomQuestion({content:'arbitrary protocol dump'}),'Room question');
  assert.equal(visibleRoomQuestion({content,details:{room_question_text:'actual\x1bquestion'}}),'actualquestion');
});
