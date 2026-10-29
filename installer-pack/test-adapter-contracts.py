"""Offline regression tests for package adapters and financial data integrity."""

from __future__ import annotations

import importlib.util
import http.server
import base64
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import unittest
import urllib.error
import urllib.parse
import uuid
import struct
import zlib
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


sys.dont_write_bytecode = True

_SCRIPT_DIRECTORY = Path(__file__).resolve().parent
ROOT = _SCRIPT_DIRECTORY if (_SCRIPT_DIRECTORY / "skills").is_dir() else _SCRIPT_DIRECTORY.parent
SCRIPT_ROOT = _SCRIPT_DIRECTORY if (_SCRIPT_DIRECTORY / "configure-codex.ps1").is_file() else ROOT / "scripts"
MEDIA_MCP_SCRIPT = (ROOT / "mcp" / "media-generation-mcp.py") if (ROOT / "mcp" / "media-generation-mcp.py").is_file() else (SCRIPT_ROOT / "media-generation-mcp.py")
MEDIA_MCP_WRAPPER = (ROOT / "mcp" / "start-media-generation-mcp.ps1") if (ROOT / "mcp" / "start-media-generation-mcp.ps1").is_file() else (SCRIPT_ROOT / "start-media-generation-mcp.ps1")
WEB_SCRIPT = ROOT / "skills" / "web-search-extraction" / "scripts" / "web_search.py"
TECHNICAL_SCRIPT = ROOT / "skills" / "china-stock-analysis" / "scripts" / "technical_analysis.py"
A_SHARE_VALUATION_SCRIPT = ROOT / "skills" / "china-stock-analysis" / "scripts" / "valuation_calculator.py"
STOCK_SCREENER_SCRIPT = ROOT / "skills" / "china-stock-analysis" / "scripts" / "stock_screener.py"
REALTIME_QUOTE_SCRIPT = ROOT / "skills" / "china-stock-analysis" / "scripts" / "realtime_quote.py"
STOCK_DATA_FETCHER_SCRIPT = ROOT / "skills" / "china-stock-analysis" / "scripts" / "data_fetcher.py"
FINANCIAL_ANALYZER_SCRIPT = ROOT / "skills" / "china-stock-analysis" / "scripts" / "financial_analyzer.py"
STOCK_REPORT_TEMPLATE = ROOT / "skills" / "china-stock-analysis" / "templates" / "analysis_report.md"
CONFIGURE_SCRIPT = SCRIPT_ROOT / "configure-codex.ps1"
RENDER_SCRIPT = SCRIPT_ROOT / "render-codex-config.ps1"
PDF_TOOL_SCRIPT = ROOT / "skills" / "pdf-processing-toolkit" / "scripts" / "pdf_tool.py"
MODEL_SYNC_SCRIPT = SCRIPT_ROOT / "sync-model-catalog.ps1"
POWERSHELL = shutil.which("powershell.exe") or shutil.which("pwsh.exe") or shutil.which("pwsh")
MACRO_CYCLE_SCRIPT = ROOT / "skills" / "macro-research" / "scripts" / "cycle_analyzer.py"
POLICY_SCRIPT = ROOT / "skills" / "macro-research" / "scripts" / "policy_analyzer.py"
SENTIMENT_SCRIPT = ROOT / "skills" / "macro-research" / "scripts" / "sentiment_monitor.py"
FUND_SCREENER_SCRIPT = ROOT / "skills" / "fund-portfolio" / "scripts" / "fund_screener.py"
FUND_RISK_SCRIPT = ROOT / "skills" / "fund-portfolio" / "scripts" / "risk_assessor.py"
FUND_BUILDER_SCRIPT = ROOT / "skills" / "fund-portfolio" / "scripts" / "portfolio_builder.py"
SIP_SCRIPT = ROOT / "skills" / "fund-portfolio" / "scripts" / "sip_calculator.py"
ALLOCATOR_SCRIPT = ROOT / "skills" / "wealth-allocation" / "scripts" / "asset_allocator.py"
REBALANCE_SCRIPT = ROOT / "skills" / "wealth-allocation" / "scripts" / "rebalance_planner.py"
TRACKER_SCRIPT = ROOT / "skills" / "wealth-allocation" / "scripts" / "portfolio_tracker.py"
VALUATION_SCRIPT = ROOT / "skills" / "venture-analysis" / "scripts" / "valuation_analyzer.py"
BUSINESS_EVALUATOR_SCRIPT = ROOT / "skills" / "venture-analysis" / "scripts" / "business_evaluator.py"
FINANCIAL_MODEL_SCRIPT = ROOT / "skills" / "venture-analysis" / "scripts" / "financial_model.py"
DUE_DILIGENCE_SCRIPT = ROOT / "skills" / "venture-analysis" / "scripts" / "due_diligence.py"
REFLECTION_SCRIPT = ROOT / "skills" / "daily-reflection" / "scripts" / "reflection_db.py"
ADAPTER_ENV_PREFIXES = ("MEDIA_", "IMAGE_2_", "SEEDANCE_", "DASHSCOPE_")


@contextmanager
def adapter_environment(values=None):
    original = dict(os.environ)
    try:
        for key in list(os.environ):
            if key.startswith(ADAPTER_ENV_PREFIXES) or key == "OPENAI_API_KEY":
                os.environ.pop(key, None)
        values = dict(values or {})
        os.environ.update(values)
        config_path = values.get("MEDIA_GENERATION_CONFIG_PATH")
        if config_path and values.get("MEDIA_GENERATION_TEST_MODE") == "1":
            gateway = str(values.get("MEDIA_GENERATION_API_BASE_URL", "")).rstrip("/")
            profile = str(values.get("MEDIA_GENERATION_API_PROFILE", "auto")).casefold()
            if profile == "auto":
                host = urllib.parse.urlsplit(gateway).hostname or ""
                profile = "wokey" if host == "api.wokey.ai" else "responses" if host in {"404token.xyz", "api.404token.xyz"} else "legacy"
            model_envs = {
                "image-2": "MEDIA_IMAGE2_MODEL", "grok-image": "MEDIA_GROK_IMAGE_MODEL",
                "grok-video": "MEDIA_GROK_VIDEO_MODEL", "seedance-video": "MEDIA_SEEDANCE_MODEL",
                "minimax-video": "MEDIA_MINIMAX_VIDEO_MODEL",
            }
            models = {provider: str(values.get(env_name, "")).strip() for provider, env_name in model_envs.items()}
            Path(config_path).parent.mkdir(parents=True, exist_ok=True)
            Path(config_path).write_text(json.dumps({"schemaVersion": 1, "gateway": gateway, "profile": profile,
                "models": models, "catalogVerifiedAt": int(time.time()), "catalogIds": sorted({value for value in models.values() if value})}), encoding="utf-8")
        yield
    finally:
        os.environ.clear()
        os.environ.update(original)


def load_module(path: Path, environment=None):
    with adapter_environment(environment):
        name = f"adapter_contract_{path.stem}_{uuid.uuid4().hex}"
        spec = importlib.util.spec_from_file_location(name, path)
        if spec is None or spec.loader is None:
            raise RuntimeError(f"Cannot import {path}")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module


