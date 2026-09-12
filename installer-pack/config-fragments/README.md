# Codex config fragments

These files are renderable, secret-free templates. Run `scripts/render-codex-config.ps1`, validate the
generated fragment, and merge only the selected blocks into the user's `config.toml`.

- `feishu-official-stdio.template.toml` invokes the local `start-feishu-mcp.ps1` wrapper and the locked,
  offline `@larksuiteoapi/lark-mcp@0.5.1` package. Codex forwards only the names of process environment
  variables (`FEISHU_APP_ID`, `FEISHU_APP_SECRET`, and only in user-token mode `FEISHU_USER_ACCESS_TOKEN`). No credential
  value is rendered into TOML, logs, dry-run output, or install records.
- The wrapper never uses `npx` and never downloads a package. It validates the locked package name, version,
  and `dist/cli.js` entrypoint, then invokes Node directly rather than an npm-generated `.cmd` shim. It refuses
  to start unless the local cache, Node >=20, required credentials, approved domain, and dot-case tool contract
  validate successfully.
- The upstream CLI still receives App Secret as a child-process argument. A trusted local process inspector
  may therefore observe it while the server runs; this is an upstream interface limitation, not a secret-store
  guarantee.
- `codex-skills-gating.template.toml` documents the `[[skills.config]]` shape used to keep unmet Skills disabled.
- `connections.template.json` is a configuration-UI input contract, not a secret store.

Never copy a template containing placeholders into an enabled configuration. Health-check the rendered
connection, then reload Codex.
