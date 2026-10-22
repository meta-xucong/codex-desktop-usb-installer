import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import jsxA11y from "eslint-plugin-jsx-a11y";
import tseslint from "typescript-eslint";

// Lint gate for the frontend: catch hook-dependency mistakes and accessibility
// regressions the way the design review surfaced them by hand. Type-aware rules
// are intentionally NOT enabled — `tsc --noEmit` already owns type correctness
// in the same CI job, and a second type-checking pass would just double the
// wall-clock for no new signal.
export default tseslint.config(
  {
    // Frontend app only. The Node-side workspaces (build scripts, the
    // Cloudflare worker, the website subproject) have their own tooling and
    // Node globals — linting them here would just spew no-undef.
    ignores: [
      "dist/",
      "node_modules/",
      "src-tauri/",
      // The portable capability pack contains independent plugin templates
      // and runtime scripts; it is validated by its own install/contract tests.
      "installer-pack/",
      // Rust crates; includes the injected renderer runtime template
      // (crates/codex-theme-engine/src/runtime/), which is browser-side JS
      // with substitution placeholders — not part of this app bundle.
      "crates/",
      "website/",
      "cloudflare/",
      // CCSwitch is a separately built upstream workspace with its own ESLint
      // policy and generated dist tree; it is not part of this manager bundle.
      "CCSwitch/",
      "release/",
      // Historical source snapshots and audit transcripts are evidence, not
      // live app sources. Lint the checked-out product tree, not those copies.
      ".codex-audit/",
      // Temporary installer/MCP/model-catalog validation trees can contain
      // extracted third-party runtimes; they are evidence, not app sources.
      ".tmp-*/**",
      // Nested release worktrees are separate checked-out repositories; lint
      // only the active product tree, not their generated/vendor contents.
      ".release-worktree-*/**",
      // Retained local test evidence is likewise outside the product source
      // tree; copied third-party scripts here must not fail the renderer lint.
      ".test-tmp/**",
      "tmp/**",
      // Local release/capability simulations contain bundled third-party
      // runtimes and extracted vendor JavaScript. They are test evidence, not
      // renderer sources, and must not make the product lint gate depend on a
      // particular audit run being present in the workspace.
      "_audit-artifacts/",
      "_codex-oneclick-test-artifacts/",
      "scripts/",
      "docs/",
      // Local reverse-engineering/probe fixtures are diagnostic artifacts, not
      // renderer sources. They intentionally contain CommonJS/Node snippets
      // and large extracted bundles with a separate runtime policy.
      ".analysis/",
      ".workbuddy/",
      "*.config.*",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: {
      "react-hooks": reactHooks,
      "jsx-a11y": jsxA11y,
    },
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: { ...globals.browser },
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    rules: {
      ...jsxA11y.flatConfigs.recommended.rules,
      // Classic hook rules only — NOT the React Compiler suite that
      // eslint-plugin-react-hooks@7 bundles into `recommended`, which flags a
      // lot of deliberate patterns here (setState-in-effect for the count-up
      // easing, the manually-memoized settings saver). Type correctness is
      // owned by `tsc` in the same job.
      // exhaustive-deps stays at its conventional `warn` severity, but the lint
      // script runs with --max-warnings=0, so in CI it (and any other warning)
      // blocks. Intentional dep omissions must be an explicit
      // `// eslint-disable-next-line react-hooks/exhaustive-deps` with a reason.
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
      // The codebase leans on the leading-underscore convention for deliberate
      // discards (e.g. `_resolve` in a Promise executor).
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_", caughtErrors: "none" },
      ],
    },
  },
  {
    // The setup file's triple-slash type reference is intentional (it pulls in
    // the jest-dom matcher types globally); an import wouldn't register them.
    files: ["vitest.setup.ts"],
    rules: { "@typescript-eslint/triple-slash-reference": "off" },
  },
  {
    // Relay contract tests execute under Node and are intentionally kept next
    // to the deployment fixtures rather than under the renderer source tree.
    files: ["tools/skill-hub-relay-sync.test.mjs"],
    languageOptions: { globals: { ...globals.node } },
  },
  {
    // Test files run under Vitest globals-free (imports describe/it/expect), but
    // use jsdom + node timers; allow both realms.
    files: ["src/**/*.test.{ts,tsx}", "vitest.setup.ts"],
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
    },
  },
  {
    // This small browser script is embedded by Go's //go:embed directive and
    // served by the VPS license admin console. Keep it in the repository lint
    // gate with the correct browser globals and deliberate discard convention.
    files: ["server/usb-license-revocation/internal/revocation/adminweb/**/*.js"],
    languageOptions: { globals: { ...globals.browser } },
    rules: {
      "@typescript-eslint/no-unused-vars": [
        "error",
        {
          argsIgnorePattern: "^_",
          varsIgnorePattern: "^_",
          caughtErrors: "all",
          caughtErrorsIgnorePattern: "^_",
        },
      ],
    },
  },
);
