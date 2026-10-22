---
name: ideate
description: "Generate image-based alternatives, remixes, or new design directions from a Product Design brief. Use when the user asks for design variants, visual exploration, remixes, or image-generated approaches from provided context."
---

# Ideate

You're tasked with generating design concepts for a user's idea.

Follow the shared Product Design routing guidance in [$index](../index/SKILL.md).

## Critical Overrides

- Refer to the Plugin router [$index](../index/SKILL.md) before proceeding.
- Follow [$critical-overrides](../../references/critical-overrides.md).

## User Context

Before starting, load [$user-context](../user-context/SKILL.md) and run its preflight script when local shell access is available.

Use the USB Offline Runtime Adapter in [$index](../index/SKILL.md): prepare visual concepts with the configured `media_generation` MCP and attach usable local image references with `reference_media` when supported. Never send a paid generation until the user approves the exact prepared request.

Do not inspect every saved reference. Inspect only what the current task needs.

## Workflow

Do not generate images until `$get-context` has satisfied the minimum required design brief.

Before generating images:

1. Understand the brief.

- Identify the target: component, screen, feature/workflow, or broad product idea.
- Identify the intended user, product surface, and goal.
- Preserve hard constraints from the user.
- Run `get-context` if the minimum required design brief isn't satisfied.

2. Resolve context.

- Use provided files, screenshots, links, and visible references.
- In a local workspace, look for nearby design documentation and other local visual context.
- Check likely design context folders such as `user-context`, `storybook/`, `.storybook/`, `design-system/`, `design-systems/`, `tokens/`, `components/`, `app/`, and generated prototype roots.
- In an existing project, look for existing product screenshots, similar flows, Storybook captures, design tokens, and component references before generating. Ask if the user can provide example screens similar to the one they are building if the existing app isn't accessible. Ensure you add design language and tokens to the media-generation prompt.

3. Inspect references directly.

- Look at screenshots, images, Figma frames, app surfaces, or other visual references before generating.
- Do not infer from filenames alone.
- If a named local path or reference is not visible, stop and ask the user to confirm the path, upload the file, start the local app, or point to the correct workspace.

4. Decide the variation mode.

- If useful local design context exists and the user has not asked for a new style, stay within that existing direction.
- If no useful design context exists, or the user asks for broad exploration, vary both concept and visual system.
- For a specific component or existing surface, vary structure, interaction, hierarchy, and emphasis before varying brand style.
- For a broad product idea, explore three meaningfully different product directions.

5. Choose an output shape supported by the configured media model before preparing the generation request.

- Pick the dimensions that best match the user's request and any provided visual reference.
- Mobile app: `390 x 844`.
- Tablet app: `834 x 1194`.
- Desktop app, dashboard, admin, or SaaS: `1440 x 1024`.
- Landing or marketing page: `1440` wide and scrollable.
- Modal, panel, widget, or component: natural container size.
- Provided screenshot, Figma frame, mockup, or reference image: match its dimensions and aspect ratio when the user wants to continue from that visual.
- Avoid crowding. Make the design fit the chosen dimensions cleanly, with realistic spacing, readable type, and no clipped content.
- Use a supported output `size` from `media_prepare` for `image-2`; do not pass unsupported dimensions. Choose the closest supported aspect ratio to the reference.

6. Check for access gaps.

- If a connector, reference, or file cannot be accessed because of auth, permissions, expired login, missing scope, suspiciously empty results, or unavailable local state, stop.
- Name the gap clearly and ask whether to troubleshoot access or continue without that source.
- Do not generate images while silently ignoring a named reference.

7. Attach images and mocks provided by the user to `media_prepare` as `reference_media` when the MCP accepts them. If a reference cannot be attached, say so and ask whether to continue without it.

8. Generate 3 independent options that have distinct information hierarchy, layout strategy, interaction model, or product framing.

Rules you must follow:

- Use `media_generation` MCP tools for generation. First call `media_capabilities`, then prepare each distinct concept with `media_prepare` using the configured image provider and `n: 1`.
- Show the exact prepared model, prompt, and parameters and wait for explicit user approval before each `media_submit`. Each submit may incur a charge. Never submit an unapproved request or automatically retry an unknown result.
- Generate three independent images unless the user chooses another count. Make one request per concept; never put multiple concepts into one image.
- Give each direction a distinct, descriptive name before generation, but do not place option numbers in the prompt. Generate sequentially and number the returned images in the order they appear in the thread, not by planned order.
- After all results return, bind each visible option number to the result in that displayed order. Do not name or describe the options in the final selection message.
- Attach provided screenshots, files, app captures, exported Figma references, and visual source material as moodboard inspiration when the MCP accepts the actual files.
- Attach existing product screenshots, similar flows, Storybook captures, design tokens, and component references as grounding material when available.
- When mock data includes dates or time-sensitive information, resolve the exact current date and include it in every media-generation prompt. Derive visible dates from that anchor; preserve dates required by the user or source design.
- If a screenshot, image, or visual file is available, attach the actual file to `media_prepare` as a reference when supported. Do not rely only on a text description.
- Only claim a visual reference was attached if the prepared request actually includes it.
- If you cannot attach the image, say that clearly and ask whether to continue with text-only direction.
- Preserve hard constraints from the brief in every image.
- After generating options, stop for the user's selection before any build work begins.
- When the user later selects option `N`, resolve it against the Nth displayed generated-image result from the most recent ideation set, not the original planned concept order. If the exact displayed result cannot be resolved, do not build from a guess; ask the user to name the concept or reattach/select the image.
- The selected option is the visual target for `$image-to-code`.

