// Optional Windows regression test; intentionally not part of any build gate.
import assert from 'node:assert/strict';
import { Buffer } from 'node:buffer';
import process from 'node:process';
import { TextDecoder } from 'node:util';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, URL } from 'node:url';
import test from 'node:test';

const installer = process.argv[2] || fileURLToPath(new URL('../installer-pack/install-product-design-plugin.ps1', import.meta.url));
const quote = (s) => `'${s.replaceAll("'", "''")}'`;
const sha256 = (p) => createHash('sha256').update(readFileSync(p)).digest('hex');
function ps(script) {
  const result = spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')], { windowsHide: true });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr?.toString());
  return result.stdout;
}
function json(file, value) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, JSON.stringify(value));
}

test('Product Design installer returns UTF-8 JSON for Unicode paths and repeat installs', { skip: process.platform !== 'win32' }, () => {
  const root = mkdtempSync(path.join(tmpdir(), 'codex-product-design-encoding-'));
  try {
    const pack = path.join(root, '安装包');
    const bundle = path.join(pack, 'plugins/product-design-offline');
    const plugin = path.join(bundle, 'plugins/product-design');
    const dependencyRoot = path.join(root, 'dependencies');
    const templates = {};
    json(path.join(plugin, '.codex-plugin/plugin.json'), { name: 'product-design', version: '1.0.0' });
    json(path.join(bundle, '.agents/plugins/marketplace.json'), { name: 'usb-product-design-offline', plugins: [{ name: 'product-design' }] });
    for (let i = 0; i < 10; i++) {
      const skill = path.join(plugin, `skills/skill-${i}/SKILL.md`);
      mkdirSync(path.dirname(skill), { recursive: true });
      writeFileSync(skill, `---\nname: skill-${i}\ndescription: fixture\n---\n`);
    }
    for (const name of ['prototype', 'mobile-app']) {
      const template = path.join(plugin, 'templates', name);
      json(path.join(template, 'package.json'), { name });
      json(path.join(template, 'package-lock.json'), { name, lockfileVersion: 3 });
      const modules = path.join(dependencyRoot, name, 'node_modules');
      mkdirSync(modules, { recursive: true });
      writeFileSync(path.join(modules, 'fixture.txt'), 'offline test fixture');
      templates[name] = { packageJsonSha256: sha256(path.join(template, 'package.json')), packageLockSha256: sha256(path.join(template, 'package-lock.json')), nodeModulesFiles: 1 };
    }
    json(path.join(dependencyRoot, 'product-design-dependencies.json'), { schemaVersion: 1, pluginVersion: '1.0.0', platform: 'win32-x64', nodeVersion: '22.22.0', templates });
    const zip = path.join(pack, 'runtime/bundled/product-design-dependencies-win-x64.zip');
    mkdirSync(path.dirname(zip), { recursive: true });
    ps(`$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.IO.Compression.FileSystem; [IO.Compression.ZipFile]::CreateFromDirectory(${quote(dependencyRoot)}, ${quote(zip)})`);
    const script = path.join(pack, 'install-product-design-plugin.ps1');
    cpSync(installer, script);
    for (const user of ['ascii-user', '周']) {
      const home = path.join(root, user, 'AppData/Local/CodexAppManager/usb-codex-home');
      for (let attempt = 0; attempt < 2; attempt++) {
        // Emulate Windows PowerShell's legacy Chinese console code page.
        const output = ps(`[Console]::OutputEncoding=[Text.Encoding]::GetEncoding(936); & ${quote(script)} -PackRoot ${quote(pack)} -TargetCodexHome ${quote(home)}`);
        const text = new TextDecoder('utf-8', { fatal: true }).decode(output);
        const report = JSON.parse(text);
        assert.equal(report.status, 'ready');
        assert.equal(report.skillCount, 10);
        assert.equal(report.installedPath, path.join(home, 'plugins/cache/usb-product-design-offline/product-design/1.0.0'));
        assert.ok(report.runtimePath.startsWith(home + path.sep));
        const config = readFileSync(path.join(home, 'config.toml'), 'utf8');
        assert.ok(config.includes(user));
        assert.equal(config.split('# BEGIN CODEX USB OFFLINE PRODUCT DESIGN v1').length - 1, 1);
      }
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
