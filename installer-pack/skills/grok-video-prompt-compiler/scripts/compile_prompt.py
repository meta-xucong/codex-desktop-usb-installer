#!/usr/bin/env python3
"""Compile a model-neutral SceneSpec into a Grok prompt and audit report."""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import sys
from copy import deepcopy
from pathlib import Path
from typing import Any


DEFAULT_SOFT_PROMPT_BYTES = 4096
RISK_LEVELS = ((75, "critical"), (50, "high"), (25, "medium"), (0, "low"))
COMPLEX_TRANSITIONS = {
    "full_body_cutout_fly_in",
    "sticker_merge",
    "clone_fusion",
    "m1",
    "m8",
    "m9",
    "完整人物飞入重合",
    "贴纸翻页",
}
CONTINUITY_MODES = {"off", "advisory", "strict"}
ANCHOR_PRIORITIES = {"critical", "important", "ambient"}
ANCHOR_VISIBILITY = {"visible", "optional", "offscreen_allowed"}
ANCHOR_CHANGE_POLICIES = {"fixed", "declared", "free"}
INPUT_ROUTES = {"direct_input", "text_summary", "excluded"}
INPUT_VERDICTS = {"approved", "rejected", "unreviewed"}
PACING_MODES = {"unspecified", "real_time", "stylized_slow", "fast"}


class SpecError(ValueError):
    pass


def _optional_bool(value: Any) -> bool | None:
    if value is None:
        return None
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        normalized = value.strip().lower()
        if normalized in {"true", "yes", "1", "on"}:
            return True
        if normalized in {"false", "no", "0", "off"}:
            return False
        if normalized in {"", "unknown", "null", "none"}:
            return None
        return None
    return bool(value)


def _text(value: Any, default: str = "") -> str:
    return default if value is None else str(value).strip()


def _same_ratio(source_ratio: str, target_ratio: str) -> bool:
    normalized_source = source_ratio.replace(" ", "").lower()
    normalized_target = target_ratio.replace(" ", "").lower()
    return normalized_source == normalized_target


def _risk_level(score: int) -> str:
    for minimum, label in RISK_LEVELS:
        if score >= minimum:
            return label
    return "low"


def _prompt_budget(provider: dict[str, Any] | None) -> dict[str, int | None]:
    """Return a quality budget and an optional host-evidenced transport limit."""
    provider = provider or {}
    soft_limit = provider.get("prompt_soft_budget_bytes", DEFAULT_SOFT_PROMPT_BYTES)
    hard_limit = provider.get("max_prompt_bytes")
    return {
        "soft_limit_bytes": soft_limit,
        "hard_limit_bytes": hard_limit,
    }


def _continuity_mode(continuity: dict[str, Any] | None) -> str:
    if not continuity:
        return "off"
    return _text(continuity.get("mode"), "off").lower()


def _continuity_anchors(continuity: dict[str, Any] | None) -> list[dict[str, Any]]:
    if not continuity:
        return []
    anchors = continuity.get("anchors", [])
    return anchors if isinstance(anchors, list) else []


def _performance_arcs(performance: dict[str, Any] | None) -> list[dict[str, Any]]:
    if not performance:
        return []
    arcs = performance.get("arcs", [])
    return arcs if isinstance(arcs, list) else []


def _dialogue_beats(performance: dict[str, Any] | None) -> list[dict[str, Any]]:
    if not performance:
        return []
    beats = performance.get("dialogue_beats", [])
    return beats if isinstance(beats, list) else []


def _continuity_inventory(continuity: dict[str, Any] | None) -> list[dict[str, Any]]:
    """Return task-declared entities whose count and location must be conserved."""
    if not continuity:
        return []
    inventory = continuity.get("inventory", [])
    return inventory if isinstance(inventory, list) else []


def _anchor_state(anchor: dict[str, Any], point: str) -> str:
    """Return the requested visible state at one end of a clip."""
    explicit = _text(anchor.get(f"{point}_state"))
    if explicit:
        return explicit
    if point == "end":
        return _text(anchor.get("start_state"))
    return ""


def _inventory_state(entity: dict[str, Any], point: str) -> str:
    """Return the requested location or ownership state at one end of a clip."""
    explicit = _text(entity.get(f"{point}_state"))
    if explicit:
        return explicit
    if point == "end":
        return _text(entity.get("start_state"))
    return ""


def _input_ledger(reference: dict[str, Any] | None) -> list[dict[str, Any]]:
    if not reference:
        return []
    ledger = reference.get("input_ledger", {})
    if not isinstance(ledger, dict):
        return []
    assets = ledger.get("assets", [])
    return assets if isinstance(assets, list) else []


def _mentions_anatomy(value: Any) -> bool:
    """Match anatomy as a word, so 'handbag' is not mistaken for a hand."""
    words = set(re.findall(r"[a-z]+", _text(value).lower()))
    return bool(words & {"hand", "forearm", "arm", "wrist", "foot", "leg", "finger", "fingers"})


def _limb_side_prefix(attachment: str) -> str:
    """Return 'actor.left' or 'actor.right' for normalized limb attachments."""
    match = re.match(r"^(.*\.(?:left|right))_(?:hand|forearm|arm|wrist|foot|leg)$", attachment)
    return match.group(1) if match else ""


def _pacing_clauses(pacing: dict[str, Any] | None) -> list[str]:
    if not pacing:
        return []
    mode = _text(pacing.get("mode"), "unspecified").lower()
    if mode == "real_time":
        clauses = ["Use normal real-time motion; no slow motion, time stretching, frozen waiting, or purposeless lingering."]
        max_idle_seconds = pacing.get("max_idle_seconds")
        if isinstance(max_idle_seconds, (int, float)) and not isinstance(max_idle_seconds, bool):
            clauses.append(f"Do not hold a visually unchanged pose or wait for longer than approximately {max_idle_seconds:g} seconds.")
        beats = pacing.get("beats", []) or []
        beat_text = " ".join(
            f"At approximately {beat['time_seconds']}s, show {beat['event']}."
            for beat in beats
        )
        if beat_text:
            clauses.append("Required visible story beats: " + beat_text)
        turning_point = _text(pacing.get("turning_point"))
        if turning_point:
            clauses.append(f"Story turning point: {turning_point}.")
        return clauses
    if mode == "stylized_slow":
        return ["Use the requested stylized slow pacing; do not interpret it as ordinary real-time action."]
    if mode == "fast":
        return ["Use brisk, natural real-time motion with no slow motion or dead time."]
    return []


