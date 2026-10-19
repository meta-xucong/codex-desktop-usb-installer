# External connection inventory

`mcp-servers.json` describes MCP servers, while `api-services.json` describes direct HTTP/API services.
`connection-fields.json` is the guided-configuration form contract. Real credentials and generated
Codex configuration stay outside this public pack. `start-feishu-mcp.ps1` and
`start-media-generation-mcp.ps1` launch local STDIO MCPs. The unified media server exposes image-2,
Grok image/video and Seedance behind one confirmation/submit/query/save contract. MiniMax is a reserved, disabled provider because its official API protocol and credentials differ;
`../manifest/media-generation-tools.json` for its safety contract.
