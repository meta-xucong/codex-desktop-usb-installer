import { createWriteStream } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const DEFAULT_PORT = 9222;
const MAX_SECONDS = 300;
const TARGET_ASSET = 'app-initial-ad16eaadf81f.js';
const GUARD_EXPRESSION = 'f||_e.isLoading||ve.isLoading||!p&&ve.reason===`statsig-disabled`||ye.isLoading||Ee.isLoading||Oe.isLoading||U===`loading`||L.status===`loading`||Je.status===`loading`||Ye.status===`loading`||(Ei.dispatchMessage';
const IPC_EVENT = 'electron-desktop-features-changed';
const IPC_RETURN = '}),iu.info(`browser_use_availability_resolved`';
const REASONS = new Set(['statsig-disabled', 'feature-disabled', 'unsupported', 'loading', 'enabled']);

export function isCodexRendererTarget(targetInfo) {
  const url = String(targetInfo?.url ?? '').toLowerCase();
  return targetInfo?.type === 'page' && url.includes('codex') && url.includes('app.asar');
}

export function normalizeGuard(raw) {
  const bool = value => value === true ? true : value === false ? false : null;
  const reason = typeof raw?.externalReason === 'string' && REASONS.has(raw.externalReason)
    ? raw.externalReason : raw?.externalReason == null ? null : 'other';
  return {
    statsigLoading: bool(raw?.statsigLoading), gate410065390: bool(raw?.gate410065390),
    inAppBrowserLoading: bool(raw?.inAppBrowserLoading), externalBrowserLoading: bool(raw?.externalBrowserLoading),
    externalBrowserReason: reason, computerUseLoading: bool(raw?.computerUseLoading),
    computerUseOtherLoading: bool(raw?.computerUseOtherLoading), skysightLoading: bool(raw?.skysightLoading),
    sitesAccessLoading: bool(raw?.sitesAccessLoading), localAccessLoading: bool(raw?.localAccessLoading),
    cloudAccessLoading: bool(raw?.cloudAccessLoading), workAccessLoading: bool(raw?.workAccessLoading),
  };
}

export function guardLocation(source) {
  const index = source.indexOf(GUARD_EXPRESSION);
  if (index < 0 || source.indexOf(GUARD_EXPRESSION, index + GUARD_EXPRESSION.length) >= 0) return null;
  return locationAt(source, index);
}

export function ipcCallReturnLocation(source) {
  const event = source.indexOf(IPC_EVENT);
  if (event < 0 || source.indexOf(IPC_EVENT, event + IPC_EVENT.length) >= 0) return null;
  const marker = source.indexOf(IPC_RETURN, event + IPC_EVENT.length);
  if (marker < 0 || source.indexOf(IPC_RETURN, marker + IPC_RETURN.length) >= 0) return null;
  // `}),iu.info` is after the synchronous renderer dispatchMessage() call returned.
  return locationAt(source, marker + 3);
}

export function locationAt(source, index) {
  const prefix = source.slice(0, index);
  const lineStart = prefix.lastIndexOf('\n') + 1;
  return { lineNumber: (prefix.match(/\n/g) ?? []).length, columnNumber: prefix.slice(lineStart).length };
}

export function isLoopbackWebSocket(url) {
  try {
    const parsed = url instanceof URL ? url : new URL(url);
    return parsed.protocol === 'ws:' && ['127.0.0.1', 'localhost', '[::1]'].includes(parsed.hostname);
  } catch { return false; }
}

export function sameLocation(actual, expected, scriptId) {
  return !!actual && actual.scriptId === scriptId && actual.lineNumber === expected.lineNumber
    && actual.columnNumber === expected.columnNumber;
}

