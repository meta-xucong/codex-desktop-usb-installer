# SceneSpec 契约

这是 Codex 提取自然语言后的中间 JSON。字段可扩展；示例中的单人、单场景、全身机位和参考图角色只是示例值，不是全局默认。

```json
{
  "version": 1,
  "model": "grok-imagine-video-1.5",
  "duration_seconds": 10,
  "ratio": "9:16",
  "resolution": "720p",
  "actor_count": 1,
  "scene_count": 1,
  "subject": "one adult woman",
  "reference": {
    "kind": "collage",
    "usage": "i2v_first_frame",
    "role_aware_channel": false,
    "source_ratio": "1:1",
    "framing": "full_body",
    "has_stickers": true,
    "has_transparency": false,
    "scene_matches_final": false,
    "input_ledger": {
      "assets": [
        {
          "id": "previous_clip_end",
          "role": "previous-shot visual handoff",
          "route": "excluded",
          "verdict": "rejected",
          "reason": "handbag handle topology failed continuity QA"
        },
        {
          "id": "approved_identity_photo",
          "role": "identity and outfit reference for first-frame synthesis",
          "route": "direct_input",
          "verdict": "approved"
        },
        {
          "id": "story_state",
          "route": "text_summary",
          "verdict": "approved",
          "summary": "the character has completed payment and is walking toward the exit"
        }
      ]
    }
  },
  "camera": {
    "locked": true,
    "full_body": true,
    "lens": "35mm"
  },
  "identity": {
    "adult": true,
    "visual_age": "24-27",
    "face_locked": true,
    "hair_locked": true,
    "body_locked": true
  },
  "outfit_changes": [
    {"time_seconds": 2, "outfit": "burgundy tailored blazer dress", "action": "right heel stamp"}
  ],
  "transition": {
    "requested": "full_body_cutout_fly_in",
    "source_layout": "upper-left"
  },
  "audio": {
    "generate": true,
    "bpm": 120,
    "beat_sync": true,
    "lyrics": false
  },
  "continuity": {
    "mode": "strict",
    "anchors": [
      {
        "id": "lead_bag",
        "subject": "the lead character's handbag",
        "priority": "critical",
        "visibility": "visible",
        "attachment": "lead.left_forearm",
        "start_state": "both short handles looped over the left forearm, bag outside the left hip",
        "change_policy": "fixed"
      }
    ]
  },
  "pacing": {
    "mode": "real_time",
    "allow_slow_motion": false,
    "max_idle_seconds": 2.5,
    "beats": [
      {"time_seconds": 0, "event": "the character enters the doorway"},
      {"time_seconds": 4, "event": "she turns back toward the person behind her"},
      {"time_seconds": 8, "event": "she gives a brief grateful reply"},
      {"time_seconds": 12, "event": "she turns forward and exits"}
    ],
    "turning_point": "she pauses at the door and turns back to acknowledge the favor"
  },
  "narrative": {
    "objective": "show that the neighbor's payment changes the woman's immediate decision",
    "required_events": [
      "the neighbor completes the payment",
      "the woman recognizes the payment succeeded",
      "the woman gives a brief grateful reply"
    ]
  },
  "dance_complexity": "high",
  "hard_constraints": ["one person", "no text", "no watermark"]
}
```

## 枚举

