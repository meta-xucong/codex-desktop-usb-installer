from pathlib import Path
import json
import re
import unittest

PACK = Path(__file__).resolve().parents[1]
SKILLS = PACK / "skills"
TOOLS = json.loads((PACK / "manifest" / "feishu-tools.json").read_text(encoding="utf-8"))
SKILL_INDEX = json.loads((PACK / "manifest" / "skill-audit.json").read_text(encoding="utf-8"))
MCP_INDEX = json.loads((PACK / "manifest" / "mcp-servers.json").read_text(encoding="utf-8"))
FEISHU_SKILLS = ("feishu-doc", "feishu-drive", "feishu-perm", "feishu-wiki")

# Tool names in this package have two namespace components after the version
# (for example drive.v1.file.list). Capture every namespace, including names
# we do not recognize, so adding a new/typoed namespace cannot escape review.
TOOL_RE = re.compile(r"\b[a-z][A-Za-z0-9]*\.v\d+\.[A-Za-z][A-Za-z0-9]*(?:\.[A-Za-z][A-Za-z0-9]*)+\b")
TOOL_ROW_RE = re.compile(r"^\|.*`(?P<tool>[a-z][A-Za-z0-9]*\.v\d+\.[A-Za-z][A-Za-z0-9]*(?:\.[A-Za-z][A-Za-z0-9]*)+)`.*\|$")

# These are the exact operations each skill teaches. Keep the behavior class
# explicit so a skill cannot silently grow an unreviewed write operation.
EXPECTED_TOOLS = {
    "feishu-doc": {
        "read": {
            "docx.v1.document.get",
            "docx.v1.document.rawContent",
            "docx.v1.documentBlock.list",
            "docx.v1.documentBlock.get",
        },
        "write": set(),
    },
    "feishu-drive": {
        "read": {"drive.v1.file.list", "drive.v1.meta.batchQuery"},
        "write": {
            "drive.v1.file.copy",
            "drive.v1.file.createFolder",
            "drive.v1.file.move",
            "drive.v1.file.delete",
        },
    },
    "feishu-perm": {
        "read": {
            "drive.v1.permissionMember.auth",
            "drive.v1.permissionMember.list",
            "drive.v1.permissionPublic.get",
        },
        "write": {
            "drive.v1.permissionMember.create",
            "drive.v1.permissionMember.update",
            "drive.v1.permissionMember.delete",
            "drive.v1.permissionMember.transferOwner",
        },
    },
    "feishu-wiki": {
        "read": {
            "wiki.v2.space.get",
            "wiki.v2.space.list",
            "wiki.v2.space.getNode",
            "wiki.v2.spaceNode.list",
        },
        "write": {
            "wiki.v2.spaceNode.copy",
            "wiki.v2.spaceNode.create",
            "wiki.v2.spaceNode.move",
            "wiki.v2.spaceNode.updateTitle",
        },
    },
}


def parse_skill_tool_rows(text):
    """Return listed tool => Markdown table row, rejecting duplicate rows."""
    rows = {}
    for line in text.splitlines():
        match = TOOL_ROW_RE.match(line)
        if match:
            tool = match.group("tool")
            if tool in rows:
                raise AssertionError(f"duplicate tool mapping row: {tool}")
            rows[tool] = line
    return rows


def parse_enabled_tools(fragment):
    match = re.search(r"(?ms)^enabled_tools\s*=\s*\[(.*?)^\]", fragment)
    if not match:
        raise AssertionError("enabled_tools array not found")
    return re.findall(r'"([^"\n]+)"', match.group(1))


def parse_tool_approval_overrides(fragment):
    overrides = {}
    seen_approval_mode = set()
    current_tool = None
    for line in fragment.splitlines():
        header = re.fullmatch(r'\[mcp_servers\.feishu\.tools\."([^"]+)"\]', line.strip())
        if header:
            current_tool = header.group(1)
            if current_tool in overrides:
                raise AssertionError(f"duplicate tool approval override: {current_tool}")
            overrides[current_tool] = None
            continue
        if current_tool is not None:
            mode = re.fullmatch(r'approval_mode\s*=\s*"([^"]+)"', line.strip())
            if mode:
                if current_tool in seen_approval_mode:
                    raise AssertionError(f"duplicate approval_mode for tool: {current_tool}")
                overrides[current_tool] = mode.group(1)
                seen_approval_mode.add(current_tool)
            elif line.strip() and not line.lstrip().startswith("#"):
                # Other settings are allowed in the block, but an empty or
                # malformed approval setting must not silently pass.
                continue
    return overrides


