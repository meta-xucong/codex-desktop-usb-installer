# Local Prototype Preflight

Use this before creating a new local prototype.

- Keep the work self-contained in the new project folder.
- Plugin UI icons live in `../assets/`. Do not put prototype starter code or generated app assets there.
- Bundled Product Design starters live in `../templates/`.
- Use the default `prototype` template for web or desktop-like prototypes.
- Use `--template mobile-app` for mobile app prototypes.
- Create the app with the bootstrap script. Resolve the script path relative to this file, then run it with an absolute path:

```bash
node /absolute/path/to/plugins/product-design/scripts/bootstrap-prototype.mjs --dest /absolute/path/to/new-prototype
```

```bash
node /absolute/path/to/plugins/product-design/scripts/bootstrap-prototype.mjs --template mobile-app --dest /absolute/path/to/new-mobile-prototype
```

- On the USB edition, the bootstrap script copies the matching Windows x64 dependencies from the local Product Design runtime. Do not run `npm install` or `npm ci`, and do not access the npm registry.
- Confirm the bootstrap result reports `dependenciesReady: true`. If an existing generated project is missing `node_modules`, restore it without network access by running the installed plugin's bootstrap script with `--mode restore-dependencies --root <project-path> --template prototype` (or `mobile-app`). The script refuses to overwrite a non-empty `node_modules` folder or a project whose lockfile differs from the bundled template.
- If the offline runtime is missing or the project lockfile differs, stop and report that the Skill Hub dependency bundle must be repaired; do not silently fall back to international npm access.
- Once a mobile template is selected and dependencies are installed, start its preview immediately so the user can see the device frame while the screen is built. Keep the preview alive through implementation and QA.
- Both templates use ordinary Vite development for localhost and Work Mode preview. Do not hardcode `localhost` or `terminal.local` in app code; use relative URLs and same-origin requests.
- Both templates are Sites-ready. `npm run build` emits static client files under `dist/client`, the required Worker at `dist/server/index.js`, and metadata at `dist/.openai/hosting.json`. Run `npm run test:sites` before handing a verified project to Sites. Do not run `init-site.sh` or replace the Product Design project with a Vinext starter.


When using the `mobile-app` template, preserve its runtime shell. Do not replace `App` with a standalone page, and do not remove `PhoneFrame`, the iPhone / Pixel 10 device picker, `KeyboardProvider`, `MobileScroll`, `KeyboardDock`, `StatusBar`, `HomeIndicator`, the platform-specific iOS / Android bottom chrome, or the Pixel camera cutout unless the user explicitly asks to change the runtime. `FlowStack` is available for multi-screen flows, but simple single-screen prototypes can mount `MobileScroll` directly inside `KeyboardProvider`. Keep `StatusBar`, the iOS home indicator, and camera cutout as overlaid device chrome. The closed-keyboard Android app viewport reserves its navigation-bar region; the keyboard-open Android state continues using the keyboard asset's built-in IME navigation strip. Put iOS safe-area content padding on each app screen rather than on the scroll wrapper. `FlowScreen.footer` is also an overlay, so screens that use fixed bottom tabs or nav bars must add their own bottom content padding instead of relying on the flow shell to reserve space.

Build app-specific UI in `src/Prototype.tsx` and `src/prototype.css`. Treat `src/App.tsx`, `src/main.tsx`, `src/styles.css`, `src/mobile/`, `public/assets/iphone/`, `public/assets/android/`, `public/assets/status/`, `vite.config.ts`, `worker/index.js`, and `scripts/prepare-sites-build.mjs` as protected runtime files. Run `npm run check:runtime` before preview or handoff; restore the runtime if the check fails.

For Sites hosting, keep the mobile project intact. `npm run build` emits static client files under `dist/client`, the required Worker at `dist/server/index.js`, and metadata at `dist/.openai/hosting.json`. Run `npm run test:sites` before handing the verified project to Sites. Do not run `init-site.sh` or replace the mobile runtime with a Vinext starter.
