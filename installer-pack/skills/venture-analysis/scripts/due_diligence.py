#!/usr/bin/env python
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Create a clearly labelled, context-specific due-diligence starter checklist."""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


TEMPLATE_VERSION = "2.0"
STAGES = ("天使轮", "Pre-A", "A轮", "B轮")

COMMON_ITEMS: dict[str, list[tuple[str, str]]] = {
    "legal": [
        ("entity", "核对设立、存续、章程、登记和组织架构文件"),
        ("capitalization", "核对完整股权表、历史变更、期权及其他潜在稀释安排"),
        ("securities", "核对历次融资、股东协议及仍有效的投资者权利"),
        ("contracts", "盘点重大客户、供应商、合作、债务及控制权变更条款"),
        ("ip", "核对知识产权权属、许可、开源使用及员工/承包商成果归属"),
        ("employment", "核对劳动、顾问、保密、竞业及关键人员安排"),
        ("disputes", "核对诉讼、仲裁、行政调查、处罚及潜在争议"),
        ("privacy", "核对个人信息、数据跨境、网络安全和事件响应义务"),
        ("permits", "核对经营许可、行业准入及持续合规要求"),
    ],
    "financial": [
        ("statements", "核对财务报表、会计政策、审计/审阅范围和调整事项"),
        ("ledger", "抽查总账、明细账、银行流水与报表勾稽关系"),
        ("revenue", "核对收入确认、合同条款、截止性、退款和递延项目"),
        ("customers", "核对客户集中度、应收账款、回款和坏账口径"),
        ("costs", "核对成本归集、供应商集中度、毛利和非经常项目"),
        ("tax", "核对税务申报、优惠依据、欠缴情形和不确定税务事项"),
        ("debt", "核对债务、担保、抵押、承诺、或有负债和表外安排"),
        ("related-parties", "识别关联方、关联交易和资金往来"),
        ("forecast", "核对预算、现金流预测及关键假设与历史数据的一致性"),
    ],
    "business": [
        ("team", "核验核心团队经历、职责、投入程度和关键人员依赖"),
        ("product", "核验产品、技术架构、交付能力、路线图和技术债"),
        ("customers", "设计客户/用户核验并检查留存、使用和流失定义"),
        ("market", "复核市场规模、竞争格局和可比口径的证据链"),
        ("economics", "复核获客、留存、毛利和单位经济指标定义及数据血缘"),
        ("go-to-market", "复核销售周期、渠道依赖、定价和续约机制"),
        ("operations", "复核供应链、服务水平、质量控制和业务连续性"),
        ("security", "复核访问控制、安全测试、备份恢复和第三方风险"),
    ],
}

STAGE_ADDITIONS: dict[str, dict[str, list[tuple[str, str]]]] = {
    "天使轮": {
        "business": [("validation", "核验当前产品验证范围以及尚未验证的核心假设")],
    },
    "Pre-A": {
        "financial": [("runway", "复核资金消耗、现金余额和跑道计算口径")],
        "business": [("cohorts", "按一致口径复核用户 cohort 与渠道质量")],
    },
    "A轮": {
        "financial": [("working-capital", "复核运营资本、账龄和现金转换周期")],
        "business": [("scaling", "核验规模化假设、交付瓶颈和关键岗位能力")],
    },
    "B轮": {
        "legal": [("multi-region", "按实际经营地区核对跨区域或跨境合规")],
        "financial": [("segments", "复核分产品、地区或业务线数据与合并口径")],
        "business": [("controls", "复核管理报告、内部控制和数据治理成熟度")],
    },
}


def _required_text(value: str, label: str) -> str:
    normalized = str(value or "").strip()
    if not normalized:
        raise ValueError(f"{label} 不能为空。")
    if len(normalized) > 200:
        raise ValueError(f"{label} 不能超过 200 个字符。")
    return normalized


def generate_dd_checklist(
    stage: str,
    jurisdiction: str,
    industry: str,
    transaction_structure: str,
) -> dict[str, Any]:
    if stage not in STAGES:
        raise ValueError("stage 不在支持列表中。")
    context = {
        "stage": stage,
        "jurisdiction": _required_text(jurisdiction, "jurisdiction"),
        "industry": _required_text(industry, "industry"),
        "transaction_structure": _required_text(transaction_structure, "transaction_structure"),
    }
    checklists: dict[str, list[dict[str, str]]] = {}
    for category, base_items in COMMON_ITEMS.items():
        items = [*base_items, *STAGE_ADDITIONS.get(stage, {}).get(category, [])]
        checklists[category] = [
            {
                "id": f"{category}.{item_id}",
                "review_question": question,
                "status": "not_reviewed",
                "applicability": "confirm",
                "evidence": "",
                "reviewer_notes": "",
            }
            for item_id, question in items
        ]
    return {
        "template_version": TEMPLATE_VERSION,
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "context": context,
        "summary": {
            "item_count": sum(len(items) for items in checklists.values()),
            "categories": list(checklists),
            "reviewed_count": 0,
        },
        "checklists": checklists,
        "template_notice": (
            "这是通用起点模板，不表示任何项目已通过核验，也不构成法律、税务、会计、"
            "技术、网络安全或投资意见。适用性、证据标准、负责人和时间表必须由交易团队及相应专业人士确认。"
        ),
        "required_customization": [
            "按法域和监管要求增删项目",
            "按行业、数据处理活动和许可证要求增删项目",
            "按交易结构、证券权利和融资文件增删项目",
            "为每项填写证据、统计期、来源、审阅人和结论",
        ],
    }


def format_checklist(checklist: dict[str, Any]) -> str:
    context = checklist["context"]
    lines = [
        f"# {context['stage']}尽职调查起点清单",
        "",
        f"- 模板版本：{checklist['template_version']}",
        f"- 生成时间（UTC）：{checklist['generated_at_utc']}",
        f"- 法域：{context['jurisdiction']}",
        f"- 行业：{context['industry']}",
        f"- 交易结构：{context['transaction_structure']}",
        "",
        f"> {checklist['template_notice']}",
        "",
        "## 使用前必须定制",
        "",
    ]
    lines.extend(f"- {item}" for item in checklist["required_customization"])
    category_names = {"legal": "法律与合规", "financial": "财务与税务", "business": "业务、技术与运营"}
    for category, items in checklist["checklists"].items():
        lines.extend(["", f"## {category_names[category]}", ""])
        for item in items:
            lines.append(f"- [ ] `{item['id']}` {item['review_question']}（适用性：待确认；证据：待填写）")
    lines.extend(
        [
            "",
            "## 完成条件",
            "",
            "只有在逐项确认适用性、记录可追溯证据并由有权限的审阅人签署后，才能更新状态；生成本文件不等于完成尽调。",
            "",
        ]
    )
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="尽职调查通用起点清单生成器")
    parser.add_argument("--stage", required=True, choices=STAGES, help="融资阶段，仅用于选择模板增量")
    parser.add_argument("--jurisdiction", required=True, help="需要核验的法域")
    parser.add_argument("--industry", required=True, help="项目行业")
    parser.add_argument("--transaction-structure", required=True, help="股权、可转债等交易结构")
    parser.add_argument("--output", help="输出文件路径")
    parser.add_argument("--json", action="store_true", help="输出 JSON；默认输出 Markdown")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        checklist = generate_dd_checklist(
            args.stage,
            args.jurisdiction,
            args.industry,
            args.transaction_structure,
        )
        output = json.dumps(checklist, ensure_ascii=False, indent=2) if args.json else format_checklist(checklist)
        if args.output:
            path = Path(args.output).expanduser().resolve()
            path.write_text(output + "\n", encoding="utf-8")
            print(f"尽调起点清单已保存到：{path}")
        else:
            print(output)
        return 0
    except (OSError, ValueError) as exc:
        payload = json.dumps({"success": False, "error": str(exc)}, ensure_ascii=False)
        print(payload if args.json else f"清单生成失败：{exc}", file=sys.stdout if args.json else sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