@unittest.skipUnless(POWERSHELL, "PowerShell is required for configure-codex.ps1 contract tests")
class CodexConfigureContractTests(unittest.TestCase):
    fragment = (
        "# Generated by codex-starter-kit. Review and merge manually into ~/.codex/config.toml.\n"
        "[mcp_servers.feishu]\n"
        "enabled = false\n"
        "command = \"powershell.exe\"\n"
        "args = [\"-NoProfile\"]\n"
    )

    def invoke(self, root: Path, existing=None, fragment=None, target_codex_home=None, enable_full_access=False):
        fragment = fragment or self.fragment
        fragment_path = root / "generated-fragment.toml"
        fragment_path.write_bytes(fragment.encode("utf-8"))
        codex_home = target_codex_home or (root / "profile" / ".codex")
        config_path = codex_home / "config.toml"
        if existing is not None:
            config_path.parent.mkdir(parents=True, exist_ok=True)
            config_path.write_bytes(existing.encode("utf-8"))
        arguments = [
                POWERSHELL,
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                str(CONFIGURE_SCRIPT),
                "-GeneratedFragment",
                str(fragment_path),
                "-TargetUserProfile",
                str(root / "profile"),
                "-KitHome",
                str(root / "kit"),
                "-PythonExe",
                sys.executable,
            ]
        if target_codex_home is not None:
            arguments.extend(["-TargetCodexHome", str(target_codex_home)])
        if enable_full_access:
            arguments.append("-EnableFullAccess")
        completed = subprocess.run(
            arguments,
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        self.assertEqual(
            completed.returncode,
            0,
            completed.stdout + "\n" + completed.stderr,
        )
        return json.loads(completed.stdout.lstrip("\ufeff")), config_path

    def test_exact_match_legacy_feishu_block_migrates_idempotently(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = "[mcp_servers.other]\nvalue = 1\n\n" + self.fragment.replace("\n", "\r\n")
            first, config_path = self.invoke(root, legacy)
            self.assertEqual(first["status"], "merged")
            self.assertTrue(first["merged"])
            first_content = config_path.read_text(encoding="utf-8-sig")
            self.assertEqual(first_content.count("# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1"), 1)
            self.assertEqual(first_content.count("[mcp_servers.feishu]"), 1)
            self.assertIn("[mcp_servers.other]", first_content)

            second, _ = self.invoke(root)
            self.assertEqual(second["status"], "merged")
            self.assertEqual(config_path.read_text(encoding="utf-8-sig"), first_content)

    def test_modified_legacy_feishu_block_remains_conflict(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            modified = self.fragment.replace("enabled = false", "enabled = true")
            legacy = "[mcp_servers.other]\nvalue = 1\n\n" + modified
            result, config_path = self.invoke(root, legacy)
            self.assertEqual(result["status"], "conflict")
            self.assertFalse(result["merged"])
            self.assertEqual(config_path.read_text(encoding="utf-8"), legacy)

    def test_comments_and_nested_tables_are_migrated_without_truncating_array_tables(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fragment = (
                "# Generated by codex-starter-kit.\n"
                "[mcp_servers.feishu]\n"
                "enabled = false\n"
                "[mcp_servers.feishu.tools.config]\n"
                "enabled = true\n"
                "[[mcp_servers.feishu.tool]]\n"
                'name = "read"\n'
            )
            legacy = (
                "[mcp_servers.other]\nvalue = 1\n\n"
                "# A harmless legacy comment\n\n"
                "[mcp_servers.feishu]\n"
                "\n"
                "enabled = false\n"
                "\n"
                "[mcp_servers.feishu.tools.config]\n"
                "enabled = true\n"
                "# Preserve this comment too\n"
                "[[mcp_servers.feishu.tool]]\n"
                'name = "read"\n'
                "\n[mcp_servers.after]\nvalue = 2\n"
            )
            first, config_path = self.invoke(root, legacy, fragment)
            self.assertEqual(first["status"], "merged")
            first_content = config_path.read_text(encoding="utf-8-sig")
            self.assertEqual(first_content.count("# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1"), 1)
            self.assertEqual(first_content.count("[[mcp_servers.feishu.tool]]"), 1)
            self.assertIn("[mcp_servers.after]", first_content)

            second, _ = self.invoke(root, fragment=fragment)
            self.assertEqual(second["status"], "merged")
            self.assertEqual(config_path.read_text(encoding="utf-8-sig"), first_content)

    def test_modified_nested_array_table_remains_conflict(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fragment = (
                "[mcp_servers.feishu]\n"
                "enabled = false\n"
                "[[mcp_servers.feishu.tool]]\n"
                'name = "read"\n'
            )
            modified = fragment.replace('name = "read"', 'name = "write"')
            result, config_path = self.invoke(root, modified, fragment)
            self.assertEqual(result["status"], "conflict")
            self.assertFalse(result["merged"])
            self.assertEqual(config_path.read_text(encoding="utf-8"), modified)

    def test_usb_full_access_is_isolated_and_idempotent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            codex_home = root / "localappdata" / "CodexAppManager" / "usb-codex-home"
            existing = (
                'approval_policy = "on-request"\n'
                'sandbox_mode = "workspace-write"\n\n'
                "[model_providers.demo]\nname = 'demo'\n"
            )
            first, config_path = self.invoke(
                root,
                existing=existing,
                target_codex_home=codex_home,
                enable_full_access=True,
            )
            self.assertEqual(first["status"], "merged")
            second, _ = self.invoke(
                root,
                target_codex_home=codex_home,
                enable_full_access=True,
            )
            self.assertEqual(second["status"], "merged")
            content = config_path.read_text(encoding="utf-8")
            parsed = tomllib.loads(content)
            self.assertEqual(parsed["approval_policy"], "never")
            self.assertEqual(parsed["sandbox_mode"], "danger-full-access")
            self.assertEqual(content.count("# BEGIN USB CODEX FULL ACCESS v1"), 1)
            self.assertIn("[model_providers.demo]", content)
            self.assertFalse((root / "profile" / ".codex" / "config.toml").exists())

    def test_custom_codex_home_without_usb_switch_does_not_enable_full_access(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            codex_home = root / "custom-codex-home"
            _, config_path = self.invoke(root, target_codex_home=codex_home)
            content = config_path.read_text(encoding="utf-8")
            parsed = tomllib.loads(content)
            self.assertNotIn("approval_policy", parsed)
            self.assertNotIn("sandbox_mode", parsed)

    def test_usb_full_access_is_written_even_when_feishu_is_unmanaged(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            codex_home = root / "usb-home"
            existing = (
                'approval_policy = "on-request"\n'
                "[model_providers.demo]\nname = 'demo'\n\n"
                "[mcp_servers.feishu]\nenabled = true\n"
            )
            result, config_path = self.invoke(
                root,
                existing=existing,
                target_codex_home=codex_home,
                enable_full_access=True,
            )
            self.assertEqual(result["status"], "conflict")
            content = config_path.read_text(encoding="utf-8")
            parsed = tomllib.loads(content)
            self.assertEqual(parsed["approval_policy"], "never")
            self.assertEqual(parsed["sandbox_mode"], "danger-full-access")
            self.assertTrue(parsed["mcp_servers"]["feishu"]["enabled"])
            self.assertIn("[model_providers.demo]", content)


@unittest.skipUnless(POWERSHELL, "PowerShell is required for Skill activation contract tests")
class SkillActivationContractTests(unittest.TestCase):
    def test_renderer_emits_manifest_derived_skill_entries(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            audit = json.loads((ROOT / "manifest" / "skill-audit.json").read_text(encoding="utf-8"))
            expected_guided = {
                skill["id"]
                for skill in audit["skills"]
                if skill.get("afterRemediationStatus") == "guided-config"
            }
            expected_total = len(audit["skills"])
            expected_enabled = expected_total - len(expected_guided)
            output = root / "generated.toml"
            completed = subprocess.run(
                [
                    POWERSHELL,
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    str(RENDER_SCRIPT),
                    "-OutputPath",
                    str(output),
                    "-SkillRoot",
                    str(ROOT / "skills"),
                    "-TargetUserProfile",
                    str(root / "profile"),
                    "-PythonExe",
                    sys.executable,
                    "-Overwrite",
                ],
                check=False,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
            self.assertEqual(completed.returncode, 0, completed.stdout + "\n" + completed.stderr)
            status = json.loads(completed.stdout.lstrip("\ufeff"))
            self.assertEqual(status["totalSkills"], expected_total)
            self.assertEqual(status["enabledSkillCount"], expected_enabled)
            self.assertEqual(status["disabledSkillCount"], len(expected_guided))
            self.assertEqual(set(status["disabledSkills"]), expected_guided)
            config = tomllib.loads(output.read_text(encoding="utf-8-sig"))
            entries = config["skills"]["config"]
            self.assertEqual(len(entries), expected_total)
            self.assertEqual(sum(1 for entry in entries if entry["enabled"]), expected_enabled)
            self.assertEqual(sum(1 for entry in entries if not entry["enabled"]), len(expected_guided))

    def render_codex_config(self, output: Path, profile: Path, codex_home: Path, media_wrapper: Path | None = None):
        arguments = [
                POWERSHELL,
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                str(RENDER_SCRIPT),
                "-OutputPath",
                str(output),
                "-SkillRoot",
                str(ROOT / "skills"),
                "-TargetUserProfile",
                str(profile),
                "-TargetCodexHome",
                str(codex_home),
                "-PythonExe",
                sys.executable,
                "-Overwrite",
            ]
        if media_wrapper is not None:
            arguments.extend(["-EnableMediaGeneration", "-MediaMcpWrapperPath", str(media_wrapper)])
        return subprocess.run(
            arguments,
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )

    def merge_codex_config(self, fragment: Path, profile: Path, codex_home: Path, kit_home: Path):
        return subprocess.run(
            [
                POWERSHELL,
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                str(CONFIGURE_SCRIPT),
                "-GeneratedFragment",
                str(fragment),
                "-TargetUserProfile",
                str(profile),
                "-TargetCodexHome",
                str(codex_home),
                "-KitHome",
                str(kit_home),
                "-PythonExe",
                sys.executable,
            ],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )

    def test_renderer_enables_all_bundled_plugins_and_servers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile = root / "profile"
            codex_home = root / "usb-codex-home"
            kit_home = root / "kit"
            output = root / "generated.toml"
            completed = self.render_codex_config(output, profile, codex_home)
            self.assertEqual(completed.returncode, 0, completed.stdout + "\n" + completed.stderr)

            status = json.loads(completed.stdout.lstrip("\ufeff"))
            expected = {
                "codex-app-tools",
                "browser",
                "unified-computer-use",
                "chrome",
                "computer-use",
                "code-review",
                "latex",
                "deep-research",
                "visualize",
                "image-guided-assets",
            }
            self.assertEqual(set(status["enabledBundledPlugins"]), expected)
            self.assertEqual(status["preservedBundledPluginSettings"], [])
            config = tomllib.loads(output.read_text(encoding="utf-8-sig"))
            plugins = config["plugins"]
            for plugin_id in expected:
                self.assertTrue(plugins[f"{plugin_id}@openai-bundled"]["enabled"])
            self.assertTrue(plugins["codex-app-tools@openai-bundled"]["mcp_servers"]["codex_app"]["enabled"])
            self.assertTrue(plugins["unified-computer-use@openai-bundled"]["mcp_servers"]["cua_repl"]["enabled"])
            self.assertTrue(plugins["code-review@openai-bundled"]["mcp_servers"]["code-review"]["enabled"])

            merged = self.merge_codex_config(output, profile, codex_home, kit_home)
            self.assertEqual(merged.returncode, 0, merged.stdout + "\n" + merged.stderr)
            installed_config = tomllib.loads((codex_home / "config.toml").read_text(encoding="utf-8-sig"))
            installed_plugins = installed_config["plugins"]
            for plugin_id in expected:
                self.assertTrue(installed_plugins[f"{plugin_id}@openai-bundled"]["enabled"])
            self.assertTrue(installed_plugins["codex-app-tools@openai-bundled"]["mcp_servers"]["codex_app"]["enabled"])
            self.assertTrue(installed_plugins["unified-computer-use@openai-bundled"]["mcp_servers"]["cua_repl"]["enabled"])
            self.assertTrue(installed_plugins["code-review@openai-bundled"]["mcp_servers"]["code-review"]["enabled"])

    def test_renderer_preserves_existing_plugin_choices_and_repeated_merge(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile = root / "profile"
            codex_home = root / "usb-codex-home"
            kit_home = root / "kit"
            codex_home.mkdir(parents=True)
            config_path = codex_home / "config.toml"
            config_path.write_text(
                '[mcp_servers.custom_local]\nenabled = true\nurl = "https://example.invalid/mcp"\n\n'
                "# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1\n"
                '[mcp_servers.media_generation]\nenabled = true\ncommand = "old-media-mcp"\n'
                "# END CODEX-STARTER-KIT MANAGED CONFIG v1\n\n"
                '[plugins."browser@openai-bundled"]\nenabled = false\n\n'
                '[plugins."codex-app-tools@openai-bundled"]\nenabled = false\n\n'
                '[plugins."codex-app-tools@openai-bundled".mcp_servers.codex_app]\nenabled = false\n\n'
                '[plugins."unified-computer-use@openai-bundled"]\nenabled = false\n\n'
                '[plugins."unified-computer-use@openai-bundled".mcp_servers.cua_repl]\nenabled = false\n\n'
                '[plugins."chrome@openai-bundled"]\nenabled = false\n\n'
                '[plugins."computer-use@openai-bundled"]\nenabled = false\n\n'
                '[plugins."code-review@openai-bundled"]\nenabled = false\n\n'
                '[plugins."code-review@openai-bundled".mcp_servers.code-review]\nenabled = false\n',
                encoding="utf-8",
            )

            media_wrapper = root / "media-health.cmd"
            media_wrapper.write_text(
                "@echo off\n"
                'echo {"ready":true,"serverVersion":"1.3.11","healthScope":"local-runtime-protocol",'
                '"runtimeStatus":"passed","protocolStatus":"passed","gatewayCatalogStatus":"not_checked",'
                '"generationCapability":"not_tested","toolCount":6}\n'
                "exit /b 0\n",
                encoding="utf-8",
            )

            output = root / "generated.toml"
            rendered = self.render_codex_config(output, profile, codex_home, media_wrapper)
            self.assertEqual(rendered.returncode, 0, rendered.stdout + "\n" + rendered.stderr)
            status = json.loads(rendered.stdout.lstrip("\ufeff"))
            self.assertEqual(
                set(status["enabledBundledPlugins"]),
                {"latex", "deep-research", "visualize", "image-guided-assets"},
            )
            self.assertEqual(
                set(status["preservedBundledPluginSettings"]),
                {"browser", "codex-app-tools", "unified-computer-use", "chrome", "computer-use", "code-review"},
            )

            merged = self.merge_codex_config(output, profile, codex_home, kit_home)
            self.assertEqual(merged.returncode, 0, merged.stdout + "\n" + merged.stderr)
            first_content = config_path.read_text(encoding="utf-8-sig")
            parsed = tomllib.loads(first_content)
            self.assertTrue(parsed["mcp_servers"]["media_generation"]["enabled"])
            self.assertTrue(parsed["mcp_servers"]["custom_local"]["enabled"])
            self.assertFalse(parsed["plugins"]["browser@openai-bundled"]["enabled"])
            self.assertFalse(parsed["plugins"]["codex-app-tools@openai-bundled"]["enabled"])
            self.assertFalse(parsed["plugins"]["codex-app-tools@openai-bundled"]["mcp_servers"]["codex_app"]["enabled"])
            self.assertFalse(parsed["plugins"]["unified-computer-use@openai-bundled"]["enabled"])
            self.assertFalse(parsed["plugins"]["unified-computer-use@openai-bundled"]["mcp_servers"]["cua_repl"]["enabled"])
            self.assertFalse(parsed["plugins"]["chrome@openai-bundled"]["enabled"])
            self.assertFalse(parsed["plugins"]["computer-use@openai-bundled"]["enabled"])
            self.assertFalse(parsed["plugins"]["code-review@openai-bundled"]["enabled"])
            self.assertFalse(parsed["plugins"]["code-review@openai-bundled"]["mcp_servers"]["code-review"]["enabled"])

            rendered_again = self.render_codex_config(output, profile, codex_home, media_wrapper)
            self.assertEqual(rendered_again.returncode, 0, rendered_again.stdout + "\n" + rendered_again.stderr)
            merged_again = self.merge_codex_config(output, profile, codex_home, kit_home)
            self.assertEqual(merged_again.returncode, 0, merged_again.stdout + "\n" + merged_again.stderr)
            self.assertEqual(config_path.read_text(encoding="utf-8-sig"), first_content)

    def test_renderer_preserves_plugin_disables_inside_managed_block(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile = root / "profile"
            codex_home = root / "usb-codex-home"
            kit_home = root / "kit"
            codex_home.mkdir(parents=True)
            config_path = codex_home / "config.toml"
            config_path.write_text(
                "# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1\n"
                '[plugins."browser@openai-bundled"]\nenabled = false\n\n'
                '[plugins."codex-app-tools@openai-bundled"]\nenabled = true\n\n'
                '[plugins."codex-app-tools@openai-bundled".mcp_servers.codex_app]\nenabled = false\n'
                "# END CODEX-STARTER-KIT MANAGED CONFIG v1\n",
                encoding="utf-8",
            )

            media_wrapper = root / "media-health.cmd"
            media_wrapper.write_text(
                "@echo off\n"
                'echo {"ready":true,"serverVersion":"1.3.11","healthScope":"local-runtime-protocol",'
                '"runtimeStatus":"passed","protocolStatus":"passed","gatewayCatalogStatus":"not_checked",'
                '"generationCapability":"not_tested","toolCount":6}\n'
                "exit /b 0\n",
                encoding="utf-8",
            )

            output = root / "generated.toml"
            rendered = self.render_codex_config(output, profile, codex_home, media_wrapper)
            self.assertEqual(rendered.returncode, 0, rendered.stdout + "\n" + rendered.stderr)
            status = json.loads(rendered.stdout.lstrip("\ufeff"))
            self.assertEqual(set(status["preservedBundledPluginSettings"]), {"browser", "codex-app-tools"})

            merged = self.merge_codex_config(output, profile, codex_home, kit_home)
            self.assertEqual(merged.returncode, 0, merged.stdout + "\n" + merged.stderr)
            first_content = config_path.read_text(encoding="utf-8-sig")
            plugins = tomllib.loads(first_content)["plugins"]
            self.assertFalse(plugins["browser@openai-bundled"]["enabled"])
            self.assertTrue(plugins["codex-app-tools@openai-bundled"]["enabled"])
            self.assertFalse(plugins["codex-app-tools@openai-bundled"]["mcp_servers"]["codex_app"]["enabled"])
            self.assertTrue(plugins["unified-computer-use@openai-bundled"]["enabled"])

            rendered_again = self.render_codex_config(output, profile, codex_home, media_wrapper)
            self.assertEqual(rendered_again.returncode, 0, rendered_again.stdout + "\n" + rendered_again.stderr)
            merged_again = self.merge_codex_config(output, profile, codex_home, kit_home)
            self.assertEqual(merged_again.returncode, 0, merged_again.stdout + "\n" + merged_again.stderr)
            self.assertEqual(config_path.read_text(encoding="utf-8-sig"), first_content)

    @unittest.skipUnless(PDF_TOOL_SCRIPT.is_file(), "Bundled PDF helper is missing")
    def test_pdf_smoke_uses_read_extract_write_and_generate_paths(self):
        completed = subprocess.run(
            [sys.executable, str(PDF_TOOL_SCRIPT), "smoke"],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        self.assertEqual(completed.returncode, 0, completed.stdout + "\n" + completed.stderr)
        self.assertIn("pdf smoke: passed", completed.stdout)


@unittest.skipUnless(POWERSHELL, "PowerShell is required for model catalog synchronization contract tests")
class ModelCatalogSyncContractTests(unittest.TestCase):
    def catalog(self):
        return {
            "schemaVersion": 1,
            "source": "test-catalog",
            "snapshotVersion": "2026-09-13T00:00:00Z",
            "clientVersion": "0.153.4",
            "preferredModel": "gpt-6-astra",
            "retiredModelIds": ["gpt-5.2"],
            "models": [
                {
                    "id": "gpt-6-astra",
                    "displayName": "GPT-6-Astra",
                    "visibility": "list",
                    "supportedInApi": True,
                    "priority": 1,
                    "defaultReasoningLevel": "medium",
                    "reasoningLevels": ["low", "medium", "high"],
                },
                {
                    "id": "gpt-reserve",
                    "displayName": "GPT-Reserve",
                    "visibility": "hide",
                    "supportedInApi": True,
                    "priority": 3,
                    "defaultReasoningLevel": "medium",
                    "reasoningLevels": ["low", "medium", "high"],
                },
                {
                    "id": "gpt-5.3-codex-spark",
                    "displayName": "GPT-5.3-Codex-Spark",
                    "visibility": "list",
                    "supportedInApi": False,
                    "priority": 26,
                    "defaultReasoningLevel": "high",
                    "reasoningLevels": ["low", "medium", "high"],
                },
            ],
        }

    def invoke(self, root: Path, catalog, config=None, what_if=False):
        pack_root = root / "pack"
        kit_home = root / "kit"
        profile = root / "profile"
        pack_root.mkdir(parents=True, exist_ok=True)
        (pack_root / "model-catalog.json").write_text(json.dumps(catalog), encoding="utf-8")
        config_path = profile / ".codex" / "config.toml"
        if config is not None:
            config_path.parent.mkdir(parents=True, exist_ok=True)
            config_path.write_text(config, encoding="utf-8")
        command = [
            POWERSHELL,
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(MODEL_SYNC_SCRIPT),
            "-PackRoot",
            str(pack_root),
            "-TargetUserProfile",
            str(profile),
            "-KitHome",
            str(kit_home),
        ]
        if what_if:
            command.append("-WhatIf")
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        if completed.returncode == 0:
            decoder = json.JSONDecoder()
            for index, character in enumerate(completed.stdout):
                if character != "{":
                    continue
                try:
                    result, end = decoder.raw_decode(completed.stdout[index:])
                except json.JSONDecodeError:
                    continue
                if not completed.stdout[index + end :].strip() and isinstance(result, dict):
                    return completed, result, config_path, kit_home
        return completed, None, config_path, kit_home

    def test_whatif_is_planned_and_writes_nothing(self):
        with tempfile.TemporaryDirectory() as directory:
            completed, result, config_path, kit_home = self.invoke(Path(directory), self.catalog(), what_if=True)
            self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
            self.assertEqual(result["status"], "planned")
            self.assertTrue(result["planned"])
            self.assertFalse(config_path.exists())
            self.assertFalse(kit_home.exists())

    def test_hidden_and_api_unsupported_official_models_migrate_but_third_party_is_preserved(self):
        for model_id in ("gpt-reserve", "gpt-5.3-codex-spark"):
            with self.subTest(model_id=model_id), tempfile.TemporaryDirectory() as directory:
                config = f'model = "{model_id}"\n\n[other]\nvalue = 1\n'
                completed, result, config_path, _ = self.invoke(Path(directory), self.catalog(), config=config)
                self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
                self.assertEqual(result["status"], "ready")
                self.assertEqual(result["migration"], "unavailable-official-model-migrated")
                migrated = config_path.read_text(encoding="utf-8")
                self.assertIn('model = "gpt-6-astra"', migrated)
                self.assertIn("[other]", migrated)

        with tempfile.TemporaryDirectory() as directory:
            config = 'model = "vendor-private-model"\n\n[other]\nvalue = 1\n'
            completed, result, config_path, _ = self.invoke(Path(directory), self.catalog(), config=config)
            self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
            self.assertEqual(result["migration"], "preserved-active-model")
            self.assertEqual(config_path.read_text(encoding="utf-8"), config)

    def test_strict_catalog_field_validation_and_idempotence(self):
        invalid = self.catalog()
        invalid["models"][0]["supportedInApi"] = "true"
        with tempfile.TemporaryDirectory() as directory:
            completed, result, _, _ = self.invoke(Path(directory), invalid)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIsNone(result)
            self.assertIn("supportedInApi", completed.stdout + completed.stderr)

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = 'model = "gpt-reserve"\n'
            first, _, config_path, kit_home = self.invoke(root, self.catalog(), config=config)
            self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
            first_config = config_path.read_text(encoding="utf-8")
            first_catalog = (kit_home / "model-catalog.json").read_text(encoding="utf-8")
            second, result, _, _ = self.invoke(root, self.catalog(), config=None)
            self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
            self.assertEqual(result["configChanged"], False)
            self.assertEqual(config_path.read_text(encoding="utf-8"), first_config)
            self.assertEqual((kit_home / "model-catalog.json").read_text(encoding="utf-8"), first_catalog)

    def test_retired_visible_active_model_is_rejected(self):
        catalog = self.catalog()
        catalog["models"].append(
            {
                "id": "gpt-5.2",
                "displayName": "GPT-5.2",
                "visibility": "list",
                "supportedInApi": True,
                "priority": 29,
                "defaultReasoningLevel": "medium",
                "reasoningLevels": ["low", "medium", "high"],
            }
        )
        with tempfile.TemporaryDirectory() as directory:
            completed, result, _, _ = self.invoke(Path(directory), catalog)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIsNone(result)
            self.assertIn("retired model", (completed.stdout + completed.stderr).lower())


class MediaGenerationMcpTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.data_dir = Path(self.temp.name) / "media-data"
        self.server = None
        self.thread = None
        self.module = load_module(MEDIA_MCP_SCRIPT)
        self.env = {
            "MEDIA_GENERATION_TEST_MODE": "1",
            "MEDIA_GENERATION_DATA_DIR": str(self.data_dir),
            "MEDIA_GENERATION_API_KEY": "contract-test-key",
            "MEDIA_GENERATION_AGENT_ID": "contract-test-route",
            "MEDIA_GENERATION_API_BASE_URL": "https://media.example.invalid",
            "MEDIA_IMAGE2_MODEL": "test-image-model",
            "MEDIA_GROK_IMAGE_MODEL": "test-grok-image",
            "MEDIA_GROK_VIDEO_MODEL": "test-grok-video",
            "MEDIA_SEEDANCE_MODEL": "test-seedance",
            "MEDIA_MINIMAX_VIDEO_MODEL": "",
            "MEDIA_GENERATION_CONFIG_PATH": str(self.data_dir / "media-generation.json"),
        }

    def tearDown(self):
        if self.server:
            self.server.shutdown()
            self.server.server_close()
        if self.thread:
            self.thread.join(timeout=2)
        self.temp.cleanup()

    def test_local_idempotency_does_not_claim_unverified_upstream_support(self):
        source = MEDIA_MCP_SCRIPT.read_text(encoding="utf-8")
        manifest = json.loads((ROOT / "manifest" / "media-generation-tools.json").read_text(encoding="utf-8"))
        submit_contract = next(item for item in manifest["tools"] if item["name"] == "media_submit")
        self.assertIn('record["idempotency_key_sha256"]', source)
        self.assertNotRegex(source, r"(?i)[\"']idempotency-key[\"']")
        self.assertEqual(manifest["healthLayers"]["providerIdempotencyContractsVerified"], [])
        self.assertIs(submit_contract["upstreamIdempotencyHeaderForwarded"], False)

    def test_prepare_is_offline_and_confirmation_is_hmac_bound(self):
        with adapter_environment(self.env):
            prepared = self.module.media_prepare({"provider": "image-2", "prompt": "A test poster"})
            self.assertFalse(prepared["api_request_sent"])
            self.assertEqual(prepared["provider"], "image-2")
            self.assertEqual(prepared["price_estimate"].split(";")[0], "unknown")
            record_path = self.data_dir / "confirmations" / f"{prepared['confirmation_id']}.json"
            record = json.loads(record_path.read_text(encoding="utf-8"))
            record["request"]["prompt"] = "tampered"
            record_path.write_text(json.dumps(record), encoding="utf-8")
            with self.assertRaises(self.module.MediaError):
                self.module._validated_record({"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"]})

    def test_submit_once_and_query_existing_async_task(self):
        calls = {"posts": 0, "gets": 0, "headers": {}, "payload": {}}
        class FakeResponse:
            def __init__(self, body): self.body = body
            def __enter__(self): return self
            def __exit__(self, *_args): return False
            def read(self, _limit): return self.body
        class FakeOpener:
            def open(self, request, timeout):
                self_timeout = timeout
                self.asserted = self_timeout
                if request.get_method() == "POST":
                    calls["posts"] += 1
                    calls["headers"] = dict(request.header_items())
                    calls["payload"] = json.loads(request.data)
                    return FakeResponse(json.dumps({"task_id": "fixture-task-1", "status": "queued"}).encode())
                calls["gets"] += 1
                return FakeResponse(json.dumps({"status": "completed", "data": [{"url": "https://assets.example.invalid/result.png"}]}).encode())
        env = dict(self.env)
        with adapter_environment(env):
            with mock.patch.object(self.module.urllib.request, "build_opener", side_effect=lambda *_args: FakeOpener()):
                prepared = self.module.media_prepare({"provider": "image-2", "prompt": "A billable call"})
                submitted = self.module.media_submit({"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"], "user_confirmation": "确认"})
                self.assertEqual(submitted["task_id"], "fixture-task-1")
                self.assertEqual(calls["posts"], 1)
                self.assertEqual(calls["payload"]["model"], "test-image-model")
                self.assertNotIn("idempotency-key", {key.lower() for key in calls["headers"]})
                with self.assertRaises(self.module.MediaError):
                    self.module.media_submit({"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"], "user_confirmation": "确认"})
                checked = self.module.media_check({"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"]})
                self.assertEqual(checked["state"], "completed_pending_save")
                self.assertEqual(calls["gets"], 1)
                record = self.module._load_json(self.module._confirmation_path(prepared["confirmation_id"]))
                self.assertTrue(record["api_request_sent"])
                self.assertEqual(record["response_summary"]["status"], "completed")
                self.assertNotIn("response", record)

    def test_check_during_submit_does_not_mark_live_request_unknown(self):
        post_started = threading.Event()
        release_post = threading.Event()
        outcomes = {}

        class FakeResponse:
            def __init__(self, body): self.body = body
            def __enter__(self): return self
            def __exit__(self, *_args): return False
            def read(self, _limit): return self.body

        class FakeOpener:
            def open(inner, request, timeout):
                if request.get_method() == "POST":
                    post_started.set()
                    if not release_post.wait(5):
                        raise TimeoutError("test fixture timed out")
                    return FakeResponse(json.dumps({"task_id": "race-task-1", "status": "queued"}).encode())
                return FakeResponse(json.dumps({"status": "completed", "data": [{"url": "https://assets.example.invalid/race.png"}]}).encode())

        with adapter_environment(self.env), mock.patch.object(self.module.urllib.request, "build_opener", side_effect=lambda *_args: FakeOpener()):
            prepared = self.module.media_prepare({"provider": "seedance-video", "prompt": "submission race fixture"})
            args = {"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"], "user_confirmation": "确认"}

            def submit():
                try:
                    outcomes["submit"] = self.module.media_submit(args)
                except Exception as exc:
                    outcomes["submit_error"] = exc

            worker = threading.Thread(target=submit)
            worker.start()
            self.assertTrue(post_started.wait(3), "submit did not reach its fake network request")
            path = self.module._confirmation_path(prepared["confirmation_id"])
            self.assertEqual(self.module._load_json(path)["state"], "submitting")
            with self.assertRaisesRegex(self.module.MediaError, "正在检查"):
                self.module.media_check(args)
            self.assertEqual(self.module._load_json(path)["state"], "submitting")

            release_post.set()
            worker.join(timeout=5)
            self.assertFalse(worker.is_alive(), "submit worker did not finish")
            self.assertNotIn("submit_error", outcomes)
            self.assertEqual(outcomes["submit"]["task_id"], "race-task-1")
            record = self.module._load_json(path)
            self.assertEqual(record["state"], "in_progress")
            self.assertEqual(record["task_id"], "race-task-1")
            checked = self.module.media_check(args)
            self.assertEqual(checked["state"], "completed_pending_save")

    @unittest.skipUnless(os.name == "nt", "Windows file-lock semantics are the release target")
    def test_confirmation_operation_lock_blocks_another_process(self):
        with adapter_environment(self.env):
            prepared = self.module.media_prepare({"provider": "seedance-video", "prompt": "cross-process lock fixture"})
            confirmation_id = prepared["confirmation_id"]
            record_path = self.module._confirmation_path(confirmation_id)
            child_code = (
                "import importlib.util,os,sys\n"
                "spec=importlib.util.spec_from_file_location('media_lock_child',os.environ['MEDIA_TEST_SCRIPT'])\n"
                "module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)\n"
                "with module._confirmation_operation_lock(os.environ['MEDIA_TEST_CONFIRMATION'], '提交'):\n"
                "    print('LOCKED',flush=True)\n"
                "    sys.stdin.readline()\n"
            )
            child_env = os.environ.copy()
            child_env.update(self.env)
            child_env["MEDIA_TEST_SCRIPT"] = str(MEDIA_MCP_SCRIPT)
            child_env["MEDIA_TEST_CONFIRMATION"] = confirmation_id
            child = subprocess.Popen(
                [sys.executable, "-c", child_code], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, text=True, encoding="utf-8", env=child_env, bufsize=1,
            )
            try:
                self.assertEqual(child.stdout.readline().strip(), "LOCKED", child.stderr.read() if child.poll() is not None and child.stderr else "")
                args = {"confirmation_id": confirmation_id, "confirmation_fingerprint": prepared["confirmation_fingerprint"]}
                with self.assertRaisesRegex(self.module.MediaError, "正在检查"):
                    self.module.media_check(args)
                self.assertEqual(self.module._load_json(record_path)["state"], "prepared")
            finally:
                if child.poll() is None:
                    child.terminate()
                child.wait(timeout=5)
                for stream in (child.stdin, child.stdout, child.stderr):
                    if stream:
                        stream.close()
            self.assertNotEqual(child.returncode, 0, "fixture must terminate while holding the OS lock")
            with self.module._confirmation_operation_lock(confirmation_id, "检查"):
                self.assertEqual(self.module._load_json(record_path)["state"], "prepared")

    def test_grok_reference_upload_contract_is_offline_and_dependency_closed(self):
        test_script = ROOT / "skills" / "grok-video-prompt-compiler" / "scripts" / "test_upload_reference.py"
        result = subprocess.run(
            [sys.executable, str(test_script)], capture_output=True, text=True, encoding="utf-8", timeout=20
        )
        self.assertEqual(result.returncode, 0, result.stdout + "\n" + result.stderr)
        self.assertIn("5 tests passed", result.stdout)

    def test_minimax_is_reserved_for_the_shared_wokey_media_slot(self):
        with adapter_environment(self.env):
            capabilities = self.module._configured()
            self.assertFalse(capabilities["providers"]["minimax-video"]["configured"])
            with self.assertRaisesRegex(self.module.MediaError, "MiniMax 视频 provider 当前不可用"):
                self.module.media_prepare({"provider": "minimax-video", "prompt": "No fallback"})

        env = dict(
            self.env,
            MEDIA_GENERATION_API_PROFILE="wokey",
            MEDIA_GENERATION_API_BASE_URL="https://api.wokey.ai",
            MEDIA_MINIMAX_VIDEO_MODEL="upstream-minimax-video",
        )
        with adapter_environment(env):
            capabilities = self.module._configured()
            self.assertFalse(capabilities["providers"]["minimax-video"]["configured"])
            with self.assertRaisesRegex(self.module.MediaError, "MiniMax 视频 provider 当前不可用"):
                self.module.media_prepare({"provider": "minimax-video", "prompt": "Configured model must not imply an adapter"})

    def test_provider_payloads_and_mcp_stdio_contract(self):
        with adapter_environment(self.env):
            grok = self.module._validate_request({"provider": "grok-video", "prompt": "Move forward", "parameters": {"duration": 5, "aspect_ratio": "16:9", "resolution": "480p"}})
            path, payload = self.module._request_payload(grok)
            self.assertEqual(path, "/v1/videos/generations")
            self.assertEqual(payload["duration"], 5)
            seedance = self.module._validate_request({"provider": "seedance-video", "prompt": "Keep order", "reference_media": [{"kind": "image", "url": "https://one.example/ref.png"}, {"kind": "video", "url": "https://two.example/ref.mp4"}]})
            path, payload = self.module._request_payload(seedance)
            self.assertEqual(path, "/api/llm/doubao/contents/generations/tasks")
            self.assertEqual([item["role"] for item in payload["content"][1:]], ["reference_image", "reference_video"])
            self.assertTrue(payload["generate_audio"])
            self.assertTrue(payload["return_last_frame"])
            self.assertFalse(payload["watermark"])
            with self.assertRaisesRegex(self.module.MediaError, "generate_audio 必须为 true"):
                self.module._validate_request({"provider": "seedance-video", "prompt": "silent output", "parameters": {"generate_audio": False}})
        environment = os.environ.copy()
        for name in list(environment):
            if name.startswith(ADAPTER_ENV_PREFIXES):
                environment.pop(name, None)
        environment["MEDIA_GENERATION_DATA_DIR"] = str(self.data_dir / "protocol-data")
        messages = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "contract-test", "version": "1"}}},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        ]
        protocol_input = ("\n".join(json.dumps(item, ensure_ascii=False) for item in messages) + "\n").encode("utf-8")
        result = subprocess.run([sys.executable, str(MEDIA_MCP_SCRIPT)], input=protocol_input, capture_output=True, env=environment, timeout=10, check=True)
        responses = [json.loads(line.decode("utf-8", errors="strict")) for line in result.stdout.splitlines() if line.strip()]
        self.assertEqual(responses[0]["result"]["serverInfo"]["name"], "codex-starter-media-generation")
        self.assertEqual([item["name"] for item in responses[1]["result"]["tools"]], ["media_refresh_models", "media_capabilities", "media_prepare", "media_submit", "media_check", "media_save"])

    def test_stdio_utf8_protocol_errors_notifications_and_eof(self):
        environment = os.environ.copy()
        for name in list(environment):
            if name.startswith(ADAPTER_ENV_PREFIXES):
                environment.pop(name, None)
        environment["PYTHONIOENCODING"] = "gbk"
        environment["PYTHONUTF8"] = "0"
        environment["MEDIA_GENERATION_DATA_DIR"] = str(self.data_dir / "stdio-data")
        messages = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "clientInfo": {"name": "中文测试", "version": "1"}}},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "media_capabilities", "arguments": {}}},
            {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
            {"jsonrpc": "2.0", "id": 4},
            [],
        ]
        raw = b"\n".join(json.dumps(item, ensure_ascii=False).encode("utf-8") for item in messages)
        raw += b"\nnot-json\n" + (b" " * (self.module.MAX_JSON_BYTES + 5)) + b"\n"
        result = subprocess.run([sys.executable, str(MEDIA_MCP_SCRIPT)], input=raw, capture_output=True, env=environment, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", errors="replace"))
        self.assertEqual(result.stderr, b"")
        lines = result.stdout.splitlines()
        self.assertTrue(all(line.startswith(b"{") for line in lines))
        replies = [json.loads(line.decode("utf-8", errors="strict")) for line in lines]
        self.assertEqual([reply.get("id") for reply in replies], [1, 2, 3, 4, None, None, None])
        self.assertEqual(replies[3]["error"]["code"], -32600)
        self.assertEqual(replies[4]["error"]["code"], -32600)
        self.assertEqual(replies[5]["error"]["code"], -32700)
        self.assertEqual(replies[6]["error"]["code"], -32600)
        self.assertIn("统一网关", lines[1].decode("utf-8"))
        caps = json.loads(replies[2]["result"]["content"][0]["text"])
        self.assertEqual(caps["generation_capability"], "not_tested")

    @unittest.skipUnless(POWERSHELL, "PowerShell is required for the installed wrapper integration test")
    def test_normal_wrapper_config_path_uses_signed_launch_key_and_preserves_raw_jsonrpc(self):
        kit = Path(self.temp.name) / "unpacked-kit"
        bin_dir = kit / "bin"
        connections = kit / "connections"
        bin_dir.mkdir(parents=True)
        connections.mkdir(parents=True)
        shutil.copy2(MEDIA_MCP_SCRIPT, bin_dir / "media-generation-mcp.py")
        shutil.copy2(MEDIA_MCP_WRAPPER, bin_dir / "start-media-generation-mcp.ps1")
        gateway = "https://media.example.invalid"
        (connections / "media-generation.connection.json").write_text(json.dumps({"schemaVersion": 1, "apiBaseUrl": gateway, "apiProfile": "responses"}), encoding="utf-8")
        (connections / "media-generation.json").write_text(json.dumps({"schemaVersion": 1, "gateway": gateway, "profile": "responses", "models": {"image-2": "gpt-image-2.5", "grok-image": "", "grok-video": "", "seedance-video": "", "minimax-video": ""}, "catalogVerifiedAt": int(time.time()), "catalogIds": ["gpt-image-2.5"]}), encoding="utf-8")
        environment = os.environ.copy()
        for name in list(environment):
            if name.startswith(ADAPTER_ENV_PREFIXES): environment.pop(name, None)
        environment["MEDIA_GENERATION_API_KEY"] = "signed-launch-test-key"
        environment["MEDIA_GENERATION_DATA_DIR"] = str(kit / "data" / "media-generation")
        messages = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "media_capabilities", "arguments": {}}},
        ]
        raw = ("\n".join(json.dumps(item, ensure_ascii=False) for item in messages) + "\n").encode("utf-8")
        completed = subprocess.run([POWERSHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(bin_dir / "start-media-generation-mcp.ps1"), "-PythonExe", sys.executable], input=raw, capture_output=True, env=environment, timeout=20)
        self.assertEqual(completed.returncode, 0, completed.stderr.decode("utf-8", errors="replace"))
        replies = [json.loads(line.decode("utf-8", errors="strict")) for line in completed.stdout.splitlines()]
        self.assertEqual([item.get("id") for item in replies], [1, 2, 3], completed.stdout.decode("utf-8", errors="replace"))
        capabilities = json.loads(replies[2]["result"]["content"][0]["text"])
        self.assertTrue(capabilities["api_key_configured"])
        self.assertTrue(capabilities["base_url_configured"])
        self.assertTrue(capabilities["providers"]["image-2"]["configured"])
        # The signed-launch credential is used by the child but never appears on MCP stdout.
        self.assertNotIn(b"signed-launch-test-key", completed.stdout)
        health = subprocess.run([POWERSHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(bin_dir / "start-media-generation-mcp.ps1"), "-HealthCheck", "-PythonExe", sys.executable], capture_output=True, env=environment, timeout=20)
        self.assertEqual(health.returncode, 0, health.stderr.decode("utf-8", errors="replace"))
        health_report = json.loads(health.stdout.decode("utf-8", errors="strict"))
        self.assertEqual(health_report["serverVersion"], "1.3.11")
        self.assertEqual(health_report["protocolStatus"], "passed")
        self.assertEqual(health_report["generationCapability"], "not_tested")

    def test_stale_submission_is_unknown_and_never_reposted(self):
        with adapter_environment(self.env):
            prepared = self.module.media_prepare({"provider": "seedance-video", "prompt": "A recovery fixture"})
            path = self.module._confirmation_path(prepared["confirmation_id"])
            record = self.module._load_json(path)
            record["state"] = "submitting"
            record["api_request_sent"] = True
            path.write_text(json.dumps(record), encoding="utf-8")
            args = {"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"], "user_confirmation": "确认"}
            with self.assertRaisesRegex(self.module.MediaError, "submission_unknown"):
                self.module.media_submit(args)
            recovered = self.module._load_json(path)
            self.assertEqual(recovered["state"], "submission_unknown")
            self.assertTrue(recovered["api_request_sent"])
            with self.assertRaisesRegex(self.module.MediaError, "没有可查询的 task_id"):
                self.module.media_check(args)

    def test_submit_timeout_persists_unknown_and_never_reposts(self):
        calls = {"count": 0}
        class FakeOpener:
            def open(inner, request, timeout):
                calls["count"] += 1
                current = self.module._load_json(self.module._confirmation_path(prepared["confirmation_id"]))
                self.assertEqual(current["state"], "submitting")
                self.assertTrue(current["api_request_sent"])
                raise urllib.error.URLError("fixture timeout")
        with adapter_environment(self.env):
            with mock.patch.object(self.module.urllib.request, "build_opener", side_effect=lambda *_args: FakeOpener()):
                prepared = self.module.media_prepare({"provider": "seedance-video", "prompt": "timeout fixture"})
                args = {"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"], "user_confirmation": "确认"}
                with self.assertRaisesRegex(self.module.MediaError, "提交状态未知"):
                    self.module.media_submit(args)
                path = self.module._confirmation_path(prepared["confirmation_id"])
                record = self.module._load_json(path)
                self.assertEqual(record["state"], "submission_unknown")
                self.assertTrue(record["api_request_sent"])
                with self.assertRaises(self.module.MediaError):
                    self.module.media_submit(args)
                self.assertEqual(calls["count"], 1)

    def test_relative_content_url_and_bearer_origin_boundary(self):
        env = dict(self.env, MEDIA_GENERATION_API_PROFILE="wokey", MEDIA_GENERATION_API_BASE_URL="https://gateway.example")
        with adapter_environment(env):
            artifact = self.module._artifacts({"status": "completed", "content_url": "/v1/videos/video-7/content"}, "seedance-video")
            self.assertEqual(artifact[0]["url"], "https://gateway.example/v1/videos/video-7/content")
            requests = []
            class FakeResponse:
                body = b"\x00\x00\x00\x18ftypisom00000000"
                headers = {"Content-Length": str(len(body)), "Content-Type": "video/mp4"}
                def __init__(self): self.stream = __import__("io").BytesIO(self.body)
                def __enter__(self): return self
                def __exit__(self, *_args): return False
                def read(self, size): return self.stream.read(size)
            class FakeOpener:
                def open(inner, request, timeout):
                    requests.append(request)
                    return FakeResponse()
            with mock.patch.object(self.module.urllib.request, "build_opener", side_effect=lambda *_args: FakeOpener()), \
                 mock.patch.object(self.module, "_validate_public_url", side_effect=lambda value, resolve=False: value), \
                 mock.patch.object(self.module, "_base", return_value="https://gateway.example"):
                self.data_dir.mkdir(parents=True, exist_ok=True)
                for index, url in enumerate((artifact[0]["url"], "https://assets.example/result.mp4", "https://gateway.example:8443/result.mp4")):
                    destination = self.data_dir / f"download-{index}.mp4"
                    self.module._safe_download(url, destination)
                    self.assertEqual(destination.read_bytes(), FakeResponse.body)
            self.assertEqual(requests[0].get_header("Authorization"), "Bearer contract-test-key")
            self.assertIsNone(requests[1].get_header("Authorization"))
            self.assertIsNone(requests[2].get_header("Authorization"))

    def test_completed_task_can_be_saved_after_local_reference_is_deleted(self):
        def png_chunk(name, body):
            return struct.pack(">I", len(body)) + name + body + struct.pack(">I", zlib.crc32(name + body) & 0xFFFFFFFF)

        source = self.data_dir / "reference.png"
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_bytes(
            b"\x89PNG\r\n\x1a\n"
            + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
            + png_chunk(b"IDAT", zlib.compress(b"\x00\xff\x00\x00"))
            + png_chunk(b"IEND", b"")
        )
        with adapter_environment(self.env):
            prepared = self.module.media_prepare({"provider": "image-2", "prompt": "reference recovery", "reference_media": [{"kind": "image", "local_path": str(source)}]})
            path = self.module._confirmation_path(prepared["confirmation_id"])
            record = self.module._load_json(path)
            record["state"] = "completed"
            record["api_request_sent"] = True
            record["artifacts"] = [{"kind": "image", "url": "https://assets.example.invalid/result.png"}]
            self.module._atomic_json(path, record)
            source.unlink()
            def fake_download(_url, destination):
                def chunk(name, body):
                    return struct.pack(">I", len(body)) + name + body + struct.pack(">I", zlib.crc32(name + body) & 0xFFFFFFFF)
                png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"\x00\xff\x00\x00")) + chunk(b"IEND", b"")
                destination.write_bytes(png)
                return len(png), "image/png"
            with mock.patch.object(self.module, "_safe_download", side_effect=fake_download):
                result = self.module.media_save({"confirmation_id": prepared["confirmation_id"], "confirmation_fingerprint": prepared["confirmation_fingerprint"]})
            self.assertTrue(result["success"])
            self.module._validate_png(Path(result["saved"][0]["path"]).read_bytes())

    def test_wokey_profile_uses_wokey_image_and_video_contracts(self):
        env = dict(self.env, MEDIA_GENERATION_API_PROFILE="wokey", MEDIA_GENERATION_API_BASE_URL="https://api.wokey.ai",
                   MEDIA_IMAGE2_MODEL="gpt-image-2", MEDIA_GROK_VIDEO_MODEL="grok-imagine-video-1.5",
                   MEDIA_SEEDANCE_MODEL="jimeng-seedance-2.0-mini")
        with adapter_environment(env):
            image = self.module._validate_request({"provider": "image-2", "model": "gpt-image-2", "prompt": "A test image", "parameters": {"size": "2048x2048", "n": 1}})
            path, payload = self.module._request_payload(image)
            self.assertEqual(path, "/v1/images/generations")
            self.assertEqual(payload["model"], "gpt-image-2")

            grok = self.module._validate_request({"provider": "grok-video", "model": "grok-imagine-video-1.5", "prompt": "A one second test", "parameters": {"duration": 1, "aspect_ratio": "16:9", "resolution": "480p"}})
            path, payload = self.module._request_payload(grok)
            self.assertEqual(path, "/v1/videos")
            self.assertEqual(payload["duration_seconds"], 1)
            self.assertEqual(payload["video_resolution"], "480p")
            self.assertEqual(payload["mode"], "text_to_video")
            self.assertEqual(self.module._query_path("grok-video", "video-test"), "/v1/videos/video-test")

            seedance = self.module._validate_request({"provider": "seedance-video", "model": "jimeng-seedance-2.0-mini", "prompt": "A four second test", "parameters": {"duration": 4, "ratio": "16:9"}})
            path, payload = self.module._request_payload(seedance)
            self.assertEqual(path, "/v1/videos")
            self.assertEqual(payload["duration_seconds"], 4)
            self.assertEqual(payload["video_resolution"], "720p")
            self.assertEqual(self.module._query_path("seedance-video", "video-test"), "/v1/videos/video-test")

            artifacts = self.module._artifacts({"status": "completed", "content_url": "/v1/videos/video-test/content"})
            self.assertEqual(artifacts, [{"kind": "video", "url": "https://api.wokey.ai/v1/videos/video-test/content"}])

    def test_wokey_auto_profile_removes_legacy_agent_requirement(self):
        env = dict(self.env, MEDIA_GENERATION_API_PROFILE="auto", MEDIA_GENERATION_API_BASE_URL="https://api.wokey.ai")
        env.pop("MEDIA_GENERATION_AGENT_ID")
        with adapter_environment(env):
            configured = self.module._configured()
            self.assertEqual(configured["api_profile"], "wokey")
            self.assertTrue(configured["providers"]["image-2"]["configured"])
            self.assertTrue(configured["providers"]["seedance-video"]["configured"])



    def test_responses_image_uses_gpt_image_25_and_saves_valid_png_in_same_submit(self):
        def chunk(name, body):
            return struct.pack(">I", len(body)) + name + body + struct.pack(">I", zlib.crc32(name + body) & 0xFFFFFFFF)

        png = (
            b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"\x00\xff\x00\x00"))
            + chunk(b"IEND", b"")
        )

        calls = {"posts": 0, "url": "", "payload": {}, "timeout": None}
        class FakeResponse:
            def __enter__(self): return self
            def __exit__(self, *_args): return False
            def read(self, _limit): return json.dumps({"output": [{"type": "image_generation_call", "result": base64.b64encode(png).decode("ascii")}] }).encode()
        class FakeOpener:
            def open(self, request, timeout):
                calls["posts"] += 1
                calls["url"] = request.full_url
                calls["timeout"] = timeout
                calls["payload"] = json.loads(request.data)
                return FakeResponse()
        env = dict(
            self.env,
            MEDIA_GENERATION_API_PROFILE="responses",
            MEDIA_GENERATION_API_BASE_URL="https://media.example.invalid",
            MEDIA_IMAGE2_MODEL="gpt-image-2.5",
        )
        env.pop("MEDIA_GENERATION_AGENT_ID", None)
        with adapter_environment(env):
            with mock.patch.object(self.module.urllib.request, "build_opener", side_effect=lambda *_args: FakeOpener()):
                prepared = self.module.media_prepare({"provider": "image-2", "prompt": "A small test image"})
                submitted = self.module.media_submit({
                    "confirmation_id": prepared["confirmation_id"],
                    "confirmation_fingerprint": prepared["confirmation_fingerprint"],
                    "user_confirmation": "确认",
                })
                self.assertEqual(calls["posts"], 1)
                self.assertEqual(calls["url"], "https://media.example.invalid/v1/responses")
                self.assertEqual(calls["timeout"], 600)
                self.assertEqual(calls["payload"]["model"], "gpt-image-2.5")
                self.assertEqual(calls["payload"]["tool_choice"], {"type": "image_generation"})
                self.assertEqual(submitted["state"], "completed_saved")
                self.assertEqual(len(submitted["saved"]), 1)
                saved_path = Path(submitted["saved"][0]["path"])
                self.assertEqual(saved_path.suffix, ".png")
                self.assertEqual(saved_path.read_bytes(), png)
                self.module._validate_png(saved_path.read_bytes())
            invalid_png = (
                b"\x89PNG\r\n\x1a\n"
                + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", b"not-a-zlib-stream")
                + chunk(b"IEND", b"")
            )
            with self.assertRaisesRegex(self.module.MediaError, "压缩数据无法解码"):
                self.module._validate_png(invalid_png)

    def test_bundled_python_healthcheck_needs_no_openai_package_or_provider_key(self):
        environment = os.environ.copy()
        for key in list(environment):
            if key.startswith(ADAPTER_ENV_PREFIXES) or key == "OPENAI_API_KEY":
                environment.pop(key, None)
        completed = subprocess.run(
            [sys.executable, str(MEDIA_MCP_SCRIPT), "--healthcheck"],
            text=True,
            capture_output=True,
            env=environment,
            timeout=10,
            check=True,
        )
        health = json.loads(completed.stdout)
        self.assertTrue(health["ready"])
        self.assertEqual(health["implementation"], "cpython")
        self.assertGreaterEqual(tuple(map(int, health["python"].split("."))), (3, 12))
        self.assertEqual(health["dependencies"], "python-standard-library")
        self.assertEqual(health["healthScope"], "local-runtime-protocol")
        self.assertEqual(health["gatewayCatalogStatus"], "not_checked")
        self.assertEqual(health["generationCapability"], "not_tested")
        self.assertEqual(health["serverVersion"], "1.3.11")

    def test_wrapper_healthcheck_does_not_merge_stderr_into_json_stdout(self):
        self.assertTrue(POWERSHELL, "Windows PowerShell is required for this wrapper contract test")
        fake_python = Path(self.temp.name) / "python-health-fixture.cmd"
        health_json = json.dumps({
            "ready": True,
            "serverVersion": "1.3.11",
            "healthScope": "local-runtime-protocol",
            "runtimeStatus": "passed",
            "protocolStatus": "passed",
            "gatewayCatalogStatus": "not_checked",
            "generationCapability": "not_tested",
            "toolCount": 6,
        }, separators=(",", ":"))
        fake_python.write_text(f"@echo off\r\necho {health_json}\r\necho diagnostic-only 1>&2\r\n", encoding="ascii")
        completed = subprocess.run(
            [POWERSHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(MEDIA_MCP_WRAPPER), "-HealthCheck", "-PythonExe", str(fake_python)],
            capture_output=True,
            timeout=20,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr.decode("utf-8", errors="replace"))
        # The PowerShell host may emit its own initialization warning to stderr;
        # the wrapper contract is that only JSON-RPC/health JSON reaches stdout.
        health = json.loads(completed.stdout.decode("utf-8", errors="strict"))
        self.assertTrue(health["ready"])
        self.assertEqual(health["protocolStatus"], "passed")