## Feedback Loop

If the user gives feedback after seeing options, generate revised options with that feedback.

If the user selects an option and gives feedback, generate a revised option with that feedback before build.

If the user likes parts of more than one option, combine those choices into a new media-generation request and show the result before build.

## Media Generation Prompt

Adapt this prompt to the current design brief and prepare it with the configured `media_generation` image provider. Follow the approval and no-retry rules above before submitting:

```text
Create realistic, production-quality UI designs with clear hierarchy, strong typography, intentional imagery, and purposeful spacing.

Design a focused primary screen, not a feature inventory. The product may support many workflows, but this frame should show the hero use case, one clear primary action, and only one or two supporting actions or content areas. Do not add cards, panels, tabs, badges, metrics, filters, or navigation items merely to advertise every feature. Let the rest of the product exist off-screen. Prefer strong hierarchy and generous whitespace; if the screen feels crammed, remove UI.

### Target Dimensions

Pick the dimensions that best match the user's request and any provided visual reference.

Default to a desktop web-app frame unless the user or reference clearly calls for mobile, tablet, or another format.

 - Mobile app: `390 x 844`
 - Tablet app: `834 x 1194`
 - Desktop app, dashboard, admin, or SaaS: `1440 x 1024`
 - Landing or marketing page: `1440` wide and scrollable
 - Modal, panel, widget, or component: natural container size
 - Provided screenshot, Figma frame, mockup, or reference image: match its dimensions and aspect ratio when the user wants to continue from that visual

Use a natural viewport ratio for the intended surface. Never stretch, squash, or warp the generated screen, imagery, typography, or UI elements to fill the canvas. If the composition does not fit naturally, recompose or simplify the layout instead.

Avoid crowding. Make the design fit the chosen dimensions cleanly, with realistic spacing, readable type, and no clipped content.

### Layout

When deciding how to lay elements out on the page, this should be your priority order for tools to differentiate sections:

1. Use spacing, grouping, alignment, typography, and hierarchy on the same product surface.
2. Use simple dividers or row separators.
3. Use a subtle surface tint only when the base surface is not enough.
4. Use borders only when separation still is not clear.
5. Use shadows/elevation last, and sparingly.

Don'ts:
 - Do not default to a centered "app card" (the whole UI is in a card on the page) on top of a contrasting page background. Use the base page surface first unless the source product or user explicitly asks for a contained app panel.
 - Do not put cards inside cards. Do not make every major section a card. Do not make each list item its own card unless each item is truly a standalone object. A normal list should usually read as one grouped surface with lightweight row separation.
 - Do not make up extraneous features. Add only the things essential to accomplish what the prototype's goal is. Don't make up more features just to fill out a UI.

### Typography

 - Anchor UI typography to readable product sizes. Body text should usually sit between 14px and 16px, with the rest of the type scale built around that baseline.
 - Keep long-form text to a comfortable line length, generally no more than 65 characters per line.
 - Use no more than 2 fonts in a UI. Prefer fonts already bundled with the project or installed on the user's computer. Do not fetch fonts from Google Fonts or another external CDN in the USB offline workflow. If no suitable font is available, use a readable system-font stack.

### Presentation

 - For mobile app concepts, output app content only. Do not include a device bezel, phone body, notch, Dynamic Island, OS status bar, clock, signal or battery indicators, home indicator, browser chrome, rounded device mask, or device shadow.
 - Do not put multiple ideas into a single image generation.
 - Vary each idea as much as possible while adhering to the constraints given entirely.

### Data Freshness

When the design includes dates or time-sensitive mock data, use the supplied current date as the anchor. Weekly views must show the real containing week with correct weekday/date pairs. Feeds, charts, notifications, and recent activity must use plausible chronological dates relative to today. Mark today when useful. Preserve dates required by the brief or source design.
```

## Output

Wait until all approved media-generation requests have returned before sending the final message that asks the user to choose.

Do not send the final selection message until every requested generated image is visible exactly once in the main chat.

If fewer outputs are visible than requested, do not resubmit automatically. Explain which approved request is missing, check its existing status with `media_check` when possible, and ask the user before preparing a new paid request.

After each approved request completes, verify the returned local image file and show or attach the actual image when the conversation surface supports it. Do not ask the user to choose from missing or uninspected outputs. Number completed generated images in the order they are shown:

- First displayed generated image = Option 1
- Second displayed generated image = Option 2
- Third displayed generated image = Option 3

Ignore the planned concept order, original request chain, request order, `Promise.all` result order, batch order, array indexes, retry order, and tool submission order.

Do not name or describe the options. For the default three images, send only:

`Which option should I build: 1, 2, or 3? Or tell me what you'd like to refine or personalize first.`

Adjust the numbers only if the user requested a different count.

If the user chooses a number, acknowledge the chosen option before routing to `$image-to-code`, for example: `Building option 2!` Do not ask for confirmation when the mapping is clear.

Done means the requested number of independent images have been generated and the user has been asked to select one.