class FeishuSkillContracts(unittest.TestCase):
    def test_each_skill_has_a_required_configured_mcp_dependency(self):
        entries = {entry["id"]: entry for entry in SKILL_INDEX["skills"]}
        for slug in FEISHU_SKILLS:
            with self.subTest(skill=slug):
                entry = entries[slug]
                self.assertEqual(entry["status"], "requires-mcp")
                self.assertFalse(entry["defaultEnabled"])
                self.assertIn("mcp.feishu.healthy", entry["enableWhen"])
                dep = next(d for d in entry["dependencies"] if d["id"] == "mcp.feishu")
                self.assertTrue(dep["required"])
                self.assertIn("不声称", dep["failureMessage"])

    def test_skill_tool_rows_are_exact_and_correctly_classified(self):
        allowed_read = set(TOOLS["readTools"])
        allowed_write = set(TOOLS["writeTools"])
        self.assertEqual(TOOLS["packageVersion"], "0.5.1")
        self.assertEqual(TOOLS["toolNameCase"], "dot")
        self.assertTrue(allowed_read.isdisjoint(allowed_write))

        for slug in FEISHU_SKILLS:
            with self.subTest(skill=slug):
                text = (SKILLS / slug / "SKILL.md").read_text(encoding="utf-8")
                expected = EXPECTED_TOOLS[slug]
                rows = parse_skill_tool_rows(text)
                referenced = set(TOOL_RE.findall(text))
                expected_all = expected["read"] | expected["write"]

                self.assertEqual(referenced, expected_all)
                self.assertEqual(set(rows), expected_all)
                self.assertLessEqual(expected["read"], allowed_read)
                self.assertLessEqual(expected["write"], allowed_write)
                for tool in expected["read"]:
                    if slug == "feishu-doc":
                        self.assertIn("当前基线只读", text)
                    else:
                        self.assertIn("只读", rows[tool], tool)
                for tool in expected["write"]:
                    if tool in TOOLS["highRiskTools"]:
                        self.assertIn("高风险", rows[tool], tool)
                    else:
                        self.assertIn("写入", rows[tool], tool)

    def test_unknown_namespaces_and_malformed_tool_suffixes_are_captured(self):
        # Guard the detector itself: the earlier prefix-limited expression
        # missed unrelated MCP namespaces and tool names with extra segments.
        examples = (
            "calendar.v4.event.create",
            "drive.v1.file.delete.extra",
            "unknown.v1.anything.run",
        )
        for candidate in examples:
            with self.subTest(candidate=candidate):
                self.assertEqual(TOOL_RE.findall(f"`{candidate}`"), [candidate])
                self.assertNotIn(candidate, set(TOOLS["readTools"] + TOOLS["writeTools"]))

    def test_approval_parser_rejects_duplicate_tool_and_default_assignments(self):
        duplicate_tool_mode = '''[mcp_servers.feishu.tools."drive.v1.file.delete"]
approval_mode = "prompt"
approval_mode = "approve"
'''
        with self.assertRaisesRegex(AssertionError, "duplicate approval_mode"):
            parse_tool_approval_overrides(duplicate_tool_mode)

        fragment = (PACK / "config-fragments" / "feishu-official-stdio.template.toml").read_text(encoding="utf-8")
        defaults = re.findall(r'(?m)^default_tools_approval_mode\s*=\s*"([^"]+)"\s*$', fragment)
        self.assertEqual(defaults, ["writes"])

    def test_high_risk_operations_have_per_tool_guards(self):
        high_risk = set(TOOLS["highRiskTools"])
        self.assertEqual(high_risk, {
            "drive.v1.file.delete",
            "drive.v1.permissionMember.delete",
            "drive.v1.permissionMember.transferOwner",
        })

        drive_rows = parse_skill_tool_rows((SKILLS / "feishu-drive" / "SKILL.md").read_text(encoding="utf-8"))
        self.assertIn("高风险", drive_rows["drive.v1.file.delete"])
        self.assertIn("回收站", drive_rows["drive.v1.file.delete"])
        drive = (SKILLS / "feishu-drive" / "SKILL.md").read_text(encoding="utf-8")
        self.assertIn("用户明确要求相应变更且目标唯一", drive)
        self.assertIn("删除、移动或覆盖语义不清时", drive)

        perm_rows = parse_skill_tool_rows((SKILLS / "feishu-perm" / "SKILL.md").read_text(encoding="utf-8"))
        self.assertIn("高风险", perm_rows["drive.v1.permissionMember.delete"])
        self.assertIn("高风险", perm_rows["drive.v1.permissionMember.transferOwner"])
        perm = (SKILLS / "feishu-perm" / "SKILL.md").read_text(encoding="utf-8")
        for guard in (
            "用户请求中明确包含目标、成员和期望权限",
            "不自行扩大权限范围",
            "不猜测 email",
            "移除协作者或转移所有者前，复述精确目标",
        ):
            self.assertIn(guard, perm)

    def test_unsupported_operations_are_not_promised(self):
        doc = (SKILLS / "feishu-doc" / "SKILL.md").read_text(encoding="utf-8")
        self.assertIn("不支持直接编辑云文档", doc)
        self.assertIn("不支持文件上传/下载", doc)
        self.assertIn("生成可审阅的 Markdown", doc)
        drive = (SKILLS / "feishu-drive" / "SKILL.md").read_text(encoding="utf-8")
        self.assertIn("不支持文件内容上传或下载", drive)
        wiki = (SKILLS / "feishu-wiki" / "SKILL.md").read_text(encoding="utf-8")
        self.assertIn("页面正文读取交给 feishu-doc", wiki)
        self.assertIn("不编辑 Wiki 页面的正文", wiki)

    def test_feishu_skills_fail_closed_when_connection_or_tool_is_missing(self):
        for slug in FEISHU_SKILLS:
            with self.subTest(skill=slug):
                text = (SKILLS / slug / "SKILL.md").read_text(encoding="utf-8")
                self.assertIn("MCP server `feishu`", text)
                self.assertTrue(any(phrase in text for phrase in ("健康", "已连接", "连接不可用")))
                if slug == "feishu-doc":
                    self.assertIn("停止外部调用", text)
                elif slug == "feishu-drive":
                    self.assertIn("不要声称已列出或修改云盘", text)
                elif slug == "feishu-perm":
                    self.assertIn("工具未启用时", text)
                    self.assertIn("不要声称权限已经生效", text)
                else:
                    self.assertIn("工具缺失时返回真实限制", text)

    def test_source_marks_feishu_as_guided_and_config_allowlist_is_exact(self):
        server = next(s for s in MCP_INDEX["servers"] if s["id"] == "feishu")
        self.assertEqual(server["status"], "guided-config")
        self.assertIn("feishu.app-id", server["enableWhen"])
        self.assertIn("feishu.app-secret", server["enableWhen"])
        self.assertIn("codex.mcp.feishu.healthy", server["enableWhen"])

        fragment = (PACK / "config-fragments" / "feishu-official-stdio.template.toml").read_text(encoding="utf-8")
        enabled = parse_enabled_tools(fragment)
        expected = TOOLS["readTools"] + TOOLS["writeTools"]
        self.assertEqual(len(enabled), len(set(enabled)), "enabled_tools contains duplicates")
        self.assertEqual(set(enabled), set(expected))
        defaults = re.findall(r'(?m)^default_tools_approval_mode\s*=\s*"([^"]+)"\s*$', fragment)
        self.assertEqual(defaults, ["writes"])
        overrides = parse_tool_approval_overrides(fragment)
        self.assertEqual(overrides, {tool: "prompt" for tool in TOOLS["highRiskTools"]})


if __name__ == "__main__":
    unittest.main()
