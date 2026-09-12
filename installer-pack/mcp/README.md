# External connection inventory

`mcp-servers.json` describes MCP servers, while `api-services.json` describes direct HTTP/API services.
`connection-fields.json` is the guided-configuration form contract. Real credentials and generated
Codex configuration stay outside this public pack. `start-feishu-mcp.ps1` is an offline-only,
redacting wrapper whose tool allowlist is locked by `../manifest/feishu-tools.json`.