- `reference.kind`: `clean_single_person`, `collage`, `multi_person`, `unknown`
- `reference.usage`: `i2v_first_frame`, `identity_only`, `keyframe`, `unknown`。这是本次任务的输入角色，不是全局默认。`i2v_first_frame` 表示图像应成为起始画面，`identity_only` 表示只提取人物特征，`keyframe` 表示图像对应指定时间位置。
- `reference.role_aware_channel`: 当前视频接口是否明确支持把该图片作为独立身份参考或指定时间关键帧；使用布尔值或 `unknown`，由 Codex 在调用前核实。
- `reference.source_ratio`: 源图比例，例如 `1:1`、`9:16`、`16:9`；未知时使用 `unknown`。
- `reference.framing`: `close_up`, `half_body`, `full_body`, `unknown`。它描述源图构图，不是目标视频机位。
- `reference.keyframe_time_seconds`: 当 `usage=keyframe` 时填写目标时间；不应把关键帧自动解释成首帧。
- `reference.input_ledger`: 可选的输入资格账本。它与 `reference.usage` 不同：前者记录每张来源素材是否有资格进入“生图首帧合成”或 I2V 的像素输入，后者记录当前视频接口如何解释最终图片。
  - `assets[].id`: 素材的稳定标识。
  - `assets[].role`: `direct_input` 时必须填写，说明它为何参与视觉输入。
  - `assets[].route`: `direct_input`、`text_summary`、`excluded`。`direct_input` 表示该文件允许作为多图编辑或视频视觉输入；`text_summary` 只保留经过人工提炼的叙事信息，不传原始像素；`excluded` 禁止将该文件传入任何后续图像/视频调用。
  - `assets[].verdict`: `approved`、`rejected`、`unreviewed`。已知不符合关键锚点的上一镜帧必须设为 `rejected + excluded`，不能因为“只参考人物/服装”又被加入多图编辑。
  - `assets[].reason`、`assets[].summary`: 可选，分别记录拒绝原因或可安全继承的文字状态。
- `transition.requested`: `in_place_change`, `full_body_cutout_fly_in`, `sticker_merge`, `clone_fusion`, `遮挡换装`
- `dance_complexity`: `low`, `medium`, `high`

## 可选 Prompt 预算

`provider` 记录当前 host、profile、model 已证实的 Prompt 长度能力，默认不填写。它不应从其他供应商、历史项目或本地习惯推断。

- `provider.prompt_soft_budget_bytes`: 可选的本地质量预算。超出时编译器只提示清理重复表达，不拒绝创建任务。
- `provider.max_prompt_bytes`: 可选的、经证实的传输硬限制。仅此字段会让编译器拒绝超限 Prompt。
- 同时给出两者时，软预算不得大于硬限制。

```json
{
  "provider": {
    "prompt_soft_budget_bytes": 4096,
    "max_prompt_bytes": 8192
  }
}
```

## 可选节奏计划

`pacing` 只在本镜有明确时长和节奏意图时使用，不默认让所有视频变快或禁止慢镜。

- `pacing.mode`: `unspecified`、`real_time`、`stylized_slow` 或 `fast`。`real_time` 用于正常速度叙事；`stylized_slow` 仅用于用户明确的慢镜或氛围表达；`fast` 是自然但利落的节奏，不等于加速播放。
- `pacing.beats[]`: 可见节拍，每个节拍都有 `time_seconds` 和会改变人物、道具、关系或场景状态的 `event`。实时叙事通常至少两个节拍；走路、等待或摆姿只有在它们确实改变故事状态时才算节拍。
- `pacing.turning_point`: 可选的剧情转折，例如角色在门口停下并回头致谢。它不要求戏剧性反转，但必须在画面中可辨认。
- `pacing.allow_slow_motion`: 实时叙事通常为 `false`。未声明 `pacing` 时，编译器不推测速度风格。
- `pacing.max_idle_seconds`: 可选的实时叙事验收上限。只在本镜不能接受长时间摆姿、等待或同一走路循环时设置；要求相邻可见状态变化不超过该间隔。不要把它套到需要留白的氛围、舞蹈或慢镜任务。

## 可选剧情覆盖

`narrative` 用于连续剧、改编、分镜或其他有明确剧情义务的片段；它不替代剧本，也不应强加给氛围、舞蹈或产品镜头。

- `narrative.objective`: 本镜要完成的单句叙事目标。
- `narrative.required_events[]`: 一到三个必须被观众看见的事实变化。它们应是剧情状态、人物关系或决定的变化，而不是“走路”“站着”“镜头停留”等可替换的动作。
- 每个 `required_events` 都要被映射到 `pacing.beats` 或连续性锚点，并在生成后逐项验收；没有覆盖时不能因为视频时长和动作正常就判为通过。