class RuntimeProvisioningContractTests(unittest.TestCase):
    def test_partial_python_venv_without_pip_is_not_treated_as_healthy(self):
        provisioner = (ROOT / "runtime" / "provision-runtime.ps1").read_text(encoding="utf-8")
        self.assertIn("function Test-VenvPython([string]$Executable)", provisioner)
        self.assertIn("@('-m','pip','--version')", provisioner)
        self.assertIn("$venvHealthy = Test-VenvPython $venvPython", provisioner)
        self.assertIn("if (-not (Test-VenvPython $manimPython))", provisioner)
        self.assertIn("if (-not $venvHealthy)", provisioner)

    def test_installer_runs_runtime_provisioner_without_a_hidden_powershell_child(self):
        installer = (SCRIPT_ROOT / "install-to-codex.ps1").read_text(encoding="utf-8")
        self.assertIn("& $runtimeScript @runtimeArguments | Out-String", installer)
        self.assertIn("if ($feishuEnabled) { $runtimeArguments.EnableFeishu = $true }", installer)
        self.assertIn("& $renderScriptPath @renderArguments", installer)
        self.assertIn("& $configMerger @configArguments", installer)
        self.assertIn("& $modelSyncScript @modelSyncArguments", installer)
        self.assertIn("$mediaRegistrationVerified = [regex]::IsMatch($mergedConfig", installer)
        self.assertIn("-and $mediaRegistrationVerified)", installer)
        self.assertNotIn("$powershellCommand.Source @runtimeArguments", installer)
        self.assertNotIn("$powershellCommand.Source @renderArguments", installer)
        self.assertNotIn("$powershellCommand.Source @configArguments", installer)
        self.assertNotIn("$powershellCommand.Source @modelSyncArguments", installer)
        self.assertIn("& $mediaWrapperTarget -HealthCheck | Out-String", installer)
        self.assertNotIn("$powershellCommand.Source -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File $mediaWrapperTarget -HealthCheck", installer)
        self.assertIn("$runtimeState = $runtimeOutput | ConvertFrom-Json", installer)
        media_wrapper = MEDIA_MCP_WRAPPER.read_text(encoding="utf-8")
        self.assertEqual(media_wrapper.count("    return"), 3)
        self.assertNotIn("    exit 0", media_wrapper)
        merger = (SCRIPT_ROOT / "configure-codex.ps1").read_text(encoding="utf-8")
        self.assertIn("$status | ConvertTo-Json -Depth 6\n    return", merger)


