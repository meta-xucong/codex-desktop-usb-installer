"""Static contract checks for the installer process-window policy."""

from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "install-to-codex.ps1": ROOT / "installer-pack" / "install-to-codex.ps1",
    "install-wizard.ps1": ROOT / "installer-pack" / "install-wizard.ps1",
    "install-all.ps1": ROOT / "installer-pack" / "install-all.ps1",
    "provision-runtime.ps1": ROOT / "installer-pack" / "runtime" / "provision-runtime.ps1",
}
BATCH_LAUNCHER = ROOT / "installer-pack" / "Install-Codex-Starter.cmd"

POWERSHELL_CALL = re.compile(
    r"(?i)&\s*(?P<executable>powershell\.exe|\$powershell(?:Command\.Source)?)\s+@(?P<arguments>[A-Za-z_]\w*)\b"
)
WINDOW_STYLE = re.compile(
    r"(?i)(?:['\"]-WindowStyle['\"]\s*,\s*['\"]Hidden['\"]|-WindowStyle\s+Hidden)"
)


def extract_array_assignment(source: str, name: str) -> tuple[int, int, str]:
    """Return the span and body of a PowerShell $name = @(...) assignment."""

    assignment = re.search(rf"\${re.escape(name)}\s*=\s*@\(", source)
    if assignment is None:
        raise AssertionError(f"missing PowerShell argument array: ${name}")

    opening = source.find("(", assignment.start(), assignment.end())
    depth = 1
    quote: str | None = None
    index = opening + 1
    while index < len(source):
        char = source[index]
        if quote is not None:
            if quote == '"' and char == "`" and index + 1 < len(source):
                index += 2
                continue
            if char == quote:
                if index + 1 < len(source) and source[index + 1] == quote:
                    index += 2
                    continue
                quote = None
            index += 1
            continue
        if char in ("'", '"'):
            quote = char
        elif char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return assignment.start(), index + 1, source[assignment.start() : index + 1]
        index += 1
    raise AssertionError(f"unterminated PowerShell argument array: ${name}")


class InstallerChainWindowStaticTests(unittest.TestCase):
    def read_target(self, name: str) -> str:
        path = TARGETS[name]
        self.assertTrue(path.is_file(), f"missing target script: {path}")
        return path.read_text(encoding="utf-8")

    def assert_hidden_array(self, source: str, name: str) -> tuple[int, int, str]:
        start, end, fragment = extract_array_assignment(source, name)
        self.assertRegex(fragment, WINDOW_STYLE, f"${name} must pass -WindowStyle Hidden")
        return start, end, fragment

    def test_every_nested_powershell_call_uses_a_hidden_argument_array(self):
        expected = {
            # install-to-codex invokes helper scripts in-process; these are
            # splatted function/script parameters, not child PowerShell hosts.
            "install-to-codex.ps1": set(),
            "install-wizard.ps1": {"installArgs"},
            "install-all.ps1": {"arguments"},
            "provision-runtime.ps1": set(),
        }

        for name, argument_names in expected.items():
            source = self.read_target(name)
            calls = list(POWERSHELL_CALL.finditer(source))
            self.assertEqual(
                {match.group("arguments") for match in calls},
                argument_names,
                f"nested PowerShell call set changed in {name}",
            )
            self.assertEqual(len(calls), len(argument_names), f"unexpected duplicate call in {name}")
            for argument_name in argument_names:
                self.assert_hidden_array(source, argument_name)

    def test_python_installer_start_process_is_hidden(self):
        source = self.read_target("provision-runtime.ps1")
        start_process_lines = [
            line
            for line in source.splitlines()
            if re.search(r"(?i)\bStart-Process\b", line)
        ]
        self.assertEqual(len(start_process_lines), 1, "expected one Python installer Start-Process call")
        statement = start_process_lines[0]
        self.assertIn("$pythonInstaller", statement)
        self.assertRegex(statement, WINDOW_STYLE)

        for name in TARGETS:
            if name == "provision-runtime.ps1":
                continue
            source = self.read_target(name)
            self.assertNotRegex(
                source,
                r"(?i)\bStart-Process\b",
                f"unexpected child process launcher in {name}; classify it in this contract first",
            )

    def test_install_all_forwards_the_isolated_codex_home(self):
        source = self.read_target("install-all.ps1")
        self.assertRegex(source, r"(?m)^\s*\[string\]\$TargetCodexHome\s*,?\s*$")
        self.assertRegex(
            source,
            r"(?s)if\s*\(-not\s*\[string\]::IsNullOrWhiteSpace\(\$TargetCodexHome\)\)\s*\{\s*\$arguments\s*\+=\s*@\('-TargetCodexHome',\s*\$TargetCodexHome\)\s*\}",
            "install-all must forward the USB-isolated CODEX_HOME to install-to-codex.ps1",
        )

    def test_install_all_forwards_usb_full_access_switch(self):
        source = self.read_target("install-all.ps1")
        self.assertRegex(source, r"(?m)^\s*\[switch\]\$EnableFullAccess\s*,?\s*$")
        self.assertRegex(
            source,
            r"(?s)if\s*\(\$EnableFullAccess\)\s*\{\s*\$arguments\s*\+=\s*'-EnableFullAccess'\s*\}",
            "the wrapper must pass full-access only when the trusted USB caller requested it",
        )

    def test_hidden_window_options_are_limited_to_nested_install_processes(self):
        expected_arrays = {
            "install-wizard.ps1": {"installArgs"},
            "install-all.ps1": {"arguments"},
        }
        for name, array_names in expected_arrays.items():
            source = self.read_target(name)
            spans = [extract_array_assignment(source, array_name) for array_name in array_names]
            for match in re.finditer(r"(?i)-WindowStyle", source):
                self.assertTrue(
                    any(start <= match.start() < end for start, end, _ in spans),
                    f"-WindowStyle in {name} is outside a nested PowerShell argument array",
                )

    def test_visible_wizard_gui_contract_remains_present(self):
        source = self.read_target("install-wizard.ps1")
        for marker in (
            "Add-Type -AssemblyName System.Windows.Forms",
            "[System.Windows.Forms.Application]::EnableVisualStyles()",
            "[System.Windows.Forms.MessageBox]::Show",
        ):
            self.assertIn(marker, source)

    def test_batch_entrypoint_hides_both_powershell_children(self):
        self.assertTrue(BATCH_LAUNCHER.is_file(), f"missing batch entrypoint: {BATCH_LAUNCHER}")
        source = BATCH_LAUNCHER.read_text(encoding="utf-8")
        calls = [line for line in source.splitlines() if re.search(r"(?i)^powershell\.exe\s", line.strip())]
        self.assertEqual(len(calls), 2, "expected wizard and noninteractive PowerShell entrypoints")
        for line in calls:
            self.assertRegex(line, r"(?i)-WindowStyle\s+Hidden", "batch PowerShell child must be hidden")


if __name__ == "__main__":
    unittest.main(verbosity=2)