## 可选表演规划

`performance` 只用于对白、表演或心理变化本身需要被验收的镜头。它不是情绪标签库：先由 LLM 根据原文或用户的剧情要求写出可见 `source_evidence`，再选择起点、转折、落点和可拍摄的表达方式。没有情绪交付目标时省略该字段。

- `performance.arcs[]`: 人物或关系的表演弧。每项必须有 `id`、`subject`、`source_evidence`、`start_state`、`end_state`，可有 `turning_point`、`priority` 与 `forbidden_readings[]`。
- `performance.dialogue_beats[]`: 台词的独立表演节拍。每项必须有 `id`、`speaker`、`line`、`source_evidence`。`critical` 节拍还必须有 `precondition`、`subtext`、`vocal_delivery` 与至少一条 `facial_cues[]`；可选 `language`、`listener_response` 与 `forbidden_readings[]`。
- `source_evidence`: 对来源中可观察叙事事实的简短摘述，不写隐藏推理。它用来避免凭空把人物演成快乐、恐惧、愤怒或暧昧。
- `priority`: `critical`、`important`、`ambient`。前两者进入 Prompt 与 QA，`ambient` 只保留在报告中。关键对白应少而明确，避免用很多台词挤掉表情和对方反应。
- `forbidden_readings[]`: 本次要排除的相近但错误的演法，例如快乐调情、惊恐拒绝、冷漠背诵。它只针对该角色/台词，不是全局负面词。

```json
{
  "performance": {
    "arcs": [
      {
        "id": "lead_shift",
        "subject": "the lead character",
        "priority": "critical",
        "source_evidence": "the character briefly recoils, then chooses to remain",
        "start_state": "guarded surprise",
        "turning_point": "the other person pauses and listens",
        "end_state": "embarrassed trust with reserve",
        "forbidden_readings": ["bright flirting", "fearful refusal"]
      }
    ],
    "dialogue_beats": [
      {
        "id": "soft_reply",
        "speaker": "the lead character",
        "line": "谢谢。",
        "language": "Mandarin Chinese",
        "priority": "critical",
        "source_evidence": "the favor has just been accepted",
        "precondition": "the speaker's lips are visible and the listener is still",
        "subtext": "relief mixed with restrained gratitude",
        "vocal_delivery": "quiet, warm, with a small breath before the final word",
        "facial_cues": ["eyes lower briefly", "a restrained exhale"],
        "listener_response": "the listener relaxes and gives space",
        "forbidden_readings": ["sales-pitch cheerfulness", "flat recitation"]
      }
    ]
  }
}
```

## 可选连续性锚点

`continuity` 是按任务启用的关系状态审计，不是单人、固定镜头或少道具的默认开关。省略它，或设置 `mode=off`，编译器不会加入任何额外连续性限制。

- `continuity.mode`: `off`、`advisory` 或 `strict`。
  - `off`: 不启用连续性约束。
  - `advisory`: 把声明的关系写入 Prompt 和 QA，并在冲突时给出风险提示。
  - `strict`: 同 `advisory`，并要求起始、转折和结尾逐项验收；失败后优先重建目标首帧或拆镜头，不能直接重复同一任务。