class DashScopeAdapterTests(unittest.TestCase):
    def test_credentials_and_model_are_required_before_network(self):
        module = load_module(WEB_SCRIPT)

        def unexpected_network(*args, **kwargs):
            raise AssertionError("network must not be used when required configuration is missing")

        module.urllib.request.urlopen = unexpected_network
        with adapter_environment({"DASHSCOPE_MODEL": "configured-search-model"}):
            result = module.web_search("test query")
        self.assertIn("DASHSCOPE_API_KEY", result["error"])

        with adapter_environment({"DASHSCOPE_API_KEY": "test-only-key"}):
            result = module.web_search("test query")
        self.assertIn("DASHSCOPE_MODEL", result["error"])

    def test_base_url_must_be_the_https_compatible_root(self):
        module = load_module(WEB_SCRIPT)
        self.assertEqual(
            module.validate_base_url("https://dashscope.example/compatible-mode/v1/"),
            "https://dashscope.example/compatible-mode/v1",
        )
        for value in (
            "http://dashscope.example/compatible-mode/v1",
            "https://user:pass@dashscope.example/compatible-mode/v1",
            "https://dashscope.example/prefix/compatible-mode/v1",
            "https://dashscope.example/compatible-mode/v1/chat/completions",
            "https://dashscope.example/compatible-mode/v1?token=secret",
        ):
            with self.subTest(value=value), self.assertRaises(ValueError):
                module.validate_base_url(value)

    def test_request_appends_exact_endpoint_and_preserves_explicit_model(self):
        module = load_module(WEB_SCRIPT)
        calls = []

        class FakeResponse:
            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, traceback):
                return False

            def read(self, limit):
                self.limit = limit
                return json.dumps(
                    {"choices": [{"message": {"content": "summary with https://source.example"}}]}
                ).encode("utf-8")

        def fake_urlopen(request, timeout):
            response = FakeResponse()
            calls.append((request, timeout, response))
            return response

        module.urllib.request.urlopen = fake_urlopen
        environment = {
            "DASHSCOPE_API_KEY": "test-only-key",
            "DASHSCOPE_MODEL": "configured-search-model",
            "DASHSCOPE_BASE_URL": "https://dashscope.example/compatible-mode/v1",
        }
        with adapter_environment(environment):
            result = module.web_search("test query", max_words=400)

        self.assertTrue(result["success"])
        self.assertEqual(len(calls), 1)
        request, timeout, response = calls[0]
        self.assertEqual(request.full_url, "https://dashscope.example/compatible-mode/v1/chat/completions")
        self.assertEqual(timeout, 120)
        self.assertEqual(response.limit, module.MAX_RESPONSE_BYTES + 1)
        payload = json.loads(request.data.decode("utf-8"))
        self.assertEqual(payload["model"], "configured-search-model")
        self.assertTrue(payload["enable_search"])
        self.assertIn("400", payload["messages"][0]["content"])


