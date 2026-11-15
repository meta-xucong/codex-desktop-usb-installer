"""Remove current-package table collisions before the existing TOML merger.

Uses the parser shipped in the pinned Python runtime to locate statements;
table-like text inside multiline values must never be treated as a header.
No configuration or credentials are printed.
"""
from __future__ import annotations

import os
from pathlib import Path
import sys
import tomllib
from tomllib import _parser as parser


def without_replaced_tables(existing: str, fragment: str) -> str:
    current = tomllib.loads(existing)
    generated = tomllib.loads(fragment)
    owned = {kind: set(generated.get(kind, {})) for kind in ("mcp_servers", "plugins")}
    skill_paths = {os.path.normcase(item["path"]) for item in generated.get("skills", {}).get("config", [])}
    skill_rows = iter(current.get("skills", {}).get("config", []))
    text = existing.replace("\r\n", "\n")
    out = parser.Output(parser.NestedDict(), parser.Flags())
    pos, start, header, remove = 0, 0, (), False
    spans = []
    markers = []
    while pos < len(text):
        pos = parser.skip_chars(text, pos, parser.TOML_WS)
        if pos == len(text):
            break
        char = text[pos]
        if char == "\n":
            pos += 1
            continue
        if char == "[":
            spans.append((start, pos, remove))
            start = pos
            out.flags.finalize_pending()
            if text.startswith("[[", pos):
                pos, header = parser.create_list_rule(text, pos, out)
            else:
                pos, header = parser.create_dict_rule(text, pos, out)
            remove = len(header) >= 2 and header[0] in owned and header[1] in owned[header[0]]
            if header == ("skills", "config"):
                remove = os.path.normcase(next(skill_rows).get("path", "")) in skill_paths
        elif char in parser.KEY_INITIAL_CHARS:
            pos = parser.key_value_rule(text, pos, out, header, float)
        pos = parser.skip_chars(text, pos, parser.TOML_WS)
        comment_start = pos
        pos = parser.skip_comment(text, pos)
        if text[comment_start:pos] in (
            "# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1",
            "# END CODEX-STARTER-KIT MANAGED CONFIG v1",
        ):
            markers.append((comment_start, pos))
        # The public parser validated every statement above, including its EOL.
        pos += 1
    spans.append((start, len(text), remove))
    removed = [(a, b) for a, b, replace in spans if replace] + markers
    # Merge overlapping marker/table ranges, preserving all other source text.
    result, end = [], 0
    for a, b in sorted(removed):
        if a > end:
            result.append(text[end:a])
        end = max(end, b)
    result.append(text[end:])
    kept = "".join(result)
    tomllib.loads(kept + "\n" + fragment)
    return kept


if __name__ == "__main__":
    try:
        config, fragment, output = map(Path, sys.argv[1:])
        existing = config.read_text(encoding="utf-8-sig") if config.exists() else ""
        output.write_text(without_replaced_tables(existing, fragment.read_text(encoding="utf-8-sig")), encoding="utf-8")
    except Exception as error:
        print(f"Package config replacement failed ({type(error).__name__}); configuration not printed.", file=sys.stderr)
        sys.exit(1)
