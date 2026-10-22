---
name: share
description: "Share a runnable prototype using the user's preferred deployment tool."
---

# Share

Prepare the user's runnable prototype for sharing. In the USB offline edition, the default is a locally previewable project; public publishing requires a connected hosting tool.

## Critical Overrides

- Refer to the Plugin router [$index](../index/SKILL.md) before proceeding.
- Follow [$critical-overrides](../../references/critical-overrides.md).

## User Context

Before starting, load [$user-context](../user-context/SKILL.md) and run its preflight script when local shell access is available.

Use saved product URLs, Figma files, screenshots, reference images, codebase paths, Storybook, tokens, design systems, brand assets, component refs, browser preferences, and share targets as grounding material when relevant.

Do not inspect every saved reference. Inspect only what the current task needs.

## Workflow

1. Confirm the prototype directory and whether the user wants local sharing or public deployment.
2. If the user invokes Product Design with @Sites, @Vercel, or another deployment tool, treat that as the selected hosting target.
3. In the USB offline edition, first check whether a deployment tool is actually connected. If none is available, do not ask the user to choose unavailable services: run or package the local project and explain that public publishing is unavailable in this setup.

4. If a deployment tool is connected and the user did not choose a target, ask one question:

> Where should I deploy this: @Sites, @Vercel, or another target?

5. Before using Sites for a fresh Product Design prototype, keep the existing project intact. Run `npm run build` and `npm run test:sites`; for `mobile-app`, run `npm run check:runtime` first. Confirm `dist/client/index.html`, `dist/server/index.js`, `dist/.openai/hosting.json`, and source `.openai/hosting.json` exist. Hand that verified project to `sites-hosting`. Do not invoke `sites-building`, run `init-site.sh`, or replace the Product Design runtime with a Vinext starter.
6. Use the selected deployment tool only when it is connected and available.
7. If the user explicitly requests public deployment but no hosting tool is connected, state that limitation and ask whether they want a local project package instead.
8. Run the deployment when possible. Do not give setup instructions if you can complete the deployment directly.
9. Return the shareable URL only after verifying the deployment is live.
10. State any misses or manual follow-up the user still needs to do.

## Rules

- Do not deploy before the user chooses or confirms the target.
- Do not claim the prototype is shared until you have a working URL.
- If the selected tool is not available, say that clearly and ask whether to use another target.