def validate_spec(spec: dict[str, Any]) -> None:
    if not isinstance(spec, dict):
        raise SpecError("SceneSpec must be a JSON object")
    if spec.get("version", 1) != 1:
        raise SpecError("Only SceneSpec version 1 is supported")
    duration = spec.get("duration_seconds")
    if (
        not isinstance(duration, (int, float))
        or isinstance(duration, bool)
        or not math.isfinite(float(duration))
        or not 1 <= duration <= 60
    ):
        raise SpecError("duration_seconds must be a number between 1 and 60")
    if spec.get("ratio") not in {"9:16", "16:9", "1:1"}:
        raise SpecError("ratio must be one of 9:16, 16:9, 1:1")
    if _text(spec.get("resolution")) not in {"480p", "720p", "1080p"}:
        raise SpecError("resolution must be 480p, 720p, or 1080p")
    for field in ("actor_count", "scene_count"):
        if field not in spec or spec[field] is None:
            continue
        value = spec[field]
        if isinstance(value, str) and value.strip().lower() in {"", "unknown", "null", "none"}:
            continue
        if not isinstance(value, int) or isinstance(value, bool) or value < 1:
            raise SpecError(f"{field} must be a positive integer")
    for field in ("reference", "camera", "identity", "transition", "audio", "strategy_overrides", "continuity", "pacing", "narrative", "performance", "provider"):
        value = spec.get(field, {})
        if value is not None and not isinstance(value, dict):
            raise SpecError(f"{field} must be an object")
    for field in ("hard_constraints", "negative_constraints"):
        value = spec.get(field, [])
        if value is not None and not isinstance(value, list):
            raise SpecError(f"{field} must be an array")
    changes = spec.get("outfit_changes", [])
    if not isinstance(changes, list):
        raise SpecError("outfit_changes must be an array")
    times: list[float] = []
    for index, change in enumerate(changes):
        if not isinstance(change, dict):
            raise SpecError(f"outfit_changes[{index}] must be an object")
        value = change.get("time_seconds")
        if (
            not isinstance(value, (int, float))
            or isinstance(value, bool)
            or not math.isfinite(float(value))
            or not 0 < value < duration
        ):
            raise SpecError(f"outfit_changes[{index}].time_seconds must be inside the video")
        times.append(float(value))
        if not _text(change.get("outfit")):
            raise SpecError(f"outfit_changes[{index}].outfit is required")
    if times != sorted(times) or len(times) != len(set(times)):
        raise SpecError("outfit change times must be strictly increasing")

    provider = spec.get("provider", {}) or {}
    for field in ("prompt_soft_budget_bytes", "max_prompt_bytes"):
        value = provider.get(field)
        if value is not None and (
            not isinstance(value, int)
            or isinstance(value, bool)
            or value < 1
        ):
            raise SpecError(f"provider.{field} must be a positive integer when specified")
    if (
        provider.get("prompt_soft_budget_bytes") is not None
        and provider.get("max_prompt_bytes") is not None
        and provider["prompt_soft_budget_bytes"] > provider["max_prompt_bytes"]
    ):
        raise SpecError("provider.prompt_soft_budget_bytes must not exceed provider.max_prompt_bytes")

    pacing = spec.get("pacing", {}) or {}
    pacing_mode = _text(pacing.get("mode"), "unspecified").lower()
    if pacing_mode not in PACING_MODES:
        raise SpecError("pacing.mode must be unspecified, real_time, stylized_slow, or fast")
    beats = pacing.get("beats", [])
    if "beats" in pacing and not isinstance(beats, list):
        raise SpecError("pacing.beats must be an array")
    beat_times: list[float] = []
    for index, beat in enumerate(beats):
        if not isinstance(beat, dict):
            raise SpecError(f"pacing.beats[{index}] must be an object")
        time_seconds = beat.get("time_seconds")
        if (
            not isinstance(time_seconds, (int, float))
            or isinstance(time_seconds, bool)
            or not math.isfinite(float(time_seconds))
            or not 0 <= time_seconds <= duration
        ):
            raise SpecError(f"pacing.beats[{index}].time_seconds must be between 0 and duration")
        if not _text(beat.get("event")):
            raise SpecError(f"pacing.beats[{index}].event is required")
        beat_times.append(float(time_seconds))
    if beat_times != sorted(beat_times) or len(beat_times) != len(set(beat_times)):
        raise SpecError("pacing beat times must be strictly increasing")
    if pacing_mode == "real_time" and len(beats) == 0:
        raise SpecError("pacing.beats is required when pacing.mode is real_time")
    max_idle_seconds = pacing.get("max_idle_seconds")
    if max_idle_seconds is not None and (
        not isinstance(max_idle_seconds, (int, float))
        or isinstance(max_idle_seconds, bool)
        or not math.isfinite(float(max_idle_seconds))
        or not 0 < float(max_idle_seconds) < duration
    ):
        raise SpecError("pacing.max_idle_seconds must be a positive number shorter than duration")

    narrative = spec.get("narrative", {}) or {}
    required_events = narrative.get("required_events", [])
    if "required_events" in narrative and not isinstance(required_events, list):
        raise SpecError("narrative.required_events must be an array")
    for index, event in enumerate(required_events):
        if not _text(event):
            raise SpecError(f"narrative.required_events[{index}] must be non-empty text")

    performance = spec.get("performance", {}) or {}
    arcs = _performance_arcs(performance)
    dialogue_beats = _dialogue_beats(performance)
    for field in ("arcs", "dialogue_beats"):
        if field in performance and not isinstance(performance.get(field), list):
            raise SpecError(f"performance.{field} must be an array")
    arc_ids: set[str] = set()
    for index, arc in enumerate(arcs):
        if not isinstance(arc, dict):
            raise SpecError(f"performance.arcs[{index}] must be an object")
        arc_id = _text(arc.get("id"))
        if not arc_id:
            raise SpecError(f"performance.arcs[{index}].id is required")
        if arc_id in arc_ids:
            raise SpecError(f"performance.arcs[{index}].id must be unique")
        arc_ids.add(arc_id)
        if not _text(arc.get("subject")):
            raise SpecError(f"performance.arcs[{index}].subject is required")
        priority = _text(arc.get("priority"), "important").lower()
        if priority not in ANCHOR_PRIORITIES:
            raise SpecError(f"performance.arcs[{index}].priority must be critical, important, or ambient")
        for field in ("source_evidence", "start_state", "end_state"):
            if not _text(arc.get(field)):
                raise SpecError(f"performance.arcs[{index}].{field} is required")
        if "forbidden_readings" in arc and not isinstance(arc.get("forbidden_readings"), list):
            raise SpecError(f"performance.arcs[{index}].forbidden_readings must be an array")
    dialogue_ids: set[str] = set()
    for index, beat in enumerate(dialogue_beats):
        if not isinstance(beat, dict):
            raise SpecError(f"performance.dialogue_beats[{index}] must be an object")
        beat_id = _text(beat.get("id"))
        if not beat_id:
            raise SpecError(f"performance.dialogue_beats[{index}].id is required")
        if beat_id in dialogue_ids:
            raise SpecError(f"performance.dialogue_beats[{index}].id must be unique")
        dialogue_ids.add(beat_id)
        for field in ("speaker", "line", "source_evidence"):
            if not _text(beat.get(field)):
                raise SpecError(f"performance.dialogue_beats[{index}].{field} is required")
        priority = _text(beat.get("priority"), "important").lower()
        if priority not in ANCHOR_PRIORITIES:
            raise SpecError(f"performance.dialogue_beats[{index}].priority must be critical, important, or ambient")
        for field in ("facial_cues", "forbidden_readings"):
            if field in beat and not isinstance(beat.get(field), list):
                raise SpecError(f"performance.dialogue_beats[{index}].{field} must be an array")
        if priority == "critical":
            for field in ("precondition", "subtext", "vocal_delivery"):
                if not _text(beat.get(field)):
                    raise SpecError(f"performance.dialogue_beats[{index}].{field} is required for critical dialogue")
            if not [cue for cue in beat.get("facial_cues", []) if _text(cue)]:
                raise SpecError(f"performance.dialogue_beats[{index}].facial_cues is required for critical dialogue")

    continuity = spec.get("continuity", {}) or {}
    mode = _continuity_mode(continuity)
    if mode not in CONTINUITY_MODES:
        raise SpecError("continuity.mode must be off, advisory, or strict")
    anchors = _continuity_anchors(continuity)
    if "anchors" in continuity and not isinstance(continuity.get("anchors"), list):
        raise SpecError("continuity.anchors must be an array")
    anchor_ids: set[str] = set()
    for index, anchor in enumerate(anchors):
        if not isinstance(anchor, dict):
            raise SpecError(f"continuity.anchors[{index}] must be an object")
        anchor_id = _text(anchor.get("id"))
        if not anchor_id:
            raise SpecError(f"continuity.anchors[{index}].id is required")
        if anchor_id in anchor_ids:
            raise SpecError(f"continuity.anchors[{index}].id must be unique")
        anchor_ids.add(anchor_id)
        if not _text(anchor.get("subject")):
            raise SpecError(f"continuity.anchors[{index}].subject is required")
        priority = _text(anchor.get("priority"), "important").lower()
        if priority not in ANCHOR_PRIORITIES:
            raise SpecError(f"continuity.anchors[{index}].priority must be critical, important, or ambient")
        visibility = _text(anchor.get("visibility"), "visible").lower()
        if visibility not in ANCHOR_VISIBILITY:
            raise SpecError(f"continuity.anchors[{index}].visibility must be visible, optional, or offscreen_allowed")
        raw_share_attachment = anchor.get("share_attachment")
        if raw_share_attachment is not None and not isinstance(raw_share_attachment, bool):
            raise SpecError(f"continuity.anchors[{index}].share_attachment must be a boolean when specified")
        change_policy = _text(anchor.get("change_policy"), "fixed").lower()
        if change_policy not in ANCHOR_CHANGE_POLICIES:
            raise SpecError(f"continuity.anchors[{index}].change_policy must be fixed, declared, or free")
        start_state = _anchor_state(anchor, "start")
        end_state = _anchor_state(anchor, "end")
        if mode != "off" and not start_state:
            raise SpecError(f"continuity.anchors[{index}].start_state is required when continuity is enabled")
        if change_policy == "fixed" and end_state and start_state and end_state != start_state:
            raise SpecError(f"continuity.anchors[{index}] has fixed change_policy but different start/end states")
        if change_policy == "declared" and end_state and start_state and end_state != start_state and not _text(anchor.get("change")):
            raise SpecError(f"continuity.anchors[{index}].change is required for a declared state change")

    inventory = _continuity_inventory(continuity)
    if "inventory" in continuity and not isinstance(continuity.get("inventory"), list):
        raise SpecError("continuity.inventory must be an array")
    inventory_ids: set[str] = set()
    for index, entity in enumerate(inventory):
        if not isinstance(entity, dict):
            raise SpecError(f"continuity.inventory[{index}] must be an object")
        entity_id = _text(entity.get("id"))
        if not entity_id:
            raise SpecError(f"continuity.inventory[{index}].id is required")
        if entity_id in inventory_ids:
            raise SpecError(f"continuity.inventory[{index}].id must be unique")
        if entity_id in anchor_ids:
            raise SpecError(f"continuity.inventory[{index}].id must not reuse a continuity.anchors id")
        inventory_ids.add(entity_id)
        if not _text(entity.get("subject")):
            raise SpecError(f"continuity.inventory[{index}].subject is required")
        quantity = entity.get("quantity")
        if not isinstance(quantity, int) or isinstance(quantity, bool) or quantity < 1:
            raise SpecError(f"continuity.inventory[{index}].quantity must be a positive integer")
        priority = _text(entity.get("priority"), "important").lower()
        if priority not in ANCHOR_PRIORITIES:
            raise SpecError(f"continuity.inventory[{index}].priority must be critical, important, or ambient")
        start_state = _inventory_state(entity, "start")
        end_state = _inventory_state(entity, "end")
        if mode != "off" and not start_state:
            raise SpecError(f"continuity.inventory[{index}].start_state is required when continuity is enabled")
        transfers = entity.get("transfers", [])
        if "transfers" in entity and not isinstance(transfers, list):
            raise SpecError(f"continuity.inventory[{index}].transfers must be an array")
        if mode != "off" and start_state and end_state and start_state != end_state and not transfers:
            raise SpecError(f"continuity.inventory[{index}].transfers is required when start_state and end_state differ")
        for transfer_index, transfer in enumerate(transfers):
            if not isinstance(transfer, dict):
                raise SpecError(f"continuity.inventory[{index}].transfers[{transfer_index}] must be an object")
            if not _text(transfer.get("from")):
                raise SpecError(f"continuity.inventory[{index}].transfers[{transfer_index}].from is required")
            if not _text(transfer.get("to")):
                raise SpecError(f"continuity.inventory[{index}].transfers[{transfer_index}].to is required")
            if "action" in transfer and not _text(transfer.get("action")):
                raise SpecError(f"continuity.inventory[{index}].transfers[{transfer_index}].action must be non-empty text when specified")
        raw_exclusive_location = entity.get("exclusive_location")
        if raw_exclusive_location is not None and not isinstance(raw_exclusive_location, bool):
            raise SpecError(f"continuity.inventory[{index}].exclusive_location must be a boolean when specified")
        forbidden_states = entity.get("forbidden_states", [])
        if "forbidden_states" in entity and not isinstance(forbidden_states, list):
            raise SpecError(f"continuity.inventory[{index}].forbidden_states must be an array")
        for state_index, state in enumerate(forbidden_states):
            if not _text(state):
                raise SpecError(f"continuity.inventory[{index}].forbidden_states[{state_index}] must be non-empty text")
    audio = spec.get("audio", {})
    if not isinstance(audio, dict):
        raise SpecError("audio must be an object")
    bpm = audio.get("bpm")
    if bpm is not None and (
        not isinstance(bpm, (int, float))
        or isinstance(bpm, bool)
        or not math.isfinite(float(bpm))
        or not 30 <= bpm <= 300
    ):
        raise SpecError("audio.bpm must be between 30 and 300")
    reference = spec.get("reference", {}) or {}
    ledger = reference.get("input_ledger", {}) or {}
    if "input_ledger" in reference and not isinstance(ledger, dict):
        raise SpecError("reference.input_ledger must be an object")
    if ledger:
        assets = _input_ledger(reference)
        if "assets" in ledger and not isinstance(ledger.get("assets"), list):
            raise SpecError("reference.input_ledger.assets must be an array")
        asset_ids: set[str] = set()
        for index, asset in enumerate(assets):
            if not isinstance(asset, dict):
                raise SpecError(f"reference.input_ledger.assets[{index}] must be an object")
            asset_id = _text(asset.get("id"))
            if not asset_id:
                raise SpecError(f"reference.input_ledger.assets[{index}].id is required")
            if asset_id in asset_ids:
                raise SpecError(f"reference.input_ledger.assets[{index}].id must be unique")
            asset_ids.add(asset_id)
            route = _text(asset.get("route"), "direct_input").lower()
            if route not in INPUT_ROUTES:
                raise SpecError(f"reference.input_ledger.assets[{index}].route must be direct_input, text_summary, or excluded")
            verdict = _text(asset.get("verdict"), "unreviewed").lower()
            if verdict not in INPUT_VERDICTS:
                raise SpecError(f"reference.input_ledger.assets[{index}].verdict must be approved, rejected, or unreviewed")
            if route == "direct_input" and not _text(asset.get("role")):
                raise SpecError(f"reference.input_ledger.assets[{index}].role is required for direct_input")
    reference_usage = _text(reference.get("usage"), "unknown").lower()
    if reference_usage == "keyframe":
        keyframe_time = reference.get("keyframe_time_seconds")
        if (
            not isinstance(keyframe_time, (int, float))
            or isinstance(keyframe_time, bool)
            or not math.isfinite(float(keyframe_time))
            or not 0 <= keyframe_time <= duration
        ):
            raise SpecError("reference.keyframe_time_seconds must be inside the video")


