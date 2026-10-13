#!/usr/bin/env python3
"""Small dependency-free tests for compile_prompt.py."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from compile_prompt import SpecError, compile_scene, example_spec  # noqa: E402


def test_complex_spec_is_high_risk_and_downgraded() -> None:
    report = compile_scene(example_spec("complex-outfit"))
    assert report["risk"]["level"] in {"high", "critical"}
    assert report["effective_strategy"]["reference"] == "preserve_requested"
    assert report["effective_spec"]["reference"]["strategy"] == "preserve_requested"
    assert report["effective_strategy"]["transition"] == "preserve_requested"
    assert len(report["downgrades"]) >= 2
    assert report["prompt_utf8_bytes"] <= 4096
    assert report["prompt_for_mcp_utf8_bytes"] <= 4096
    assert "extra visible subjects" in report["negative_prompt"]
    assert "one person" in report["prompt"]


def test_simple_spec_stays_low_risk() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 6,
        "ratio": "9:16",
        "resolution": "720p",
        "actor_count": 1,
        "scene_count": 1,
        "scene": "a quiet studio",
        "reference": {"kind": "clean_single_person", "has_stickers": False},
        "camera": {"locked": True, "full_body": True},
        "identity": {"adult": True, "visual_age": "adult"},
        "outfit_changes": [{"time_seconds": 3, "outfit": "black evening dress"}],
        "transition": {"requested": "in_place_change"},
        "audio": {"generate": False},
    }
    report = compile_scene(spec)
    assert report["risk"]["level"] == "low"
    assert report["downgrades"] == []


def test_invalid_change_is_rejected() -> None:
    spec = example_spec("complex-outfit")
    spec["outfit_changes"][1]["time_seconds"] = 2
    try:
        compile_scene(spec)
    except SpecError as exc:
        assert "strictly increasing" in str(exc)
    else:
        raise AssertionError("duplicate change time should fail")


def test_transparent_non_final_reference_gets_explicit_downgrades() -> None:
    spec = example_spec("complex-outfit")
    spec["reference"]["usage"] = "i2v_first_frame"
    spec["reference"]["has_transparency"] = True
    report = compile_scene(spec)
    assert any("透明 PNG" in item["requested"] for item in report["downgrades"])
    assert any("非最终场景" in item["requested"] for item in report["downgrades"])


def test_identity_only_reference_is_not_sent_as_first_frame() -> None:
    spec = example_spec("complex-outfit")
    spec["reference"] = {
        "kind": "clean_single_person",
        "usage": "identity_only",
        "role_aware_channel": False,
        "scene_matches_final": False,
    }
    report = compile_scene(spec)
    assert report["effective_strategy"]["reference"] == "preserve_requested"
    assert any("身份参考" in item["requested"] for item in report["downgrades"])
    assert any("角色分离通道" in finding for finding in report["risk"]["findings"])
    assert any("原始身份肖像" in item for item in report["recommendations"])


def test_ratio_and_framing_mismatch_are_audited() -> None:
    spec = example_spec("complex-outfit")
    spec["reference"] = {
        "kind": "clean_single_person",
        "usage": "i2v_first_frame",
        "source_ratio": "1:1",
        "framing": "half_body",
        "scene_matches_final": False,
    }
    report = compile_scene(spec)
    assert report["risk"]["score"] >= 35
    assert report["effective_strategy"]["reference"] == "preserve_requested"
    assert "stretched body" in report["negative_prompt"]


def test_keyframe_role_requires_time_capable_channel() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 10,
        "ratio": "9:16",
        "resolution": "720p",
        "reference": {
            "kind": "clean_single_person",
            "usage": "keyframe",
            "keyframe_time_seconds": 6,
            "role_aware_channel": False,
        },
        "transition": {},
        "audio": {"generate": False},
    }
    report = compile_scene(spec)
    assert report["effective_strategy"]["reference"] == "preserve_requested"
    assert any("关键帧输入" in finding for finding in report["risk"]["findings"])
    assert any("关键帧" in item["requested"] for item in report["downgrades"])
    assert "time zero" in " ".join(report["qa_checklist"])


def test_task_specific_constraints_are_not_added_when_unspecified() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 8,
        "ratio": "16:9",
        "resolution": "1080p",
        "scene": "a convenience store with a customer and a clerk",
        "subject": "two adults interacting at a checkout counter",
        "reference": {"kind": "clean_single_person", "usage": "identity_only", "role_aware_channel": True},
        "identity": {"face_locked": True},
        "camera": {"locked": False, "full_body": False},
        "transition": {"requested": "in_place_change"},
        "audio": {"generate": False},
        "hard_constraints": ["keep the clerk visible"],
    }
    report = compile_scene(spec)
    assert "two adults interacting" in report["prompt"]
    assert "exactly" not in report["prompt"]
    assert "Use full-body framing" not in report["prompt"]
    assert "one continuous scene" not in report["prompt"]
    assert "Object conservation" not in report["prompt"]
    assert "extra visible subjects" not in report["negative_prompt"]
    assert "camera movement" not in report["negative_prompt"]
    assert report["downgrades"] == []


def test_continuity_is_opt_in_and_does_not_simplify_multi_subject_scene() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 10,
        "ratio": "16:9",
        "resolution": "720p",
        "actor_count": 3,
        "scene_count": 1,
        "subject": "three dancers with a red umbrella on a theater stage",
        "scene": "a live theater stage with moving spotlights",
        "reference": {"usage": "unknown"},
        "camera": {"locked": False, "full_body": True},
        "transition": {"requested": "one continuous ensemble move"},
        "audio": {"generate": False},
        "continuity": {
            "mode": "strict",
            "anchors": [
                {
                    "id": "lead_pair",
                    "subject": "dancer A and dancer B",
                    "priority": "critical",
                    "visibility": "visible",
                    "start_state": "dancer A is frame-left of dancer B",
                    "end_state": "dancer A is frame-right of dancer B",
                    "change_policy": "declared",
                    "change": "one visible crossing move",
                },
                {
                    "id": "umbrella",
                    "subject": "the red umbrella",
                    "priority": "important",
                    "visibility": "visible",
                    "attachment": "dancer_c.right_hand",
                    "start_state": "held upright in dancer C's right hand",
                    "change_policy": "fixed",
                },
            ],
        },
    }
    report = compile_scene(spec)
    assert "exactly 3 visible subjects" in report["prompt"]
    assert "moving spotlights" in report["prompt"]
    assert "Continuity for dancer A and dancer B" in report["prompt"]
    assert "Continuity for the red umbrella" in report["prompt"]
    assert "one visible crossing move" in report["prompt"]
    assert any("lead_pair" in item for item in report["qa_checklist"])
    assert "extra visible subjects" not in report["negative_prompt"]


def test_continuity_attachment_collision_and_handoff_are_audited() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 8,
        "ratio": "9:16",
        "resolution": "480p",
        "scene": "a traveler waits at a platform",
        "subject": "one traveler holding a camera and a suitcase",
        "reference": {"usage": "i2v_first_frame", "source_ratio": "9:16", "scene_matches_final": True},
        "camera": {"locked": True, "full_body": True},
        "transition": {"requested": "single waiting beat"},
        "audio": {"generate": False},
        "continuity": {
            "mode": "strict",
            "anchors": [
                {
                    "id": "camera",
                    "subject": "the camera",
                    "priority": "critical",
                    "visibility": "visible",
                    "attachment": "traveler.right_hand",
                    "start_state": "held in the traveler's right hand",
                    "change_policy": "fixed",
                },
                {
                    "id": "suitcase",
                    "subject": "the suitcase handle",
                    "priority": "critical",
                    "visibility": "visible",
                    "attachment": "traveler.right_hand",
                    "start_state": "held in the traveler's right hand",
                    "handoff_from": "standing beside the traveler's left foot",
                    "change_policy": "fixed",
                },
            ],
        },
    }
    report = compile_scene(spec)
    findings = "\n".join(report["risk"]["findings"])
    assert report["risk"]["score"] >= 40
    assert "上一镜交接状态" in findings
    assert "同时要求占用 traveler.right_hand" in findings
    assert any("先修正目标首帧" in item["effective"] for item in report["downgrades"])


def test_inventory_conservation_compiles_unique_prop_transfer_without_simplifying_people() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 9,
        "ratio": "16:9",
        "resolution": "480p",
        "actor_count": 2,
        "scene_count": 1,
        "scene": "a convenience-store checkout counter at night",
        "subject": "a woman thanks a cashier after taking her milk",
        "reference": {"usage": "reference_to_video"},
        "camera": {"locked": False, "full_body": False},
        "transition": {},
        "audio": {"generate": False},
        "continuity": {
            "mode": "strict",
            "inventory": [
                {
                    "id": "milk_carton",
                    "subject": "the sealed milk carton",
                    "quantity": 1,
                    "priority": "critical",
                    "start_state": "one carton rests on the checkout counter",
                    "end_state": "the same carton is held only in the woman's right hand; her left hand is empty",
                    "transfers": [
                        {
                            "action": "one visible pickup",
                            "from": "the checkout counter",
                            "to": "the woman's right hand",
                        }
                    ],
                    "exclusive_location": True,
                    "forbidden_states": [
                        "two milk cartons",
                        "the same carton held by both hands",
                        "a carton still on the counter after pickup",
                    ],
                }
            ],
        },
    }
    report = compile_scene(spec)
    assert "Show exactly 2 visible subjects" in report["prompt"]
    assert "Object conservation for the sealed milk carton" in report["prompt"]
    assert "exactly 1 physical instance exists throughout" in report["prompt"]
    assert "The only allowed transfer sequence is: one visible pickup" in report["prompt"]
    assert "never two hands, actors, or locations at once" in report["prompt"]
    assert "duplicate the sealed milk carton" in report["negative_prompt"]
    assert "the same sealed milk carton in multiple hands" in report["negative_prompt"]
    assert any("conservation entity milk_carton" in item for item in report["qa_checklist"])
    assert any("conservation transfer for milk_carton" in item for item in report["qa_checklist"])
    assert report["risk"]["score"] >= 10


def test_inventory_conservation_rejects_underspecified_transfers_and_counts() -> None:
    base = {
        "version": 1,
        "duration_seconds": 6,
        "ratio": "9:16",
        "resolution": "480p",
        "reference": {},
        "camera": {},
        "transition": {},
        "audio": {"generate": False},
        "continuity": {
            "mode": "advisory",
            "inventory": [
                {
                    "id": "phone",
                    "subject": "the phone",
                    "quantity": 1,
                    "start_state": "on the counter",
                    "end_state": "in the customer's hand",
                }
            ],
        },
    }
    try:
        compile_scene(base)
    except SpecError as exc:
        assert "transfers is required" in str(exc)
    else:
        raise AssertionError("a changed inventory state without a transfer should fail")

    base["continuity"]["inventory"][0]["transfers"] = [{"from": "the counter", "to": "the customer's hand"}]
    base["continuity"]["inventory"][0]["quantity"] = 0
    try:
        compile_scene(base)
    except SpecError as exc:
        assert "quantity must be a positive integer" in str(exc)
    else:
        raise AssertionError("a non-positive inventory quantity should fail")


def test_ambient_inventory_does_not_consume_prompt_or_negative_budget() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 6,
        "ratio": "9:16",
        "resolution": "480p",
        "scene": "a cafe table",
        "subject": "two adults talking",
        "reference": {},
        "camera": {},
        "transition": {},
        "audio": {"generate": False},
        "continuity": {
            "mode": "advisory",
            "inventory": [
                {
                    "id": "table_flower",
                    "subject": "the small flower vase",
                    "quantity": 1,
                    "priority": "ambient",
                    "start_state": "on the center of the table",
                }
            ],
        },
    }
    report = compile_scene(spec)
    assert "Object conservation for the small flower vase" not in report["prompt"]
    assert "duplicate the small flower vase" not in report["negative_prompt"]
    assert any("conservation entity table_flower" in item for item in report["qa_checklist"])


def test_critical_prop_on_limb_requires_visible_anatomy_anchor_when_needed() -> None:
    base = {
        "version": 1,
        "duration_seconds": 8,
        "ratio": "9:16",
        "resolution": "480p",
        "scene": "a hotel lobby",
        "subject": "one traveler beside a revolving door",
        "reference": {"usage": "i2v_first_frame", "source_ratio": "9:16", "scene_matches_final": True},
        "camera": {"locked": True, "full_body": True},
        "transition": {},
        "audio": {"generate": False},
        "continuity": {
            "mode": "strict",
            "anchors": [
                {
                    "id": "bag",
                    "subject": "the handbag",
                    "priority": "critical",
                    "visibility": "visible",
                    "attachment": "traveler.left_forearm",
                    "start_state": "two handles looped over the left forearm",
                    "change_policy": "fixed",
                }
            ],
        },
    }
    report = compile_scene(base)
    assert any("没有单独的可见人体锚点" in item for item in report["risk"]["findings"])

    base["continuity"]["anchors"].append({
        "id": "left_hand_complete",
        "subject": "the traveler's left hand and forearm",
        "priority": "critical",
        "visibility": "visible",
        "attachment": "traveler.left_hand",
        "start_state": "fully visible, connected to the forearm, with a relaxed palm and distinct fingers",
        "change_policy": "fixed",
    })
    report = compile_scene(base)
    assert not any("关键锚点 bag 依赖" in item for item in report["risk"]["findings"])


def test_rejected_source_cannot_reenter_visual_input_chain() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 8,
        "ratio": "9:16",
        "resolution": "480p",
        "scene": "an apartment lobby",
        "subject": "one traveler walking toward the door",
        "reference": {
            "usage": "i2v_first_frame",
            "source_ratio": "9:16",
            "scene_matches_final": True,
            "input_ledger": {
                "assets": [
                    {
                        "id": "bad_previous_end_frame",
                        "role": "identity reference",
                        "route": "direct_input",
                        "verdict": "rejected",
                        "reason": "suitcase handle disappeared",
                    },
                    {
                        "id": "story_state",
                        "route": "text_summary",
                        "verdict": "approved",
                        "summary": "the traveler has reached the lobby",
                    },
                ]
            },
        },
        "camera": {},
        "transition": {},
        "audio": {"generate": False},
    }
    report = compile_scene(spec)
    findings = "\n".join(report["risk"]["findings"])
    assert report["risk"]["score"] >= 35
    assert "已被验收拒绝" in findings
    assert report["creation_policy"] == "blocked"
    assert report["blocking_findings"]
    assert any("从所有多图编辑和 I2V 输入中排除" in item["effective"] for item in report["downgrades"])
    assert "Do not use rejected prior frames" in report["prompt"]
    assert any("bad_previous_end_frame" in item for item in report["qa_checklist"])


def test_excluded_source_keeps_story_summary_without_risk() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 8,
        "ratio": "9:16",
        "resolution": "480p",
        "scene": "an apartment lobby",
        "subject": "one traveler walking toward the door",
        "reference": {
            "usage": "i2v_first_frame",
            "source_ratio": "9:16",
            "scene_matches_final": True,
            "input_ledger": {
                "assets": [
                    {
                        "id": "bad_previous_end_frame",
                        "role": "previous-shot visual handoff",
                        "route": "excluded",
                        "verdict": "rejected",
                        "reason": "suitcase handle disappeared",
                    },
                    {
                        "id": "story_state",
                        "route": "text_summary",
                        "verdict": "approved",
                        "summary": "the traveler has reached the lobby",
                    },
                ]
            },
        },
        "camera": {},
        "transition": {},
        "audio": {"generate": False},
    }
    report = compile_scene(spec)
    assert report["risk"]["score"] == 0
    assert report["creation_policy"] == "ready"
    assert "Do not use rejected prior frames" not in report["prompt"]


def test_real_time_pacing_compiles_story_beats_and_qa() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 15,
        "ratio": "9:16",
        "resolution": "480p",
        "scene": "a convenience-store doorway at night",
        "subject": "one woman carrying milk pauses before leaving",
        "reference": {"usage": "i2v_first_frame", "source_ratio": "9:16", "scene_matches_final": True},
        "camera": {"locked": True, "full_body": True},
        "transition": {},
        "audio": {"generate": False},
        "continuity": {
            "mode": "strict",
            "anchors": [
                {
                    "id": "exit_path",
                    "subject": "the woman's movement path",
                    "priority": "critical",
                    "visibility": "visible",
                    "start_state": "inside the store, body facing away from the camera toward the doorway",
                    "end_state": "crosses the doorway away from the camera toward the street",
                    "change_policy": "declared",
                    "change": "two normal steps toward the exit",
                }
            ],
        },
        "pacing": {
            "mode": "real_time",
            "allow_slow_motion": False,
            "max_idle_seconds": 2.5,
            "beats": [
                {"time_seconds": 0, "event": "she takes two steps toward the open door"},
                {"time_seconds": 4, "event": "she turns back to acknowledge a neighbor"},
                {"time_seconds": 9, "event": "she gives a quick grateful nod"},
                {"time_seconds": 12, "event": "she turns forward and exits"},
            ],
            "turning_point": "she pauses at the threshold and turns back",
        },
        "narrative": {
            "objective": "show her gratitude before departure",
            "required_events": [
                "she acknowledges the neighbor's help",
                "she gives a brief grateful reply",
            ],
        },
    }
    report = compile_scene(spec)
    assert report["risk"]["score"] == 0
    assert "Use normal real-time motion" in report["prompt"]
    assert "Do not hold a visually unchanged pose" in report["prompt"]
    assert "At approximately 4s, show she turns back" in report["prompt"]
    assert "Story turning point" in report["prompt"]
    assert "Continuity for the woman's movement path" in report["prompt"]
    assert "Required plot coverage" in report["prompt"]
    assert any("required plot event" in item for item in report["qa_checklist"])
    assert any("normal-speed real time" in item for item in report["qa_checklist"])
    assert any("unchanged pose" in item for item in report["qa_checklist"])
    assert any("around 9s" in item for item in report["qa_checklist"])


def test_real_time_pacing_reports_weak_or_conflicting_plan() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 15,
        "ratio": "9:16",
        "resolution": "480p",
        "scene": "a corridor",
        "subject": "one person walks",
        "reference": {},
        "camera": {},
        "transition": {},
        "audio": {"generate": False},
        "pacing": {
            "mode": "real_time",
            "allow_slow_motion": True,
            "beats": [{"time_seconds": 0, "event": "the person walks"}],
        },
    }
    report = compile_scene(spec)
    findings = "\n".join(report["risk"]["findings"])
    assert report["risk"]["score"] >= 30
    assert "少于两个可见节拍" in findings
    assert "速度语义存在冲突" in findings


def test_invalid_pacing_idle_limit_is_rejected() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 10,
        "ratio": "9:16",
        "resolution": "480p",
        "reference": {},
        "camera": {},
        "transition": {},
        "audio": {"generate": False},
        "pacing": {
            "mode": "real_time",
            "beats": [{"time_seconds": 0, "event": "the person enters"}],
            "max_idle_seconds": 10,
        },
    }
    try:
        compile_scene(spec)
    except SpecError as exc:
        assert "max_idle_seconds" in str(exc)
    else:
        raise AssertionError("idle limit equal to duration should fail")


def test_multi_subject_and_multi_scene_are_risks_not_silent_deletions() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 12,
        "ratio": "16:9",
        "resolution": "720p",
        "actor_count": 3,
        "scene_count": 2,
        "scene": "a night market, then a riverside street",
        "subject": "three friends walk and talk, then separate",
        "reference": {"kind": "multi_person", "usage": "i2v_first_frame", "scene_matches_final": True},
        "camera": {"locked": False, "full_body": False},
        "identity": {},
        "transition": {"requested": "in_place_change"},
        "audio": {"generate": False},
    }
    report = compile_scene(spec)
    assert report["effective_spec"]["actor_count"] == 3
    assert report["effective_spec"]["scene_count"] == 2
    assert "three friends" in report["prompt"]
    assert "exactly 3 visible subjects" in report["prompt"]
    assert any("多个主体" in finding for finding in report["risk"]["findings"])
    assert any("多场景" in finding for finding in report["risk"]["findings"])
    assert not any("拼贴、贴纸" in finding for finding in report["risk"]["findings"])


def test_final_multi_person_first_frame_is_not_misclassified_as_collage() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 10,
        "ratio": "9:16",
        "resolution": "480p",
        "actor_count": 2,
        "scene_count": 1,
        "scene": "a checkout counter",
        "subject": "two adults complete a payment",
        "reference": {
            "kind": "multi_person",
            "usage": "i2v_first_frame",
            "source_ratio": "9:16",
            "scene_matches_final": True,
            "has_stickers": False,
        },
        "camera": {"locked": True, "full_body": False},
        "transition": {},
        "audio": {"generate": False},
    }
    report = compile_scene(spec)
    assert not any("拼贴、贴纸" in finding for finding in report["risk"]["findings"])
    assert not any(item["requested"] == "直接使用拼贴/贴纸参考图" for item in report["downgrades"])


def test_unknown_task_fields_do_not_create_constraints() -> None:
    spec = {
        "version": 1,
        "duration_seconds": 8,
        "ratio": "9:16",
        "resolution": "720p",
        "actor_count": "unknown",
        "scene_count": "unknown",
        "reference": {"usage": "unknown"},
        "camera": {"locked": "unknown", "full_body": "unknown"},
        "audio": {"generate": "unknown", "beat_sync": "unknown"},
    }
    report = compile_scene(spec)
    assert "exactly" not in report["prompt"]
    assert "locked camera" not in report["prompt"]
    assert "full-body framing" not in report["prompt"]
    assert "Generate lyric-free music" not in report["prompt"]
    assert "Do not generate music" not in report["prompt"]
    assert "Performance arc" not in report["prompt"]
    assert "Dialogue performance" not in report["prompt"]
    assert any("没有明确" in finding for finding in report["risk"]["findings"])


def test_hard_constraints_and_identity_details_reach_prompt() -> None:
    spec = example_spec("complex-outfit")
    spec["identity"].update({
        "country_ethnicity": "original Chinese woman",
        "hair": "deep chestnut high ponytail",
        "skin_tone": "natural warm",
    })
    report = compile_scene(spec)
    assert "deep chestnut high ponytail" in report["prompt"]
    assert "no watermark" in report["prompt"]


def test_performance_plan_compiles_source_grounded_dialogue() -> None:
    spec = example_spec("complex-outfit")
    spec["actor_count"] = 2
    spec["performance"] = {
        "arcs": [
            {
                "id": "lead_shift",
                "subject": "the lead woman",
                "priority": "critical",
                "source_evidence": "she recoils briefly, then chooses to remain present",
                "start_state": "guarded surprise",
                "turning_point": "the other person pauses and listens",
                "end_state": "embarrassed trust without losing reserve",
                "forbidden_readings": ["bright flirting", "fearful refusal", "flat neutrality"],
            }
        ],
        "dialogue_beats": [
            {
                "id": "soft_reproach",
                "speaker": "the lead woman",
                "line": "你又故意欺负我……",
                "language": "Mandarin Chinese",
                "priority": "critical",
                "source_evidence": "she has just stepped back from a sudden intimate moment",
                "precondition": "her lips are visible after the pause, with no walking or contact",
                "subtext": "a restrained, embarrassed complaint rather than a rejection",
                "vocal_delivery": "low, breath-held, with the ending trailing softly",
                "facial_cues": ["a small shoulder release", "eyes briefly avert then return"],
                "listener_response": "the other adult becomes still and listens without interrupting",
                "forbidden_readings": ["cheerful teasing", "panic", "flat recitation"],
            }
        ],
    }
    report = compile_scene(spec)
    assert "Performance arc for the lead woman" in report["prompt"]
    assert 'say exactly in Mandarin Chinese: "你又故意欺负我……"' in report["prompt"]
    assert "restrained, embarrassed complaint" in report["prompt"]
    assert any("performance arc lead_shift" in item for item in report["qa_checklist"])
    assert any("dialogue beat soft_reproach" in item for item in report["qa_checklist"])
    assert report["performance_plan"]["dialogue_beats"][0]["id"] == "soft_reproach"
    assert any("replace only that dialogue performance" in item for item in report["post_production"])


def test_critical_dialogue_requires_observable_delivery_plan() -> None:
    spec = example_spec("complex-outfit")
    spec["performance"] = {
        "dialogue_beats": [
            {
                "id": "missing_delivery",
                "speaker": "the lead woman",
                "line": "谢谢。",
                "source_evidence": "she is grateful",
                "priority": "critical",
                "precondition": "she faces the listener",
                "facial_cues": ["a small nod"],
            }
        ]
    }
    try:
        compile_scene(spec)
    except SpecError as exc:
        assert "subtext is required for critical dialogue" in str(exc)
    else:
        raise AssertionError("critical dialogue without a delivery plan should fail")


def test_prompt_soft_budget_warns_without_blocking() -> None:
    spec = example_spec("complex-outfit")
    spec["scene"] = "x" * 5000
    report = compile_scene(spec)
    assert report["prompt_for_mcp_utf8_bytes"] > 4096
    assert report["prompt_budget"]["soft_limit_bytes"] == 4096
    assert report["prompt_budget"]["hard_limit_bytes"] is None
    assert report["prompt_budget"]["soft_budget_exceeded"] is True
    assert report["creation_policy"] == "ready"
    assert any("quality budget" in item for item in report["recommendations"])


def test_configured_provider_prompt_limit_is_enforced() -> None:
    spec = example_spec("complex-outfit")
    spec["scene"] = "x" * 5000
    spec["provider"] = {"max_prompt_bytes": 4096}
    try:
        compile_scene(spec)
    except SpecError as exc:
        assert "configured provider.max_prompt_bytes" in str(exc)
    else:
        raise AssertionError("a host-evidenced prompt limit should fail")


def test_nested_types_and_string_booleans_are_handled() -> None:
    spec = example_spec("complex-outfit")
    spec["camera"]["locked"] = "true"
    spec["reference"]["has_stickers"] = "false"
    report = compile_scene(spec)
    assert report["risk"]["level"] in {"high", "critical"}
    spec["camera"] = "bad"
    try:
        compile_scene(spec)
    except SpecError as exc:
        assert "camera must be an object" in str(exc)
    else:
        raise AssertionError("invalid nested type should fail")


def test_report_is_json_serializable() -> None:
    report = compile_scene(example_spec("complex-outfit"))
    json.dumps(report, ensure_ascii=False)


if __name__ == "__main__":
    tests = [
        test_complex_spec_is_high_risk_and_downgraded,
        test_simple_spec_stays_low_risk,
        test_invalid_change_is_rejected,
        test_transparent_non_final_reference_gets_explicit_downgrades,
        test_identity_only_reference_is_not_sent_as_first_frame,
        test_ratio_and_framing_mismatch_are_audited,
        test_keyframe_role_requires_time_capable_channel,
        test_task_specific_constraints_are_not_added_when_unspecified,
        test_continuity_is_opt_in_and_does_not_simplify_multi_subject_scene,
        test_continuity_attachment_collision_and_handoff_are_audited,
        test_inventory_conservation_compiles_unique_prop_transfer_without_simplifying_people,
        test_inventory_conservation_rejects_underspecified_transfers_and_counts,
        test_ambient_inventory_does_not_consume_prompt_or_negative_budget,
        test_critical_prop_on_limb_requires_visible_anatomy_anchor_when_needed,
        test_rejected_source_cannot_reenter_visual_input_chain,
        test_excluded_source_keeps_story_summary_without_risk,
        test_real_time_pacing_compiles_story_beats_and_qa,
        test_real_time_pacing_reports_weak_or_conflicting_plan,
        test_invalid_pacing_idle_limit_is_rejected,
        test_multi_subject_and_multi_scene_are_risks_not_silent_deletions,
        test_final_multi_person_first_frame_is_not_misclassified_as_collage,
        test_unknown_task_fields_do_not_create_constraints,
        test_hard_constraints_and_identity_details_reach_prompt,
        test_performance_plan_compiles_source_grounded_dialogue,
        test_critical_dialogue_requires_observable_delivery_plan,
        test_prompt_soft_budget_warns_without_blocking,
        test_configured_provider_prompt_limit_is_enforced,
        test_nested_types_and_string_booleans_are_handled,
        test_report_is_json_serializable,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")
    print(f"{len(tests)} tests passed")