export async function handlePausedEvent({ hitBreakpoints, guardBreakpoint, ipcBreakpoint, readGuard, resume, emit }) {
  const guardHit = !!guardBreakpoint && hitBreakpoints.includes(guardBreakpoint);
  const ipcHit = !!ipcBreakpoint && hitBreakpoints.includes(ipcBreakpoint);
  if (!guardHit && !ipcHit) return { ownedPause: false, resumed: false };
  try {
    if (guardHit) {
      const raw = await readGuard();
      emit({ event: 'renderer-guard-snapshot', state: normalizeGuard(raw) });
    }
    if (ipcHit) {
      emit({ event: 'renderer-ipc-call-returned', callReturned: true, mainAcceptance: 'not_observed' });
    }
  } catch {
    emit({ event: guardHit ? 'renderer-guard-snapshot-unavailable' : 'ipc-observation-error' });
  } finally {
    await resume();
  }
  return { ownedPause: true, resumed: true };
}

function emit(stream, event) {
  if (stream.writableEnded || stream.destroyed) return;
  stream.write(`${JSON.stringify({ at: new Date().toISOString(), ...event })}\n`);
}

class Cdp {
  constructor(ws, onDisconnect) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = new Set();
    this.closed = false;
    ws.addEventListener('message', event => {
      let message;
      try { message = JSON.parse(event.data); } catch { return; }
      if (message.id) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        if (message.error) pending.reject(new Error('CDP command failed'));
        else pending.resolve(message.result ?? {});
      } else {
        for (const listener of this.listeners) void listener(message);
      }
    });
    ws.addEventListener('close', () => {
      this.closed = true;
      for (const pending of this.pending.values()) pending.reject(new Error('CDP disconnected'));
      this.pending.clear();
      onDisconnect();
    });
    ws.addEventListener('error', () => { /* The close event performs disconnect cleanup. */ });
  }
  on(listener) { this.listeners.add(listener); return () => this.listeners.delete(listener); }
  send(method, params = {}, sessionId) {
    if (this.closed || this.ws.readyState !== WebSocket.OPEN) return Promise.reject(new Error('CDP disconnected'));
    const id = this.nextId++;
    const payload = { id, method, params };
    if (sessionId) payload.sessionId = sessionId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error('CDP timeout')); }, 5000);
      this.pending.set(id, {
        resolve: value => { clearTimeout(timer); resolve(value); },
        reject: error => { clearTimeout(timer); reject(error); },
      });
      this.ws.send(JSON.stringify(payload));
    });
  }
}

const EVAL_GUARD = `(()=>{const v=f=>{try{return f()}catch{return null}};return {
  statsigLoading:v(()=>f), gate410065390:v(()=>p),
  inAppBrowserLoading:v(()=>_e.isLoading), externalBrowserLoading:v(()=>ve.isLoading),
  externalReason:v(()=>ve.reason), computerUseLoading:v(()=>ye.isLoading),
  computerUseOtherLoading:v(()=>Ee.isLoading), skysightLoading:v(()=>Oe.isLoading),
  sitesAccessLoading:v(()=>U==="loading"), localAccessLoading:v(()=>L.status==="loading"),
  cloudAccessLoading:v(()=>Je.status==="loading"), workAccessLoading:v(()=>Ye.status==="loading")
}})()`;