def audit_and_downgrade(spec: dict[str, Any]) -> dict[str, Any]:
    validate_spec(spec)
    reference = spec.get("reference", {}) or {}
    camera = spec.get("camera", {}) or {}
    identity = spec.get("identity", {}) or {}
    transition = spec.get("transition", {}) or {}
    audio = spec.get("audio", {}) or {}
    changes = spec.get("outfit_changes", [])
    overrides = spec.get("strategy_overrides", {}) or {}
    continuity = spec.get("continuity", {}) or {}
    continuity_mode = _continuity_mode(continuity)
    anchors = _continuity_anchors(continuity)
    inventory = _continuity_inventory(continuity)
    pacing = spec.get("pacing", {}) or {}
    pacing_mode = _text(pacing.get("mode"), "unspecified").lower()
    pacing_beats = pacing.get("beats", []) if isinstance(pacing.get("beats", []), list) else []
    narrative = spec.get("narrative", {}) or {}
    required_events = narrative.get("required_events", []) if isinstance(narrative.get("required_events", []), list) else []
    performance = spec.get("performance", {}) or {}
    performance_arcs = _performance_arcs(performance)
    dialogue_beats = _dialogue_beats(performance)
    score = 0
    findings: list[str] = []
    blocking_findings: list[str] = []
    downgrades: list[dict[str, str]] = []
    recommendations: list[str] = []
    effective = deepcopy(spec)
    for field in ("reference", "camera", "identity", "transition", "audio"):
        if not isinstance(effective.get(field), dict):
            effective[field] = {}
    effective["reference"]["strategy"] = "preserve_requested"
    effective["transition"]["strategy"] = "preserve_requested"
    effective["audio"]["strategy"] = "preserve_requested"
    effective["scene_strategy"] = "preserve_requested"
    if _text(overrides.get("reference")):
        effective["reference"]["strategy"] = _text(overrides.get("reference"))
    if _text(overrides.get("transition")):
        effective["transition"]["strategy"] = _text(overrides.get("transition"))
    if _text(overrides.get("audio")):
        effective["audio"]["strategy"] = _text(overrides.get("audio"))
    if _text(overrides.get("scene")):
        effective["scene_strategy"] = _text(overrides.get("scene"))

    if continuity_mode != "off":
        occupied_slots: dict[str, list[dict[str, Any]]] = {}
        for anchor in anchors:
            attachment = _text(anchor.get("attachment"))
            visibility = _text(anchor.get("visibility"), "visible").lower()
            if attachment and visibility == "visible":
                occupied_slots.setdefault(attachment.lower(), []).append(anchor)
            start_state = _anchor_state(anchor, "start")
            end_state = _anchor_state(anchor, "end")
            change_policy = _text(anchor.get("change_policy"), "fixed").lower()
            if change_policy == "declared" and start_state != end_state:
                recommendations.append(
                    f"为连续性锚点 {anchor['id']} 在首帧和结尾帧核对已声明变化：{start_state} -> {end_state}。"
                )
            if _text(anchor.get("handoff_from")) and _text(anchor.get("handoff_from")) != start_state:
                score += 20
                findings.append(
                    f"连续性锚点 {anchor['id']} 的上一镜交接状态与本镜起始状态不一致。"
                )
                downgrades.append({
                    "requested": f"延续 {anchor['id']} 的上一镜状态",
                    "effective": "先修正目标首帧或显式声明一次状态变化，再提交 I2V",
                })
        for attachment, attached in occupied_slots.items():
            carried_objects = [item for item in attached if not _mentions_anatomy(item.get("subject"))]
            if len(carried_objects) > 1 and not all(_optional_bool(item.get("share_attachment")) is True for item in carried_objects):
                critical = [item for item in carried_objects if _text(item.get("priority"), "important").lower() == "critical"]
                score += 20 if critical else 10
                labels = "、".join(_text(item.get("id")) for item in carried_objects)
                findings.append(f"连续性锚点 {labels} 同时要求占用 {attachment}，可能造成持有关系重排或道具丢失。")
                recommendations.append("将同一肢体上的道具改为单一承载关系，或在首帧中给它们明确、可见且不冲突的接触点。")
        for anchor in anchors:
            attachment = _text(anchor.get("attachment")).lower()
            subject = _text(anchor.get("subject")).lower()
            priority = _text(anchor.get("priority"), "important").lower()
            if priority != "critical" or not attachment or not any(token in attachment for token in ("hand", "forearm", "arm", "foot", "leg")):
                continue
            is_anatomy_anchor = _mentions_anatomy(subject)
            if is_anatomy_anchor:
                continue
            side_prefix = _limb_side_prefix(attachment)
            if not side_prefix:
                continue
            supporting_limb = any(
                other is not anchor
                and (
                    _text(other.get("attachment")).lower() == side_prefix
                    or _text(other.get("attachment")).lower().startswith(side_prefix + "_")
                )
                for other in anchors
                if _text(other.get("visibility"), "visible").lower() == "visible"
                and _mentions_anatomy(other.get("subject"))
            )
            if not supporting_limb:
                score += 10
                findings.append(f"关键锚点 {anchor['id']} 依赖 {attachment}，但没有单独的可见人体锚点；道具可能保留而手脚被遮挡或缺失。")
                recommendations.append("若该承载肢体必须完整可见，增加独立人体锚点并在首帧验收手掌、手指或脚部；若允许遮挡，将该关系降为 optional。")
        if len(anchors) >= 5:
            score += 10
            findings.append("本镜要求维持五个以上连续性锚点，短视频模型可能在动作中重排次要关系。")
            recommendations.append("保留复杂画面，但将本镜的 critical 锚点限制为叙事真正不可丢失的关系，其余设为 important 或 ambient。")
        for entity in inventory:
            entity_id = _text(entity.get("id"))
            start_state = _inventory_state(entity, "start")
            end_state = _inventory_state(entity, "end")
            transfers = entity.get("transfers", [])
            priority = _text(entity.get("priority"), "important").lower()
            if _text(entity.get("handoff_from")) and _text(entity.get("handoff_from")) != start_state:
                score += 20
                findings.append(f"守恒实体 {entity_id} 的上一镜交接状态与本镜起始状态不一致。")
                downgrades.append({
                    "requested": f"延续守恒实体 {entity_id} 的上一镜状态",
                    "effective": "先修正目标首帧或显式声明一次道具转移，再提交视频任务",
                })
            if start_state != end_state:
                recommendations.append(
                    f"为守恒实体 {entity_id} 在首帧、每次转移和结尾核对同一实例：{start_state} -> {end_state}。"
                )
            if priority == "critical" and transfers:
                score += 10
                findings.append(f"关键守恒实体 {entity_id} 发生位置或持有者转移；短视频模型仍可能出现复制、双持或旧位置残留。")
                recommendations.append("将该实体的数量、唯一位置和转移后旧位置清空写入 Prompt，并在转移点逐帧验收；失败时优先拆成交接前后两镜。")
        if len(inventory) >= 4:
            score += 10
            findings.append("本镜同时要求守恒四个以上可移动实体，模型可能优先保留人物动作而重排次要道具。")
            recommendations.append("保留必要道具，但仅把叙事关键的实体列入 inventory；环境陈设仍用普通锚点或场景描述。")
        if continuity_mode == "strict":
            recommendations.append("严格连续性模式要求在首帧、叙事转折和结尾逐项验收锚点；失败时重做应先改首帧状态，而非只加长 Prompt。")

    if pacing_mode == "real_time":
        if len(pacing_beats) < 2:
            score += 15
            findings.append("实时叙事片段少于两个可见节拍，可能退化为无事件的等待或慢动作过场。")
            recommendations.append("为实时叙事至少定义起始动作和一个中段/结尾事件，并让每个事件改变人物、道具、关系或场景状态。")
        if _optional_bool(pacing.get("allow_slow_motion")) is True:
            score += 15
            findings.append("本镜同时要求实时节奏和允许慢动作，速度语义存在冲突。")
            recommendations.append("明确选择正常速度或慢动作；实时剧情默认关闭慢动作。")
        if "turning_point" in pacing and not _text(pacing.get("turning_point")):
            score += 10
            findings.append("实时叙事声明了转折字段，但没有提供有效转折事件。")
        if narrative and not required_events:
            score += 10
            findings.append("实时剧情片声明了叙事信息但没有指定本镜必须完成的剧情事件，节拍可能退化为无关动作。")
            recommendations.append("为本镜列出一到三个不可替代的剧情事件，并把每个事件映射到可见节拍和首帧状态。")
        if pacing.get("max_idle_seconds") is None:
            recommendations.append("若本镜不能接受长时间摆姿或等待，可按本次叙事节奏设置 pacing.max_idle_seconds，并在生成后按该间隔验收。")
        recommendations.append("按 beat 时间点抽查动作是否以正常速度发生，不能用静态站立或缓慢摆姿替代剧情节拍。")
    elif pacing_mode == "stylized_slow":
        recommendations.append("本镜明确是风格化慢节奏；不要把慢动作结果误判为实时剧情失败。")

    active_arcs = [
        arc for arc in performance_arcs
        if _text(arc.get("priority"), "important").lower() in {"critical", "important"}
    ]
    active_dialogue_beats = [
        beat for beat in dialogue_beats
        if _text(beat.get("priority"), "important").lower() in {"critical", "important"}
    ]
    if active_arcs or active_dialogue_beats:
        recommendations.append("按表演弧和对白节拍验收可见表情、声线和对方反应；不要只因台词字面正确就判定情绪交付成功。")
    for arc in active_arcs:
        if not [_text(item) for item in arc.get("forbidden_readings", []) if _text(item)]:
            recommendations.append(f"为表演弧 {arc['id']} 增加一到三个禁止误读，避免模型把相近情绪演成通用表情。")
    for beat in active_dialogue_beats:
        if not [_text(item) for item in beat.get("forbidden_readings", []) if _text(item)]:
            recommendations.append(f"为对白节拍 {beat['id']} 增加禁止误读，尤其排除与原文相反的情绪。")
        if isinstance(spec.get("actor_count"), int) and spec["actor_count"] > 1 and not _text(beat.get("listener_response")):
            recommendations.append(f"若对白节拍 {beat['id']} 有听者，补充其可见回应，避免场面变成单人朗读。")
    critical_dialogue_count = sum(
        1 for beat in active_dialogue_beats
        if _text(beat.get("priority"), "important").lower() == "critical"
    )
    dialogue_capacity = max(1, int(float(spec["duration_seconds"]) // 4))
    if critical_dialogue_count > dialogue_capacity:
        score += 10
        findings.append("关键对白节拍相对于本镜时长过密，模型可能保留字面台词却牺牲情绪、口型或对方反应。")
        recommendations.append("保留剧情，但把每句关键对白安排在独立近景窗口，或拆成多个片段；不要只压缩动作来迁就台词。")

    reference_kind = _text(reference.get("kind"), "unknown").lower()
    reference_usage = _text(reference.get("usage"), "unknown").lower()
    has_stickers = _optional_bool(reference.get("has_stickers")) is True
    visual_reference = reference_usage in {"i2v_first_frame", "keyframe"}
    composite_reference = reference_kind == "collage" or has_stickers
    for asset in _input_ledger(reference):
        route = _text(asset.get("route"), "direct_input").lower()
        verdict = _text(asset.get("verdict"), "unreviewed").lower()
        asset_id = _text(asset.get("id"))
        if route == "direct_input" and verdict == "rejected":
            score += 35
            finding = f"参考素材 {asset_id} 已被验收拒绝，却仍计划直接参与图像或视频输入；它会把已知的结构错误带入下一镜。"
            findings.append(finding)
            blocking_findings.append(finding)
            downgrades.append({
                "requested": f"继续直接使用已拒绝素材 {asset_id}",
                "effective": "从所有多图编辑和 I2V 输入中排除该素材；只保留经验证的文字身份摘要或重新提供已验收素材",
            })
        elif route == "direct_input" and verdict == "unreviewed":
            score += 10
            findings.append(f"参考素材 {asset_id} 会直接参与生成，但尚未按本镜关键锚点验收。")
            recommendations.append("生成前先按关键锚点验收该参考素材；验收失败时将它改为 excluded 或仅保留文字摘要。")
    if reference and reference_usage == "unknown":
        score += 5
        findings.append("本次任务提供了参考图，但没有明确它是首帧、身份参考还是关键帧。")
        recommendations.append("调用视频接口前明确参考图角色；不要让接口默认决定它出现在首帧、关键帧还是仅用于身份引导。")
    identity_reference_usage = {"identity_only", "identity_reference", "character_reference"}
    role_channel = _optional_bool(reference.get("role_aware_channel"))
    role_channel_unknown = role_channel is None
    if reference_usage in identity_reference_usage and role_channel is False:
        score += 25
        findings.append("本次任务要求身份参考，但当前调用链没有确认的角色分离通道；原始肖像可能成为首帧或强视觉锚点。")
        downgrades.append({
            "requested": "只使用原图作为人物身份参考，不让原图出现在视频中",
            "effective": "原图只用于提取人物特征；按本次主体、场景和镜头要求准备目标画面输入",
        })
        recommendations.append("本次调用不要把原始身份肖像作为普通 I2V 图片传入；如无独立身份通道，先准备任务所需的画面输入。")
    elif reference_usage in identity_reference_usage and role_channel_unknown:
        recommendations.append("调用前确认本次接口是否支持独立身份参考；未确认前不要把身份图当成首帧或关键帧使用。")
    elif reference_usage == "keyframe" and role_channel is False:
        score += 25
        findings.append("本次任务要求关键帧输入，但当前调用链没有确认的时间定位通道；图片可能只会被当作首帧或普通视觉参考。")
        downgrades.append({
            "requested": "在指定时间使用参考图作为关键帧",
            "effective": "确认接口支持时间关键帧；否则拆成片段或把图片改作本次片段的起始画面",
        })
        recommendations.append("不要把关键帧时间位置只写进 Prompt；调用参数必须明确支持该时间位置。")
    elif reference_usage == "keyframe" and role_channel_unknown:
        recommendations.append("调用前确认本次接口是否支持指定时间关键帧；未确认前不要把关键帧当作首帧使用。")
    if composite_reference and (visual_reference or reference_usage == "unknown"):
        score += 30
        findings.append("本次画面输入含拼贴、贴纸或多人物元素，Grok 可能把它们解释为场景中的真实主体。")
        downgrades.append({
            "requested": "直接使用拼贴/贴纸参考图",
            "effective": "保留原画面并加强主体/层级 Prompt，或按本次主体、场景和镜头要求准备目标画面输入",
        })
        recommendations.append("确认拼贴/贴纸是目标画面还是仅为参考；只有前者才直接作为画面输入。")

    if _optional_bool(reference.get("has_transparency")) is True and (visual_reference or reference_usage == "unknown"):
        score += 10
        findings.append("透明参考图的底层 RGB 可能仍携带被移除的旧画面。")
        downgrades.append({
            "requested": "透明 PNG 作为 I2V 参考",
            "effective": "按本次画面用途决定是否压平为不透明图后再作为参考",
        })
        recommendations.append("上传前核验公网下载的图像像素，确认透明区域没有旧拼贴或贴纸底图。")

    scene_matches_final = _optional_bool(reference.get("scene_matches_final"))
    if scene_matches_final is False and (visual_reference or reference_usage == "unknown"):
        score += 15
        findings.append("本次画面输入的参考图背景不是最终视频场景，模型可能在开头改变背景或构图。")
        downgrades.append({
            "requested": "使用非最终场景参考图作为画面输入",
            "effective": "先合成符合本次镜头的最终场景输入，再做 I2V",
        })
        recommendations.append("本次画面输入应同时匹配目标人物、目标场景、目标机位和目标时间位置。")

    source_ratio = _text(reference.get("source_ratio"), "").lower()
    target_ratio = _text(spec["ratio"]).lower()
    if visual_reference and source_ratio and source_ratio != "unknown" and not _same_ratio(source_ratio, target_ratio):
        score += 10
        findings.append(f"源图比例 {source_ratio} 与目标视频比例 {target_ratio} 不一致，直接适配可能造成画面拉伸或错误裁切。")
        downgrades.append({
            "requested": f"直接用 {source_ratio} 源图生成 {target_ratio} 视频",
            "effective": "按本次镜头需要先按目标比例重新构图，等比裁切或补背景后再作为画面输入",
        })
        recommendations.append("禁止非等比缩放；在目标比例画布中保留自然人体比例，并为头顶、双手和双脚留边。")

    framing = _text(reference.get("framing"), "unknown").lower()
    if visual_reference and _optional_bool(camera.get("full_body")) is True and framing in {"close_up", "half_body", "portrait"}:
        score += 10
        findings.append("源图是近景或半身构图，直接要求全身视频会迫使模型补全腿部和服装，增加比例漂移。")
        downgrades.append({
            "requested": "用近景/半身画面输入生成全身视频",
            "effective": "先准备符合本次机位的完整全身画面输入，再做 I2V",
        })
        recommendations.append("只有本次镜头要求全身时，才准备完整全身画面输入。")

    requested_transition = _text(transition.get("requested")).lower()
    if requested_transition in COMPLEX_TRANSITIONS:
        score += 25
        findings.append("本次请求包含完整人物飞入、贴纸融合或分身机制，可能出现主体重影、残留或时序不稳定。")
        downgrades.append({
            "requested": _text(transition.get("requested")),
            "effective": "候选方案：拆镜头、遮罩合成，或改为原地变化；是否采用由本次任务决定",
        })
    changes_count = len(changes)
    if changes_count >= 3:
        score += 15
        findings.append("连续三次以上换装会增加人物身份、动作和时间轴漂移。")
        recommendations.append("保留服装顺序，但把时间点视为近似；生成后抽取每个节点前后帧。")
    if changes_count >= 4:
        score += 10
        findings.append("四次以上换装不适合一次 Grok 任务严格完成。")
        recommendations.append("生产级成片建议拆成片段或后期用遮罩/剪辑锁定换装时刻。")
    if _optional_bool(audio.get("beat_sync")) is True and _optional_bool(audio.get("generate")) is True:
        score += 15
        findings.append("生成音乐与画面重拍的帧级同步不能只靠 Prompt 保证。")
        recommendations.append("生成后分析音轨或直接使用外部 120 BPM 音轨，再做后期卡点。")
    actor_count = spec.get("actor_count")
    if isinstance(actor_count, int) and actor_count > 1:
        score += 15
        findings.append("本次任务包含多个主体，连续身份、视线和互动关系的稳定性风险上升。")
        downgrades.append({
            "requested": "多个主体在一个连续视频中稳定互动",
            "effective": "候选方案：拆成镜头段落或减少同一时间出现的主体；保留原剧情需由本次任务决定",
        })
        recommendations.append("若互动关系是剧情核心，优先按镜头拆分并为每个主体设定明确位置和动作。")
    scene_count = spec.get("scene_count")
    if isinstance(scene_count, int) and scene_count > 1:
        score += 15
        findings.append("多场景连续性不适合单个 Grok I2V 任务。")
        downgrades.append({
            "requested": "单个任务内连续生成多个场景",
            "effective": "候选方案：拆成单场景片段后再剪辑；是否拆分由本次任务决定",
        })
        recommendations.append("按场景拆分生成，再通过剪辑统一色调和音乐。")
    if _text(spec.get("dance_complexity")).lower() == "high":
        score += 10
        findings.append("复杂连续舞蹈通常只能保持动作意图，不能保证逐拍复现。")

    if all(_optional_bool(identity.get(field)) is True for field in ("face_locked", "hair_locked", "body_locked")):
        recommendations.append("把脸、发型和身材比例锁定字段写入首帧与 Prompt，并在关键帧逐项核对。")

    if not changes:
        recommendations.append("无换装任务可直接按本次主体、场景和镜头要求编译，生成后核对实际时长和画面连续性。")
    return {
        "risk": {"score": score, "level": _risk_level(score), "findings": findings},
        "blocking_findings": blocking_findings,
        "effective_spec": effective,
        "downgrades": downgrades,
        "recommendations": recommendations,
    }


def _timeline(changes: list[dict[str, Any]], transition_strategy: str) -> str:
    if not changes:
        return ""
    parts = []
    for change in changes:
        action = _text(change.get("action"))
        suffix = f" during {action}" if action else ""
        location = " in place" if transition_strategy == "in_place_beat_change" else ""
        parts.append(f"At approximately {change['time_seconds']}s, change{location} to {change['outfit']}{suffix}.")
    return " ".join(parts)


def _continuity_clauses(continuity: dict[str, Any] | None) -> list[str]:
    """Compile only task-declared, high-value continuity relations into the prompt."""
    mode = _continuity_mode(continuity)
    if mode == "off":
        return []
    clauses: list[str] = []
    for anchor in _continuity_anchors(continuity):
        priority = _text(anchor.get("priority"), "important").lower()
        if priority not in {"critical", "important"}:
            continue
        subject = _text(anchor.get("subject"))
        start_state = _anchor_state(anchor, "start")
        end_state = _anchor_state(anchor, "end")
        change_policy = _text(anchor.get("change_policy"), "fixed").lower()
        if change_policy == "declared" and start_state != end_state:
            change = _text(anchor.get("change"))
            clauses.append(
                f"Continuity for {subject}: start {start_state}; by the end {end_state}; only this declared change is allowed: {change}."
            )
        elif change_policy == "fixed" and start_state:
            clauses.append(f"Continuity for {subject}: keep {start_state} throughout.")
    for entity in _continuity_inventory(continuity):
        priority = _text(entity.get("priority"), "important").lower()
        if priority not in {"critical", "important"}:
            continue
        subject = _text(entity.get("subject"))
        quantity = entity["quantity"]
        start_state = _inventory_state(entity, "start")
        end_state = _inventory_state(entity, "end")
        clause = (
            f"Object conservation for {subject}: exactly {quantity} physical instance{'s' if quantity != 1 else ''} "
            f"exists throughout; start {start_state}; by the end {end_state}. "
            "Never duplicate, split, morph, replace, or lose the declared instance."
        )
        transfers = entity.get("transfers", [])
        if transfers:
            transfer_text = "; ".join(
                f"{_text(transfer.get('action'), 'one visible transfer')} from {_text(transfer.get('from'))} to {_text(transfer.get('to'))}"
                for transfer in transfers
            )
            clause += f" The only allowed transfer sequence is: {transfer_text}."
        if entity.get("exclusive_location") is True:
            clause += " At every visible moment, it occupies one holder or location only, never two hands, actors, or locations at once."
        forbidden_states = [_text(state) for state in entity.get("forbidden_states", []) if _text(state)]
        if forbidden_states:
            clause += " Never show these states: " + "; ".join(forbidden_states) + "."
        clauses.append(clause)
    return clauses


def _performance_clauses(performance: dict[str, Any] | None) -> list[str]:
    """Compile source-grounded performance choices, not generic emotion labels."""
    clauses: list[str] = []
    for arc in _performance_arcs(performance):
        priority = _text(arc.get("priority"), "important").lower()
        if priority not in {"critical", "important"}:
            continue
        subject = _text(arc.get("subject"))
        start_state = _text(arc.get("start_state"))
        turning_point = _text(arc.get("turning_point"))
        end_state = _text(arc.get("end_state"))
        clause = f"Performance arc for {subject}: begin {start_state}"
        if turning_point:
            clause += f"; turn when {turning_point}"
        clause += f"; resolve as {end_state}."
        forbidden = [_text(item) for item in arc.get("forbidden_readings", []) if _text(item)]
        if forbidden:
            clause += " Do not play it as: " + "; ".join(forbidden) + "."
        clauses.append(clause)
    for beat in _dialogue_beats(performance):
        priority = _text(beat.get("priority"), "important").lower()
        if priority not in {"critical", "important"}:
            continue
        speaker = _text(beat.get("speaker"))
        language = _text(beat.get("language"), "the requested language")
        line = _text(beat.get("line"))
        clause = f"Dialogue performance for {speaker}: say exactly in {language}: \"{line}\"."
        precondition = _text(beat.get("precondition"))
        if precondition:
            clause += f" Deliver it only when {precondition}."
        subtext = _text(beat.get("subtext"))
        if subtext:
            clause += f" Subtext: {subtext}."
        vocal_delivery = _text(beat.get("vocal_delivery"))
        if vocal_delivery:
            clause += f" Voice: {vocal_delivery}."
        facial_cues = [_text(cue) for cue in beat.get("facial_cues", []) if _text(cue)]
        if facial_cues:
            clause += " Visible cues: " + "; ".join(facial_cues) + "."
        listener_response = _text(beat.get("listener_response"))
        if listener_response:
            clause += f" Listener response: {listener_response}."
        forbidden = [_text(item) for item in beat.get("forbidden_readings", []) if _text(item)]
        if forbidden:
            clause += " Do not play this line as: " + "; ".join(forbidden) + "."
        clauses.append(clause)
    return clauses


def build_prompt(spec: dict[str, Any], audit: dict[str, Any]) -> tuple[str, str]:
    effective = audit["effective_spec"]
    identity = effective.get("identity", {}) or {}
    camera = effective.get("camera", {}) or {}
    audio = effective.get("audio", {}) or {}
    reference = effective.get("reference", {}) or {}
    transition = effective.get("transition", {}) or {}
    continuity = effective.get("continuity", {}) or {}
    continuity_mode = _continuity_mode(continuity)
    pacing = effective.get("pacing", {}) or {}
    narrative = effective.get("narrative", {}) or {}
    performance = effective.get("performance", {}) or {}
    scene = _text(effective.get("scene"))
    subject = _text(effective.get("subject"))
    ethnicity = _text(identity.get("country_ethnicity"))
    hair = _text(identity.get("hair"))
    skin = _text(identity.get("skin_tone"))
    identity_details: list[str] = []
    if _optional_bool(identity.get("adult")) is True:
        identity_details.append("clearly adult")
    if _text(identity.get("visual_age")):
        identity_details.append(f"visual age {_text(identity.get('visual_age'))}")
    if ethnicity:
        identity_details.append(ethnicity)
    if skin:
        identity_details.append(f"{skin} skin")
    if hair:
        identity_details.append(hair)
    identity_clauses: list[str] = []
    if identity_details:
        identity_clauses.append("Identity details: " + ", ".join(identity_details) + ".")
    if _optional_bool(identity.get("face_locked")) is True:
        identity_clauses.append("Preserve the same recognizable face.")
    if _optional_bool(identity.get("hair_locked")) is True:
        identity_clauses.append("Preserve the same hairstyle.")
    body_proportion_guard = _optional_bool(identity.get("body_locked")) is True
    if body_proportion_guard:
        identity_clauses.append("Preserve natural body proportions; no stretching or elongated limbs.")

    subject_clauses: list[str] = []
    if subject:
        subject_clauses.append(f"Subjects: {subject}.")
    actor_count = effective.get("actor_count")
    if isinstance(actor_count, int) and actor_count > 0:
        subject_clauses.append(f"Show exactly {actor_count} visible subject{'s' if actor_count != 1 else ''}.")

    scene_clauses: list[str] = []
    if scene:
        scene_clauses.append(f"Scene: {scene}.")
    scene_count = effective.get("scene_count")
    if isinstance(scene_count, int) and scene_count == 1:
        scene_clauses.append("Use one continuous scene.")
    elif isinstance(scene_count, int) and scene_count > 1:
        scene_clauses.append(f"Preserve the requested {scene_count} scene segments.")

    camera_clauses: list[str] = []
    camera_locked = _optional_bool(camera.get("locked"))
    full_body = _optional_bool(camera.get("full_body"))
    if camera_locked is True:
        camera_clauses.append("Use a locked camera.")
    elif camera_locked is False:
        camera_clauses.append("Allow camera movement as requested.")
    if full_body is True:
        camera_clauses.append("Use full-body framing with the subject inside the frame.")
    elif full_body is False:
        camera_clauses.append("Use the requested non-full-body framing.")
    lens = _text(camera.get("lens"))
    if lens:
        camera_clauses.append(f"Use a {lens} lens.")

    transition_clauses: list[str] = []
    requested_transition = _text(transition.get("requested"))
    transition_strategy = _text(transition.get("strategy"))
    if requested_transition:
        transition_clauses.append(f"Requested transition: {requested_transition}.")
    if transition_strategy and transition_strategy != "preserve_requested":
        transition_clauses.append(f"Execution strategy: {transition_strategy}.")

    reference_usage = _text(reference.get("usage"), "unknown").lower()
    input_ledger = _input_ledger(reference)
    rejected_direct_inputs = [
        asset for asset in input_ledger
        if _text(asset.get("route"), "direct_input").lower() == "direct_input"
        and _text(asset.get("verdict"), "unreviewed").lower() == "rejected"
    ]
    if rejected_direct_inputs:
        transition_clauses.append("Do not use rejected prior frames as visual input; use only the approved target opening frame.")
    if reference_usage == "i2v_first_frame":
        transition_clauses.append("Reference image role: use it as the opening frame.")
    elif reference_usage in {"identity_only", "identity_reference", "character_reference"}:
        transition_clauses.append("Reference image role: use it for identity guidance only, not as the requested composition.")
    elif reference_usage == "keyframe":
        keyframe_time = reference.get("keyframe_time_seconds")
        transition_clauses.append(f"Reference image role: use it at approximately {keyframe_time}s as a keyframe.")

    bpm = audio.get("bpm")
    audio_clauses: list[str] = []
    audio_generate = _optional_bool(audio.get("generate"))
    if audio_generate is True:
        music = "Generate lyric-free music"
        if bpm:
            music += f" at {bpm} BPM"
        audio_clauses.append(music + ".")
    elif audio_generate is False:
        audio_clauses.append("Do not generate music.")
    if _optional_bool(audio.get("beat_sync")) is True:
        audio_clauses.append("Attempt the requested beat synchronization, subject to verification.")

    transition_strategy = _text(transition.get("strategy"))
    timeline = _timeline(effective.get("outfit_changes", []), transition_strategy)
    narrative_clauses: list[str] = []
    objective = _text(narrative.get("objective"))
    required_events = narrative.get("required_events", []) or []
    if objective:
        narrative_clauses.append(f"Narrative objective: {objective}.")
    if required_events:
        narrative_clauses.append("Required plot coverage: " + "; ".join(_text(event) for event in required_events if _text(event)) + ".")
    task_clauses = [
        f"Create a {effective['duration_seconds']}-second {effective['ratio']} video at {effective['resolution']}.",
        *subject_clauses,
        *identity_clauses,
        *scene_clauses,
        *camera_clauses,
        *transition_clauses,
        *narrative_clauses,
    ]
    if timeline:
        task_clauses.append(timeline)
    task_clauses.extend(_continuity_clauses(continuity))
    task_clauses.extend(_performance_clauses(performance))
    task_clauses.extend(_pacing_clauses(pacing))
    task_clauses.extend(audio_clauses)
    prompt = " ".join(task_clauses)
    hard_constraints = effective.get("hard_constraints", []) or []
    if hard_constraints:
        prompt += " Hard constraints: " + "; ".join(_text(item) for item in hard_constraints if _text(item)) + "."

    negatives: list[str] = []
    explicit_negatives = effective.get("negative_constraints", []) or []
    negatives.extend(_text(item) for item in explicit_negatives if _text(item))
    if isinstance(actor_count, int) and actor_count == 1:
        negatives.extend(["extra visible subjects", "clones", "ghosts"])
    if transition_strategy == "in_place_beat_change":
        negatives.extend(["persistent cutouts", "sticker overlays", "double exposure"])
    if body_proportion_guard:
        negatives.extend(["stretched body", "elongated limbs"])
    if camera_locked is True:
        negatives.extend(["unrequested camera movement", "zoom", "reframing"])
    if full_body is True:
        negatives.append("unrequested crop")
    if continuity_mode != "off":
        for entity in _continuity_inventory(continuity):
            priority = _text(entity.get("priority"), "important").lower()
            if priority not in {"critical", "important"}:
                continue
            subject = _text(entity.get("subject"))
            if entity.get("quantity") == 1:
                negatives.extend([f"duplicate {subject}", f"split {subject}", f"vanishing {subject}"])
            if entity.get("exclusive_location") is True:
                bare_subject = re.sub(r"^(?:the|a|an|one)\s+", "", subject, flags=re.IGNORECASE)
                negatives.append(f"the same {bare_subject} in multiple hands, actors, or locations")
            negatives.extend(_text(state) for state in entity.get("forbidden_states", []) if _text(state))
    negative = ", ".join(dict.fromkeys(negatives))
    return prompt, negative


def compile_scene(spec: dict[str, Any]) -> dict[str, Any]:
    audit = audit_and_downgrade(spec)
    prompt, negative = build_prompt(spec, audit)
    prompt_for_mcp = f"{prompt}\nNegative constraints: {negative}"
    prompt_bytes = len(prompt.encode("utf-8"))
    prompt_for_mcp_bytes = len(prompt_for_mcp.encode("utf-8"))
    effective = audit["effective_spec"]
    prompt_budget = _prompt_budget(effective.get("provider", {}) or {})
    hard_limit = prompt_budget["hard_limit_bytes"]
    soft_limit = prompt_budget["soft_limit_bytes"]
    if hard_limit is not None and prompt_for_mcp_bytes > hard_limit:
        raise SpecError(
            "compiled MCP prompt exceeds configured provider.max_prompt_bytes "
            f"({hard_limit} UTF-8 bytes): {prompt_for_mcp_bytes}"
        )
    prompt_budget["soft_budget_exceeded"] = prompt_for_mcp_bytes > soft_limit
    prompt_budget["hard_limit_enforced"] = hard_limit is not None
    if prompt_budget["soft_budget_exceeded"]:
        audit["recommendations"].append(
            f"Compiled MCP prompt is {prompt_for_mcp_bytes} UTF-8 bytes, above the {soft_limit}-byte quality budget. "
            "Keep all required constraints, but remove duplicated wording before retrying a semantic failure; this is not a provider rejection."
        )
    changes = effective.get("outfit_changes", [])
    continuity = effective.get("continuity", {}) or {}
    continuity_mode = _continuity_mode(continuity)
    pacing = effective.get("pacing", {}) or {}
    pacing_mode = _text(pacing.get("mode"), "unspecified").lower()
    narrative = effective.get("narrative", {}) or {}
    performance = effective.get("performance", {}) or {}
    qa = [
        "Verify actual duration and resolution.",
    ]
    reference_usage = _text(effective.get("reference", {}).get("usage"), "unknown").lower()
    input_ledger = _input_ledger(effective.get("reference", {}) or {})
    if reference_usage == "identity_only" and effective.get("reference", {}).get("role_aware_channel") is False:
        qa.append("Check that the original identity image is not displayed as the opening frame.")
    elif reference_usage == "i2v_first_frame":
        qa.append("Check that the opening frame matches the requested reference-frame role.")
    elif reference_usage == "keyframe":
        qa.append("Check the reference image appears at the requested keyframe position, not by default at time zero.")
    for asset in input_ledger:
        if _text(asset.get("route"), "direct_input").lower() == "direct_input" and _text(asset.get("verdict"), "unreviewed").lower() == "rejected":
            qa.append(f"Verify rejected source {asset['id']} was excluded from the visual-input chain.")
    actor_count = effective.get("actor_count")
    if isinstance(actor_count, int):
        qa.append(f"Verify exactly {actor_count} visible subject{'s' if actor_count != 1 else ''} and check their continuity.")
    if _optional_bool(effective.get("camera", {}).get("full_body")) is True:
        qa.append("Check full-body framing and crop boundaries.")
    if any(_optional_bool(effective.get("identity", {}).get(field)) is True for field in ("face_locked", "hair_locked", "body_locked")):
        qa.append("Check the requested face, hairstyle and body-proportion continuity.")
    scene_count = effective.get("scene_count")
    if isinstance(scene_count, int) and scene_count > 1:
        qa.append("Check scene transitions and continuity according to the requested scene segments.")
    if effective.get("scene"):
        qa.append("Check background continuity against the requested scene.")
    if continuity_mode != "off":
        for anchor in _continuity_anchors(continuity):
            subject = _text(anchor.get("subject"))
            start_state = _anchor_state(anchor, "start")
            end_state = _anchor_state(anchor, "end")
            change_policy = _text(anchor.get("change_policy"), "fixed").lower()
            visibility = _text(anchor.get("visibility"), "visible").lower()
            if visibility == "visible":
                qa.append(f"Check continuity anchor {anchor['id']}: {subject} is visibly {start_state} at the start.")
            if change_policy == "declared" and end_state != start_state:
                qa.append(f"Check continuity anchor {anchor['id']} ends as declared: {end_state}.")
            elif visibility == "visible":
                qa.append(f"Check continuity anchor {anchor['id']} remains {start_state} through the clip.")
        for entity in _continuity_inventory(continuity):
            subject = _text(entity.get("subject"))
            quantity = entity["quantity"]
            entity_id = _text(entity.get("id"))
            start_state = _inventory_state(entity, "start")
            end_state = _inventory_state(entity, "end")
            qa.append(
                f"Check conservation entity {entity_id}: exactly {quantity} physical instance{'s' if quantity != 1 else ''} of {subject} exists at the start, every transfer, and the end; no duplicate, split, morph, replacement, or disappearance."
            )
            qa.append(f"Check conservation entity {entity_id} starts as declared: {start_state}.")
            for transfer in entity.get("transfers", []):
                action = _text(transfer.get("action"), "one visible transfer")
                source = _text(transfer.get("from"))
                destination = _text(transfer.get("to"))
                qa.append(
                    f"Check conservation transfer for {entity_id}: the same instance makes {action} from {source} to {destination}, then no longer remains at {source}."
                )
            if end_state != start_state:
                qa.append(f"Check conservation entity {entity_id} ends as declared: {end_state}.")
            if entity.get("exclusive_location") is True:
                qa.append(f"Check conservation entity {entity_id} never occupies multiple hands, actors, or locations at once.")
            for forbidden_state in entity.get("forbidden_states", []):
                if _text(forbidden_state):
                    qa.append(f"Check conservation entity {entity_id} never shows forbidden state: {_text(forbidden_state)}.")
    if pacing_mode == "real_time":
        qa.append("Check motion is normal-speed real time with no slow motion, time stretching, frozen waiting, or purposeless lingering.")
        max_idle_seconds = pacing.get("max_idle_seconds")
        if isinstance(max_idle_seconds, (int, float)) and not isinstance(max_idle_seconds, bool):
            qa.append(f"Check no visually unchanged pose or wait lasts longer than approximately {max_idle_seconds:g} seconds.")
        for beat in pacing.get("beats", []) or []:
            qa.append(f"Check story beat around {beat['time_seconds']}s: {beat['event']}.")
        if _text(pacing.get("turning_point")):
            qa.append(f"Check the visible turning point: {_text(pacing['turning_point'])}.")
    elif pacing_mode == "stylized_slow":
        qa.append("Check the requested stylized slow pacing rather than treating it as a real-time action shot.")
    for event in narrative.get("required_events", []) or []:
        if _text(event):
            qa.append(f"Check required plot event: {_text(event)}.")
    for arc in _performance_arcs(performance):
        priority = _text(arc.get("priority"), "important").lower()
        if priority not in {"critical", "important"}:
            continue
        qa.append(
            f"Check performance arc {arc['id']}: {_text(arc.get('subject'))} visibly moves from "
            f"{_text(arc.get('start_state'))} to {_text(arc.get('end_state'))}; do not accept a generic or forbidden reading."
        )
    for beat in _dialogue_beats(performance):
        priority = _text(beat.get("priority"), "important").lower()
        if priority not in {"critical", "important"}:
            continue
        qa.append(
            f"Check dialogue beat {beat['id']}: {_text(beat.get('speaker'))} visibly delivers \"{_text(beat.get('line'))}\" "
            f"with the declared subtext, voice, facial cues, and listener response."
        )
        forbidden = [_text(item) for item in beat.get("forbidden_readings", []) if _text(item)]
        if forbidden:
            qa.append(f"Check dialogue beat {beat['id']} never reads as: {'; '.join(forbidden)}.")
    qa.extend(f"Check outfit around {change['time_seconds']}s: {change['outfit']}." for change in changes)
    if _optional_bool(effective.get("audio", {}).get("generate")) is True or _optional_bool(effective.get("audio", {}).get("beat_sync")) is True:
        qa.append("Verify the requested audio track and measure beat alignment instead of trusting the prompt.")
    post_production = []
    if len(changes) >= 3 or _optional_bool(effective.get("audio", {}).get("beat_sync")) is True:
        post_production.append("Use external editing for exact beat-locked changes when this task requires frame-accurate timing.")
    if _optional_bool(effective.get("audio", {}).get("beat_sync")) is True:
        post_production.append("Replace generated audio when BPM or beat alignment is not exact.")
    if len(changes) >= 4:
        post_production.append("Consider separate clips or masks for each outfit interval instead of one Grok task.")
    if isinstance(actor_count, int) and actor_count > 1:
        post_production.append("Consider shot-based editing when multi-subject continuity is unstable.")
    if isinstance(effective.get("scene_count"), int) and effective.get("scene_count") > 1:
        post_production.append("Consider assembling the requested scene segments in post-production.")
    if continuity_mode == "strict":
        post_production.append("When a critical continuity anchor fails, rebuild the target keyframe or split the beat; do not retry the same prompt unchanged.")
    if _dialogue_beats(performance):
        post_production.append("If a required line, lip sync, or emotional delivery fails, retain the approved picture and replace only that dialogue performance in post-production.")
    return {
        "compiler_version": "1.0",
        "model": _text(spec.get("model"), "grok-imagine-video-1.5"),
        "source_spec": spec,
        "effective_spec": audit["effective_spec"],
        "risk": audit["risk"],
        "creation_policy": "blocked" if audit["blocking_findings"] else "ready",
        "blocking_findings": audit["blocking_findings"],
        "effective_strategy": {
            "reference": audit["effective_spec"]["reference"]["strategy"],
            "transition": audit["effective_spec"]["transition"]["strategy"],
            "audio": audit["effective_spec"]["audio"]["strategy"],
            "scene": audit["effective_spec"]["scene_strategy"],
        },
        "performance_plan": {
            "arcs": _performance_arcs(performance),
            "dialogue_beats": _dialogue_beats(performance),
        },
        "downgrades": audit["downgrades"],
        "recommendations": audit["recommendations"],
        "prompt": prompt,
        "negative_prompt": negative,
        "prompt_utf8_bytes": prompt_bytes,
        "prompt_for_mcp": prompt_for_mcp,
        "prompt_for_mcp_utf8_bytes": prompt_for_mcp_bytes,
        "prompt_budget": prompt_budget,
        "qa_checklist": qa,
        "post_production": post_production,
    }


def example_spec(name: str) -> dict[str, Any]:
    if name != "complex-outfit":
        raise SpecError(f"unknown example: {name}")
    return {
        "version": 1,
        "model": "grok-imagine-video-1.5",
        "duration_seconds": 10,
        "ratio": "9:16",
        "resolution": "720p",
        "actor_count": 1,
        "scene_count": 1,
        "scene": "a luxury high-rise apartment with floor-to-ceiling windows and a city skyline",
        "reference": {"kind": "collage", "has_stickers": True, "scene_matches_final": False},
        "camera": {"locked": True, "full_body": True, "lens": "35mm"},
        "identity": {"adult": True, "visual_age": "24-27", "face_locked": True, "hair_locked": True, "body_locked": True},
        "outfit_changes": [
            {"time_seconds": 2, "outfit": "burgundy tailored blazer dress", "action": "right heel stamp"},
            {"time_seconds": 4, "outfit": "ivory fitted knit dress", "action": "palms clap"},
            {"time_seconds": 6, "outfit": "royal-blue fitted two-piece outfit", "action": "return to front"},
            {"time_seconds": 8, "outfit": "deep emerald velvet dress", "action": "arms snap open"},
        ],
        "transition": {"requested": "full_body_cutout_fly_in", "source_layout": "collage"},
        "audio": {"generate": True, "bpm": 120, "beat_sync": True, "lyrics": False},
        "dance_complexity": "high",
        "hard_constraints": ["one person", "no text", "no watermark"],
    }


def _reject_output_overwrite(output_path: Path, input_path: Path) -> None:
    """Prevent a report destination from replacing its source SceneSpec."""
    try:
        resolved_output = output_path.resolve()
        resolved_input = input_path.resolve()
    except (OSError, RuntimeError) as exc:
        raise SpecError(f"cannot verify output/input paths safely: {exc}") from exc
    if resolved_output == resolved_input:
        raise SpecError("output path must not overwrite the input SceneSpec")
    try:
        if os.path.samefile(resolved_output, resolved_input):
            raise SpecError("output path must not overwrite the input SceneSpec")
    except FileNotFoundError:
        # A new output file has no identity yet; canonical path comparison above
        # still protects a direct alias.
        pass
    except OSError as exc:
        raise SpecError(f"cannot verify output/input file identity safely: {exc}") from exc


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--input", type=Path, help="SceneSpec JSON file")
    group.add_argument("--example", choices=["complex-outfit"])
    parser.add_argument("--output", type=Path, help="Write report JSON to this path")
    args = parser.parse_args(argv)
    try:
        if args.input and args.output:
            _reject_output_overwrite(args.output, args.input)
        if args.input:
            spec = json.loads(args.input.read_text(encoding="utf-8"))
        else:
            spec = example_spec(args.example)
        report = compile_scene(spec)
        encoded = json.dumps(report, ensure_ascii=False, indent=2)
        if args.output:
            args.output.write_text(encoded + "\n", encoding="utf-8")
        else:
            print(encoded)
        return 0
    except (OSError, json.JSONDecodeError, SpecError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
