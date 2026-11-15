// Static packaging contracts. These do not execute PowerShell or a Windows EXE.
import test from 'node:test';
import { Buffer } from 'node:buffer';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = relative => readFileSync(path.join(root, relative), 'utf8');
const resource = 'src-tauri/resources/ccswitch';
const manifest = JSON.parse(read(`${resource}/manifest.json`));
const commit = '85ad4969b3399500baaea9d46b1e08f87aba80e1';
const patch = 'source-patches/ccswitch-usb-product-85ad4969.patch';
const marker = 'CODEX_USB_V4_FIELD_WRITE_V1';
const sha = bytes => createHash('sha256').update(bytes).digest('hex').toUpperCase();

test('new source is pinned and existing binary metadata stays truthful', () => {
  assert.equal(manifest.sourceVersion, '4.0.20');
  assert.equal(manifest.sourceCommit, commit);
  assert.equal(manifest.sourcePatch, patch);
  const exe = readFileSync(path.join(root, resource, 'CodexSwitch.exe'));
  assert.equal(sha(exe), manifest.artifact.sha256.toUpperCase());
  if (manifest.artifact.buildStatus === 'built') {
    assert.equal(manifest.artifact.sourceVersion, manifest.sourceVersion);
    assert.equal(manifest.artifact.sourceCommit, manifest.sourceCommit);
    assert.ok(exe.includes(Buffer.from(marker)));
  } else {
    assert.equal(manifest.artifact.buildStatus, 'pending-rebuild');
    assert.equal(manifest.artifact.sourceVersion, '3.14.1');
    assert.notEqual(manifest.artifact.sourceCommit, commit);
    assert.ok(!exe.includes(Buffer.from(marker)), 'old EXE must not claim new field writer');
  }
});

test('final patch hash is checked or explicitly blocked pending source export', () => {
  const patchPath = path.join(root, resource, patch);
  if (existsSync(patchPath)) {
    assert.equal(sha(readFileSync(patchPath)), manifest.sourcePatchSha256);
  } else {
    assert.equal(manifest.artifact.buildStatus, 'pending-rebuild');
    assert.equal(manifest.sourcePatchSha256, 'PENDING_SOURCE_PATCH_EXPORT');
  }
  const builder = read('tools/build-ccswitch-with-env-patch.ps1');
  assert.ok(builder.includes('$actualSourcePatchHash -ne ([string]$sourceManifest.sourcePatchSha256)'));
});

test('all publication gates require the actual new field-writer marker', () => {
  for (const file of [
    'tools/build-ccswitch-with-env-patch.ps1',
    'tools/build-installer.ps1',
    'tools/verify-oneclick-usb.ps1',
    `${resource}/verify-product.ps1`,
  ]) {
    const source = read(file);
    assert.ok(source.includes(marker), `${file} must reject stale EXE`);
    assert.ok(source.includes('CODEX_USB_FULL_IMPORT_V1'), `${file} must reject missing full import`);
    assert.ok(!source.includes('CODEX_USB_MCP_PRESERVATION_V1'), `${file} must not require obsolete 3.x overlay`);
    assert.ok(!source.includes('CODEX_USB_MCP_SYNC_FAILURE_RECOVERY_V1'));
  }
});

test('release verifier rejects pending or mismatched artifact provenance', () => {
  const verifier = read(`${resource}/verify-product.ps1`);
  assert.ok(verifier.includes('$manifest.artifact.buildStatus -eq "built"'));
  assert.ok(verifier.includes('$manifest.artifact.sourceCommit -eq $manifest.sourceCommit'));
  assert.ok(verifier.includes('$manifest.artifact.sourceVersion -eq $manifest.sourceVersion'));
  assert.ok(verifier.includes(commit));
  assert.ok(verifier.includes('$sourcePatchHash -eq ([string]$manifest.sourcePatchSha256)'));
});

test('builder cannot mutate source after exact patch verification or reuse a skipped build', () => {
  const builder = read('tools/build-ccswitch-with-env-patch.ps1');
  assert.ok(builder.includes('if ($SkipBuild) { throw'));
  assert.ok(!builder.includes('$updatedContent = $codexContent -replace'));
  assert.ok(!builder.includes('Set-Content -LiteralPath $codexAdapterPath'));
  assert.ok(builder.includes('pnpm.cmd install --frozen-lockfile'));
  assert.ok(builder.includes('pnpm.cmd run typecheck'));
  assert.ok(builder.includes('pnpm.cmd exec vitest run'));
  assert.ok(builder.includes('tests/utils/modelAliases.test.ts'));
  assert.ok(builder.includes('pnpm.cmd tauri build --no-bundle --ci'));
  assert.ok(builder.includes('cargo check --locked'));
  assert.ok(builder.includes('status --porcelain --untracked-files=all |'));
  assert.ok(builder.includes('cargo test --locked --lib $test.Filter -- --test-threads=1'));
  assert.ok(builder.includes('&projection\\.provider'));
  assert.ok(!builder.includes('-SkipTests -UsbPolicy DevelopmentAutoEnroll'));
});

test('installer stages source patch with executable and validates staged product', () => {
  const installer = read('tools/build-installer.ps1');
  assert.ok(installer.includes('$sourcePatch = Join-Path $CcswitchArtifactDir $patchRelative'));
  assert.ok(installer.includes('Copy-Item -LiteralPath $sourcePatch -Destination $destinationPatch'));
  assert.ok(installer.includes('-ArtifactDir $ccswitchResourceDir -SkipNetwork'));
});

test('layout refuses mixed binaries before propagating source metadata to both copies', () => {
  const layout = read('tools/build-oneclick-usb-layout.ps1');
  assert.ok(layout.includes('@("resources\\ccswitch", "ccswitch")'));
  assert.ok(layout.includes('Hash -ine $canonicalProduct.artifact.sha256'));
  assert.ok(layout.includes('@("manifest.json", "verify-product.ps1", "SOURCE-NOTICE.txt", $canonicalPatchRelative)'));
  assert.ok(layout.includes('-ArtifactDir $destinationDir -SkipNetwork'));
  const verifier = read('tools/verify-oneclick-usb.ps1');
  assert.ok(verifier.includes('& $embeddedVerifier -ArtifactDir (Join-Path $root "resources\\ccswitch") -SkipNetwork'));
});
