# SceneSpec 提取协议

当用户只给自然语言时，Codex 先按本协议提取 JSON，再调用编译器。不要让外部模型直接输出最终 Grok Prompt，因为那会绕过风险审计和降级记录。

## 提取指令

```text
You convert a user's video request into SceneSpec version 1.
Return JSON only. Preserve explicit user requirements; do not invent details.
Use null, empty arrays, or "unknown" when the request does not establish a fact.
Treat reference role, subject count, scene count, framing, camera movement, audio generation,
and hard negatives as task-specific fields. Do not default them to identity reference,
first frame, one subject, one scene, locked camera, full body, or no extra people.
Add continuity only when cross-frame relationships are materially important to this task.
Continuity is not a reason to simplify a crowd, remove a character, or lock the camera.
For each critical relationship, record a concrete visible start_state and (when it changes)
an explicit end_state and declared change. Use anchors for people, props, spatial blocking,
costume state, gaze, or environment. Do not add anchors for incidental detail. If two visible
objects claim the same hand, forearm, seat, or screen area, either state their physically
shared attachment explicitly or report the conflict before generation.
For every source image or prior-video frame considered for a new generation, add
reference.input_ledger.assets entry. Only route=direct_input with verdict=approved may be
uploaded to image editing or video generation. A frame that failed visual or continuity QA
must be route=excluded and verdict=rejected; retain any useful story context only as a
route=text_summary entry. Never use a rejected frame's pixels as a weak identity or outfit
reference in a multi-image edit.
For a narrative clip that must run at normal speed, add pacing.mode=real_time and at least
two time-stamped pacing.beats that visibly change story state. Add pacing.turning_point when
the scene has a reversal, recognition, decision, exchange, or other visible dramatic turn.
Do not infer this for atmospheric, dance, product, or user-requested slow-motion clips.
When the user explicitly rejects lingering or static posing in a real-time narrative shot, set
task-specific pacing.max_idle_seconds and require every interval within that limit to contain a
visible state change. Do not add this limit to clips where stillness is intentional.
For serialized story, adaptation, or storyboard work, add narrative.objective and one to three
narrative.required_events. Each required event must be a viewer-visible change of story fact,
relationship, decision, or exchange and must map to a pacing beat or continuity anchor. Do not
claim that generic walking, posing, or waiting covers an irreplaceable plot event.
When a beat depends on entering, leaving, approaching, handing over, or relative blocking,
add a separate continuity anchor for the camera-relative spatial path. State which side of the
door/counter/camera the actor starts on, where they end, and which direction their body and feet
face. Require the synthesized first frame to visibly establish that geometry; verbs such as
"leave" or "walk over" are not sufficient by themselves.
Use reference.usage=identity_only only when the user explicitly wants identity guidance;
use i2v_first_frame only when the image is intended to be the opening frame; use keyframe
only when a time position is specified. Preserve multiple subjects and visual elements when
the request calls for a story scene.
Mark a reference as collage when it contains a central subject plus surrounding figures,
stickers, cutouts, dashed outlines, or multiple people, even if the user wants those
figures to disappear later.
Map exact timestamps to outfit_changes[].time_seconds without averaging them.
Separate requested transition from the eventual model-specific strategy.
```

## 审计后再生成

1. 将提取结果交给 `compile_prompt.py --input`。
2. 把报告中的 `risk`, `downgrades`, `effective_strategy` 和 `recommendations` 告诉用户。
3. 只有用户接受降级方案后才调用视频 MCP；用户明确要求直接生成时可以继续，但仍要先报告风险。
4. 不把 `effective_spec` 反写成用户原始需求；它只是 Grok 执行计划。
5. 对开启 `continuity.mode=strict` 的片段，首帧要先按锚点逐项验收；相邻片段用上一镜已验收的 `end_state` 填入下一镜的 `handoff_from`。若两者不一致，先修正首帧或声明变化，不要直接提交视频任务。
6. 有输入资格账本时，必须先审计它。`rejected + direct_input` 是阻断性来源污染，不得创建付费任务；将其从图片输入中移除并改为 `excluded` 或 `text_summary` 后才可继续。

## 示例映射

“四周有四个服装贴纸，2/4/6/8 秒飞入中央换装”必须至少得到：

```json
{
  "reference": {"kind": "collage", "has_stickers": true},
  "outfit_changes": [
    {"time_seconds": 2, "outfit": "..."},
    {"time_seconds": 4, "outfit": "..."},
    {"time_seconds": 6, "outfit": "..."},
    {"time_seconds": 8, "outfit": "..."}
  ],
  "transition": {"requested": "full_body_cutout_fly_in"},
  "audio": {"generate": true, "beat_sync": true}
}
```
