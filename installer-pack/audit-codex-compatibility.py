"""Audit the starter kit against the current native Codex contracts.

The audit is intentionally offline and standard-library-only. It checks source
and generated pack layouts, but never reads user credentials or Codex history.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path


MIN_CODEX_VERSION = (0, 144, 0)
EXPECTED_AGENTS = 7
FRONTMATTER_KEYS = {"name", "description"}
MODEL_CATALOG_FIELDS = {
    "id",
    "displayName",
    "visibility",
    "supportedInApi",
    "priority",
    "defaultReasoningLevel",
    "reasoningLevels",
}
MODEL_CATALOG_TOP_LEVEL_FIELDS = {
    "schemaVersion",
    "source",
    "snapshotVersion",
    "clientVersion",
    "preferredModel",
    "retiredModelIds",
    "models",
}
CANONICAL_TEXT_SUFFIXES = {
    ".cmd",
    ".env",
    ".in",
    ".json",
    ".lock",
    ".md",
    ".ps1",
    ".py",
    ".sha256",
    ".toml",
    ".txt",
    ".yaml",
    ".yml",
}
OLD_FEISHU_TOOL_MARKERS = (
    "feishu_doc_",
    "feishu_drive_",
    "feishu_perm_",
    "feishu_wiki_",
    "mcp__feishu__",
)


class Audit:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.errors: list[str] = []
        self.warnings: list[str] = []
        self.checks: list[str] = []
        self.metrics: dict[str, object] = {}

    def error(self, message: str) -> None:
        self.errors.append(message)

    def warn(self, message: str) -> None:
        self.warnings.append(message)

    def passed(self, message: str) -> None:
        self.checks.append(message)


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def canonical_pack_bytes(path: Path) -> bytes:
    data = path.read_bytes()
    if path.suffix.lower() not in CANONICAL_TEXT_SUFFIXES:
        return data
    return data.replace(b"\r\n", b"\n").replace(b"\r", b"\n")


def validate_model_catalog(catalog: object) -> list[str]:
    errors: list[str] = []
    if not isinstance(catalog, dict):
        return ["Model catalog must be a JSON object."]
    required = {"schemaVersion", "preferredModel", "retiredModelIds", "models"}
    missing = sorted(required - catalog.keys())
    unknown = sorted(set(catalog) - MODEL_CATALOG_TOP_LEVEL_FIELDS)
    errors.extend(f"Model catalog is missing required field: {field}." for field in missing)
    errors.extend(f"Model catalog contains unknown field: {field}." for field in unknown)

    schema_version = catalog.get("schemaVersion")
    if type(schema_version) is not int or schema_version != 1:
        errors.append("Model catalog schemaVersion must be integer 1.")
    preferred = catalog.get("preferredModel")
    if not isinstance(preferred, str) or not preferred.strip():
        errors.append("Model catalog preferredModel must be a non-empty string.")
    for field in ("source", "snapshotVersion", "clientVersion"):
        if field in catalog and (not isinstance(catalog[field], str) or not catalog[field].strip()):
            errors.append(f"Model catalog {field} must be a non-empty string.")

    retired_ids: set[str] = set()
    retired = catalog.get("retiredModelIds")
    if not isinstance(retired, list):
        errors.append("Model catalog retiredModelIds must be an array.")
    else:
        for value in retired:
            if not isinstance(value, str) or not value.strip():
                errors.append("Model catalog retiredModelIds entries must be non-empty strings.")
                continue
            key = value.casefold()
            if key in retired_ids:
                errors.append(f"Model catalog retiredModelIds contains a duplicate: {value}.")
            retired_ids.add(key)

    models = catalog.get("models")
    if not isinstance(models, list) or not models:
        errors.append("Model catalog models must be a non-empty array.")
        return errors

    model_by_id: dict[str, dict] = {}
    for index, model in enumerate(models):
        context = f"Model catalog model entry {index}"
        if not isinstance(model, dict):
            errors.append(f"{context} must be an object.")
            continue
        missing_fields = sorted(MODEL_CATALOG_FIELDS - model.keys())
        unknown_fields = sorted(set(model) - MODEL_CATALOG_FIELDS)
        errors.extend(f"{context} is missing required field: {field}." for field in missing_fields)
        errors.extend(f"{context} contains unknown field: {field}." for field in unknown_fields)
        model_id = model.get("id")
        if not isinstance(model_id, str) or not re.fullmatch(r"[a-z0-9][a-z0-9.-]*", model_id):
            errors.append(f"{context} id must match the lowercase model-id format.")
        else:
            key = model_id.casefold()
            if key in model_by_id:
                errors.append(f"Model catalog model IDs must be unique: {model_id}.")
            model_by_id[key] = model
        display_name = model.get("displayName")
        if not isinstance(display_name, str) or not display_name.strip():
            errors.append(f"{context} displayName must be a non-empty string.")
        if not isinstance(model.get("visibility"), str) or model.get("visibility") not in {"list", "hide"}:
            errors.append(f"{context} visibility must be list or hide.")
        if type(model.get("supportedInApi")) is not bool:
            errors.append(f"{context} supportedInApi must be boolean.")
        if type(model.get("priority")) is not int:
            errors.append(f"{context} priority must be an integer.")
        default_level = model.get("defaultReasoningLevel")
        levels = model.get("reasoningLevels")
        if not isinstance(default_level, str) or not default_level.strip():
            errors.append(f"{context} defaultReasoningLevel must be a non-empty string.")
        if not isinstance(levels, list) or not levels or any(not isinstance(level, str) or not level.strip() for level in levels):
            errors.append(f"{context} reasoningLevels must be a non-empty string array.")
        elif len({level.casefold() for level in levels}) != len(levels):
            errors.append(f"{context} reasoningLevels must be unique.")
        elif default_level not in levels:
            errors.append(f"{context} defaultReasoningLevel must be listed in reasoningLevels.")

    for retired_id in retired_ids:
        retired_entry = model_by_id.get(retired_id)
        if retired_entry and retired_entry.get("visibility") == "list" and retired_entry.get("supportedInApi") is True:
            errors.append(f"Retired model {retired_id} cannot remain visible and active in the model catalog.")

    preferred_entry = model_by_id.get(preferred.casefold()) if isinstance(preferred, str) else None
    if preferred_entry is None:
        errors.append("Model catalog preferredModel is absent from models.")
    elif preferred_entry.get("visibility") != "list" or preferred_entry.get("supportedInApi") is not True:
        errors.append("Model catalog preferredModel must be visible (list) and supportedInApi=true.")
    return errors


def find_root(explicit: str | None) -> Path:
    if explicit:
        return Path(explicit).expanduser().resolve()
    script = Path(__file__).resolve()
    for candidate in (script.parent, script.parent.parent):
        if (candidate / "manifest" / "imported-skills.txt").is_file() and (candidate / "skills").is_dir():
            return candidate
    raise SystemExit("Cannot locate a source tree or capability pack; pass --root.")


def manifest_skill_ids(root: Path) -> list[str]:
    path = root / "manifest" / "imported-skills.txt"
    return [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip() and not line.startswith("#")]


def frontmatter_keys_and_values(path: Path) -> tuple[set[str], dict[str, str]]:
    text = path.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        return set(), {}
    end = text.find("\n---", 4)
    if end < 0:
        return set(), {}
    keys: set[str] = set()
    values: dict[str, str] = {}
    for line in text[4:end].splitlines():
        match = re.match(r"^([A-Za-z][A-Za-z0-9_-]*):\s*(.*)$", line)
        if match:
            key, value = match.groups()
            keys.add(key)
            values[key] = value.strip().strip('"\'')
    return keys, values


def check_skills(audit: Audit, skill_ids: list[str]) -> None:
    errors_before = len(audit.errors)
    audit_path = audit.root / "manifest" / "skill-audit.json"
    if audit_path.is_file():
        catalog_ids = [str(entry.get("id", "")) for entry in load_json(audit_path).get("skills", [])]
        if len(skill_ids) != len(catalog_ids) or set(skill_ids) != set(catalog_ids):
            audit.error("imported-skills.txt and skill-audit.json do not describe the same Skill set.")
    if len(skill_ids) != len(set(skill_ids)):
        audit.error(f"Skill ids must be unique; found {len(skill_ids)} entries/{len(set(skill_ids))} unique.")
    names: set[str] = set()
    for skill_id in skill_ids:
        directory = audit.root / "skills" / skill_id
        skill_file = directory / "SKILL.md"
        if not skill_file.is_file():
            audit.error(f"Missing Skill: skills/{skill_id}/SKILL.md")
            continue
        keys, values = frontmatter_keys_and_values(skill_file)
        if keys != FRONTMATTER_KEYS:
            audit.error(f"skills/{skill_id}/SKILL.md frontmatter keys are {sorted(keys)}; expected only name and description.")
        if values.get("name") != skill_id:
            audit.error(f"Skill name mismatch for {skill_id}: {values.get('name')!r}")
        if not values.get("description"):
            audit.error(f"Skill description is empty: {skill_id}")
        if values.get("name") in names:
            audit.error(f"Duplicate Skill name: {values.get('name')}")
        names.add(values.get("name", ""))

        text = skill_file.read_text(encoding="utf-8")
        lowered = text.casefold()
        if skill_id != "web-search-extraction" and ("web-search-extraction" in lowered or "web_search.py" in lowered):
            audit.error(f"Cross-Skill hard-coded web adapter remains in {skill_id}.")
        if re.search(r"(?<![A-Za-z0-9_-])image2(?![A-Za-z0-9_-])", lowered):
            audit.error(f"Stale image2 tool contract remains in {skill_id}.")
        scripts_dir = directory / "scripts"
        source_scripts = [path for path in scripts_dir.rglob("*") if path.is_file() and path.suffix.casefold() != ".pyc" and "__pycache__" not in path.parts] if scripts_dir.is_dir() else []
        if source_scripts:
            has_path_rule = "<skill-directory>" in text and ("absolute" in lowered or "绝对路径" in text)
            if not has_path_rule:
                audit.error(f"Script-bearing Skill lacks an absolute path rule: {skill_id}")
    audit.metrics["skills"] = len(skill_ids)
    if len(audit.errors) == errors_before:
        audit.passed("Skill count, names, frontmatter, native routing, and script path rules")


def check_agents(audit: Audit) -> None:
    errors_before = len(audit.errors)
    catalog_path = audit.root / "manifest" / "agent-audit.json"
    if not catalog_path.is_file():
        audit.error("Missing manifest/agent-audit.json")
        return
    catalog = load_json(catalog_path)
    entries = catalog.get("agents", [])
    if len(entries) != EXPECTED_AGENTS:
        audit.error(f"Expected {EXPECTED_AGENTS} Agent entries, found {len(entries)}.")
    agent_files = sorted((audit.root / "agents").glob("*.toml"))
    if len(agent_files) != EXPECTED_AGENTS:
        audit.error(f"Expected {EXPECTED_AGENTS} Agent TOML files, found {len(agent_files)}.")
    names: set[str] = set()
    for path in agent_files:
        try:
            data = tomllib.loads(path.read_text(encoding="utf-8-sig"))
        except (OSError, tomllib.TOMLDecodeError) as exc:
            audit.error(f"Invalid Agent TOML {path.name}: {exc}")
            continue
        for field in ("name", "description", "developer_instructions"):
            if not str(data.get(field, "")).strip():
                audit.error(f"Agent {path.name} is missing {field}.")
        for unsupported in ("mcp", "model"):
            if unsupported in data:
                audit.error(f"Agent {path.name} contains unnecessary/unsupported top-level field {unsupported}.")
        name = str(data.get("name", ""))
        if name in names:
            audit.error(f"Duplicate Agent name: {name}")
        names.add(name)
        instructions = str(data.get("developer_instructions", ""))
        if "web-search-extraction" in instructions and (
            "Codex 原生网页" not in instructions or "明确选择" not in instructions
        ):
            audit.error(f"Agent {path.name} does not route web research through native Codex first with explicit adapter opt-in.")
        if "pdf-processing-toolkit" in instructions and "许可证" not in instructions:
            audit.error(f"Agent {path.name} references the quarantined PDF Skill without its license gate.")
        if "createContent/" in instructions or "createContent\\" in instructions:
            audit.error(f"Agent {path.name} retains a fixed legacy output directory.")
    audit.metrics["agents"] = len(agent_files)
    if len(audit.errors) == errors_before:
        audit.passed("Agent TOML native contract")


def check_openai_metadata(audit: Audit) -> None:
    errors_before = len(audit.errors)
    skill_audit = load_json(audit.root / "manifest" / "skill-audit.json")
    expected = {
        str(entry["id"])
        for entry in skill_audit.get("skills", [])
        if entry.get("afterRemediationStatus") != "core-ready"
    }
    yaml_files = sorted((audit.root / "skills").glob("*/agents/openai.yaml"))
    actual = {path.parents[1].name for path in yaml_files}
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        audit.error(f"agents/openai.yaml coverage differs from non-core Skills; missing={missing}, extra={extra}")
    for path in yaml_files:
        skill_id = path.parents[1].name
        text = path.read_text(encoding="utf-8")
        fields: dict[str, str] = {}
        for key in ("display_name", "short_description", "default_prompt"):
            match = re.search(rf'^\s{{2}}{key}:\s*"([^"]*)"\s*$', text, re.MULTILINE)
            if not match:
                audit.error(f"{path.relative_to(audit.root).as_posix()} lacks quoted interface.{key}.")
            else:
                fields[key] = match.group(1)
        short = fields.get("short_description", "")
        if short and not 25 <= len(short) <= 64:
            audit.error(f"{skill_id} short_description length is {len(short)}; expected 25-64 characters.")
        prompt = fields.get("default_prompt", "")
        if prompt and f"${skill_id}" not in prompt:
            audit.error(f"{skill_id} default_prompt must explicitly mention ${skill_id}.")
        if not re.search(r"^\s{2}allow_implicit_invocation:\s*false\s*$", text, re.MULTILINE):
            audit.error(f"Non-core Skill must disable implicit invocation: {skill_id}")
        if skill_id.startswith("feishu-"):
            if not re.search(r'^\s{6}value:\s*"feishu"\s*$', text, re.MULTILINE):
                audit.error(f"{skill_id} UI metadata lacks the feishu MCP dependency.")
            if not re.search(r'^\s{6}transport:\s*"stdio"\s*$', text, re.MULTILINE):
                audit.error(f"{skill_id} UI metadata lacks the stdio transport.")
    audit.metrics["openaiYaml"] = len(yaml_files)
    if len(audit.errors) == errors_before:
        audit.passed("Non-core Skill UI metadata and explicit invocation policy")


def check_feishu(audit: Audit) -> None:
    errors_before = len(audit.errors)
    contract_path = audit.root / "manifest" / "feishu-tools.json"
    template_path = audit.root / "config-fragments" / "feishu-official-stdio.template.toml"
    if not contract_path.is_file() or not template_path.is_file():
        audit.error("Feishu tool contract or config template is missing.")
        return
    contract = load_json(contract_path)
    read_tools = contract.get("readTools", [])
    write_tools = contract.get("writeTools", [])
    high_risk = contract.get("highRiskTools", [])
    tools = read_tools + write_tools
    if (
        contract.get("packageName") != "@larksuiteoapi/lark-mcp"
        or contract.get("packageVersion") != "0.5.1"
        or contract.get("cliRelativePath") != "node_modules/@larksuiteoapi/lark-mcp/dist/cli.js"
    ):
        audit.error("Feishu package entrypoint contract is not locked to the reviewed 0.5.1 CLI.")
    if contract.get("toolNameCase") != "dot" or not tools or len(tools) != len(set(tools)):
        audit.error("Feishu contract must contain unique dot-case tools.")
    for tool in tools:
        if not re.fullmatch(r"[A-Za-z0-9]+(?:\.[A-Za-z0-9]+)+", tool):
            audit.error(f"Invalid Feishu dot-case tool: {tool}")
    if not set(high_risk).issubset(write_tools):
        audit.error("Feishu high-risk tools must be a subset of write tools.")

    template = template_path.read_text(encoding="utf-8")
    rendered = template
    replacements = {
        "__FEISHU_ENABLED__": "false",
        "__FEISHU_WRAPPER__": '"C:\\\\safe\\\\start-feishu-mcp.ps1"',
        "__FEISHU_MCP_CACHE__": '"C:\\\\safe\\\\feishu-cache"',
        "__FEISHU_DOMAIN__": '"https://open.feishu.cn"',
        "__FEISHU_AUTH_MODE__": '"tenant"',
        "__FEISHU_ENV_VARS__": '["FEISHU_APP_ID", "FEISHU_APP_SECRET"]',
    }
    for key, value in replacements.items():
        rendered = rendered.replace(key, value)
    if re.search(r"__[A-Z0-9_]+__", rendered):
        audit.error("Feishu template contains unknown placeholders.")
        return
    try:
        parsed = tomllib.loads(rendered)
    except tomllib.TOMLDecodeError as exc:
        audit.error(f"Feishu template does not render as valid TOML: {exc}")
        return
    server = parsed.get("mcp_servers", {}).get("feishu", {})
    if set(server.get("enabled_tools", [])) != set(tools):
        audit.error("Feishu enabled_tools differs from manifest/feishu-tools.json.")
    if server.get("default_tools_approval_mode") != "writes":
        audit.error("Feishu default tool approval must be writes.")
    if set(server.get("env_vars", [])) != {"FEISHU_APP_ID", "FEISHU_APP_SECRET"}:
        audit.error("Tenant-mode Feishu config must forward only App ID and App Secret variable names.")
    tool_config = server.get("tools", {})
    for tool in high_risk:
        if tool_config.get(tool, {}).get("approval_mode") != "prompt":
            audit.error(f"High-risk Feishu tool is not prompt-gated: {tool}")

    for path in (audit.root / "skills").rglob("*"):
        if path.is_file() and path.suffix.casefold() in CANONICAL_TEXT_SUFFIXES:
            text = path.read_text(encoding="utf-8", errors="replace").casefold()
            for marker in OLD_FEISHU_TOOL_MARKERS:
                if marker in text:
                    audit.error(f"Stale Feishu tool marker {marker!r} remains in {path.relative_to(audit.root).as_posix()}.")
    wrapper_candidates = (
        audit.root / "scripts" / "start-feishu-mcp.ps1",
        audit.root / "mcp" / "start-feishu-mcp.ps1",
    )
    wrapper = next((path for path in wrapper_candidates if path.is_file()), None)
    if wrapper is None:
        audit.error("Feishu local wrapper is missing.")
    else:
        wrapper_text = wrapper.read_text(encoding="utf-8")
        if re.search(r"(?i)\bnpx\b|lark-mcp\.cmd", wrapper_text):
            audit.error("Feishu wrapper may invoke a network/package-manager shim.")
        if (
            "& $node.Source $cliPath @arguments" not in wrapper_text
            and "& $nodePath $cliPath @arguments" not in wrapper_text
        ):
            audit.error("Feishu wrapper must invoke the reviewed JavaScript entrypoint directly with Node.")
    audit.metrics["feishuTools"] = len(tools)
    if len(audit.errors) == errors_before:
        audit.passed("Feishu official tool names, limits, allowlist, and approval policy")


def check_audit_statuses(audit: Audit) -> None:
    errors_before = len(audit.errors)
    path = audit.root / "manifest" / "skill-audit.json"
    if not path.is_file():
        audit.error("Missing manifest/skill-audit.json")
        return
    data = load_json(path)
    entries = data.get("skills", [])
    status_counts: dict[str, int] = {}
    default_statuses = {"core-ready", "auto-installable-runtime", "unsupported"}
    guided_statuses = {"guided-config"}
    for entry in entries:
        status = str(entry.get("afterRemediationStatus", ""))
        status_counts[status] = status_counts.get(status, 0) + 1
        expected_default = status in default_statuses
        if bool(entry.get("installByDefault")) != expected_default:
            audit.error(
                f"Skill {entry.get('id')} installByDefault={entry.get('installByDefault')} "
                f"does not match status policy {status}."
            )
        if bool(entry.get("defaultEnabled")) != expected_default:
            audit.error(
                f"Skill {entry.get('id')} defaultEnabled={entry.get('defaultEnabled')} "
                f"does not match status policy {status}."
            )
        if status not in default_statuses | guided_statuses:
            audit.error(f"Unknown post-remediation Skill status: {status}")
    enabled_entries = [entry for entry in entries if bool(entry.get("installByDefault"))]
    expected_enabled_count = sum(status_counts.get(status, 0) for status in default_statuses)
    if len(enabled_entries) != expected_enabled_count:
        audit.error(f"One-step policy must default-enable {expected_enabled_count} Skills, found {len(enabled_entries)}.")

    for skill_id in ("canvas", "healthcheck", "node-connect"):
        external = next((entry for entry in entries if entry.get("id") == skill_id), None)
        if not external or external.get("afterRemediationStatus") != "unsupported" or not external.get("installByDefault"):
            audit.error(f"{skill_id} must be config-enabled with an unsupported safe-failure status.")
        skill_path = audit.root / "skills" / skill_id / "SKILL.md"
        text = skill_path.read_text(encoding="utf-8").casefold() if skill_path.is_file() else ""
        for marker in ("后端", "不可用", "不得", "不伪造"):
            if marker not in text:
                audit.error(f"{skill_id} Skill documentation lacks safe-failure marker: {marker}")

    pdf = next((entry for entry in entries if entry.get("id") == "pdf-processing-toolkit"), None)
    pdf_dir = audit.root / "skills" / "pdf-processing-toolkit"
    pdf_path = pdf_dir / "SKILL.md"
    license_path = pdf_dir / "LICENSE.txt"
    smoke_path = pdf_dir / "scripts" / "pdf_tool.py"
    if not pdf or pdf.get("status") != "requires-runtime" or pdf.get("afterRemediationStatus") != "auto-installable-runtime":
        audit.error("pdf-processing-toolkit must be an auto-installable runtime Skill after license remediation.")
    redistribution = pdf.get("redistribution", {}) if pdf else {}
    if redistribution.get("status") != "maintained-original" or redistribution.get("sourceBodyIncluded") is not True:
        audit.error("pdf-processing-toolkit must identify its maintained original implementation.")
    if not license_path.is_file() or "mit license" not in license_path.read_text(encoding="utf-8").casefold():
        audit.error("pdf-processing-toolkit is missing its self-maintained MIT LICENSE.txt.")
    pdf_text = pdf_path.read_text(encoding="utf-8").casefold() if pdf_path.is_file() else ""
    if "license.txt" not in pdf_text or "pypdf" not in pdf_text or "pdfplumber" not in pdf_text or "reportlab" not in pdf_text:
        audit.error("pdf-processing-toolkit documentation lacks the runtime/license dependency contract.")
    if not smoke_path.is_file():
        audit.error("pdf-processing-toolkit PDF smoke-test script is missing.")
    else:
        smoke_text = smoke_path.read_text(encoding="utf-8")
        for marker in ("PdfReader", "pdfplumber", "reportlab", "def smoke", "def merge", "def extract", "def generate"):
            if marker not in smoke_text:
                audit.error(f"PDF smoke helper lacks required operation marker: {marker}")
    audit.metrics["statusCounts"] = status_counts
    if len(audit.errors) == errors_before:
        audit.passed("Compatibility statuses, one-step Skill activation, safe unsupported fallbacks, and PDF license/runtime contract")


def check_external_adapters(audit: Audit) -> None:
    errors_before = len(audit.errors)
    media_script = audit.root / "scripts" / "media-generation-mcp.py"
    media_wrapper = audit.root / "scripts" / "start-media-generation-mcp.ps1"
    # The editable source keeps runnable scripts under scripts/, while the
    # generated copyable pack installs local MCP launchers under mcp/.
    if not media_script.is_file():
        media_script = audit.root / "mcp" / "media-generation-mcp.py"
    if not media_wrapper.is_file():
        media_wrapper = audit.root / "mcp" / "start-media-generation-mcp.ps1"
    media_manifest = audit.root / "manifest" / "media-generation-tools.json"
    media_template = audit.root / "config-fragments" / "media-generation-stdio.template.toml"
    image_skill = audit.root / "skills" / "image-2" / "SKILL.md"
    seedance_skill = audit.root / "skills" / "seedance2-0-video-gen" / "SKILL.md"
    web_script = audit.root / "skills" / "web-search-extraction" / "scripts" / "web_search.py"
    web_skill = audit.root / "skills" / "web-search-extraction" / "SKILL.md"
    required_files = (media_script, media_wrapper, media_manifest, media_template, image_skill, seedance_skill, web_script, web_skill)
    missing = [path.relative_to(audit.root).as_posix() for path in required_files if not path.is_file()]
    if missing:
        audit.error(f"Unified media MCP files are missing: {missing}")
        return

    media_code = media_script.read_text(encoding="utf-8")
    for marker in (
        '"image-2"', '"grok-image"', '"grok-video"', '"seedance-video"',
        "hmac.compare_digest", "CONFIRMATION_TTL_SECONDS", 'record["state"] = "submitting"',
        '"Idempotency-Key"', "submission_unknown", "MAX_REFERENCE_BYTES", "MAX_DOWNLOAD_BYTES",
        '"media_prepare"', '"media_submit"', '"media_check"', '"media_save"',
        '"/v1/images/generations"', '"/v1/videos/generations"',
        '"/api/llm/openai/v1/images/generations"',
        '"/api/llm/doubao/contents/generations/tasks"',
    ):
        if marker not in media_code:
            audit.error(f"Unified media MCP lacks required contract marker: {marker}")
    if "requests." in media_code or "httpx." in media_code:
        audit.error("Unified media MCP has an undeclared third-party Python dependency.")
    if '"IMAGE_2_API_KEY"' in media_code or '"SEEDANCE_API_KEY"' in media_code:
        audit.error("Unified media MCP still reads retired provider-specific credentials.")
    if (audit.root / "skills" / "image-2" / "scripts" / "image_2_gen.py").exists():
        audit.error("Retired direct Image-2 adapter script remains in the Skill bundle.")
    if (audit.root / "skills" / "seedance2-0-video-gen" / "scripts" / "seedance_video_gen.py").exists():
        audit.error("Retired direct Seedance adapter script remains in the Skill bundle.")
    if "media_prepare" not in image_skill.read_text(encoding="utf-8") or "media_submit" not in image_skill.read_text(encoding="utf-8"):
        audit.error("Image Skill does not route through the unified MCP confirmation contract.")
    if "media_prepare" not in seedance_skill.read_text(encoding="utf-8") or "media_check" not in seedance_skill.read_text(encoding="utf-8"):
        audit.error("Seedance Skill does not route through the unified MCP task contract.")

    fields_path = audit.root / "manifest" / "connection-fields.json"
    services_path = audit.root / "manifest" / "api-services.json"
    mcp_path = audit.root / "manifest" / "mcp-servers.json"
    fields = load_json(fields_path).get("fields", [])
    field_ids = [str(item.get("id", "")) for item in fields]
    expected_fields = {
        "dashscope.model", "media-generation.api-base-url", "media-generation.api-key",
        "media-generation.agent-id", "media-generation.image2-model",
        "media-generation.grok-image-model", "media-generation.grok-video-model",
        "media-generation.seedance-model", "media-generation.minimax-video-model",
    }
    if len(field_ids) != 17 or len(field_ids) != len(set(field_ids)):
        audit.error(f"Expected 17 unique guided connection fields after consolidation, found {len(field_ids)}/{len(set(field_ids))}.")
    if not expected_fields.issubset(field_ids):
        audit.error(f"Unified media connection fields are incomplete: {sorted(expected_fields - set(field_ids))}")
    if any(field_id.startswith(("image-2.", "seedance.")) for field_id in field_ids):
        audit.error("Retired duplicate Image-2/Seedance configuration fields remain in the installer form.")
    secret_fields = {str(item.get("id")) for item in fields if item.get("secret")}
    if not {"dashscope.api-key", "media-generation.api-key"}.issubset(secret_fields):
        audit.error("Unified media/API keys are not marked secret in the connection schema.")

    services = {str(item.get("id")): item for item in load_json(services_path).get("services", [])}
    dashscope_service = services.get("dashscope-web-search", {})
    if set(services) != {"dashscope-web-search"}:
        audit.error(f"Direct API manifest must contain only DashScope; media generation belongs to the unified MCP, found {sorted(services)}.")
    if dashscope_service.get("defaultUrl") != "https://dashscope.aliyuncs.com/compatible-mode/v1" or dashscope_service.get("path") != "/chat/completions":
        audit.error("DashScope service contract changed unexpectedly.")
    if dashscope_service.get("credentialFields") != ["dashscope.api-key"]:
        audit.error("DashScope service credential fields differ from the reviewed contract.")

    mcp = {str(item.get("id")): item for item in load_json(mcp_path).get("servers", [])}.get("media-generation", {})
    if mcp.get("transport") != "stdio" or mcp.get("configFragment") != "config-fragments/media-generation-stdio.template.toml":
        audit.error("Unified media MCP must be a local STDIO server using its checked-in config template.")
    if set(mcp.get("connectionFields", [])) != {item for item in field_ids if item.startswith("media-generation.")}:
        audit.error("Media MCP connection field closure differs from the installer form.")
    tool_names = [str(item.get("name", "")) for item in load_json(media_manifest).get("tools", [])]
    if tool_names != ["media_capabilities", "media_prepare", "media_submit", "media_check", "media_save"]:
        audit.error(f"Unified media tool contract is incomplete or out of order: {tool_names}")
    for stale_template in (audit.root / "manifest" / "image-2.template.env", audit.root / "manifest" / "seedance.template.env"):
        if stale_template.exists():
            audit.error(f"Retired provider-specific template remains: {stale_template.name}")

    web_code = web_script.read_text(encoding="utf-8")
    web_docs = web_skill.read_text(encoding="utf-8")
    for marker in (
        'DEFAULT_BASE_URL = "https://dashscope.aliyuncs.com/compatible-mode/v1"',
        'os.environ.get("DASHSCOPE_MODEL")', "validate_base_url", "MAX_RESPONSE_BYTES",
        'endpoint = f"{base_url}/chat/completions"',
    ):
        if marker not in web_code:
            audit.error(f"DashScope adapter lacks required contract marker: {marker}")
    if "qwen-flash" in web_code or "qwen-flash" in web_docs:
        audit.error("DashScope adapter retains a guessed model default.")
    if "禁止行为" in web_docs and "Codex 原生" in web_docs:
        audit.error("DashScope documentation may still forbid the native Codex fallback.")

    audit.metrics["connectionFields"] = len(field_ids)
    audit.metrics["mediaMcpTools"] = len(tool_names)
    if len(audit.errors) == errors_before:
        audit.passed("Unified media MCP providers, immutable confirmation, single-submit protection, shared connection fields, and DashScope direct API contract")

def check_local_write_boundaries(audit: Audit) -> None:
    errors_before = len(audit.errors)
    travel_directory = audit.root / "skills" / "travel-planner"
    for stale_path in (travel_directory / "index.js", travel_directory / "package.json"):
        if stale_path.exists():
            audit.error(f"Travel planner retains a non-functional placeholder entrypoint: {stale_path.relative_to(audit.root).as_posix()}")

    china_script = audit.root / "skills" / "china-stock-analysis" / "scripts" / "data_fetcher.py"
    start_article = audit.root / "skills" / "wechat-article-creator" / "scripts" / "start_article.py"
    generate_cover = audit.root / "skills" / "wechat-article-creator" / "scripts" / "generate_cover.py"
    required_files = (china_script, start_article, generate_cover)
    missing = [path.relative_to(audit.root).as_posix() for path in required_files if not path.is_file()]
    if missing:
        audit.error(f"Local state boundary scripts are missing: {missing}")
        return

    china_code = china_script.read_text(encoding="utf-8")
    for marker in (
        "CHINA_STOCK_CACHE_DIR",
        'Path.cwd() / "codex-data"',
        "_validate_code",
        "DATA_TYPES",
        'status == "complete"',
    ):
        if marker not in china_code:
            audit.error(f"China stock cache lacks required project-data marker: {marker}")
    if "Path(__file__)" in china_code or "os.path.dirname(os.path.dirname(__file__))" in china_code:
        audit.error("China stock cache may still write relative to the installed Skill directory.")

    start_code = start_article.read_text(encoding="utf-8")
    cover_code = generate_cover.read_text(encoding="utf-8")
    for name, code in (("start_article.py", start_code), ("generate_cover.py", cover_code)):
        for marker in ("WECHAT_ARTICLE_DATA_DIR", 'Path.cwd() / "codex-data"'):
            if marker not in code:
                audit.error(f"{name} lacks required project-data marker: {marker}")
        if "Path(__file__).parent.parent" in code or "Path(__file__).resolve().parent.parent" in code:
            audit.error(f"{name} may still write relative to the installed Skill directory.")
    if "safe_filename_component" not in start_code:
        audit.error("WeChat article filenames are not normalized before writing.")
    for marker in ("40_000_000", '{".jpg", ".jpeg"}', 'suffix == ".png"'):
        if marker not in cover_code:
            audit.error(f"WeChat cover generation lacks required output guard: {marker}")

    for generated_directory in (
        travel_directory / "drafts",
        audit.root / "skills" / "wechat-article-creator" / "drafts",
        audit.root / "skills" / "wechat-article-creator" / "output",
        audit.root / "skills" / "china-stock-analysis" / ".cache",
    ):
        if generated_directory.exists():
            message = f"Generated user state is present inside a Skill tree: {generated_directory.relative_to(audit.root).as_posix()}"
            # The working source tree may contain user-authored drafts/output from
            # earlier runs. build-pack.py excludes these transient directories;
            # a generated installer-pack must never contain them.
            if audit.root.name.lower() == "installer-pack":
                audit.error(message)
            else:
                audit.warn(message + " (excluded from the generated installer pack)")

    if len(audit.errors) == errors_before:
        audit.passed("Local state remains outside installed Skills and placeholder entrypoints are removed")


def check_financial_data_truthfulness(audit: Audit) -> None:
    errors_before = len(audit.errors)
    technical_path = audit.root / "skills" / "china-stock-analysis" / "scripts" / "technical_analysis.py"
    realtime_path = audit.root / "skills" / "china-stock-analysis" / "scripts" / "realtime_quote.py"
    data_fetcher_path = audit.root / "skills" / "china-stock-analysis" / "scripts" / "data_fetcher.py"
    financial_analyzer_path = audit.root / "skills" / "china-stock-analysis" / "scripts" / "financial_analyzer.py"
    stock_report_template_path = audit.root / "skills" / "china-stock-analysis" / "templates" / "analysis_report.md"
    stock_screener_path = audit.root / "skills" / "china-stock-analysis" / "scripts" / "stock_screener.py"
    a_share_valuation_path = audit.root / "skills" / "china-stock-analysis" / "scripts" / "valuation_calculator.py"
    cycle_path = audit.root / "skills" / "macro-research" / "scripts" / "cycle_analyzer.py"
    policy_path = audit.root / "skills" / "macro-research" / "scripts" / "policy_analyzer.py"
    sentiment_path = audit.root / "skills" / "macro-research" / "scripts" / "sentiment_monitor.py"
    fund_screener_path = audit.root / "skills" / "fund-portfolio" / "scripts" / "fund_screener.py"
    fund_risk_path = audit.root / "skills" / "fund-portfolio" / "scripts" / "risk_assessor.py"
    fund_builder_path = audit.root / "skills" / "fund-portfolio" / "scripts" / "portfolio_builder.py"
    sip_path = audit.root / "skills" / "fund-portfolio" / "scripts" / "sip_calculator.py"
    allocator_path = audit.root / "skills" / "wealth-allocation" / "scripts" / "asset_allocator.py"
    rebalance_path = audit.root / "skills" / "wealth-allocation" / "scripts" / "rebalance_planner.py"
    tracker_path = audit.root / "skills" / "wealth-allocation" / "scripts" / "portfolio_tracker.py"
    business_evaluator_path = audit.root / "skills" / "venture-analysis" / "scripts" / "business_evaluator.py"
    valuation_path = audit.root / "skills" / "venture-analysis" / "scripts" / "valuation_analyzer.py"
    financial_model_path = audit.root / "skills" / "venture-analysis" / "scripts" / "financial_model.py"
    due_diligence_path = audit.root / "skills" / "venture-analysis" / "scripts" / "due_diligence.py"
    required_files = (
        technical_path,
        realtime_path,
        data_fetcher_path,
        financial_analyzer_path,
        stock_report_template_path,
        stock_screener_path,
        a_share_valuation_path,
        cycle_path,
        policy_path,
        sentiment_path,
        fund_screener_path,
        fund_risk_path,
        fund_builder_path,
        sip_path,
        allocator_path,
        rebalance_path,
        tracker_path,
        business_evaluator_path,
        valuation_path,
        financial_model_path,
        due_diligence_path,
    )
    missing = [path.relative_to(audit.root).as_posix() for path in required_files if not path.is_file()]
    if missing:
        audit.error(f"Financial data integrity scripts are missing: {missing}")
        return

    technical_code = technical_path.read_text(encoding="utf-8")
    for marker in ("fetch_market_data", "stock_zh_a_hist", 'adjust="qfq"', 'report["data_source"]', "raise SystemExit(main())"):
        if marker not in technical_code:
            audit.error(f"A-share technical analysis lacks a real-data/fail-closed marker: {marker}")
    for forbidden in (
        "fetch_mock_data",
        "import random",
        "当前使用模拟数据",
        "calculate_technical_score",
        "generate_technical_suggestions",
        "买入信号",
        "卖出信号",
        "建议单笔亏损",
    ):
        if forbidden in technical_code:
            audit.error(f"A-share technical analysis retains simulated-market behavior: {forbidden}")

    realtime_code = realtime_path.read_text(encoding="utf-8")
    for marker in (
        "REALTIME_SOURCE_INTERFACE",
        '"market_timestamp"',
        '"retrieved_at_utc"',
        '"success": not errors',
        "raise SystemExit(main())",
        "不推断主力身份",
    ):
        if marker not in realtime_code:
            audit.error(f"A-share realtime quote lacks provenance/fail-closed marker: {marker}")
    for forbidden in ('"signals": signals', "【主力动向判断】", "signals.append("):
        if forbidden in realtime_code:
            audit.error(f"A-share realtime quote retains unsupported participant inference: {forbidden}")

    data_fetcher_code = data_fetcher_path.read_text(encoding="utf-8")
    for marker in (
        '"requested_components"',
        '"sources"',
        '"completeness"',
        '"status": status',
        'parser.add_argument("--allow-partial"',
        'status == "complete"',
        "raise SystemExit(main())",
    ):
        if marker not in data_fetcher_code:
            audit.error(f"A-share data fetcher lacks provenance/completeness marker: {marker}")
    for forbidden in (
        "except IOError:\n        pass",
        'print(f"获取全部A股失败:',
        'print(f"获取指数成分股失败:',
        'return {"dividend_history": [], "dividend_count": 0}',
    ):
        if forbidden in data_fetcher_code:
            audit.error(f"A-share data fetcher retains silent-success behavior: {forbidden}")

    financial_analyzer_code = financial_analyzer_path.read_text(encoding="utf-8")
    for marker in (
        "_validate_provenance",
        '"input_retrieved_at"',
        '"period_changes"',
        '"diagnostics"',
        "不内置行业阈值、综合评分",
        "raise SystemExit(main())",
    ):
        if marker not in financial_analyzer_code:
            audit.error(f"A-share financial analyzer lacks neutral/provenance marker: {marker}")
    for forbidden in (
        "def _calculate_score",
        "def _assess_",
        '"score":',
        '"ranking":',
        '"risk_level":',
        "投资结论",
    ):
        if forbidden in financial_analyzer_code:
            audit.error(f"A-share financial analyzer retains a built-in rating or ranking: {forbidden}")

    stock_report_template = stock_report_template_path.read_text(encoding="utf-8")
    for forbidden in (
        "overall_score",
        "investment_recommendation",
        "safety_price",
        "建议买入价",
        "行业均值",
    ):
        if forbidden in stock_report_template:
            audit.error(f"A-share report template retains an unsupported score/target field: {forbidden}")

    explicit_contracts = (
        (
            cycle_path,
            (
                "validate_cycle_rules",
                'parser.add_argument("--gdp-trend", type=float, required=True',
                'parser.add_argument("--inflation-trend", type=float, required=True',
                'parser.add_argument("--data-as-of", required=True',
                'parser.add_argument("--source", action="append", required=True',
                'parser.add_argument("--rules", required=True',
                '"rules_as_of"',
                "raise SystemExit(main())",
            ),
            ("gdp_growth > 5.0", "inflation > 2.5", "allocations = {", "transitions = {", "suggestions = {", "default=5.0", "default=2.0"),
        ),
        (
            policy_path,
            (
                "validate_policy_input",
                'parser.add_argument("--input", required=True',
                '"data_as_of"',
                '"sources"',
                '"transmission_hypotheses"',
                '"scenario_implications"',
                "raise SystemExit(main())",
            ),
            ("impacts = {", "policies = {", "suggest_asset_allocation", "单只政策主题股票亏损15%"),
        ),
        (
            a_share_valuation_path,
            (
                "validate_valuation_assumptions",
                'parser.add_argument("--assumptions", required=True',
                'parser.add_argument("--data-as-of", required=True',
                'parser.add_argument("--data-source", action="append", required=True',
                '"base_free_cash_flow"',
                '"assumptions_as_of"',
                "raise SystemExit(main())",
            ),
            ("growth_rate = 10", "dividend_growth = 3", "default=10", "default=3", "fair_pe", "平均内在价值", "投资结论"),
        ),
        (
            stock_screener_path,
            ("SORT_COLUMNS", "source_interfaces", "筛选条件未执行", "raise SystemExit(main())"),
            ('add_argument("--roe-min"', 'add_argument("--debt-ratio-max"', 'add_argument("--dividend-min"', "calculate_score", "return pd.DataFrame()"),
        ),
        (
            fund_screener_path,
            ("SOURCE_INTERFACE", "matched_count_before_limit", "raise RuntimeError", "raise SystemExit(main())"),
            ('add_argument("--max-drawdown"', 'add_argument("--min-sharpe"', 'add_argument("--max-fee"', "except Exception as e:\n        return []"),
        ),
    )
    for path, required_markers, forbidden_markers in explicit_contracts:
        code = path.read_text(encoding="utf-8")
        for marker in required_markers:
            if marker not in code:
                audit.error(f"Explicit financial/macro contract {path.name} lacks marker: {marker}")
        for marker in forbidden_markers:
            if marker in code:
                audit.error(f"Explicit financial/macro contract {path.name} retains a fabricated or ignored behavior: {marker}")

    sentiment_code = sentiment_path.read_text(encoding="utf-8")
    for marker in (
        "validate_sentiment_data",
        'parser.add_argument("--input", required=True',
        'parser.add_argument("--index", required=True',
        '"data_as_of"',
        '"sources"',
        '"weights"',
        "raise SystemExit(main())",
    ):
        if marker not in sentiment_code:
            audit.error(f"Macro sentiment analysis lacks an explicit-data/fail-closed marker: {marker}")
    for forbidden in (
        "mock_data",
        "模拟数据（实际应从数据源获取）",
        "indicators.get(key, 50)",
        "suggested_action",
        "generate_sentiment_suggestions",
        "建议加仓",
        "建议减仓",
    ):
        if forbidden in sentiment_code:
            audit.error(f"Macro sentiment analysis retains fabricated defaults: {forbidden}")

    fund_risk_code = fund_risk_path.read_text(encoding="utf-8")
    for marker in (
        "validate_portfolio_data",
        'parser.add_argument("--portfolio", required=True',
        '"annual_volatility"',
        '"correlation"',
        '"data_as_of"',
        "raise SystemExit(main())",
    ):
        if marker not in fund_risk_code:
            audit.error(f"Fund risk assessment lacks an explicit-data/scenario marker: {marker}")
    for forbidden in (
        "FUND_VOLATILITY",
        "默认示例组合",
        'parser.add_argument("--funds"',
        "risk_free_rate: float =",
        '"risk_rating"',
        '"risk_score"',
        "generate_suggestions",
        "止损线",
    ):
        if forbidden in fund_risk_code:
            audit.error(f"Fund risk assessment retains a fabricated market default: {forbidden}")

    scenario_contracts = (
        (
            fund_builder_path,
            ("validate_allocation_model", 'parser.add_argument("--model", required=True', '"model_as_of"', '"sources"', '"fund_type_notes"'),
            ("get_allocation_model", 'models = {', "配置基于历史数据", '"fund_recommendations"', '"advice"', "投资官建议"),
        ),
        (
            sip_path,
            ('parser.add_argument("--expected-return", type=float, required=True', "--smart-expected-return", "scenario_notice"),
            ("base_return * 1.15", "智能定投能提升15%", "default=8"),
        ),
        (
            allocator_path,
            ("validate_policy_template", 'parser.add_argument("--template", required=True', '"policy_as_of"', '"sources"'),
            ("get_allocation_template", "determine_risk_level", "templates = {"),
        ),
        (
            rebalance_path,
            ("transaction_cost_rate", 'parser.add_argument("--transaction-cost-rate", type=float, required=True', "validate_allocations", '"methodology"'),
            ("total_sell * 0.001", "default=100000", "default=5.0", "generate_rebalance_suggestions", '"suggestions"', "止损线"),
        ),
        (
            tracker_path,
            ('parser.add_argument("--history", required=True', 'parser.add_argument("--risk-free-rate", type=float, required=True', '"data_as_of"', '"sources"', '"methodology"'),
            ('"沪深300": 8.0', "risk_free_rate: float =", "history: List[Dict] = None", "months: int = 12", '"suggestions"', "generate_suggestions", "止损线"),
        ),
        (
            business_evaluator_path,
            ("validate_business_input", 'parser.add_argument("--input", required=True', '"weighted_score_out_of_10"', '"inputs_as_of"', '"sources"'),
            ("len(value_proposition)", "competition_scores = {", "overall_score", "recommendation", 'default="中"'),
        ),
        (
            valuation_path,
            ("validate_valuation_assumptions", 'parser.add_argument("--assumptions", required=True', '"scorecard_factor_range"', '"assumptions_as_of"', '"sources"', '"method_range_envelope"'),
            ("base_valuations = {", "行业估值倍数（简化数据）", 'default="SaaS"', "generate_investment_suggestion", '"negotiation_range"', '"suggestions"'),
        ),
        (
            financial_model_path,
            (
                'parser.add_argument("--annual-net-user-growth-rate", type=float, required=True',
                'parser.add_argument("--liquidity-buffer-months", type=float, required=True',
                'parser.add_argument("--as-of", required=True',
                'parser.add_argument("--source", action="append", required=True',
                '"inputs_as_of"',
                '"sources"',
            ),
            ("ltv = arpu * 12", "recommended_rounds", "calculate_funding_rounds", "healthy\"", "default=1000", "default=100000"),
        ),
        (
            due_diligence_path,
            (
                'parser.add_argument("--jurisdiction", required=True',
                'parser.add_argument("--industry", required=True',
                'parser.add_argument("--transaction-structure", required=True',
                '"template_notice"',
                '"status": "not_reviewed"',
                '"applicability": "confirm"',
                "raise SystemExit(main())",
            ),
            ("generate_dd_timeline", '"timeline"', '"owner"', '"high_priority"', "适合谁", "大幅压低估值"),
        ),
    )
    for path, required_markers, forbidden_markers in scenario_contracts:
        code = path.read_text(encoding="utf-8")
        for marker in required_markers:
            if marker not in code:
                audit.error(f"Financial scenario entrypoint {path.name} lacks marker: {marker}")
        for marker in forbidden_markers:
            if marker in code:
                audit.error(f"Financial scenario entrypoint {path.name} retains implicit/fabricated assumption: {marker}")

    if len(audit.errors) == errors_before:
        audit.passed("Financial analysis scripts require real, attributable inputs and fail closed")


def check_no_placeholder_success(audit: Audit) -> None:
    errors_before = len(audit.errors)
    reflection_path = audit.root / "skills" / "daily-reflection" / "scripts" / "reflection_db.py"
    if not reflection_path.is_file():
        audit.error("Daily reflection implementation is missing.")
        return
    reflection_code = reflection_path.read_text(encoding="utf-8")
    for marker in ("Counter(normalized)", "monthly_themes", "sorted(counts.items()"):
        if marker not in reflection_code:
            audit.error(f"Daily reflection theme extraction lacks implementation marker: {marker}")
    theme_body = reflection_code.split("def extract_themes", 1)[-1].split("\n\n# ====", 1)[0]
    if "placeholder for theme extraction" in reflection_code or "return []" in theme_body:
        audit.error("Daily reflection still reports successful monthly themes from a placeholder implementation.")
    polish_path = audit.root / "skills" / "wechat-article-creator" / "scripts" / "polish_text.py"
    if not polish_path.is_file():
        audit.error("WeChat editorial checker is missing.")
    else:
        polish_code = polish_path.read_text(encoding="utf-8")
        if "--auto-fix" in polish_code:
            audit.error("WeChat editorial checker still advertises an unimplemented --auto-fix option.")
        for forbidden in ("OVERALL SCORE", "Ready to publish"):
            if forbidden in polish_code:
                audit.error(f"WeChat editorial checker presents heuristic output as publication readiness: {forbidden}")
    if len(audit.errors) == errors_before:
        audit.passed("Advertised local entrypoints do not retain known placeholder-success behavior")


def check_windows_console_literals(audit: Audit) -> None:
    """Catch direct CLI status literals that crash under the target Windows GBK console."""
    errors_before = len(audit.errors)
    for path in sorted((audit.root / "skills").glob("*/scripts/*.py")):
        for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if "print(" not in line:
                continue
            try:
                line.encode("gbk")
            except UnicodeEncodeError:
                relative = path.relative_to(audit.root).as_posix()
                audit.error(f"Windows GBK-unsafe direct print literal: {relative}:{line_number}")
    if len(audit.errors) == errors_before:
        audit.passed("Direct Python CLI status literals are safe on the target Windows console")


def check_pack(audit: Audit) -> None:
    errors_before = len(audit.errors)
    pack_path = audit.root / "pack.json"
    if not pack_path.is_file():
        return
    pack = load_json(pack_path)
    if not re.fullmatch(r"[0-9a-f]{40}", str(pack.get("sourceCommit", ""))):
        audit.error("pack.json sourceCommit is not a 40-character commit.")
    if str(pack.get("sourceCommitRole", "")) not in {"build-input", "upstream-base"}:
        audit.error("pack.json sourceCommitRole must be build-input or upstream-base.")
    if not re.fullmatch(r"[0-9a-f]{64}", str(pack.get("sourceTreeSha256", ""))):
        audit.error("pack.json sourceTreeSha256 is not a SHA-256 digest.")
    file_manifest_path = audit.root / "file-manifest.json"
    if not file_manifest_path.is_file():
        audit.error("Pack is missing file-manifest.json.")
        return
    file_manifest = load_json(file_manifest_path)
    for entry in file_manifest.get("files", []):
        relative = str(entry.get("path", ""))
        path = (audit.root / relative).resolve()
        try:
            path.relative_to(audit.root)
        except ValueError:
            audit.error(f"Pack manifest path escapes root: {relative}")
            continue
        if not path.is_file():
            audit.error(f"Pack manifest target is missing: {relative}")
            continue
        digest = hashlib.sha256(canonical_pack_bytes(path)).hexdigest()
        if digest != entry.get("sha256"):
            audit.error(f"Pack hash mismatch: {relative}")
    if len(audit.errors) == errors_before:
        audit.passed("Pack provenance and per-file hashes")


def check_model_catalog(audit: Audit) -> None:
    errors_before = len(audit.errors)
    catalog_path = audit.root / "model-catalog.json"
    script_root = audit.root / "scripts" if (audit.root / "scripts").is_dir() else audit.root
    sync_path = audit.root / "sync-model-catalog.ps1"
    if not sync_path.is_file():
        sync_path = script_root / "sync-model-catalog.ps1"
    installer_path = audit.root / "install-to-codex.ps1"
    if not installer_path.is_file():
        installer_path = script_root / "install-to-codex.ps1"
    if not catalog_path.is_file() or not sync_path.is_file() or not installer_path.is_file():
        audit.error("Model catalog, synchronizer, or installer integration is missing.")
        return
    raw = catalog_path.read_text(encoding="utf-8")
    if re.search(
        r"(?i)model_messages|persistent_instructions|instructions_template|api[_-]?key|app[_-]?secret|access[_-]?token|password|bearer",
        raw,
    ):
        audit.error("Model catalog contains runtime instructions or credential-like fields.")
    catalog = load_json(catalog_path)
    for error in validate_model_catalog(catalog):
        audit.error(error)
    if not isinstance(catalog, dict):
        return
    retired_ids = catalog.get("retiredModelIds")
    if not isinstance(retired_ids, list) or "gpt-5.2" not in retired_ids:
        audit.error("Model catalog must explicitly record retired gpt-5.2.")
    sync_text = sync_path.read_text(encoding="utf-8-sig")
    if "SupportsShouldProcess" not in sync_text or ".ShouldProcess(" not in sync_text or "'planned'" not in sync_text:
        audit.error("Model synchronizer must support ShouldProcess/WhatIf and emit planned status.")
    installer_text = installer_path.read_text(encoding="utf-8")
    if "sync-model-catalog.ps1" not in installer_text or "model-catalog.json" not in installer_text:
        audit.error("Capability installer does not invoke/install the model synchronization contract.")
    if any(path.name.casefold() == "models_cache.json" for path in audit.root.rglob("*")):
        audit.error("Raw models_cache.json must never be shipped in the public capability pack.")
    if len(audit.errors) == errors_before:
        audit.passed("Sanitized upstream model catalog and default-model migration contract")


def check_codex(audit: Audit, required: bool) -> None:
    executable = shutil.which("codex")
    if not executable:
        message = "Codex CLI is not available on PATH."
        (audit.error if required else audit.warn)(message)
        return
    try:
        result = subprocess.run([executable, "--version"], check=True, capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.SubprocessError) as exc:
        (audit.error if required else audit.warn)(f"Could not query Codex CLI: {exc}")
        return
    match = re.search(r"(\d+)\.(\d+)\.(\d+)", result.stdout + result.stderr)
    if not match:
        audit.error(f"Could not parse Codex version: {(result.stdout + result.stderr).strip()}")
        return
    version = tuple(int(value) for value in match.groups())
    audit.metrics["codexVersion"] = ".".join(str(value) for value in version)
    if version < MIN_CODEX_VERSION:
        audit.error(f"Codex {audit.metrics['codexVersion']} is older than the minimum 0.144.0.")
    else:
        audit.passed(f"Codex CLI {audit.metrics['codexVersion']} compatibility floor")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", help="Source tree or generated installer-pack root")
    parser.add_argument("--require-codex", action="store_true", help="Fail when the Codex CLI cannot be queried")
    parser.add_argument("--json", action="store_true", help="Emit one machine-readable JSON object")
    args = parser.parse_args()
    root = find_root(args.root)
    audit = Audit(root)
    try:
        linked_items = []
        for path in root.rglob("*"):
            is_junction = getattr(path, "is_junction", lambda: False)
            if path.is_symlink() or is_junction():
                linked_items.append(path.relative_to(root).as_posix())
        if linked_items:
            audit.error(f"Source/pack contains symbolic links or junctions: {', '.join(linked_items[:5])}")
        else:
            audit.passed("No symbolic links or junctions in the auditable tree")
        skill_ids = manifest_skill_ids(root)
        check_skills(audit, skill_ids)
        check_agents(audit)
        check_openai_metadata(audit)
        check_feishu(audit)
        check_audit_statuses(audit)
        check_external_adapters(audit)
        check_local_write_boundaries(audit)
        check_financial_data_truthfulness(audit)
        check_no_placeholder_success(audit)
        check_windows_console_literals(audit)
        check_model_catalog(audit)
        check_pack(audit)
        check_codex(audit, args.require_codex)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
        audit.error(f"Audit could not complete: {exc}")
    result = {
        "schemaVersion": 1,
        "root": str(root),
        "ok": not audit.errors,
        "metrics": audit.metrics,
        "checks": audit.checks,
        "warnings": audit.warnings,
        "errors": audit.errors,
    }
    if args.json:
        print(json.dumps(result, ensure_ascii=False, indent=2))
    else:
        print(f"Codex compatibility audit: {'PASS' if result['ok'] else 'FAIL'}")
        for check in audit.checks:
            print(f"  PASS  {check}")
        for warning in audit.warnings:
            print(f"  WARN  {warning}")
        for error in audit.errors:
            print(f"  FAIL  {error}")
        print("  METRICS " + json.dumps(audit.metrics, ensure_ascii=False, sort_keys=True))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