async function instrumentTarget(cdp, targetId, sessionId, stream, state) {
  const breakpoints = new Map();
  const processedScripts = new Set();
  state.sessions.set(sessionId, { targetId, breakpoints, pausedByUs: false });
  const detach = cdp.on(async message => {
    if (message.sessionId !== sessionId) return;
    if (message.method === 'Debugger.scriptParsed') {
      const { scriptId, url = '' } = message.params;
      if (!url.includes(TARGET_ASSET) || processedScripts.has(scriptId)) return;
      processedScripts.add(scriptId);
      try {
        const { scriptSource = '' } = await cdp.send('Debugger.getScriptSource', { scriptId }, sessionId);
        const guard = guardLocation(scriptSource);
        const ipcReturn = ipcCallReturnLocation(scriptSource);
        if (guard) await setResolvedBreakpoint(cdp, sessionId, scriptId, 'guard', guard, breakpoints, stream);
        if (ipcReturn) await setResolvedBreakpoint(cdp, sessionId, scriptId, 'ipc-return', ipcReturn, breakpoints, stream);
        emit(stream, { event: 'instrumentation-status', guard: breakpoints.has('guard') ? 'armed' : 'not_available',
          ipcCallReturn: breakpoints.has('ipc-return') ? 'armed' : 'not_available' });
      } catch {
        emit(stream, { event: 'instrumentation-error', step: 'source-or-breakpoint' });
      }
      return;
    }
    if (message.method !== 'Debugger.paused') return;
    const ids = message.params.hitBreakpoints ?? [];
    const guardBreakpoint = breakpoints.get('guard')?.breakpointId;
    const ipcBreakpoint = breakpoints.get('ipc-return')?.breakpointId;
    const ownsPause = (guardBreakpoint && ids.includes(guardBreakpoint)) || (ipcBreakpoint && ids.includes(ipcBreakpoint));
    if (!ownsPause) return;
    state.sessions.get(sessionId).pausedByUs = true;
    const frame = message.params.callFrames?.[0];
    await handlePausedEvent({
      hitBreakpoints: ids, guardBreakpoint, ipcBreakpoint,
      readGuard: async () => {
        if (!frame?.callFrameId) throw new Error('no local frame');
        const result = await cdp.send('Debugger.evaluateOnCallFrame', {
          callFrameId: frame.callFrameId, expression: EVAL_GUARD, returnByValue: true, silent: true,
        }, sessionId);
        return result.result?.value ?? null;
      },
      resume: async () => { await cdp.send('Debugger.resume', {}, sessionId); },
      emit: event => {
        if (event.event === 'renderer-guard-snapshot') state.guardObserved = true;
        if (event.event === 'renderer-ipc-call-returned') state.ipcCallReturnObserved = true;
        emit(stream, event);
      },
    }).catch(() => {});
    if (state.sessions.has(sessionId)) state.sessions.get(sessionId).pausedByUs = false;
  });
  try {
    await cdp.send('Debugger.enable', {}, sessionId);
    await cdp.send('Runtime.enable', {}, sessionId);
  } catch {
    detach();
    state.sessions.delete(sessionId);
    emit(stream, { event: 'target-attach-error' });
  }
  return detach;
}

async function setResolvedBreakpoint(cdp, sessionId, scriptId, kind, location, breakpoints, stream) {
  const result = await cdp.send('Debugger.setBreakpoint', { location: { scriptId, ...location } }, sessionId);
  if (!sameLocation(result.actualLocation, location, scriptId)) {
    await cdp.send('Debugger.removeBreakpoint', { breakpointId: result.breakpointId }, sessionId).catch(() => {});
    emit(stream, { event: 'breakpoint-location-mismatch', kind });
    return;
  }
  breakpoints.set(kind, { breakpointId: result.breakpointId, location });
}

async function fetchJson(url) {
  const response = await fetch(url, { signal: AbortSignal.timeout(1500) });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return response.json();
}

