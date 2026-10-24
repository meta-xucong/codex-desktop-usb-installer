#!/usr/bin/env python3
"""Check that the frozen prompt-vector matrix covers each pack Skill and Agent once."""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path


def main() -> int:
    pack = Path(sys.argv[1] if len(sys.argv) > 1 else "installer-pack").resolve()
    vectors = Path(sys.argv[2] if len(sys.argv) > 2 else "docs/USB-CAPABILITY-AUDIT-TEST-VECTORS-20261018.md").resolve()
    audit = json.loads((pack / "manifest" / "skill-audit.json").read_text(encoding="utf-8"))
    agent_audit = json.loads((pack / "manifest" / "agent-audit.json").read_text(encoding="utf-8"))
    lines = vectors.read_text(encoding="utf-8").splitlines()
    skill_rows: list[tuple[str, str]] = []
    agent_rows: list[tuple[str, str]] = []
    skill_empty_fields: list[str] = []
    agent_empty_fields: list[str] = []
    for line in lines:
        if not line.startswith("| S"):
            continue
        cells = [cell.strip() for cell in line.split("|")]
        if len(cells) > 2:
            match = re.fullmatch(r"(S\d{3})\s+`([^`]+)`", cells[1])
            if match:
                skill_rows.append((match.group(1), match.group(2)))
                if len(cells) < 5 or any(not cells[index] for index in (2, 3, 4)):
                    skill_empty_fields.append(match.group(1))
    for line in lines:
        if not line.startswith("| A"):
            continue
        cells = [cell.strip() for cell in line.split("|")]
        if len(cells) > 1:
            match = re.match(r"A\d{2}\s+(.+)$", cells[1])
            if match:
                agent_rows.append((cells[1].split()[0], match.group(1).strip()))
                if len(cells) < 5 or any(not cells[index] for index in (2, 3, 4)):
                    agent_empty_fields.append(cells[1].split()[0])

    expected_skills = sorted(x.get("id", "") for x in audit.get("skills", []))
    actual_skills = sorted(name for _, name in skill_rows)
    expected_agents = sorted(x.get("name", "") for x in agent_audit.get("agents", []))
    actual_agents = sorted(name for _, name in agent_rows)
    agent_ids = [case_id for case_id, _ in agent_rows]
    result = {
        "skillVectors": len(skill_rows),
        "uniqueSkillVectorIds": len({case_id for case_id, _ in skill_rows}),
        "uniqueSkillNames": len(set(actual_skills)),
        "missingSkills": sorted(set(expected_skills) - set(actual_skills)),
        "extraSkills": sorted(set(actual_skills) - set(expected_skills)),
        "duplicateSkillIds": sorted(case_id for case_id in {case_id for case_id, _ in skill_rows}
                                     if sum(1 for candidate, _ in skill_rows if candidate == case_id) > 1),
        "skillRowsWithEmptyPromptOrAssertion": sorted(skill_empty_fields),
        "agentVectors": len(agent_rows),
        "uniqueAgentVectorIds": len(set(agent_ids)),
        "missingAgentVectorIds": sorted({f"A{i:02d}" for i in range(1, 8)} - set(agent_ids)),
        "extraAgentVectorIds": sorted(set(agent_ids) - {f"A{i:02d}" for i in range(1, 8)}),
        "duplicateAgentVectorIds": sorted(case_id for case_id in set(agent_ids) if agent_ids.count(case_id) > 1),
        "agentRowsWithEmptyPromptOrAssertion": sorted(agent_empty_fields),
        "uniqueAgentNames": len(set(actual_agents)),
        "missingAgents": sorted(set(expected_agents) - set(actual_agents)),
        "extraAgents": sorted(set(actual_agents) - set(expected_agents)),
    }
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if (
        result["skillVectors"] == 56 and result["uniqueSkillVectorIds"] == 56
        and result["uniqueSkillNames"] == 56 and not result["missingSkills"]
        and not result["extraSkills"] and not result["duplicateSkillIds"]
        and not result["skillRowsWithEmptyPromptOrAssertion"]
        and result["agentVectors"] == 7 and result["uniqueAgentVectorIds"] == 7
        and not result["missingAgentVectorIds"] and not result["extraAgentVectorIds"]
        and not result["duplicateAgentVectorIds"] and not result["agentRowsWithEmptyPromptOrAssertion"]
        and result["uniqueAgentNames"] == 7
        and not result["missingAgents"] and not result["extraAgents"]
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
