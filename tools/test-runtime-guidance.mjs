import assert from 'node:assert/strict';
import console from 'node:console';
import { readFileSync } from 'node:fs';
import { URL } from 'node:url';
const guide = readFileSync(new URL('../docs/USB-RUNTIME-TOOL-ACCEPTANCE.md', import.meta.url), 'utf8');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const snippets = new Map([...guide.matchAll(/<!-- snippet: ([\w-]+) -->\s*```js\n([\s\S]*?)```/g)].map(match => [match[1], match[2]]));
assert.ok(snippets.size >= 2, 'No executable examples found');
for (const [name, code] of snippets) {
  assert.doesNotThrow(() => new AsyncFunction(code), `${name}: parse error`);
}
const output = [];
const catalog = [
  { name: 'mcp__another_registration__media_capabilities', description: 'Read-only capabilities' },
  { name: 'exec_command', description: 'Run shell commands' },
  { name: 'mcp__node_repl_local__js', description: 'Execute JavaScript' },
  { name: 'unrelated', description: 'Unrelated tool' },
];
await new AsyncFunction('ALL_TOOLS', 'text', snippets.get('discovery'))(catalog, value => output.push(value));
assert.deepEqual(output[0].map(tool => tool.name), catalog.slice(0, 3).map(tool => tool.name));
assert.deepEqual(output[0][0], catalog[0], 'The real registration must be preserved');
const summarize = async result => {
  const rows = [];
  await new AsyncFunction('result', 'text', snippets.get('media-summary'))(result, row => rows.push(row));
  return rows[0];
};
const state = { success: true, gateway_catalog: { status: 'fresh', model_count: 51 }, providers: { 'image-2': { configured: true } }, generation_capability: 'not_tested', secret: 'do-not-output-this' };
const contentOnly = await summarize({ content: [{ type: 'text', text: JSON.stringify(state) }], isError: false });
assert.equal(contentOnly.configured['image-2'], true);
assert.equal(contentOnly.catalogModelCount, 51);
assert.equal(contentOnly.generationTested, false);
assert.ok(!JSON.stringify(contentOnly).includes('do-not-output-this'));
assert.deepEqual(await summarize({ structuredContent: state, isError: false }), contentOnly);
const missing = await summarize({ content: [{ type: 'text', text: 'non-JSON diagnostic' }] });
assert.equal(missing.payloadParsed, false);
assert.equal(missing.configured['image-2'], null);
const failed = await summarize({ structuredContent: { success: false }, isError: true });
assert.equal(failed.toolError, true);
assert.equal(failed.businessSuccess, false);
let targetInvoked = false;
await assert.rejects(async () => {
  const run = new AsyncFunction('target', 'await target(');
  await run(() => { targetInvoked = true; });
}, SyntaxError);
assert.equal(targetInvoked, false, 'Syntax failure happens before any target call');
console.log(`PASS: ${snippets.size} examples parse; nested discovery, content-only/structured/error/unknown results, and pre-execution syntax failure checked. No product UI or media calls executed.`);

// Execute the instructions that actually ship in AGENTS and the media skill.
const entrySources = [
  '../src-tauri/src/app/codex_agents_hint.rs',
  '../installer-pack/skills/image-2/SKILL.md',
].map(path => readFileSync(new URL(path, import.meta.url), 'utf8').replaceAll('\r\n', '\n'));
const entrySnippets = new Map(entrySources.flatMap(source =>
  [...source.matchAll(/<!-- snippet: ([\w-]+) -->\s*```js\n([\s\S]*?)```/g)]
    .map(match => [match[1], match[2]])
));
const discoverMedia = new AsyncFunction('ALL_TOOLS', 'text', entrySnippets.get('media-entry-discovery'));
const probeMedia = new AsyncFunction('ALL_TOOLS', 'tools', 'text', entrySnippets.get('media-entry-capabilities'));
assert.ok(entrySnippets.has('media-entry-discovery') && entrySnippets.has('media-entry-capabilities'));
const mediaCatalog = ['capabilities', 'refresh_models', 'prepare', 'submit', 'check', 'save']
  .map(name => ({ name: `mcp__media_generation__media_${name}`, description: '统一媒体工具' }));
const discovered = [];
await discoverMedia([...mediaCatalog, {name: 'mcp__cua_repl__getState', description: 'Applications and browsers'}], value => discovered.push(value));
assert.deepEqual(discovered, [mediaCatalog]);
const currentName = 'mcp__observed_server__media_capabilities';
const currentCatalog = [{name: currentName, description: '只读能力查询'}];
const capabilityState = {
  success: true, api_key_configured: true, base_url_configured: true,
  gateway_catalog: {status: 'not_checked'},
  providers: {'image-2': {available: true, configured: true}, 'grok-video': {available: true, configured: true}},
  secret: 'must-not-be-printed',
};
let capabilityCalls = 0;
const runProbe = async (catalog, reply) => {
  const rows = [];
  const toolMap = new Proxy({}, { get: (_, name) => {
    assert.equal(name, currentName, 'Only the observed read-only capability tool may be called');
    return async args => { assert.deepEqual(args, {}); capabilityCalls++; return reply; };
  }});
  await probeMedia(catalog, toolMap, row => rows.push(row));
  assert.equal(rows.length, 1);
  return rows[0];
};
const contentReply = {content: [{type: 'text', text: JSON.stringify(capabilityState)}], isError: false};
const ready = await runProbe(currentCatalog, contentReply);
assert.equal(ready.success, true);
assert.equal(ready.catalogStatus, 'not_checked');
assert.equal(ready.providers['grok-video'].configured, true);
assert.equal(ready.providers['seedance-video'].configured, null);
assert.ok(!JSON.stringify(ready).includes('must-not-be-printed'));
assert.deepEqual(await runProbe(currentCatalog, {structuredContent: capabilityState}), ready);
const errorState = await runProbe(currentCatalog, {structuredContent: {success: false}, isError: true});
assert.equal(errorState.toolError, true);
assert.equal(errorState.success, false);
const callsBeforeMissing = capabilityCalls;
assert.equal((await runProbe([], contentReply)).status, 'not_found_in_current_catalog');
assert.equal((await runProbe([...currentCatalog, {name: 'mcp__second__media_capabilities'}], contentReply)).status, 'ambiguous');
assert.equal(capabilityCalls, callsBeforeMissing);
await assert.rejects(runProbe(currentCatalog, {content: [{type: 'text', text: 'not JSON'}]}), SyntaxError);
console.log('PASS: shipped media entry discovers nested tools, calls only the observed capabilities tool, parses content/structured replies, preserves errors and unknown state, and never substitutes CUA or generation calls.');
