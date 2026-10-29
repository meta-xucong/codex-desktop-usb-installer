import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import crypto from 'node:crypto';
import fs from 'node:fs';
import { collect, guardLocation, ipcCallReturnLocation, isCodexRendererTarget, isLoopbackWebSocket, normalizeGuard, handlePausedEvent } from './collector.mjs';

const GUARD = 'f||_e.isLoading||ve.isLoading||!p&&ve.reason===`statsig-disabled`||ye.isLoading||Ee.isLoading||Oe.isLoading||U===`loading`||L.status===`loading`||Je.status===`loading`||Ye.status===`loading`||(Ei.dispatchMessage';
const SOURCE = `function nSu(){if(${GUARD})(Ei.dispatchMessage(\`electron-desktop-features-changed\`,{}),iu.info(\`browser_use_availability_resolved\`,{}));}`;
const TARGET = { targetId: 't1', type: 'page', url: 'file:///C:/Users/MSI-15/AppData/Local/Programs/Codex/resources/app.asar/index.html' };

function frame(value) {
  const payload = Buffer.from(JSON.stringify(value));
  if (payload.length < 126) return Buffer.concat([Buffer.from([0x81, payload.length]), payload]);
  if (payload.length < 65536) { const head=Buffer.alloc(4); head[0]=0x81;head[1]=126;head.writeUInt16BE(payload.length,2);return Buffer.concat([head,payload]); }
  throw new Error('mock frame too large');
}
function parseClientFrames(state, chunk, onJson) {
  state.buffer = Buffer.concat([state.buffer, chunk]);
  while (state.buffer.length >= 2) {
    const a=state.buffer[0], b=state.buffer[1], masked=(b&0x80)!==0; let len=b&0x7f, offset=2;
    if (len===126) { if(state.buffer.length<4)return;len=state.buffer.readUInt16BE(2);offset=4; }
    else if(len===127) { if(state.buffer.length<10)return;len=Number(state.buffer.readBigUInt64BE(2));offset=10; }
    const maskBytes=masked?4:0; if(state.buffer.length<offset+maskBytes+len)return;
    let payload=state.buffer.subarray(offset+maskBytes,offset+maskBytes+len);
    if(masked){const mask=state.buffer.subarray(offset,offset+4);payload=Buffer.from(payload);for(let i=0;i<payload.length;i++)payload[i]^=mask[i%4];}
    state.buffer=state.buffer.subarray(offset+maskBytes+len);
    const opcode=a&0x0f;
    if(opcode===8){state.socket.end();return;}
    if(opcode===1) onJson(JSON.parse(payload.toString('utf8')));
  }
}

