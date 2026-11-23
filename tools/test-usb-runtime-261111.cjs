const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');
const assert = require('node:assert/strict');
const acorn = require('acorn');

const source = fs.readFileSync(path.join(__dirname, '../crates/codex-win-engine/src/portable_runtime_261111.rs'), 'utf8');
const patches = new Map([...source.matchAll(/\(br#"(.*?)"#, br#"(.*?)"#, "(.*?)"\)/gs)].map(([, before, after, label]) => [label, { before, after }]));
const is261117 = process.env.USB_RUNTIME_TEST_LAYOUT === '261117';
if (is261117) {
  const current = fs.readFileSync(path.join(__dirname, '../crates/codex-win-engine/src/portable_runtime_261117.rs'), 'utf8');
  const changed = [...current.matchAll(/\(br#"(.*?)"#, br#"(.*?)"#, "(.*?)"\)/gs)];
  assert.equal(changed.length, 7);
  for (const [, before, after, label] of changed) patches.set(label, { before, after });
}
const code = label => patches.get(label).after;
const logger = () => ({ info() {}, warning() {} });

function browserFixture() {
  const owner = { id: 1 };
  const window = { owner, window: { show() { this.shown++; }, shown: 0, isVisible() { return true; } }, activeConversationId: 'thread', activeBrowserTabId: 'tab', tabSourceWindow: null };
  const state = { visible: false, bounds: null, page: { view: { webContents: { id: 2, isDestroyed() { return false; }, closeExtensionActionContextMenu() {} } } } };
  const messages = [];
  const Waiter = vm.runInNewContext(`(${code('iab-rendered-state-waiter')})`, { setTimeout, clearTimeout, [is261117 ? 'ekp' : 'en$']: logger });
  const waiter = new Waiter();
  let delegate;
  const actions = {
    getPageWindowState() { return window; },
    getThreadStateForWindow() { return state; },
    ensureThreadStateForWindow() { return state; },
    updateThreadSessionState() {},
    browserUseOpenRequests: waiter,
    browserSessionRegistry: { getDelegate() { return delegate; } },
    windowManager: { sendMessageToWebContents(_owner, message) { messages.push(message); } },
  };
  const routeOpen = vm.runInNewContext(`(${code('iab-explicit-route-selection')})`);
  const context = { [is261117 ? 'eIr' : 'eiE']: logger, [is261117 ? 'eko' : 'enL']: (_window, _thread, tab) => tab, [is261117 ? 'eIo' : 'eij']: routeOpen };
  delegate = vm.runInNewContext(`(function(){return {${code('iab-delegate-rendered-state')}}})`, context).call({ actions });
  const Host = vm.runInNewContext(`(class { ${code('iab-host-return')} })`);
  const host = new Host();
  host.route = { conversationId: 'thread', windowId: 1 };
  host.registry = { isSubagentBrowserUseRoute() { return false; }, getDelegate() { return delegate; } };
  host.setBrowserUseActive = () => {};
  host.isBrowserVisibleForBrowserUse = tab => delegate.isBrowserVisibleForBrowserUse(host.route, tab);
  const schema = { parse(value) { return value; } };
  const commands = vm.runInNewContext(`({${code('iab-command-await')}}).handlers`, {
    e: 'tab', t: host, erN: 'set', erj: 'get', erJ: 'viewport', erV: 'reset',
    erq: schema, erU: schema, erL: schema, erB: schema, erY: schema, erZ: schema, erK: schema, erG: schema,
    eSd: 'set', eSa: 'get', eSb: 'viewport', eSf: 'reset',
    eSu: schema, eSh: schema, eSo: schema, eSl: schema, eSS: schema, eSk: schema, eSw: schema, eSy: schema,
  });
  const resolve = () => waiter.resolveForWindow(window, () => ({ browserTabId: 'tab', webContents: { id: 2 }, pageKey: 'page' }));
  return { commands, host, delegate, window, state, waiter, messages, resolve };
}

test('all replacements fit their real slots', () => {
  assert.equal(patches.size, 9);
  for (const [label, patch] of patches) assert.ok(Buffer.byteLength(patch.after) <= Buffer.byteLength(patch.before), label);
});

test('early bootstrap uses the existing Windows roaming path and preserves an explicit user-data override', () => {
  const patch = patches.get('bootstrap-appdata');
  const evaluate = (source, override) => vm.runInNewContext(source, {
    process: { env: { APPDATA: 'C:/profile/Roaming', CODEX_ELECTRON_USER_DATA_PATH: override } },
    i: { join: (...parts) => parts.join('/') }, [is261117 ? 'c' : 'l']: { m: () => 'Codex' }, [is261117 ? 'l' : 'c']: { x: { resolve: () => 'prod' } },
    r: { app: { getPath() { throw new Error("Failed to get 'appData' path"); } } },
  });
  assert.throws(() => evaluate(patch.before), /appData/);
  assert.equal(evaluate(patch.after), 'C:/profile/Roaming/Codex');
  assert.equal(evaluate(patch.after, ' D:/explicit '), 'D:/explicit');
});

test('onboarding preserves USB COMMANDS for non-coding while retaining role-specific UI choices', () => {
  const patch = patches.get('onboarding-default-detail-mode');
  const evaluate = (source, workMode) => JSON.parse(JSON.stringify(vm.runInNewContext(source, {
    [is261117 ? 'b' : 'y']: { workMode }, [is261117 ? 'H' : 'L']: { _e: 'STEPS_COMMANDS', O$: 'STEPS_PROSE' },
  })));
  const original = evaluate(patch.before, 'non_coding');
  assert.equal(original.threadDetailLevel, 'STEPS_PROSE');
  assert.deepEqual(evaluate(patch.after, 'non_coding'), { ...original, threadDetailLevel: 'STEPS_COMMANDS' });
  assert.deepEqual(evaluate(patch.after, 'coding'), evaluate(patch.before, 'coding'));
});

test('IAB set waits for the rendered owner/tab, not only pending visibility or an attached webview', async () => {
  const f = browserFixture();
  let completed = false;
  const pending = f.commands.set({ visible: true }).then(() => { completed = true; });
  await Promise.resolve();
  assert.equal(completed, false);
  assert.equal(f.window.window.shown, 1);
  assert.equal(f.messages[0].type, 'toggle-browser-panel');
  f.resolve();
  await Promise.resolve();
  assert.equal(completed, false);
  f.state.visible = true;
  f.state.bounds = { width: 500, height: 500 };
  f.window.activeBrowserTabId = 'other';
  f.resolve();
  await Promise.resolve();
  assert.equal(completed, false);
  f.window.activeBrowserTabId = 'tab';
  f.resolve();
  await pending;
  assert.equal(f.commands.get({}).visible, true);
  assert.equal(f.waiter.requests.size, 0);
});

test('explicit show selects a currently hidden panel even if its cached state is visible', async () => {
  const f = browserFixture();
  f.state.visible = true;
  f.state.bounds = { width: 500, height: 500 };
  await f.commands.set({ visible: true });
  assert.equal(f.messages.length, 1);
  assert.equal(f.messages[0].browserTabId, 'tab');
  assert.equal(f.window.window.shown, 1);
});

test('show on another route asks for its panel and resolves only after route activation', async () => {
  const f = browserFixture();
  f.window.activeConversationId = 'other-thread';
  const pending = f.commands.set({ visible: true });
  assert.equal(f.messages[0].type, 'browser-sidebar-open-panel-without-animation');
  assert.equal(f.messages[0].conversationId, 'thread');
  f.state.visible = true;
  f.state.bounds = {};
  f.window.activeConversationId = 'thread';
  f.resolve();
  await pending;
});

test('hide and ordinary non-wait calls keep their behavior; rejected waits propagate', async () => {
  const f = browserFixture();
  assert.equal(f.host.setBrowserVisibleForBrowserUse(true, 'tab'), undefined);
  assert.equal(f.waiter.requests.size, 0);
  const pending = f.commands.set({ visible: true });
  f.waiter.rejectForOwner(f.window.owner, new Error('owner closed'));
  await assert.rejects(pending, /owner closed/);
  await f.commands.set({ visible: false });
  assert.equal(f.commands.get({}).visible, false);
  assert.equal(f.waiter.requests.size, 0);
});

test('native CUA resolver uses the package helper and retains the original bundled path without an override', () => {
  const recorded = [];
  const env = { OAI_SKY_WINDOWS_BIN: 'X:/bundle/resources/helper.exe' };
  const resolve = vm.runInNewContext(`(${code('cua-helper-resolver')})`, { process: { env, arch: 'x64' }, w: value => recorded.push(value), R: (_root, parts) => parts.join('/') });
  assert.equal(resolve('root'), env.OAI_SKY_WINDOWS_BIN);
  delete env.OAI_SKY_WINDOWS_BIN;
  assert.equal(resolve('root'), 'bin/windows/codex-computer-use.exe');
  assert.equal(recorded.length, 2);
});

test('CUA notify unwrap recognizes the v3 helper but leaves unrelated commands untouched', () => {
  const unwrap = vm.runInNewContext(`(${code('cua-notify-unwrap')})`, { [is261117 ? 'aB' : 's8']: 'turn-ended', [is261117 ? 'aL' : 's5']: '--previous-notify', [is261117 ? 'a$' : 's7']: value => Array.isArray(value) && value.every(x => typeof x === 'string') });
  const inner = ['custom-notify', 'argument'];
  const outer = ['X:\\bundle\\codex-computer-use-mta-wrapper-candidate-v3.exe', 'turn-ended', '--previous-notify', JSON.stringify(inner)];
  assert.equal(JSON.stringify(unwrap(outer)), JSON.stringify(inner));
  assert.equal(unwrap(inner), inner);
  assert.equal(unwrap([...outer.slice(0, 3), '{invalid']), null);
  assert.equal(unwrap(null), null);
});

test('new real bundle syntax remains valid after exact fixed-width replacements when supplied', { skip: !process.env.USB_RUNTIME_AUDIT_DIR }, () => {
  const matched = new Set();
  for (const name of ['new-main.js', 'new-startup.js', 'early-bootstrap.js', 'new-onboarding.js']) {
    const before = fs.readFileSync(path.join(process.env.USB_RUNTIME_AUDIT_DIR, name), 'utf8');
    let after = before;
    for (const [label, patch] of patches) if (after.includes(patch.before)) {
      assert.equal(after.split(patch.before).length - 1, 1);
      matched.add(label);
      after = after.replace(patch.before, patch.after.padEnd(patch.before.length, ' '));
    }
    assert.equal(Buffer.byteLength(after), Buffer.byteLength(before));
    acorn.parse(after, { ecmaVersion: 'latest', sourceType: 'module' });
  }
  assert.deepEqual([...matched].sort(), [...patches.keys()].sort());
});
