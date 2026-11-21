const { execFile } = require('node:child_process');
const path = require('node:path');
const { createRequire } = require('node:module');
const { promisify } = require('node:util');

const run = promisify(execFile);
const [packageRoot, domain, allowedTools] = process.argv.slice(2);
const fromPackage = createRequire(path.join(packageRoot, 'package.json'));
const { LarkMcpTool, TokenMode } = fromPackage('./dist/mcp-tool');
const { DefaultCache } = fromPackage('@larksuiteoapi/node-sdk');
const { oapiHttpInstance } = fromPackage('./dist/utils/http-instance');
const { McpServer } = fromPackage('@modelcontextprotocol/sdk/server/mcp.js');
const { StdioServerTransport } = fromPackage('@modelcontextprotocol/sdk/server/stdio.js');

const setupEntry = process.env.CODEX_USB_FEISHU_SETUP_ENTRY;
const powershell = path.join(process.env.SystemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
const readSavedCredentials = Buffer.from(`
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
@{
    appId = [Environment]::GetEnvironmentVariable('FEISHU_APP_ID', 'User')
    appSecret = [Environment]::GetEnvironmentVariable('FEISHU_APP_SECRET', 'User')
} | ConvertTo-Json -Compress
`, 'utf16le').toString('base64');

const options = {
  domain,
  tokenMode: TokenMode.TENANT_ACCESS_TOKEN,
  toolsOptions: { allowTools: allowedTools.split(','), language: 'zh' },
  httpInstance: oapiHttpInstance,
  logger: { warn() {}, error() {}, debug() {}, info() {}, trace() {} },
};
const names = { toolNameCase: 'dot' };
const server = new McpServer({ name: 'Feishu/Lark MCP Server', version: '0.5.1' });
const response = (message, isError = false) => ({ isError, content: [{ type: 'text', text: message }] });
let connection;

async function readCredentials() {
  const { stdout } = await run(powershell, [
    '-NoProfile', '-NonInteractive', '-EncodedCommand', readSavedCredentials,
  ], { windowsHide: true });
  const saved = JSON.parse(stdout);
  if (saved.appId != null || saved.appSecret != null) return saved;
  // Explicit process credentials remain supported by the standalone MCP and
  // the installer. A saved pair takes precedence over inherited values.
  return { appId: process.env.FEISHU_APP_ID, appSecret: process.env.FEISHU_APP_SECRET };
}

function toolHandlers(credentials) {
  if (connection?.appId === credentials.appId && connection.appSecret === credentials.appSecret) {
    return connection.handlers;
  }
  const handlers = new Map();
  // The SDK's shared cache is keyed by App ID, which can survive a Secret change.
  new LarkMcpTool({ ...options, ...credentials, cache: new DefaultCache() }).registerMcpServer({
    tool(name, _description, _schema, handler) { handlers.set(name, handler); },
  }, names);
  connection = { ...credentials, handlers };
  return handlers;
}

new LarkMcpTool(options).registerMcpServer({
  tool(name, description, schema) {
    server.tool(name, description, schema, async (args, extra) => {
      let credentials;
      try {
        credentials = await readCredentials();
      } catch {
        return response('无法读取本机飞书凭据。请在管理器「连接与扩展 → 飞书」检查连接。', true);
      }
      const { appId, appSecret } = credentials;
      if (!/^cli_[A-Za-z0-9_-]+$/.test(appId) || !appSecret || appSecret.length < 8) {
        return response('飞书已启用，待连接。请先使用飞书 connect 工具打开本机表单，再填写并保存。', true);
      }
      return toolHandlers(credentials).get(name)(args, extra);
    });
  },
}, names);

server.registerTool('connect', {
    title: '连接飞书',
    description: '用户要求连接飞书时，打开管理器「连接与扩展」的本机凭据表单。无参数。请用户在本机密码框输入 App Secret，保存后可在当前会话继续。',
    annotations: { readOnlyHint: true },
  },
  async () => {
    if (!setupEntry) {
      return response('请在管理器「连接与扩展 → 飞书」填写并保存凭据。', true);
    }
    try {
      await run(setupEntry, ['--connect-feishu'], { windowsHide: true });
      return response('已请求打开本机飞书表单。请在表单里填写并保存，然后在当前会话继续。');
    } catch {
      return response('本机飞书表单未打开。请在管理器「连接与扩展 → 飞书」打开表单。', true);
    }
  });

server.connect(new StdioServerTransport()).catch(() => {
  console.error('Could not start Feishu MCP stdio.');
  process.exitCode = 1;
});
