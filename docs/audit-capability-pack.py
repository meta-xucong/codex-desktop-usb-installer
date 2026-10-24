#!/usr/bin/env python3
"""Read-only structural audit for a Codex capability installer-pack."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import re
import sys
from pathlib import Path

try:
    import tomllib
except ImportError:  # pragma: no cover - missing parser must fail closed
    tomllib = None


TRANSIENT_SEGMENTS = {".git", ".pytest_cache", "__pycache__"}
PACK_METADATA_PATH = Path(__file__).resolve().parents[1] / "tools" / "refresh-capability-pack-metadata.py"
PACK_METADATA_SPEC = importlib.util.spec_from_file_location("capability_pack_metadata", PACK_METADATA_PATH)
if PACK_METADATA_SPEC is None or PACK_METADATA_SPEC.loader is None:  # pragma: no cover - broken checkout
    raise RuntimeError(f"cannot load capability pack metadata helper: {PACK_METADATA_PATH}")
PACK_METADATA = importlib.util.module_from_spec(PACK_METADATA_SPEC)
PACK_METADATA_SPEC.loader.exec_module(PACK_METADATA)


def is_transient(path: Path, root: Path) -> bool:
    relative = path.relative_to(root)
    return any(part in TRANSIENT_SEGMENTS for part in relative.parts) or relative.suffix.lower() == ".pyc"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def canonical_sha256(path: Path) -> str:
    """Hash a package file using the same text newline rules as its manifest."""
    return PACK_METADATA.sha256(path)


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else "installer-pack").resolve()
    pack = json.loads((root / "pack.json").read_text(encoding="utf-8"))
    audit = json.loads((root / "manifest" / "skill-audit.json").read_text(encoding="utf-8"))
    agent_audit = json.loads((root / "manifest" / "agent-audit.json").read_text(encoding="utf-8"))
    skills = audit.get("skills", [])
    agent_entries = agent_audit.get("agents", [])
    skill_dirs = sorted(p.name for p in (root / "skills").iterdir() if p.is_dir())
    agents = sorted((root / "agents").glob("*.toml"))
    default_skills = sorted(pack.get("defaultEnabled", {}).get("skills", []))
    default_agents = sorted(pack.get("defaultEnabled", {}).get("agents", []))
    agent_files = sorted(f"agents/{p.name}" for p in agents)
    manifest_ids = sorted(x.get("id", "") for x in skills)
    results: dict[str, object] = {
        "root": str(root),
        "packVersion": pack.get("version"),
        "sourceCommit": pack.get("sourceCommit"),
        "counts": {
            "skillDirectories": len(skill_dirs), "skillManifestEntries": len(skills),
            "agentFiles": len(agents), "defaultSkills": len(default_skills),
            "defaultAgents": len(default_agents),
            "agentManifestEntries": len(agent_entries),
            "skillStatusCounts": {
                status: sum(1 for entry in skills if entry.get("afterRemediationStatus") == status)
                for status in sorted({entry.get("afterRemediationStatus") for entry in skills})
            },
        },
        "agentDirectoryOnly": sorted({Path(x).stem for x in agent_files} - {x.get("id", "") for x in agent_entries}),
        "agentManifestOnly": sorted({x.get("id", "") for x in agent_entries} - {Path(x).stem for x in agent_files}),
        "skillDirectoryOnly": sorted(set(skill_dirs) - set(manifest_ids)),
        "skillManifestOnly": sorted(set(manifest_ids) - set(skill_dirs)),
        "missingSkillMarkdown": [], "invalidSkillFrontmatter": [],
        "missingRelativeReferences": [], "invalidAgentToml": [],
        "agentManifestSkillReferencesMissing": [],
        "agentManifestSkillsMissingFromInstructions": [],
        "defaultSkillMissing": sorted(set(default_skills) - set(skill_dirs)),
        "defaultAgentMissing": sorted(set(default_agents) - set(agent_files)),
        "duplicateIds": {}, "defaultStateMismatches": [], "fileManifestMissingFiles": [],
        "fileManifestExtraFiles": [], "checksumsMissingFiles": [], "checksumsExtraFiles": [],
        "checksumsMismatches": [], "tomlParserUnavailable": tomllib is None,
        "tree": {}, "packFileManifestMismatches": [],
    }
    results["duplicateIds"] = {
        "skillDirectories": sorted(x for x in set(skill_dirs) if skill_dirs.count(x) > 1),
        "skillManifest": sorted(x for x in set(manifest_ids) if manifest_ids.count(x) > 1),
        "defaultSkills": sorted(x for x in set(default_skills) if default_skills.count(x) > 1),
        "agentFiles": sorted(x for x in set(agent_files) if agent_files.count(x) > 1),
        "agentManifest": sorted(x for x in {x.get("id", "") for x in agent_entries}
                                if sum(1 for e in agent_entries if e.get("id") == x) > 1),
        "defaultAgents": sorted(x for x in set(default_agents) if default_agents.count(x) > 1),
    }
    manifest_default_skills = sorted(x.get("id", "") for x in skills if x.get("defaultEnabled") is True)
    if sorted(default_skills) != manifest_default_skills:
        results["defaultStateMismatches"].append("pack.defaultEnabled.skills != skill-audit.defaultEnabled entries")
    manifest_install_skills = sorted(x.get("id", "") for x in skills if x.get("installByDefault") is True)
    if sorted(default_skills) != manifest_install_skills:
        results["defaultStateMismatches"].append("pack.defaultEnabled.skills != skill-audit.installByDefault entries")
    status_default_skills = sorted(x.get("id", "") for x in skills
                                   if x.get("afterRemediationStatus") in {"core-ready", "auto-installable-runtime", "unsupported"})
    if sorted(default_skills) != status_default_skills:
        results["defaultStateMismatches"].append("pack.defaultEnabled.skills != default-enabled status classes")
    if sorted(default_agents) != sorted(agent_files):
        results["defaultStateMismatches"].append("pack.defaultEnabled.agents != agent TOML files")

    for skill_id in skill_dirs:
        skill_file = root / "skills" / skill_id / "SKILL.md"
        if not skill_file.is_file():
            results["missingSkillMarkdown"].append(skill_id)
            continue
        text = skill_file.read_text(encoding="utf-8-sig")
        front = re.match(r"\A---\s*\r?\n(.*?)\r?\n---(?:\s|$)", text, re.S)
        if not front:
            results["invalidSkillFrontmatter"].append(skill_id)
        elif not re.search(r"(?m)^name:\s*[^\s#].*$", front.group(1)) or not re.search(r"(?m)^description:\s*", front.group(1)):
            results["invalidSkillFrontmatter"].append(skill_id)
        for target in re.findall(r"\[[^\]]*\]\(([^)]+)\)", text):
            target = target.split("#", 1)[0].strip().strip("<>")
            if not target or re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", target) or target.startswith("/"):
                continue
            if not (skill_file.parent / target).resolve().exists():
                results["missingRelativeReferences"].append({"skill": skill_id, "target": target})

    if tomllib is not None:
        parsed_agents = {}
        for agent_file in agents:
            try:
                parsed_agents[agent_file.stem] = tomllib.loads(agent_file.read_text(encoding="utf-8"))
            except Exception as exc:  # report parse diagnostics, continue all files
                results["invalidAgentToml"].append({"agent": agent_file.stem, "error": str(exc)})
        known_skill_ids = set(skill_dirs)
        for agent_id, data in parsed_agents.items():
            body = data.get("developer_instructions", "")
            entry = next((x for x in agent_entries if x.get("id") == agent_id), {})
            expected = set(entry.get("requiredSkills", []))
            refs = {skill_id for skill_id in known_skill_ids if re.search(rf"(?<![\w-]){re.escape(skill_id)}(?![\w-])", body)}
            absent = sorted(expected - refs)
            if absent:
                results["agentManifestSkillsMissingFromInstructions"].append({"agent": agent_id, "skills": absent})
        for entry in agent_entries:
            refs = set(entry.get("requiredSkills", [])) | set(entry.get("optionalSkills", [])) | set(entry.get("blockedSkills", []))
            missing = sorted(refs - known_skill_ids)
            if missing:
                results["agentManifestSkillReferencesMissing"].append({"agent": entry.get("id"), "skills": missing})

    all_files = sorted(p for p in root.rglob("*") if p.is_file())
    transient_files = [p for p in all_files if is_transient(p, root)]
    files = [p for p in all_files if not is_transient(p, root)]
    results["transientFilesExcluded"] = sorted(p.relative_to(root).as_posix() for p in transient_files)
    rows = [f"{p.relative_to(root).as_posix()}|{sha256(p)}" for p in files]
    fingerprint_bytes = ("\n".join(rows)).encode("utf-8")
    results["tree"] = {
        "files": len(files),
        "transientFilesExcludedCount": len(transient_files),
        "serialization": "sorted relative POSIX path|raw-file-sha256 rows joined by LF, UTF-8 without BOM, no trailing LF",
        "sha256": hashlib.sha256(fingerprint_bytes).hexdigest(),
    }

    file_manifest_path = root / "file-manifest.json"
    if file_manifest_path.exists():
        fm = json.loads(file_manifest_path.read_text(encoding="utf-8"))
        listed_paths = sorted(item["path"] for item in fm.get("files", []))
        actual_paths = sorted(p.relative_to(root).as_posix() for p in files
                              if p.name not in {"file-manifest.json", "checksums.sha256"})
        results["fileManifestMissingFiles"] = sorted(set(actual_paths) - set(listed_paths))
        results["fileManifestExtraFiles"] = sorted(set(listed_paths) - set(actual_paths))
        for item in fm.get("files", []):
            target = root / item["path"]
            if not target.is_file() or canonical_sha256(target) != item.get("sha256"):
                results["packFileManifestMismatches"].append(item["path"])

    checksums_path = root / "checksums.sha256"
    if checksums_path.exists():
        checksum_entries: dict[str, str] = {}
        for line in checksums_path.read_text(encoding="utf-8-sig").splitlines():
            line = line.strip()
            if not line:
                continue
            digest, separator, rel = line.partition(" *")
            if not separator:
                digest, separator, rel = line.partition("  ")
            if not separator:
                results["checksumsMismatches"].append({"line": line, "error": "unparseable"})
                continue
            if rel in checksum_entries:
                results["checksumsMismatches"].append({"path": rel, "error": "duplicate entry"})
            checksum_entries[rel] = digest.lower()
        actual_checksum_scope = {p.relative_to(root).as_posix() for p in files if p.name != "checksums.sha256"}
        results["checksumsMissingFiles"] = sorted(actual_checksum_scope - set(checksum_entries))
        results["checksumsExtraFiles"] = sorted(set(checksum_entries) - actual_checksum_scope)
        for rel, expected in checksum_entries.items():
            target = root / rel
            if target.is_file() and canonical_sha256(target) != expected:
                results["checksumsMismatches"].append({"path": rel, "error": "sha256 mismatch"})

    print(json.dumps(results, ensure_ascii=False, indent=2))
    structural_errors = any(results[k] for k in (
        "skillDirectoryOnly", "skillManifestOnly", "missingSkillMarkdown",
        "invalidSkillFrontmatter", "missingRelativeReferences", "invalidAgentToml",
        "agentManifestSkillReferencesMissing", "agentManifestSkillsMissingFromInstructions",
        "agentDirectoryOnly",
        "agentManifestOnly", "defaultSkillMissing", "defaultAgentMissing",
        "defaultStateMismatches", "fileManifestMissingFiles", "fileManifestExtraFiles", "packFileManifestMismatches",
        "checksumsMissingFiles", "checksumsExtraFiles", "checksumsMismatches",
    ))
    structural_errors = structural_errors or results["tomlParserUnavailable"] or any(results["duplicateIds"].values())
    return 1 if structural_errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
