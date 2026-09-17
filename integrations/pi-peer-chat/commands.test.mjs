import test from 'node:test';
import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';

// Only the visual loader is substituted. Run the real command handlers,
// real name resolver, real Unix collector and real request persistence.
registerHooks({ resolve(specifier, context, next) {
 if (specifier === 'typebox') return {url:'data:text/javascript,'+encodeURIComponent('export const Type = {Object: x=>x, Array: x=>x, String: x=>x, Integer: x=>x, Optional: x=>x};'),shortCircuit:true};
 if (specifier === '@earendil-works/pi-coding-agent') return {url:'data:text/javascript,'+encodeURIComponent('export class BorderedLoader { constructor() { this.signal = new AbortController().signal; } }'),shortCircuit:true};
 return next(specifier,context);
}});
const {default:extension}=await import('./index.ts');
const A='01a0ac41-48a9-73b0-8c0c-ba9f09503601',B='01a0ac41-368a-7276-bcb2-446ddca0aded';
function socketReply(socket,data){return new Promise((resolve,reject)=>{const c=net.createConnection(socket);c.on('error',reject);c.on('connect',()=>c.write(JSON.stringify(data)+'\n'));c.on('data',d=>{c.destroy();resolve(JSON.parse(d));});});}
function harness(t,{mode='demand',cancel=false,noReply=false}={}) {
 const commands={},tools={},events={},messages=[],notices=[],sent=[],paths=[];let composeTitle;
 const agents=[{agent:'pi',agent_status:'idle',pane_id:'test:p1',tokens:{name:'Aporia'},agent_session:{kind:'path',value:`/tmp/date_${A}.jsonl`}},{agent:'pi',agent_status:'idle',pane_id:'test:p2',tokens:{name:'Phronesis'},agent_session:{kind:'path',value:`/tmp/date_${B}.jsonl`}}];
 const pi={on:(n,f)=>events[n]=f,registerCommand:(n,c)=>commands[n]=c,registerTool:tool=>tools[tool.name]=tool,sendMessage:m=>messages.push(m),exec:async(cmd,args)=>{
  assert.equal(cmd,'herdr');assert.ok(!args.includes('--wait'));
  if(args[1]==='list')return {code:0,stdout:JSON.stringify({result:{agents}}),stderr:''};
  assert.equal(args[1],'prompt');sent.push(args);
  const match=args[3].match(/node '[^']+reply\.mjs' '([^']+request\.json)'/);assert.ok(match,args[3]);
  paths.push(match[1]);const req=JSON.parse(fs.readFileSync(match[1],'utf8'));
  if(!noReply && !cancel)setTimeout(()=>socketReply(req.socket,{id:req.id,session:B,text:'TEST RESPONSE'}).catch(()=>{}),10);
  return {code:0,stdout:JSON.stringify({result:{type:'agent_prompted'}}),stderr:''};
 }};
 const ctx={mode:'tui',sessionManager:{getSessionId:()=>A},ui:{editor:async title=>{composeTitle=title;return 'Test message';},confirm:async()=>true,notify:(...n)=>notices.push(n),custom:async fn=>new Promise(resolve=>{const loader=fn({}, {}, {},resolve);if(cancel)setTimeout(()=>loader.onAbort(),20);})}};
 process.env.HERDR_ENV='1';process.env.HERDR_PANE_ID='test:p1';extension(pi);
 t.after(()=>{events.session_shutdown();for(const p of paths)fs.rmSync(path.dirname(p),{recursive:true,force:true});});
 return {commands,tools,ctx,events,messages,notices,sent,paths,get title(){return composeTitle;}};
}
test('demand prompts, dispatches without --wait, collects specific reply, finishes',async t=>{
 const h=harness(t);await h.commands.demand.handler('Phronesis --timeout 2',h.ctx);
 assert.match(h.title,/Phronesis/);assert.equal(h.sent.length,1);
 assert.ok(h.messages.some(m=>m.customType==='peer-chat-reply'&&m.content.includes('TEST RESPONSE')));
 assert.equal(JSON.parse(fs.readFileSync(h.paths[0])).status,'completed');
});
test('talk returns before any reply and leaves an optional response collector',async t=>{
 const h=harness(t,{mode:'talk',noReply:true});await h.commands.talk.handler('all --timeout 2',h.ctx);
 assert.equal(h.sent.length,1);assert.equal(JSON.parse(fs.readFileSync(h.paths[0])).status,'open');
 h.events.session_shutdown();assert.equal(JSON.parse(fs.readFileSync(h.paths[0])).status,'session-closed');
});
test('demand Escape cancels and retains request record',async t=>{
 const h=harness(t,{cancel:true});await h.commands.demand.handler('Phronesis --timeout 2',h.ctx);
 assert.equal(JSON.parse(fs.readFileSync(h.paths[0])).status,'cancelled');
});
test('agent talk derives sender, skips human editor, and returns without a reply',async t=>{
 const h=harness(t,{noReply:true});
 h.ctx.ui.editor=()=>{throw Error('Must not prompt human');};
 const result=await h.tools.talk.execute('t1',{recipients:['Phronesis'],message:'Agent question',timeout_seconds:2},undefined,undefined,h.ctx);
 assert.equal(result.details.status,'open');assert.equal(h.sent.length,1);
 assert.match(h.sent[0][3],/^\[BEGIN \| from: Aporia \| to: Phronesis/);
 assert.ok(!h.sent[0][3].includes('Angus via Aporia'));
 const saved=JSON.parse(fs.readFileSync(h.paths[0]));assert.equal(saved.origin,'agent');
});
test('agent demand returns response text without using a waiting dialog',async t=>{
 const h=harness(t);h.ctx.ui.custom=()=>{throw Error('Tool must not open modal');};
 const progress=[];
 const result=await h.tools.demand.execute('d1',{recipients:['Phronesis'],message:'Agent question',timeout_seconds:2},undefined,x=>progress.push(x),h.ctx);
 assert.equal(result.details.status,'completed');assert.match(result.content[0].text,/TEST RESPONSE/);
 assert.match(result.content[0].text,/from: Phronesis \| to: Aporia/);assert.ok(progress.length);
});
test('agent demand abort signal stops waiting, preserves receipt state',async t=>{
 const h=harness(t,{noReply:true});const controller=new AbortController();
 const pending=h.tools.demand.execute('d2',{recipients:['Phronesis'],message:'Agent question',timeout_seconds:2},controller.signal,undefined,h.ctx);
 const timer=setTimeout(()=>controller.abort(),30);t.after(()=>clearTimeout(timer));
 const result=await pending;assert.equal(result.details.status,'cancelled');
});
test('agent tool errors are propagated without sends; all cannot mix with names',async t=>{
 const h=harness(t);
 await assert.rejects(h.tools.talk.execute('t2',{recipients:['all','Phronesis'],message:'x'},undefined,undefined,h.ctx),/all alone/);
 await assert.rejects(h.tools.talk.execute('t3',{recipients:['Phronesis'],message:'x',timeout_seconds:0},undefined,undefined,h.ctx),/Timeout/);
 assert.equal(h.sent.length,0);
});
