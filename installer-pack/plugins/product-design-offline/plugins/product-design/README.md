# Product Design

Turn early product ideas, live URLs, and static screenshots into prototypes teams can review, refine, and carry forward.

## When to use this plugin

Use Product Design when you want to understand an existing experience, explore a new direction, or create a prototype that people can interact with and review. You can start from a written brief, screenshots, a design image, a live URL, an existing prototype, or product references you have saved for future work, such as product URLs, Figma files, screenshots, codebase paths, or design tokens.

## Get started

Try asking:

`@Product Design Help me get started`

The plugin will walk you through how to save useful product context such as product URLs, relevant Figma files, screenshots, codebase paths, Storybook links, tokens, and brand assets.

Already have a task in mind? Skip setup and ask for the workflow you need directly.

## Example workflows

| Workflow | Try this | Skill | Result |
| --- | --- | --- | --- |
| Prototype a new idea | `Build a clickable prototype for this product idea` | `ideate` → `image-to-code` | Three visual directions to choose from, followed by a runnable prototype based on your selection |
| Recreate a live surface | `Clone this URL into an editable prototype` | `url-to-code` | An editable local prototype grounded in the visible source |
| Implement a selected design | `Turn this selected mockup into a responsive prototype` | `image-to-code` | A faithful, interactive implementation of the design you selected |
| Explore visual directions | `Turn this product idea into three visual directions` | `ideate` | Distinct concepts you can compare before choosing what to build |
| Redesign an existing screen | `Redesign this settings screen and show me three directions before building` | `ideate` → `image-to-code` | Three grounded redesign options, followed by a prototype only after you choose a direction |
| Audit a product flow | `Audit this onboarding flow and identify the highest-impact UX and accessibility issues` | `audit` | Screenshot-backed findings tied to the steps in the experience |
| Research user friction | `Research the biggest UX problems users are reporting for this product` | `research` | A source-grounded view of user pain, severity, confidence, and product opportunities |
| Critique a prototype | `Critique this prototype and identify the highest-impact design issues` | `audit` | An evidence-grounded UX, design, and accessibility review |
| Share a prototype | `Share this prototype with the team` | `share` | A working shareable URL when a hosting tool is connected and deployment succeeds; otherwise a local project |

### Tip: Refine a prototype with Annotations

When the current Codex browser exposes the Annotations tool, use it to point at exact elements to change. If it is unavailable, describe the requested changes in chat or share a screenshot.

## Integrations

Product Design can use available tools when they are connected:

| Tool | What it unlocks |
| --- | --- |
| In-app `@Browser` | Capturing and inspecting live product surfaces when Codex exposes the browser; otherwise provide screenshots or files |
| Figma MCP | Optional remote integration enabled from the installer's Offline Components screen; it requires network, OAuth, a Figma account, and file access. Exported screenshots/PDFs/tokens work as offline references |
| `media_generation` MCP | Generating visual alternatives and assets through the configured upstream, with explicit approval for each prepared request |
| Local prototype runtime | Node.js and both Windows x64 starter dependency trees are prebundled and installed by Skill Hub; newly scaffolded projects build without npm or internet access |
| Sites, Vercel, or another hosting tool | Publishing only if a deployment tool is connected. No public hosting account or credential is bundled |

You can still begin with a written brief, screenshots, image files, URLs, local prototypes, or saved design context when a connected tool is not needed.
