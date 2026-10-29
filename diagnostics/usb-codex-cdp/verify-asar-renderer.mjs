import fs from 'node:fs';
import crypto from 'node:crypto';
import { guardLocation, ipcCallReturnLocation } from './collector.mjs';

const EXPECTED_VERSION = '26.1016.62342';
const EXPECTED_RENDERER_SHA256 = '84d1c97245fec7be606fd1d263459d9a4c841836751c6ba5e9b348b69687332f';
const ASSET = 'webview/assets/app-initial-ad16eaadf81f.js';
const asarPath = process.argv[2];
if (!asarPath) throw new Error('usage: node verify-asar-renderer.mjs <app.asar>');
const bytes = fs.readFileSync(asarPath);
if (bytes.readUInt32LE(0) !== 4) throw new Error('unrecognized ASAR size pickle');
const headerSize = bytes.readUInt32LE(4);
const jsonSize = bytes.readUInt32LE(12);
const jsonStart = 16;
const dataStart = 8 + headerSize;
if (jsonStart + jsonSize > dataStart || dataStart > bytes.length) throw new Error('invalid ASAR header bounds');
const header = JSON.parse(bytes.subarray(jsonStart, jsonStart + jsonSize).toString('utf8'));
function entryAt(path) {
  let node = { files: header.files };
  for (const segment of path.split('/')) node = node.files?.[segment];
  if (!node || !Number.isSafeInteger(node.size) || !/^[0-9]+$/.test(String(node.offset))) throw new Error(`missing ASAR entry: ${path}`);
  const start = dataStart + Number(node.offset), end = start + node.size;
  if (end > bytes.length) throw new Error(`ASAR entry out of bounds: ${path}`);
  return bytes.subarray(start, end);
}
const pkg = JSON.parse(entryAt('package.json').toString('utf8'));
const renderer = entryAt(ASSET);
const rendererHash = crypto.createHash('sha256').update(renderer).digest('hex');
const source = renderer.toString('utf8');
const guard = guardLocation(source);
const ipcReturn = ipcCallReturnLocation(source);
const functionMarker = 'function nSu()';
const functionOffset = source.indexOf(functionMarker);
const result = {
  internalVersion: pkg.version,
  expectedInternalVersion: EXPECTED_VERSION,
  rendererPath: ASSET,
  rendererBytes: renderer.length,
  rendererSha256: rendererHash,
  expectedRendererSha256: EXPECTED_RENDERER_SHA256,
  nSuFunctionOffsetUtf16: functionOffset,
  guardExpressionCount: source.split('f||_e.isLoading||ve.isLoading||!p&&ve.reason===`statsig-disabled`||ye.isLoading||Ee.isLoading||Oe.isLoading||U===`loading`||L.status===`loading`||Je.status===`loading`||Ye.status===`loading`||(Ei.dispatchMessage').length - 1,
  guardBreakpointCandidate: guard,
  ipcEventCount: source.split('electron-desktop-features-changed').length - 1,
  ipcCallReturnCandidate: ipcReturn,
  semantics: { ipcPoint: 'after renderer dispatchMessage call returns; main-process acceptance is not observable here' },
};
console.log(JSON.stringify(result, null, 2));
if (pkg.version !== EXPECTED_VERSION || rendererHash !== EXPECTED_RENDERER_SHA256 || functionOffset < 0 || !guard || !ipcReturn
  || result.guardExpressionCount !== 1 || result.ipcEventCount !== 1) process.exitCode = 1;