async function startMockCdp({ mode='normal' }={}) {
  const commands=[]; const breakpointIds=[]; let socket;
  const server=http.createServer((req,res)=>{
    if(req.url==='/json/version') { const p=server.address().port;res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify({webSocketDebuggerUrl:`ws://127.0.0.1:${p}/devtools/browser/mock`}));return; }
    res.writeHead(404);res.end();
  });
  server.on('upgrade',(req,s)=>{
    socket=s; const accept=crypto.createHash('sha1').update(req.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    s.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    const state={buffer:Buffer.alloc(0),socket:s};
    const send=(message)=>s.write(frame(message));
    const respond=(m,result={})=>send({id:m.id,result,...(m.sessionId?{sessionId:m.sessionId}:{})});
    s.on('data',chunk=>parseClientFrames(state,chunk,m=>{
      commands.push(m.method);
      if(m.method==='Target.setDiscoverTargets') {
        respond(m);
        send({method:'Target.targetCreated',params:{targetInfo:{...TARGET,targetId:'other',url:'file:///C:/Other/resources/app.asar/index.html'}}});
        if(mode!=='timeout') send({method:'Target.targetCreated',params:{targetInfo:TARGET}});
      } else if(m.method==='Target.getTargets') {
        respond(m,{targetInfos:mode==='timeout'?[]:[TARGET]});
        if(mode==='disconnect') setTimeout(()=>s.destroy(),20);
      } else if(m.method==='Target.attachToTarget') {
        respond(m,{sessionId:'s1'});
      } else if(m.method==='Debugger.enable') {
        respond(m);
        send({sessionId:'s1',method:'Debugger.scriptParsed',params:{scriptId:'script1',url:`file:///Codex/resources/app.asar/${'app-initial-ad16eaadf81f.js'}`}});
      } else if(m.method==='Runtime.enable') respond(m);
      else if(m.method==='Debugger.getScriptSource') respond(m,{scriptSource:SOURCE});
      else if(m.method==='Debugger.setBreakpoint') {
        const id=`b${breakpointIds.length+1}`;breakpointIds.push(id);const actualLocation={...m.params.location,...(mode==='mismatch'?{columnNumber:m.params.location.columnNumber+1}:{})};respond(m,{breakpointId:id,actualLocation});
        if(breakpointIds.length===2&&mode!=='mismatch') setTimeout(()=>{
          const callFrame={callFrameId:'frame1',location:{scriptId:'script1',lineNumber:0,columnNumber:0},scopeChain:[]};
          send({sessionId:'s1',method:'Debugger.paused',params:{reason:'other',hitBreakpoints:['foreign'],callFrames:[callFrame]}});
          send({sessionId:'s1',method:'Debugger.paused',params:{reason:'other',hitBreakpoints:[breakpointIds[0]],callFrames:[callFrame]}});
          send({sessionId:'s1',method:'Debugger.paused',params:{reason:'other',hitBreakpoints:[breakpointIds[1]],callFrames:[callFrame]}});
        },20);
      } else if(m.method==='Debugger.evaluateOnCallFrame') respond(m,{result:{type:'object',value:{statsigLoading:false,gate410065390:true,externalReason:'statsig-disabled',token:'never-log'}}});
      else respond(m);
    }));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  return {port:server.address().port,commands,close:()=>new Promise(resolve=>{if(socket&&!socket.destroyed)socket.destroy();server.close(()=>resolve());})};
}

function tempOutput() {
  return `${process.env.TEMP || process.cwd()}\\codex-cdp-mock-${process.pid}-${crypto.randomUUID()}.jsonl`;
}

// Static fixture tests exercise the exact marker forms separately; verify-msix-renderer.mjs validates them against the pinned MSIX.
test('locates exact guard and post-call return point; no event-string-only dispatch claim', () => {
  const g=guardLocation(SOURCE), i=ipcCallReturnLocation(SOURCE);
  assert.deepEqual(g,{lineNumber:0,columnNumber:18});
  assert.deepEqual(i,{lineNumber:0,columnNumber:SOURCE.indexOf('}),iu.info(`browser_use_availability_resolved`')+3});
});

test('target filter requires Codex app.asar and loopback websocket', () => {
  assert.equal(isCodexRendererTarget(TARGET),true);
  assert.equal(isCodexRendererTarget({...TARGET,url:'file:///C:/Other/resources/app.asar/index.html'}),false);
  assert.equal(isLoopbackWebSocket('ws://127.0.0.1:9222/devtools/browser/x'),true);
  assert.equal(isLoopbackWebSocket('ws://192.168.1.2:9222/x'),false);
  assert.equal(isLoopbackWebSocket('wss://127.0.0.1:9222/x'),false);
});

test('normalizer emits only fixed booleans and capped reason enum', () => {
  const value=normalizeGuard({statsigLoading:true,gate410065390:false,externalReason:'customer-secret',token:'secret'});
  assert.equal(value.statsigLoading,true);assert.equal(value.gate410065390,false);assert.equal(value.externalBrowserReason,'other');
  assert.equal(JSON.stringify(value).includes('secret'),false);assert.equal(Object.keys(value).length,12);
});

test('only resumes pauses that match this collector’s breakpoint IDs', async () => {
  let resumed=0,read=0;
  const other=await handlePausedEvent({hitBreakpoints:['foreign'],guardBreakpoint:'guard',ipcBreakpoint:'ipc',readGuard:async()=>{read++;return{};},resume:async()=>{resumed++;},emit:()=>{}});
  assert.deepEqual(other,{ownedPause:false,resumed:false});assert.equal(resumed,0);assert.equal(read,0);
  const own=await handlePausedEvent({hitBreakpoints:['guard'],guardBreakpoint:'guard',readGuard:async()=>({gate410065390:true}),resume:async()=>{resumed++;},emit:()=>{}});
  assert.equal(own.resumed,true);assert.equal(resumed,1);
});

test('real WebSocket CDP mock attaches only matching target, verifies locations, samples and resumes own pauses', async t=>{
  const mock=await startMockCdp();t.after(()=>mock.close());const output=tempOutput();t.after(()=>{try{fs.unlinkSync(output);}catch{ /* The file may not exist if collection fails. */ }});
  await collect({port:mock.port,seconds:1,output});
  const text=fs.readFileSync(output,'utf8');const rows=text.trim().split(/\r?\n/).map(JSON.parse);
  assert.equal(mock.commands.includes('Target.setAutoAttach'),false);
  assert.equal(mock.commands.filter(x=>x==='Target.attachToTarget').length,1);
  assert.equal(mock.commands.filter(x=>x==='Debugger.resume').length,2);
  assert.equal(rows.some(x=>x.event==='renderer-guard-snapshot'&&x.state.gate410065390===true),true);
  const ipc=rows.find(x=>x.event==='renderer-ipc-call-returned');assert.equal(ipc.callReturned,true);assert.equal(ipc.mainAcceptance,'not_observed');
  assert.equal(text.includes('never-log'),false);
  assert.equal(rows.at(-1).guardSnapshot,'observed');
  assert.equal(rows.at(-1).ipcCallReturn,'observed');
  assert.equal(rows.at(-1).mainAcceptance,'not_observed');
});

test('real WebSocket CDP mock disconnect is reported without claiming main acceptance', async t=>{
  const mock=await startMockCdp({mode:'disconnect'});t.after(()=>mock.close());const output=tempOutput();t.after(()=>{try{fs.unlinkSync(output);}catch{ /* The file may not exist if collection fails. */ }});
  await collect({port:mock.port,seconds:1,output});
  const rows=fs.readFileSync(output,'utf8').trim().split(/\r?\n/).map(JSON.parse);
  assert.equal(rows.some(x=>x.event==='cdp-disconnected'),true);assert.equal(rows.at(-1).mainAcceptance,'not_observed');
});

test('real WebSocket CDP mock timeout reports not observed rather than false', async t=>{
  const mock=await startMockCdp({mode:'timeout'});t.after(()=>mock.close());const output=tempOutput();t.after(()=>{try{fs.unlinkSync(output);}catch{ /* The file may not exist if collection fails. */ }});
  await collect({port:mock.port,seconds:1,output});
  const rows=fs.readFileSync(output,'utf8').trim().split(/\r?\n/).map(JSON.parse);const stopped=rows.at(-1);
  assert.equal(stopped.reason,'duration-expired');assert.equal(stopped.mainAcceptance,'not_observed');
  assert.equal(rows.some(x=>x.event==='renderer-ipc-call-returned'),false);
  assert.equal(JSON.stringify(rows).includes('dispatched":false'),false);
});



test('real WebSocket CDP mock refuses unresolved/moved breakpoint locations', async t=>{
  const mock=await startMockCdp({mode:'mismatch'});t.after(()=>mock.close());const output=tempOutput();t.after(()=>{try{fs.unlinkSync(output);}catch{ /* The file may not exist if collection fails. */ }});
  await collect({port:mock.port,seconds:1,output});
  const rows=fs.readFileSync(output,'utf8').trim().split(/\r?\n/).map(JSON.parse);
  assert.equal(rows.filter(x=>x.event==='breakpoint-location-mismatch').length,2);
  assert.equal(rows.some(x=>x.event==='renderer-guard-snapshot'),false);
  assert.equal(rows.some(x=>x.event==='renderer-ipc-call-returned'),false);
  assert.equal(rows.at(-1).guardSnapshot,'not_observed');
  assert.equal(rows.at(-1).ipcCallReturn,'not_observed');
});