- `continuity.anchors[]`: 只列出本镜真正不可错位的道具、人物位置、人物关系、服装状态、视线关系、运动路径或环境状态。它既可用于单人拿包，也可用于群像中“演员 A 一直在演员 B 左侧”“红伞始终由角色 C 持有”“车辆始终停在画面右侧”。
- `anchors[].id`: 在本镜唯一的稳定标识。
- `anchors[].subject`: 人可读的受约束对象或关系，例如 `actor A and actor B`、`the red umbrella`、`the stage door`。
- `anchors[].priority`: `critical`、`important`、`ambient`。`critical` 和 `important` 会编译入 Prompt；`ambient` 只进入审计和验收，用来保留画面层次而不挤占 Prompt 预算。
- `anchors[].visibility`: `visible`、`optional`、`offscreen_allowed`。只有明确要求可见时才要求每个抽查点出现；画外人物不应被错误地标成 `visible`。
- `anchors[].attachment`: 可选的接触或空间槽位，例如 `lead.left_forearm`、`actor_b.right_hand`、`frame_left`。多个可见锚点占用同一槽位会被标记为冲突，除非它们都设置 `share_attachment=true`。
- `anchors[].start_state` 和 `anchors[].end_state`: 描述本镜开始和结束的关系状态。启用连续性后必须有 `start_state`；未填写 `end_state` 时默认等于起始状态。
- `anchors[].change_policy`: `fixed`、`declared`、`free`。`fixed` 不允许状态变化；`declared` 允许且只允许写在 `change` 中的一次变化；`free` 用于不需要锁定的关系。
- `anchors[].change`: 当 `change_policy=declared` 且起止状态不同必须填写，说明允许变化本身。
- `anchors[].handoff_from`: 可选，填写上一镜已经验收的结尾状态。与 `start_state` 不一致时，生成前阻断并要求修正目标首帧或显式声明变化。

### 可选道具守恒账本

`continuity.inventory[]` 专门约束一个可移动实体的实例数量、位置和转移路径。它解决的是“同一盒牛奶在换手后变成两盒”“手机凭空替换物品”“包带或产品在动作中消失”这类错误；普通背景物、可替换陈设和不影响剧情的道具不应默认加入。

- `inventory[].id`: 本镜唯一标识，且不能与 `anchors[].id` 重名。
- `inventory[].subject`: 人可读的实体名称，例如 `the sealed milk carton`、`the black handbag`。
- `inventory[].quantity`: 整个镜头中应存在的物理实例数，正整数。它不是接触次数，也不是画面中允许出现的人数。
- `inventory[].priority`: `critical`、`important`、`ambient`。前两者进入 Prompt、负面约束和 QA；`ambient` 只进入审计和 QA，不占用提示词或负面约束预算。
- `inventory[].start_state` / `end_state`: 实体在起始和结尾的可见位置或归属。启用连续性时必须有起始状态；未写结尾时默认为起始状态。
- `inventory[].transfers[]`: 当起止状态不同必须提供。每项写 `from`、`to`，可写 `action`，例如一次拿取、递交、放下或挂到手臂。
- `inventory[].exclusive_location`: 可选布尔值。`true` 表示同一实例在每个可见时刻只能归属一个手、演员或位置，适合“右手拿牛奶，左手保持空”的剧情。需要双手共同托住同一个物体时不要启用它。
- `inventory[].forbidden_states[]`: 本镜特有的禁止状态，例如 `two milk cartons`、`the same carton held by both hands`、`a carton still on the counter after pickup`。不要将它误用为全局禁止词。
- `inventory[].handoff_from`: 可选，上一镜已验收的实体状态；它与 `start_state` 不一致时产生交接风险。

例如，收银台上唯一一盒牛奶被拿起后，柜台不再残留牛奶，左手也不能再凭空出现第二盒：

```json
{
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
            "to": "the woman's right hand"
          }
        ],
        "exclusive_location": true,
        "forbidden_states": [
          "two milk cartons",
          "the same carton held by both hands",
          "a carton still on the counter after pickup"
        ]
      }
    ]
  }
}
```

守恒账本不会消除多人互动：例如一个服务员把唯一的杯子递给顾客，`quantity=1` 与一次从服务员右手到顾客左手的 `transfer` 仍然可以保留两人的可见交接。它也不是物理模拟保证，仍需在起始、每个转移点和结尾验收；失败时通常应拆分交接前后镜头，而不是继续堆叠负面词。

对于“进入/离开/靠近/交接”这种方向本身影响叙事的动作，用单独路径锚点描述几何关系。例如：`subject=the lead's movement path`，`start_state=inside the store, body and feet oriented away from the camera toward the open door`，`end_state=crosses the doorway away from the camera toward the street`，`change_policy=declared`，`change=two normal steps toward the exit`。同时在首帧中验证镜头一侧、门的位置和人物朝向真实成立；不能只用动词“leave”交给模型推断。

