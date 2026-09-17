import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { contactsFromMarkdown, liveContacts, parseArgs, resolveTargets, newRequest, envelope, frame, acceptReply } from './core.mjs';
import { startCollector } from './collector.mjs';
const exec = promisify(execFile);
const A='01a0ac41-48a9-73b0-8c0c-ba9f09503601', B='01a0ac41-368a-7276-bcb2-446ddca0aded', C='01a0a895-9e0b-7335-9861-98eb50fd272b';
const sender={name:'Aporia',session:A,pane:'wB:p2'};
const peers=[{name:'Phronesis',session:B,pane:'wB:p1'},{name:'Lattice',session:C,pane:'w9:p3'}];
const request=()=>newRequest('demand',sender,structuredClone(peers),'Hello',120);
const agent=(name,id,pane)=>({agent:'pi',agent_status:'idle',pane_id:pane,tokens:{name},agent_session:{kind:'path',value:`/tmp/date_${id}.jsonl`}});
function send(socket,data){return new Promise((resolve,reject)=>{const c=net.createConnection(socket);let b='';c.setEncoding('utf8');c.on('connect',()=>c.write(JSON.stringify(data)+'\n'));c.on('data',d=>{b+=d;if(b.includes('\n')){c.destroy();resolve(JSON.parse(b.split('\n')[0]));}});c.on('error',reject);});}
async function fixture(t,req=request(),options={}) {
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'peer-test-'));
 const c=await startCollector(req,root,options);
 t.after(()=>{c.close('test-cleanup');fs.rmSync(root,{recursive:true,force:true});});return c;
}
test('contact parsing strips decorations and checkmarks, keeps multi-word names',()=>{
 const d=contactsFromMarkdown(`## 🎵 Tuning Fork ✅\n- Session ID: \`${B}\`\n## Other heading\nNo id\n`);
 assert.deepEqual(d,[{name:'Tuning Fork',session:B}]);
});
test('live mapping uses session identity, not stale pane in markdown',()=>{
 const live=liveContacts([agent('◈ Phronesis',B,'new:p8'),agent('Aporia',A,'wB:p2')],[{name:'Phronesis',session:B}]);
 assert.equal(resolveTargets(['phronesis'],live,A)[0].pane,'new:p8');
 assert.equal(resolveTargets(['all'],live,A).length,1);
 assert.equal(resolveTargets(['Phronesis','Phronesis'],live,A).length,1);
 assert.throws(()=>resolveTargets(['Nobody'],live,A),/No live/);
 assert.throws(()=>resolveTargets(['Phronesis'],[...live,...live],A),/Ambiguous/);
});
test('arguments quotes, commas, timeout and all constraints',()=>{
 assert.deepEqual(parseArgs('Phronesis, "Tuning Fork" --timeout 45'),{names:['Phronesis','Tuning Fork'],timeout:45});
 for(const s of ['', 'all Phronesis','Phronesis all','all --timeout 0','all --timeout wat','all --bad','"unclosed']) assert.throws(()=>parseArgs(s));
});
test('envelope has matching identity boundaries and required reply instructions',()=>{
 const r=request(), text=envelope(r,peers[0],'/tmp/request.json','/tmp/reply.mjs');
 const lines=text.split('\n');assert.equal(lines[0].replace('[BEGIN','[END'),lines.at(-1));
 assert.ok(text.includes('Angus via Aporia'));assert.ok(text.includes(B));assert.ok(text.includes('--file'));
 assert.ok(text.includes('NOT authenticated'));assert.equal(frame('a\nb','c',r.id,'body').split('\n').length,3);
});
test('reply correlation, size, duplicate, expiration, closed state',()=>{
 const r=request();assert.throws(()=>acceptReply(r,{id:'wrong',session:B,text:'x'}));
 assert.throws(()=>acceptReply(r,{id:r.id,session:A,text:'x'}));
 assert.throws(()=>acceptReply(r,{id:r.id,session:B,text:'x'.repeat(32001)}));
 assert.equal(acceptReply(r,{id:r.id,session:B,text:'answer'}).duplicate,false);
 assert.equal(acceptReply(r,{id:r.id,session:B,text:'answer'}).duplicate,true);
 assert.throws(()=>acceptReply(r,{id:r.id,session:B,text:'different'}));
 assert.throws(()=>acceptReply(r,{id:r.id,session:C,text:'answer'},Date.parse(r.expires)));
 r.status='cancelled';assert.throws(()=>acceptReply(r,{id:r.id,session:C,text:'answer'}));
});
test('real IPC concurrent replies complete only requested slots and persist',async t=>{
 let count=0;const c=await fixture(t,request(),{onReply:()=>count++});c.finishDispatch();
 const bad=await send(c.request.socket,{id:c.request.id,session:A,text:'spoof'});assert.equal(bad.ok,false);
 const rs=await Promise.all(peers.map(p=>send(c.request.socket,{id:c.request.id,session:p.session,text:p.name})));
 assert.ok(rs.every(r=>r.ok));await c.finished;assert.equal(count,2);assert.equal(c.request.status,'completed');
 assert.equal(JSON.parse(fs.readFileSync(c.requestPath)).status,'completed');assert.ok(!fs.existsSync(c.request.socket));
});
test('real reply helper works without recipient extension reload',async t=>{
 const c=await fixture(t);c.recordDispatch(C,'skipped','test');c.finishDispatch();
 const file=path.join(path.dirname(c.requestPath),'answer.txt');fs.writeFileSync(file,'A real answer');
 const helper=fileURLToPath(new URL('./reply.mjs',import.meta.url));
 const result=await exec(process.execPath,[helper,c.requestPath,'--file',file],{env:{...process.env,PI_SESSION_ID:B}});
 assert.match(result.stdout,/recorded/);await c.finished;assert.equal(c.request.status,'partial');
 assert.equal(c.request.replies[B].text,'A real answer');
});
test('timeout preserves partial replies and closes socket',async t=>{
 const r=request();r.expires=new Date(Date.now()+100).toISOString();const c=await fixture(t,r);c.finishDispatch();
 assert.equal((await send(r.socket,{id:r.id,session:B,text:'partial'})).ok,true);
 await c.finished;assert.equal(r.status,'timed-out');assert.equal(r.replies[B].text,'partial');assert.ok(!r.replies[C]);
});
test('cancellation is idempotent, persisted, and does not fabricate remote completion',async t=>{
 const c=await fixture(t);c.close('cancelled');c.close('completed');await c.finished;
 assert.equal(c.request.status,'cancelled');assert.deepEqual(c.request.replies,{});
});