export async function collect({ port = DEFAULT_PORT, seconds = 180, output }) {
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('port must be 1..65535');
  if (!Number.isInteger(seconds) || seconds < 1 || seconds > MAX_SECONDS) throw new Error(`seconds must be 1..${MAX_SECONDS}`);
  const stream = createWriteStream(output, { flags: 'wx' });
  const stopAt = Date.now() + seconds * 1000;
  const state = { sessions: new Map(), targetIds: new Set(), targetsAttached: false, guardObserved: false, ipcCallReturnObserved: false };
  let interrupted = false;
  let disconnected = false;
  let cdp;
  const onSigint = () => { interrupted = true; };
  process.on('SIGINT', onSigint);
  try {
    let versionInfo;
    while (!interrupted && Date.now() < stopAt) {
      try { versionInfo = await fetchJson(`http://127.0.0.1:${port}/json/version`); break; }
      catch { await delay(200); }
    }
    if (!versionInfo?.webSocketDebuggerUrl) throw new Error('loopback CDP endpoint did not appear');
    const wsUrl = new URL(versionInfo.webSocketDebuggerUrl);
    if (!isLoopbackWebSocket(wsUrl)) throw new Error('refusing non-loopback CDP endpoint');
    const ws = new WebSocket(wsUrl);
    await Promise.race([
      new Promise((resolve, reject) => {
        ws.addEventListener('open', resolve, { once: true });
        ws.addEventListener('error', () => reject(new Error('CDP connection failed')), { once: true });
      }),
      delay(5000).then(() => { throw new Error('CDP connection timeout'); }),
    ]);
    cdp = new Cdp(ws, () => { disconnected = true; emit(stream, { event: 'cdp-disconnected' }); });
    emit(stream, { event: 'connected', loopback: true });
    const attachPage = async targetInfo => {
      if (!isCodexRendererTarget(targetInfo) || state.targetIds.has(targetInfo.targetId) || cdp.closed) return;
      state.targetIds.add(targetInfo.targetId);
      try {
        const { sessionId } = await cdp.send('Target.attachToTarget', { targetId: targetInfo.targetId, flatten: true });
        await instrumentTarget(cdp, targetInfo.targetId, sessionId, stream, state);
        state.targetsAttached = true;
        emit(stream, { event: 'codex-renderer-attached' });
      } catch {
        emit(stream, { event: 'target-attach-error' });
      }
    };
    cdp.on(message => {
      if (message.method === 'Target.targetCreated' || message.method === 'Target.targetInfoChanged') {
        void attachPage(message.params.targetInfo);
      } else if (message.method === 'Target.targetDestroyed') {
        state.targetIds.delete(message.params.targetId);
      } else if (message.method === 'Target.detachedFromTarget') {
        state.sessions.delete(message.params.sessionId);
      }
    });
    await cdp.send('Target.setDiscoverTargets', { discover: true });
    const { targetInfos = [] } = await cdp.send('Target.getTargets');
    for (const targetInfo of targetInfos) void attachPage(targetInfo);
    while (!interrupted && !disconnected && Date.now() < stopAt && ws.readyState === WebSocket.OPEN) await delay(100);
    for (const [sessionId, session] of state.sessions) {
      if (session.pausedByUs) await cdp.send('Debugger.resume', {}, sessionId).catch(() => {});
      for (const { breakpointId } of session.breakpoints.values()) {
        await cdp.send('Debugger.removeBreakpoint', { breakpointId }, sessionId).catch(() => {});
      }
      await cdp.send('Target.detachFromTarget', { sessionId }).catch(() => {});
    }
    if (ws.readyState === WebSocket.OPEN) ws.close();
    emit(stream, { event: 'stopped', reason: interrupted ? 'user-stop' : disconnected ? 'cdp-disconnected' : 'duration-expired',
      rendererTargets: state.targetsAttached ? 'observed' : 'not_observed',
      guardSnapshot: state.guardObserved ? 'observed' : 'not_observed',
      ipcCallReturn: state.ipcCallReturnObserved ? 'observed' : 'not_observed', mainAcceptance: 'not_observed' });
  } catch (error) {
    if (cdp?.closed || disconnected) {
      // Once the socket is gone no remote resume is possible; do not claim a breakpoint was recovered.
      emit(stream, { event: 'stopped', reason: 'cdp-disconnected', mainAcceptance: 'not_observed' });
    } else {
      throw error;
    }
  } finally {
    process.off('SIGINT', onSigint);
    if (cdp?.ws?.readyState === WebSocket.OPEN) cdp.ws.close();
    stream.end();
    await new Promise(resolve => stream.once('finish', resolve));
  }
}

function parseArgs(argv) {
  const args = { port: DEFAULT_PORT, seconds: 180, output: '' };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--port') args.port = Number(argv[++i]);
    else if (argv[i] === '--seconds') args.seconds = Number(argv[++i]);
    else if (argv[i] === '--output') args.output = argv[++i];
    else if (argv[i] === '--help') args.help = true;
    else throw new Error(`unknown option: ${argv[i]}`);
  }
  return args;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const args = parseArgs(process.argv.slice(2));
    if (args.help || !args.output) {
      console.log('Usage: node collector.mjs --output <new-jsonl-file> [--port 9222] [--seconds 1..300]');
      process.exitCode = args.help ? 0 : 2;
    } else await collect(args);
  } catch (error) {
    console.error(`collector failed: ${error.message}`);
    process.exitCode = 1;
  }
}