class FinancialDataIntegrityTests(unittest.TestCase):
    def test_technical_analysis_reads_akshare_without_simulated_fallback(self):
        module = load_module(TECHNICAL_SCRIPT)
        calls = []

        class FakeSeries(list):
            def tolist(self):
                return list(self)

        class FakeFrame:
            columns = ("日期", "收盘", "最高", "最低", "成交量")
            empty = False

            def __init__(self, rows):
                self.rows = rows

            def __len__(self):
                return len(self.rows)

            def tail(self, count):
                return FakeFrame(self.rows[-count:])

            def __getitem__(self, key):
                return FakeSeries(row[key] for row in self.rows)

        rows = [
            {
                "日期": f"2026-01-{(index % 28) + 1:02d}",
                "收盘": 10 + index,
                "最高": 11 + index,
                "最低": 9 + index,
                "成交量": 1000 + index,
            }
            for index in range(80)
        ]

        def fake_history(**kwargs):
            calls.append(kwargs)
            return FakeFrame(rows)

        prior = sys.modules.get("akshare")
        sys.modules["akshare"] = SimpleNamespace(stock_zh_a_hist=fake_history)
        try:
            prices, highs, lows, volumes, as_of = module.fetch_market_data("000001", "daily", 60)
        finally:
            if prior is None:
                sys.modules.pop("akshare", None)
            else:
                sys.modules["akshare"] = prior

        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]["symbol"], "000001")
        self.assertEqual(calls[0]["adjust"], "qfq")
        self.assertEqual(len(prices), 60)
        self.assertEqual((prices[-1], highs[-1], lows[-1], volumes[-1]), (89.0, 90.0, 88.0, 1079.0))
        self.assertEqual(as_of, rows[-1]["日期"])
        report = module.generate_technical_analysis("000001", prices, highs, lows, volumes, "daily")
        self.assertNotIn("technical_score", report)
        self.assertNotIn("suggestions", report)
        self.assertIn("不生成买卖", report["methodology"])

    def test_sentiment_analysis_requires_complete_attributed_input(self):
        module = load_module(SENTIMENT_SCRIPT)
        with self.assertRaisesRegex(ValueError, "as_of"):
            module.generate_sentiment_report("沪深300", {})

        data = {
            "as_of": "2026-09-08T15:00:00+08:00",
            "sources": ["Exchange bulletin https://source.example/market"],
            "indicators": {key: 50 for key in module.INDICATOR_KEYS},
            "weights": {key: 1 / len(module.INDICATOR_KEYS) for key in module.INDICATOR_KEYS},
            "breadth": {"advancing_stocks": 2100, "declining_stocks": 1900},
            "volume": {"current_volume": 8500, "avg_volume": 8000, "price_change": 0.5},
            "fund_flow": {"northbound_flow": -20, "main_force_flow": 30, "retail_flow": 5},
        }
        report = module.generate_sentiment_report("沪深300", data)
        self.assertEqual(report["data_as_of"], data["as_of"])
        self.assertEqual(report["sources"], data["sources"])
        self.assertEqual(report["market_breadth"]["advancing_stocks"], 2100)
        self.assertEqual(report["fund_flow"]["total_flow"], 15)
        self.assertEqual(report["weighted_index"]["value"], 50)
        self.assertNotIn("suggestions", report)
        self.assertNotIn("overall_assessment", report)

    def test_financial_entrypoints_have_no_mock_market_payloads(self):
        technical_code = TECHNICAL_SCRIPT.read_text(encoding="utf-8")
        sentiment_code = SENTIMENT_SCRIPT.read_text(encoding="utf-8")
        fund_risk_code = FUND_RISK_SCRIPT.read_text(encoding="utf-8")
        for marker in ("fetch_mock_data", "import random", "当前使用模拟数据"):
            self.assertNotIn(marker, technical_code)
        for marker in ("mock_data", "indicators.get(key, 50)"):
            self.assertNotIn(marker, sentiment_code)
        for marker in ("FUND_VOLATILITY", "默认示例组合", 'parser.add_argument("--funds"'):
            self.assertNotIn(marker, fund_risk_code)

    def test_fund_risk_requires_attributed_market_inputs_and_explicit_scenarios(self):
        module = load_module(FUND_RISK_SCRIPT)
        with self.assertRaisesRegex(ValueError, "as_of"):
            module.assess_risk({})
        portfolio = {
            "as_of": "2026-09-08",
            "sources": ["Fund reports https://source.example/funds"],
            "allocations": {"FUND_A": 60, "FUND_B": 40},
            "annual_volatility": {"FUND_A": 10, "FUND_B": 5},
            "asset_classes": {"FUND_A": "equity", "FUND_B": "bond"},
            "correlation": 0.2,
            "expected_return": 6,
            "risk_free_rate": 2,
            "drawdown_multiplier": 2.5,
        }
        result = module.assess_risk(portfolio)
        self.assertEqual(result["data_as_of"], portfolio["as_of"])
        self.assertEqual(result["sources"], portfolio["sources"])
        self.assertAlmostEqual(result["metrics"]["annual_volatility"], 6.69, places=2)
        self.assertAlmostEqual(result["metrics"]["scenario_drawdown"], 16.73, places=2)
        self.assertEqual(result["assumptions"]["correlation"], 0.2)
        self.assertNotIn("risk_rating", result)
        self.assertNotIn("suggestions", result)