### 人体与接触点

道具与肢体是不同锚点。`attachment=lead.left_forearm` 只说明包的承载位置，不保证左手、手腕、手指或整条手臂在画面中完整存在。只有本镜确实需要某个肢体可见时，再添加一个人体锚点：

```json
{
  "id": "left_hand_complete",
  "subject": "the lead character's left hand",
  "priority": "critical",
  "visibility": "visible",
  "attachment": "lead.left_hand",
  "start_state": "fully visible in front of the handbag, with a natural relaxed palm and distinct fingers; not occluded by the bag",
  "change_policy": "fixed"
}
```

这不是要求所有镜头都展示双手。特写、遮挡、群像和画外动作仍可按任务设为 `optional` 或 `offscreen_allowed`。只有缺失肢体会破坏剧情、道具逻辑或参考首帧时，才将它升为 `critical`。

### 多人物示例

```json
{
  "actor_count": 3,
  "continuity": {
    "mode": "advisory",
    "anchors": [
      {
        "id": "duet_blocking",
        "subject": "dancer A and dancer B",
        "priority": "critical",
        "visibility": "visible",
        "start_state": "dancer A is frame-left of dancer B, both facing the audience",
        "end_state": "dancer A and dancer B exchange positions during one crossing move",
        "change_policy": "declared",
        "change": "one clearly visible crossing move"
      },
      {
        "id": "third_dancer",
        "subject": "dancer C",
        "priority": "important",
        "visibility": "visible",
        "start_state": "center-rear, holding a red umbrella",
        "change_policy": "fixed"
      }
    ]
  }
}
```

## 提取规则

1. 用户明确的参数优先于默认值。
2. “中央人物 + 四周小人物/贴纸/拼贴”必须标记为 `collage`，即使最终只想保留一个人。
3. “同一张脸/发型/身材比例”分别映射到 identity 锁定字段。
4. “2 秒、4 秒换装”映射为 `outfit_changes[].time_seconds`，不得平均化。
5. 无法从需求判断的字段用默认值或 `unknown`，并在审计报告中说明。
6. 用户说“只参考人物、不出现在视频头一帧”时写 `reference.usage=identity_only`；用户说“作为关键帧”时写 `reference.usage=keyframe` 和时间位置。
7. 源图是方形/横图而目标是竖屏，或源图是近景而本次任务要求全身时，分别填写 `source_ratio` 和 `framing`，仅在该图参与视频画面输入时触发构图审计。
8. `actor_count`、`scene_count`、`camera.locked` 和 `camera.full_body` 只有用户明确提出或任务方案明确选择时才填写；未知不要用 `1`、`true` 代替。
9. 仅当剧情、道具、角色走位或镜头交接需要跨帧一致时才添加 `continuity`。不要为了规避风险而把多人场景改成单人、把群像改成画外人、或默认锁死镜头。
10. `state` 应描述可见的关系而非抽象意图：写“短提手套在左前臂、包体在左胯外侧”，不要只写“包在左边”；写“角色 A 在角色 B 的画面左侧”，不要只写“站位正确”。
11. `strict` 不是自动通过闸门。Codex 必须先人工检查合成首帧是否满足每个 `critical` 锚点，再允许上传和创建付费视频任务；检查失败应重建首帧，而不是依靠负面词补救。
12. 当关键道具由人物肢体承载且该肢体需要完整可见时，分别写道具锚点和人体锚点。首帧验收必须同时确认道具结构、连接点、完整肢体、手指/脚部和无遮挡关系，而不是只确认“东西在画面里”。
13. 当剧情依赖唯一、限量或不可替换的物件发生拿取、放下、换手、递交、穿戴或装入时，添加 `continuity.inventory`，声明数量、转移和本镜特有的禁止状态。不要从普通 `anchors` 的起止位置推断“模型会自行保持物品只有一件”。
