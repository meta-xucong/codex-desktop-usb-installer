---
name: safe-disk-operations
description: Safely inspect, clean, delete, move, uninstall, or reconfigure local files, applications, disks, and runtime configuration. Use when a request can remove data, overwrite user changes, cross filesystem boundaries, alter partitions or drive letters, uninstall software or services, clear caches, empty trash, or modify protected runtime JSON. Require exact scoped targets, perform read-only checks first, prefer recoverable actions, preserve unowned content, and stop when authorization or rollback safety is unclear.
---

# Safe Disk and System Operations

Apply this workflow to destructive or hard-to-reverse local system changes. Do not use it for ordinary arithmetic, accounting, investment
analysis, or other unrelated financial work.

## Classify the request

Treat these as in scope:

- deleting files or folders, batch cleanup, cache cleanup, or emptying trash;
- moving data across disks, merging folder trees, or replacing destination files;
- uninstalling applications, drivers, services, startup items, or system components;
- formatting, shrinking, extending, merging, or relabeling partitions and volumes;
- creating, replacing, or deleting protected runtime configuration such as `runtime-config.json`;
- recursive operations, commands with globs, or operations derived from computed paths.

Read-only inventory and diagnosis are allowed when relevant. A request to inspect usage, list a named directory, or explain risk does not
authorize cleanup, deletion, movement, uninstallation, or configuration writes.

## Safety gate

Before changing state:

1. Confirm that the user's request clearly authorizes the exact operation. If the target, scope, overwrite policy, or desired end state is
   ambiguous, stop and ask one concise question.
2. Resolve every source and destination to an absolute path. Reject an empty path, unresolved variable, wildcard-derived destructive target,
   filesystem root, user-profile root, workspace root, or another broader target than the user named.
3. Inspect the target read-only: verify existence, item type, size/count when useful, reparse points or junctions, ownership boundary, free
   space for moves, and destination conflicts.
4. Identify user-created or locally modified files. Never delete or overwrite them merely because a previous installer or manifest owns the
   surrounding directory.
5. Prefer a recoverable action: application uninstallers, operating-system storage tools, moving to trash/quarantine, backups, or copying and
   verifying before removing the source.
6. State the exact impact and rollback path before high-impact partition, format, service, driver, or protected-config changes. If reliable
   rollback is absent or the requested scope is unsafe, refuse that operation and offer a safer alternative.

## Files and directories

- Keep one shell end-to-end. On Windows, use native PowerShell file cmdlets with `-LiteralPath`; do not enumerate paths in PowerShell and pass
  them to `cmd.exe`, batch built-ins, or another shell for deletion or movement.
- Do not build destructive commands from string concatenation, command substitution, unresolved environment variables, or broad globs.
- For recursive deletion or movement, validate that every resolved target stays inside the user-named directory.
- When merging trees, define collision behavior first. Default to preserving the destination and reporting conflicts.
- For copy-then-delete migration, compare file counts and hashes for important data before removing the source.
- If an operation removes material data, report what was removed and whether recovery is available.

## Installed or generated content

Use an ownership manifest only as one input. Before overwrite or removal:

1. validate the manifest identity and allowed roots;
2. compare the current file hash with the recorded installed hash;
3. skip locally modified files;
4. skip a managed tree that contains unowned files unless the user explicitly names those files;
5. reject symbolic links or junctions that can redirect the operation outside the approved root.

Never replace an unowned same-name Skill, Agent, configuration file, or application directory just because `-Force` is available.

## Protected runtime configuration

For `runtime-config.json` or an equivalent runtime configuration:

1. read and parse the current file without changing it;
2. show the requested field-level before/after values and note service-restart impact;
3. create a timestamped backup beside the file when it exists;
4. apply only the authorized fields with a structured parser, not textual search-and-replace;
5. write to a temporary file, parse it, then atomically replace the target where supported;
6. restore the backup if validation or the immediate health check fails;
7. never put tokens or secrets into logs, diffs, backups committed to source control, or chat output.

If the user has not specified the exact field/value or the change could alter model, permission, credential, or filesystem scope beyond the
request, pause before writing.

## Partitions, formats, drivers, and services

These operations can make a machine unbootable or permanently destroy data. Perform only read-only diagnosis until all of the following are
known: exact device/volume identity, verified backup, requested final layout/state, encryption status, dependent services/applications, and a
credible recovery method. Require a separate explicit confirmation containing the resolved target and operation immediately before an
irreversible format, partition-table write, or system-critical driver/service removal.

Do not provide or execute a guessed `diskpart`, `diskutil`, filesystem format, registry deletion, or service-removal command.

## Completion report

Report:

- the exact paths, applications, services, or volumes inspected and changed;
- items skipped because they were unowned, modified, linked, ambiguous, or outside scope;
- verification performed after the change;
- backup, trash, quarantine, or other recovery location;
- any restart or manual follow-up still required.
