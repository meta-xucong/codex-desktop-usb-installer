# Critical Overrides

These rules override generic assistant defaults for Product Design work.

## USB Offline Runtime Adapter

In the USB offline edition, the runtime adapters documented in `skills/index/SKILL.md` override assumptions elsewhere in this plugin about connected services or built-in tools:

- Use the configured `media_generation` MCP for image generation and follow its approval, status-check, and no-automatic-retry rules. Do not direct users to built-in ImageGen.
- Use a browser only when the user-selected browser surface is exposed in Codex. If unavailable, use provided screenshots/files and state that live capture or visual verification could not be done. Do not refer to an unbundled Browser skill.
- Figma is an optional remote MCP configured through the installer's Offline Components screen; it requires network, OAuth, a Figma account, and file access. Otherwise use exported files or tokens.
- Treat hosting as unavailable until a deployment tool is actually connected. The USB Skill Hub bundles Node.js and both starter dependency trees for offline local builds; do not imply that Sites, Vercel, or publishing credentials are bundled.

## Product Design Owns The Sites-Ready Build

When Product Design is explicitly invoked, Product Design owns visual selection, template selection, and build initialization. Bundled templates support local preview and are Sites-compatible; publishing still requires a connected hosting tool. The availability or mention of `@Sites` does not activate `sites-building`.

- Do not invoke `sites-building`, run `init-site.sh`, or initialize a Vinext starter during Product Design ideation or implementation. Select the visual target first, then initialize the appropriate Product Design template.
- For a mobile visual target, initialize `mobile-app`. Never substitute a responsive webpage or replace its protected runtime.
- Preview and verify locally first when Node dependencies and a browser are available. Do not deploy to Sites unless the user explicitly asks to share, publish, or deploy and Sites is connected. When the user chooses an available Sites target, keep the verified Product Design project intact and hand it to `sites-hosting`; do not reinitialize or rebuild it in a Sites starter.

## Context

- When working inside an existing project or product, find similar flows, screens, components, and UX patterns first. Build on the product's existing design system. Do not reinvent the wheel. Look for style sheets, tokens, and other materials that constitute the design and adhere to them in your work.

## Saved User Context

- If `user-context.md` exists, use it by default.
- Use saved product URLs, Figma files, screenshots, reference images, codebase paths, Storybook, tokens, design systems, brand assets, component refs, browser preferences, and share targets to ground Product Design work.
- Ideation, prototypes, audits, clones, and critiques should match the saved product context unless the user asks for something different.
- When a workflow needs visual grounding, attach or include relevant saved screenshots, reference images, tokens, design language, and component references in the configured image-generation MCP, ideation, prototype, audit, and critique work.

## How to communicate

- Follow [communication-protocol](communication-protocol.md)

## Build Handoff

- After an app, prototype, clone, redesign, or image-to-code build, lead with the working prototype only when it has been opened and verified in an available preview surface. In ChatGPT Work Mode, use the cloud browser when available; in Codex Desktop, use the in-app browser when available. If no browser is exposed, report the preview as unverified and provide the local project path rather than claiming the prototype is open. Do not present internal service URLs as user-facing links. This handoff is not blocked by publishing setup.
- After that preview handoff, say: `I've finished building. Let me know if I can tighten anything up or build out more functionality.`
- Only in ChatGPT Desktop, add: `You can also suggest and make updates with the annotation tool.` Do not mention the annotation tool in ChatGPT Work Mode.
- Add one short share nudge only when a publishing tool is available or the user has asked about sharing. Check the current tools before naming `@Sites`, `@Vercel`, or another target. If none is connected, offer the local project without implying that public publishing is ready.
- Keep the wording plain and human.

## Explore vs. Design vs. Build

- Do not build from under-specified product context alone.
- Do not treat "try to fulfill first" as permission to skip source capture or design mock creation. Follow the workflows prescribed in this plugin as contracts.
- For URLs, capture and open a screenshot first. If the reference cannot be captured, opened, or attached, stop before generating options from prose only.
- Never invent a better first screen, landing page, hero, card style, icon set, image style, color palette, radius, spacing, or typography when cloning or matching a provided source. Match the source.
- Check the work like a senior designer. Look for broken layouts, cropped images, bad padding, bad margins, wrong font styles, wrong font weights, incorrect borders, and incorrect border radii.
- Screenshots are not QA by themselves. Put the reference image and the prototype screenshot together in the same comparison input, then judge the visible differences from that combined input. Use the same viewport and state, fix visible mismatches, then compare again.
- Bring the app or website's core experience to life. Navigation, links, tabs, menus, primary CTAs, and any inputs, filters, toggles, selections, forms, or visible states needed for the main task, conversion path, or user journey must work and use realistic mock data. Controls outside the core experience may be visual-only. Do not build new pages or routes unless the user asks for them.

## Browser user

- Only use the user's chosen browser. If you need to use the Playwright CLI or MCP directly, ask the user before proceeding.
- Provide URLs, screenshots, mocks, Figma files, or other visual sources, including detailed art direction to the configured image-generation MCP when generating designs and assets.

## Working with and making assets

- Never fake visible assets with ASCII, prose, text symbols, emoji, placeholder boxes, CSS art, div art, handcrafted SVGs, inline SVGs, or approximate code drawings. Use real source assets when available. Use the configured image-generation MCP for image assets when source assets are missing, following the USB adapter's approval rules. Use the closest matching icon library for icons.
- Work like a designer. Measure the component or section first, then create or place the asset to fit that slot. Match the needed dimensions, crop, subject, palette, and density. Do not lazily crop sprite sheets, stretch screenshots, or use images that do not fit seamlessly into the design.