class LocalFeatureContractTests(unittest.TestCase):
    def test_monthly_theme_extraction_is_deterministic_and_non_placeholder(self):
        module = load_module(REFLECTION_SCRIPT)
        reflections = [
            {"wins": ["坚持跑步", "完成阅读"], "learnings": [], "gratitude": [], "free_notes": "早点休息"},
            {"wins": ["坚持跑步"], "learnings": ["完成阅读"], "gratitude": [], "free_notes": "早点休息；复盘计划"},
        ]
        first = module.extract_themes(reflections)
        second = module.extract_themes(list(reversed(reflections)))
        self.assertEqual(first, second)
        self.assertEqual(set(first[:3]), {"早点休息", "坚持跑步", "完成阅读"})
        self.assertIn("复盘计划", first)


class FinancialScenarioContractTests(unittest.TestCase):
    def test_macro_cycle_uses_explicit_trends_and_reviewed_rules(self):
        module = load_module(MACRO_CYCLE_SCRIPT)
        with self.assertRaisesRegex(ValueError, "rules.as_of"):
            module.generate_cycle_analysis(5, 2, 4, 3, "2026-09-08", ["source"], {})

        policies = {}
        for key in module.PHASE_KEYS:
            policies[key] = {
                "label": key,
                "description": "reviewed interpretation",
                "characteristics": ["reviewed characteristic"],
                "allocation": {"cash": 100},
                "allocation_basis": "user-reviewed policy",
            }
        rules = {
            "as_of": "2026-09-08",
            "sources": ["policy https://source.example/cycle"],
            "phase_policies": policies,
        }
        report = module.generate_cycle_analysis(
            4.9,
            2.1,
            5.0,
            2.0,
            "2026-09-08",
            ["statistics https://source.example/macro"],
            rules,
        )
        self.assertEqual(report["observation"]["phase_key"], "growth_down_inflation_up")
        self.assertEqual(report["phase"]["allocation"], {"cash": 100.0})
        self.assertIn("不是脚本预测", report["methodology"])

    def test_policy_analysis_requires_evidence_and_conditional_scenarios(self):
        module = load_module(POLICY_SCRIPT)
        with self.assertRaisesRegex(ValueError, "transmission_hypotheses"):
            module.generate_policy_analysis({})
        raw = {
            "as_of": "2026-09-08",
            "sources": ["policy https://source.example/original"],
            "policy_type": "test policy",
            "action": "test action",
            "context": "documented scope",
            "evidence": ["verifiable text"],
            "transmission_hypotheses": [
                {"statement": "conditional effect", "basis": "published evidence", "confidence": "uncertain"}
            ],
            "scenario_implications": [
                {"scenario": "if implemented", "implication": "possible effect", "monitor": ["observable metric"]}
            ],
            "risks": ["implementation may differ"],
        }
        report = module.generate_policy_analysis(raw)
        self.assertEqual(report["sources"], raw["sources"])
        self.assertEqual(report["scenario_implications"], raw["scenario_implications"])
        self.assertIn("不内置", report["methodology"])

    def test_a_share_valuation_uses_only_attributed_explicit_assumptions(self):
        module = load_module(A_SHARE_VALUATION_SCRIPT)
        assumptions = {
            "as_of": "2026-09-08",
            "sources": ["model basis https://source.example/valuation"],
            "dcf": {
                "base_free_cash_flow": 100,
                "cash_flow_growth": 5,
                "discount_rate": 10,
                "terminal_growth": 2,
                "forecast_years": 3,
                "total_shares": 10,
                "basis": "same currency units",
            },
            "ddm": {
                "current_dividend_per_share": 1,
                "dividend_growth": 2,
                "required_return": 8,
                "basis": "reviewed dividend scenario",
            },
            "margin_of_safety": 20,
        }
        data = {
            "code": "000001",
            "basic_info": {"name": "test", "pe_ttm": 12, "pb": 1.2},
            "valuation": {"pe_ttm_percentile": 40, "pb_percentile": 30},
            "price": {"latest_price": 10},
        }
        report = module.calculate_valuation(
            data,
            assumptions,
            ["dcf", "ddm", "relative"],
            "2026-09-08",
            ["market data https://source.example/stock"],
        )
        self.assertEqual(report["assumptions_as_of"], assumptions["as_of"])
        self.assertEqual(report["methods"]["relative"]["PE_historical_band"], "25–50 percentile")
        self.assertNotIn("average", report)
        self.assertIn("不是买入建议", report["methods"]["dcf"]["margin_of_safety_scenario"]["notice"])
        invalid = json.loads(json.dumps(assumptions))
        invalid["dcf"]["discount_rate"] = 1
        with self.assertRaisesRegex(ValueError, "必须大于"):
            module.calculate_valuation(data, invalid, ["dcf"], "2026-09-08", ["source"])

    def test_market_screeners_do_not_advertise_ignored_filters(self):
        stock = load_module(STOCK_SCREENER_SCRIPT)
        stock_options = {option for action in stock.build_parser()._actions for option in action.option_strings}
        self.assertNotIn("--roe-min", stock_options)
        self.assertNotIn("--debt-ratio-max", stock_options)
        self.assertNotIn("--dividend-min", stock_options)
        with self.assertRaisesRegex(ValueError, "六位"):
            stock._parse_scope("custom:../bad")

        fund = load_module(FUND_SCREENER_SCRIPT)
        fund_options = {option for action in fund.build_parser()._actions for option in action.option_strings}
        self.assertNotIn("--max-drawdown", fund_options)
        self.assertNotIn("--min-sharpe", fund_options)
        self.assertNotIn("--max-fee", fund_options)

    def test_fund_screener_preserves_source_and_fails_on_provider_error(self):
        module = load_module(FUND_SCREENER_SCRIPT)

        class FakeFrame:
            empty = False

            def iterrows(self):
                rows = [
                    {"基金代码": "A", "基金简称": "Alpha", "基金类型": "混合", "近1年": "8%", "近3年": "6%"},
                    {"基金代码": "B", "基金简称": "Beta", "基金类型": "债券", "近1年": "3%", "近3年": "4%"},
                ]
                return enumerate(rows)

        provider = SimpleNamespace(fund_open_fund_rank_em=lambda: FakeFrame())
        report = module.screen_funds("混合", 5, 5, 20, provider=provider)
        self.assertEqual(report["source_interface"], module.SOURCE_INTERFACE)
        self.assertEqual([item["code"] for item in report["funds"]], ["A"])

        def fail():
            raise OSError("offline")

        with self.assertRaisesRegex(RuntimeError, "接口失败"):
            module.screen_funds(provider=SimpleNamespace(fund_open_fund_rank_em=fail))

    def test_sip_comparison_uses_only_explicit_return_scenarios(self):
        module = load_module(SIP_SCRIPT)
        result = module.calculate_smart_sip(1000, 2, 5, 7)
        self.assertEqual(result["base_annual_return"], 5)
        self.assertEqual(result["smart_scenario_annual_return"], 7)
        self.assertIn("不假设", result["scenario_notice"])

    def test_allocation_renderers_require_reviewed_templates(self):
        builder = load_module(FUND_BUILDER_SCRIPT)
        model = {
            "risk_level": "reviewed",
            "description": "user-reviewed scenario",
            "as_of": "2026-09-08",
            "sources": ["policy https://source.example/policy"],
            "allocation": {"cash": 40, "equity": 60},
            "scenario_annual_return": 4,
            "scenario_max_drawdown": 15,
            "fund_type_notes": {},
        }
        portfolio = builder.generate_portfolio(model, 100000, 3)
        self.assertEqual(portfolio["allocation"]["cash"]["amount"], 40000)
        self.assertEqual(portfolio["model_as_of"], model["as_of"])

        allocator = load_module(ALLOCATOR_SCRIPT)
        policy = {
            "risk_level": "reviewed",
            "description": "user-reviewed policy",
            "as_of": "2026-09-08",
            "sources": ["policy https://source.example/policy"],
            "allocation": {"survival": 50, "growth": 40, "aggressive": 10},
            "scenario_annual_return": 4,
            "scenario_max_drawdown": 15,
            "emergency_months": 6,
            "rebalance_threshold": 5,
        }
        plan = allocator.generate_allocation_plan(100000, 10000, 3, policy)
        self.assertEqual(plan["allocation"]["survival"]["amount"], 50000)
        self.assertEqual(plan["emergency_fund"]["shortfall"], 10000)

    def test_rebalance_cost_and_tracker_benchmarks_are_explicit(self):
        rebalance = load_module(REBALANCE_SCRIPT)
        plan = rebalance.calculate_rebalance_plan(
            {"A": 60, "B": 40}, {"A": 50, "B": 50}, 1000, 5, 2
        )
        self.assertEqual(plan["summary"]["total_sell"], 100)
        self.assertEqual(plan["summary"]["estimated_cost"], 2)
        self.assertEqual(plan["transaction_cost_rate"], 2)
        self.assertNotIn("suggestions", plan)

        tracker = load_module(TRACKER_SCRIPT)
        report = tracker.track_portfolio(
            100,
            110,
            [{"value": 100}, {"value": 105}, {"value": 110}],
            2,
            2,
            {"verified benchmark": 5},
            "2026-09-08",
            ["ledger https://source.example/ledger"],
        )
        self.assertEqual(report["benchmark_comparison"]["verified benchmark"]["benchmark_return"], 5)
        self.assertEqual(report["risk_free_rate"], 2)
        self.assertNotIn("suggestions", report)

    def test_venture_models_require_attributed_assumptions(self):
        valuation = load_module(VALUATION_SCRIPT)
        assumptions = {
            "as_of": "2026-09-08",
            "sources": ["comparable deals https://source.example/deals"],
            "comparable_valuation_range": [500, 1000],
            "revenue_multiple_range": [3, 5],
            "scorecard_base_valuation": 800,
            "scorecard_weights": {"team": 0.3, "product": 0.25, "market": 0.2, "competition": 0.15, "timing": 0.1},
            "scorecard_factor_range": [0.5, 1.5],
            "range_factor": 0.2,
        }
        args = SimpleNamespace(
            stage="天使轮", industry="test", revenue=1_000_000, growth_rate=20,
            team_score=7, product_score=7, market_score=7, competition_score=7, timing_score=7,
            target_return=30, exit_multiple=5, years=5,
        )
        report = valuation.generate_valuation_report(args, assumptions)
        self.assertEqual(report["assumptions_as_of"], assumptions["as_of"])
        self.assertEqual(report["sources"], assumptions["sources"])
        self.assertNotIn("analysis", report)
        self.assertNotIn("negotiation_range", report)
        self.assertIn("不生成谈判", report["methodology"])

        financial = load_module(FINANCIAL_MODEL_SCRIPT)
        financial_args = SimpleNamespace(
            years=2,
            initial_active_users=1000,
            annual_net_user_growth_rate=10,
            monthly_arpu=100,
            cac_per_new_user=50,
            gross_margin=70,
            monthly_churn_rate=3,
            monthly_fixed_costs=50000,
            monthly_other_cash_outflow=10000,
            initial_cash=500000,
            liquidity_buffer_months=6,
            as_of="2026-09-08",
            source=["ledger https://source.example/ledger"],
        )
        model = financial.generate_model(financial_args)
        self.assertEqual(model["inputs_as_of"], financial_args.as_of)
        self.assertNotIn("recommended_rounds", json.dumps(model))
        self.assertEqual(model["assumptions"]["liquidity_buffer_months"], 6)
        financial_args.monthly_churn_rate = 0
        with self.assertRaisesRegex(ValueError, "monthly_churn_rate"):
            financial.generate_model(financial_args)

    def test_business_evaluator_only_calculates_reviewed_rubric(self):
        module = load_module(BUSINESS_EVALUATOR_SCRIPT)
        raw = {
            "project_name": "test venture",
            "industry": "test",
            "as_of": "2026-09-08",
            "sources": ["memo https://source.example/memo"],
            "evidence": ["verified evidence"],
            "market": {
                "tam": 100,
                "sam": 50,
                "som": 10,
                "currency": "CNY",
                "as_of": "2026-09-08",
                "sources": ["study https://source.example/market"],
                "basis": "same-period bottom-up estimate",
            },
            "criteria": [
                {"id": "team", "label": "team", "weight": 0.4, "score": 8, "basis": "evidence A", "risks": []},
                {"id": "product", "label": "product", "weight": 0.6, "score": 6, "basis": "evidence B", "risks": ["risk"]},
            ],
        }
        report = module.generate_evaluation(raw)
        self.assertAlmostEqual(report["weighted_score_out_of_10"], 6.8)
        self.assertIn("不把分数映射", report["methodology"])
        invalid = json.loads(json.dumps(raw))
        invalid["criteria"][0]["weight"] = 0.5
        with self.assertRaisesRegex(ValueError, "合计 1"):
            module.generate_evaluation(invalid)

    def test_realtime_quote_is_attributed_and_provider_failures_are_not_empty_success(self):
        module = load_module(REALTIME_QUOTE_SCRIPT)

        class Response:
            def __init__(self, value):
                self.value = value

            def read(self):
                return self.value

        fields = ["测试股份", "10", "9", "11", "12", "8", "10.9", "11.1", "10000", "110000"]
        fields.extend(["0"] * 20)
        fields.extend(["2026-09-08", "14:59:00"])
        payload = f'var hq_str_sh600519="{",".join(fields)}";\n'.encode("gbk")
        rows = module.fetch_realtime_sina(["sh600519"], opener=lambda *_args, **_kwargs: Response(payload))
        quote = rows["sh600519"]
        self.assertEqual(quote["market_timestamp"], "2026-09-08 14:59:00")
        self.assertEqual(quote["source_interface"], module.REALTIME_SOURCE_INTERFACE)
        self.assertEqual(quote["volume_shares"], 10000)

        def offline(*_args, **_kwargs):
            raise OSError("offline")

        with self.assertRaisesRegex(RuntimeError, "接口失败"):
            module.fetch_realtime_sina(["sh600519"], opener=offline)
        with self.assertRaisesRegex(ValueError, "六位"):
            module.get_sina_symbol("not-a-code")

        minute = module.analyze_minute_volume(
            [
                {"time": "2026-09-08 09:30:00", "volume_shares": 100, "amount_cny": 1000, "close": 10},
                {"time": "2026-09-08 14:30:00", "volume_shares": 300, "amount_cny": 3000, "close": 11},
            ]
        )
        self.assertEqual(minute["distribution"]["close_30min"]["percent"], 75)
        self.assertNotIn("signals", minute)
        self.assertIn("不推断", minute["methodology"])

    def test_stock_data_fetcher_reports_completeness_and_provenance(self):
        module = load_module(STOCK_DATA_FETCHER_SCRIPT)

        class Frame:
            empty = False

            def __init__(self, rows):
                self.rows = rows

            def to_dict(self, orient):
                self.assert_orient = orient
                return self.rows

        provider = SimpleNamespace(
            __version__="test",
            stock_individual_info_em=lambda **_kwargs: Frame(
                [
                    {"item": "股票简称", "value": "测试股份"},
                    {"item": "总市值", "value": "1000"},
                ]
            ),
        )
        result = module.fetch_stock_data("600519", "basic", use_cache=False, provider=provider)
        self.assertEqual(result["completeness"]["status"], "complete")
        self.assertEqual(result["basic_info"]["name"], "测试股份")
        self.assertEqual(result["sources"], ["AkShare akshare.stock_individual_info_em"])

        def fail(**_kwargs):
            raise OSError("offline")

        failed = module.fetch_stock_data(
            "600519",
            "basic",
            use_cache=False,
            provider=SimpleNamespace(__version__="test", stock_individual_info_em=fail),
        )
        self.assertEqual(failed["completeness"]["status"], "failed")
        self.assertTrue(failed["completeness"]["errors"])
        self.assertNotIn("basic_info", failed)

    def test_financial_analyzer_has_no_builtin_rating_score_or_ranking(self):
        module = load_module(FINANCIAL_ANALYZER_SCRIPT)
        stock = {
            "code": "600519",
            "retrieved_at_utc": "2026-09-08T08:00:00+00:00",
            "sources": ["AkShare akshare.stock_financial_abstract"],
            "completeness": {"status": "complete", "errors": []},
            "basic_info": {"name": "测试股份"},
            "financial_indicators": {
                "records": [
                    {
                        "日期": "2026-06-30",
                        "净资产收益率": "12%",
                        "销售毛利率": "30%",
                        "销售净利率": "10%",
                        "总资产周转率": "0.5",
                        "权益乘数": "2",
                        "营业收入增长率": "8%",
                        "应收账款增长率": "10%",
                    },
                    {
                        "日期": "2025-12-31",
                        "净资产收益率": "11%",
                        "销售毛利率": "28%",
                        "营业收入增长率": "7%",
                        "应收账款增长率": "9%",
                    },
                ],
                "source_interface": "akshare.stock_financial_abstract",
            },
        }
        report = module.analyze_stock(stock)
        self.assertEqual(report["period_changes"]["newest_minus_comparison"]["roe_percent"], 1)
        self.assertEqual(report["dupont"]["calculated_roe_percent"], 10)
        for forbidden_key in ("score", "rating", "ranking", "recommendation"):
            self.assertNotIn(forbidden_key, report)
        comparison = module.compare_stocks([stock])
        self.assertNotIn("ranking", comparison)
        with self.assertRaisesRegex(ValueError, "sources"):
            module.analyze_stock({"retrieved_at_utc": "2026-09-08", "financial_indicators": [{}]})

    def test_user_allocation_notes_are_not_relabelled_as_fund_recommendations(self):
        module = load_module(FUND_BUILDER_SCRIPT)
        model = {
            "risk_level": "reviewed",
            "description": "reviewed policy",
            "as_of": "2026-09-08",
            "sources": ["policy https://source.example/policy"],
            "allocation": {"cash": 100},
            "scenario_annual_return": 1,
            "scenario_max_drawdown": 0,
            "fund_type_notes": {"cash": ["user note"]},
        }
        result = module.generate_portfolio(model, 1000, 1)
        self.assertEqual(result["fund_type_notes"]["cash"], ["user note"])
        self.assertNotIn("fund_recommendations", result)
        self.assertNotIn("advice", result)
        self.assertNotIn("投资官建议", module.format_report(result))

    def test_due_diligence_is_a_contextual_template_without_auto_timeline(self):
        module = load_module(DUE_DILIGENCE_SCRIPT)
        with self.assertRaisesRegex(ValueError, "jurisdiction"):
            module.generate_dd_checklist("A轮", "", "software", "equity")
        checklist = module.generate_dd_checklist("A轮", "中国大陆", "软件", "股权融资")
        serialized = json.dumps(checklist, ensure_ascii=False)
        self.assertEqual(checklist["context"]["jurisdiction"], "中国大陆")
        self.assertNotIn("timeline", checklist)
        self.assertNotIn("owner", serialized)
        self.assertNotIn("high_priority", serialized)
        self.assertIn("通用起点模板", checklist["template_notice"])

    def test_stock_report_template_contains_no_score_target_or_trade_fields(self):
        template = STOCK_REPORT_TEMPLATE.read_text(encoding="utf-8")
        for forbidden in ("overall_score", "investment_recommendation", "safety_price", "建议买入价", "行业均值"):
            self.assertNotIn(forbidden, template)


if __name__ == "__main__":
    unittest.main(verbosity=2)
