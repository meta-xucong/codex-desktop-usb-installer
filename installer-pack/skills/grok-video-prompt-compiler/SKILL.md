---
name: grok-video-prompt-compiler
description: 将自然语言视频需求先提取为结构化 SceneSpec，再针对 Grok 兼容视频接口和具体 host/profile/model 做能力风险审计、可选降级、短 Prompt 编译和验收建议。用户要求用 Grok 生成视频、把一句话转换成 Grok Prompt、设计视频 Prompt 编译器、或希望避免视频模型的多人物/复杂换装/精确卡点问题时使用。
---

# Grok 视频 Prompt 编译器

把创意需求拆成可审计的规格，再生成适合 Grok 的短 Prompt。Skill 不承诺模型会帧级执行；它必须明确哪些要求被保留、哪些要求被降级，以及哪些结果需要后期完成。

## 工作流

1. 读取用户需求和参考图，提取 `SceneSpec`。每次按具体需求确定 `reference.usage`：`i2v_first_frame`、`identity_only`、`keyframe` 或 `unknown`；不能把某一种参考图用途写成全局默认。对于当前没有独立身份参考/关键帧通道的调用链，只有在本次任务明确要求身份参考或关键帧时，才提出相应替代方案。参考图有拼贴、贴纸或多人物时，显式标记 `reference.kind=collage`，但不要因此默认删除这些画面元素。只有本镜确实需要跨帧关系稳定时，才添加 `continuity` 锚点；它不推导出单人、单场景、固定镜头或少道具。
   对每一张候选来源素材写入 `reference.input_ledger`：只有 `direct_input + approved` 的素材可传给生图或视频接口；已知验收失败的帧必须是 `excluded + rejected`，只能保留为文字剧情摘要，不能以“只参考脸或服装”为由再次混入多图编辑。
2. 保留用户的原始要求，同时为 Grok 计算 `effective_strategy`。不要静默删除要求。
3. 运行 `scripts/compile_prompt.py` 做 JSON 校验、风险评分、降级和 Prompt 编译。
4. 调用视频 MCP 前，先对当前 `host + profile + model` 做本地 preflight，确认请求字段、视觉输入语义、时长/分辨率限制、能力证据状态和是否存在独立身份参考或时间关键帧通道。不得把模型官方直连 API 的字段自动套到第三方网关，也不得把其他日期或其他 host 的实测结果视为当前能力。读取 `references/grok-capability-matrix.md` 和 `references/provider-profile.md` 中与当前 host 匹配的近期观测，但仍以当前 profile 的契约和本次响应为准。
5. 需要公网视觉输入 URL 时，按 `references/reference-upload.md` 固定执行本地预检、必要时生成处理副本、上传、重新 GET 验证；不要直接把未经验证的 URL 交给视频 MCP。上传传输和图像语义处理分离：脚本不擅自把身份图改成首帧，也不擅自裁切、拉伸或去除画面元素。
6. 先向用户报告 `risk.level`、warnings、downgrades、provider capability status 和 post-production recommendations，再调用视频 MCP。视觉输入能力为 `unknown` 或 `uncertified` 时默认不创建付费任务；只有用户明确接受一次受限认证，才使用 `allow_uncertified_visual_input=true`。高风险任务仍可执行，但不能宣称严格卡点或复杂贴纸机制已被保证。
7. 生成视频后按本次报告抽查关键时间点，并核对时长、分辨率、音轨、参考图角色、主体数量、背景漂移、镜头裁切、发型/身体比例、关键动作和结尾状态。任务完成只证明服务返回了视频，不证明输入图按预期语义生效。
8. 如果本地 MCP、配置、网关或模型近期发生切换，重新读取实际加载的配置并运行本地测试；不要只看 GitHub HEAD 或历史成功记录来判断当前进程已修复。

## SceneSpec 提取规则

使用 `references/scenespec.md` 的字段。必需填写视频时长、画幅和分辨率；主体数量、场景数量、参考图角色、机位、构图、换装、转场、音频和硬约束都按本次需求填写，未知字段使用 `null`、`unknown` 或省略，不套用单人、单场景或固定机位默认值。

保留用户的具体服装名、时间点、动作和硬约束。对于不确定的内容使用 `null` 或空数组，不要编造参考图中看不清的细节。

## Grok 策略

- 调用统一媒体 MCP 的 Grok 视频工具时，为每张图片显式填写 `reference_media[].role`：`first_frame` 表示图片必须作为精确首帧，`reference` 表示仅提供人物/商品/服装/场景参考、开头由模型生成。不要只靠 Prompt 猜测或省略角色；省略仅为旧客户端兼容，服务端会按首帧解释。
- 只有 `grok-imagine-video-1.5` 可在已认证的 404token `responses` 路由中组合一张 `first_frame` 和最多 14 张 `reference`；旧 `grok-imagine-video` 的参考图与首帧不可混用，lite 和未知别名不推断支持参考图。其他 profile/host 必须有各自证据；Wokey 路由目前应在提交前拒绝图片参考。
- `reference_image_url` 的含义由本次接口和任务角色决定：指定为首帧时按首帧处理，指定为身份图时先确认接口是否支持独立身份参考，指定为关键帧时先确认是否支持时间位置；不要只靠 Prompt 猜测。
- 参考图模式必须和已确认的 provider 契约一一对应：I2V 首帧、身份/服装参考、风格参考、指定时间关键帧是不同能力。接口没有对应字段时，不得把身份图或关键帧静默降级为首帧；必须报告不可用能力和候选方案。
- `host + profile + model + date` 是能力证据的最小粒度。还要区分客户端请求契约、网关内部传输和上游契约；历史成功只说明当时该组合和该链路可用。一旦返回图像输入缺失、字段无效或不支持的错误，停止重复提交，把该组合标为待重新认证。
- 图像输入被拒绝而文本生视频可用时，二者是不同能力：只在用户接受“无视觉参考锁定”的降级后，创建一次 T2V 任务；结果报告必须写明图片没有提交给模型，不能称为人物原型已被使用。反过来，任务成功也不能单独证明模型读取了图片，必须结合输入模式和开头画面验收。
- 参考图是目标首帧、身份图还是关键帧，分别检查它与目标场景、比例、机位和时间位置是否匹配；只有本次任务要求时，才准备目标场景首帧或目标比例重构图。
- 源图比例与输出比例不一致时，只有在该图参与视频画面输入时才审计裁切/补背景/非等比缩放；身份参考且有独立通道时不应强行改成首帧。
- 多人物、贴纸、完整人物飞入、分身融合、复杂换装等效果按本次需求单独审计；可以保留原要求，也可以报告候选降级，不得默认改成单人原地变装，也不得因风险规则自动删除剧情中的演员或视觉元素。
- 多个叙事节点、对白和心理描写集中在短时长内时，按信息密度决定是否拆分系列脚本；“3 个 15 秒片段”只是本次小说场景的示例，不是通用上限。
- 中文对白、风铃声和精确音画关系需要按本次任务决定由 Grok 生成还是后期完成，并在报告中标明可验证程度。
- 固定镜头、单场景、人物数量、全身构图、发型和身体比例都只是任务字段；只有用户明确要求或首帧一致性确实需要时，才写入 Prompt。
- 当本镜涉及关键道具、角色走位、服装状态、视线关系或相邻镜头交接时，使用 `continuity.anchors` 记录可见起始状态、允许变化、接触/空间槽位和优先级。它适用于群像和多人物互动：不要以“避免错位”为由默认删掉演员、道具或相机运动。
- 连续性锚点应锁定叙事必要关系而不是穷举全部细节。`critical`/`important` 关系进入 Prompt 和 QA，`ambient` 只进入审计；多个可见锚点占用同一只手、前臂、座位或构图槽位时，先明确共享机制或重构首帧，不能让模型自行分配。
- 当剧情依赖进入、离开、靠近、交接、追逐或相对走位时，另设人物与门、柜台、另一角色或镜头之间的空间/路径锚点。起始状态要写清人物朝向和镜头位于哪一侧，终止状态写清目的地和允许的移动；首帧本身必须看得出这条几何关系。单独写“离开”“走过去”不能保证模型理解朝里还是朝外。
- 道具锚点不自动证明人体结构正确。若本镜需要看见承担道具的肢体，必须另设人体锚点，例如“左手完整可见于包体前方、手指自然分开、未被包遮住”。不要从“包挂在前臂”推导出“左手必然完整存在”。这同样适用于多人互动中的手、脚、视线和接触点。
- 当剧情依赖唯一、限量或不可替换的物件发生拿取、放下、换手、递交、穿戴或装入时，使用可选的 `continuity.inventory`：声明物理实例数量、允许转移、转移后的唯一位置和本镜特有的禁止状态。它会同时进入 Prompt、负面约束、风险审计和转移点 QA；不要对普通布景或本来允许双手共同持有的物件默认启用，也不要因道具守恒而删减多人交互。
- 相邻片段使用已验收的上一镜结尾状态作为下一镜 `handoff_from`。状态不一致时，先改目标首帧或显式声明变化；不要把同一份 I2V Prompt 原样重复提交。
- `continuity.mode=strict` 时，先对合成首帧逐项做视觉验收，确认关键接触点、角色位置和道具归属真实存在，才上传并创建付费任务。首帧不满足锚点时重建首帧；不要期望视频 Prompt 自动修复它。
- 上一镜任务成功不等于它的帧能成为下一镜来源。只有通过本镜所需连续性锚点验收的帧，才可成为 `handoff_from` 或 `direct_input`。失败帧的叙事状态可作为 `text_summary`，其像素必须从所有后续生图、I2V 和参考图输入中排除。
- 当任务是实时剧情片段时，添加 `pacing.mode=real_time` 和有时间点的 `pacing.beats`。每个 15 秒镜头应有至少两个可见状态变化和一个剧情事件；把走路、等待、转身等动作写成节拍，不要用“自然地完成”填满时长。实时模式会显式禁止慢动作、时间拉伸和无意义停顿。氛围片、舞蹈、展示片或用户明确要求慢镜时使用 `stylized_slow` 或不启用该规则。
- 如果实时剧情还不能接受长时间摆姿或等待，按本镜设置可选的 `pacing.max_idle_seconds`，并在结果中检验每段可见状态是否在该时限内发生变化。它只约束本次实时叙事；不默认加速慢镜、氛围、舞蹈或产品观察。
- 当用户要求连续剧情、分镜或原著改编时，为每镜添加可选的 `narrative.objective` 和 `narrative.required_events`。它们是本镜必须让观众看懂的事实变化，例如“邻居完成付款”“角色确认付款成功”“角色向邻居道谢”；每个事件都要有相应首帧状态和 `pacing.beat`。不能把走路、摆姿或无关镜头误写成已覆盖核心剧情。纯氛围、舞蹈、产品和不讲故事的片段不应被迫填写。
- 对叙事和对白，先由 LLM 判断情绪是否影响本镜的观众理解；有来源证据时，提炼简短的 `source_evidence`，选择有起点、转折和落点的 `performance.arcs`，再为关键台词创建 `performance.dialogue_beats`。每个关键对白必须写潜台词、声线、可见微表情、可选听者反应和一到三个禁止误读；不要用害羞、深情等孤立标签代替可演绎指令。没有情绪交付目标时省略该字段，不为所有视频预设情绪。完整字段与压缩规则见 `references/performance-direction.md`。
- 是否压平、裁切或重建参考图由 Codex 根据审计结果决定；上传流程本身默认不改图。
- Prompt 采用短英文模板，避免把所有视觉细节堆成一个长叙述。`4096` UTF-8 bytes 只是默认的质量软预算，超出时报告重复或过密风险但不阻断。只有当前 `host + profile + model` 有明确证据时，才在 `provider.max_prompt_bytes` 配置硬限制并拒绝超限请求；不得把本地软预算写成服务端上限。

## 输出协议

执行脚本后输出 JSON，至少包含：

- `risk`: level、score、findings
- `effective_strategy`: 本次参考图角色、换装策略、同步策略，并说明是否需要目标场景首帧
- `downgrades`: 每一项包含原要求和替代方案
- `prompt`、`negative_prompt` 和可直接传给视频 MCP 的 `prompt_for_mcp`
- `performance_plan`: 本镜声明的情绪弧、台词表演节拍及其来源证据；未启用时为空数组。
- `qa_checklist`: 生成后要检查的项目
- `post_production`: 需要剪辑、遮罩或单独配乐完成的项目

调用视频 MCP 时保留用户选定的模型和参数。先读取 `get_video_preflight`，再使用与 profile 对应的字段和限制；不要把 `xai-video-v1`、兼容网关或其他 host 的字段混用。读取任务返回的 `visual_input_mode`：`text_to_video` 表示本次没有视觉输入，`image_to_video` 表示本次提交按首帧语义处理，`reference_to_video` 表示本次提交按独立参考语义处理；它是请求模式回显，不是模型已正确理解图片的证明。创建任务后只轮询同一个 request ID；不要因为内容偏差盲目重复付费任务，先审计并得到用户确认。

## 资源

- `references/design.md`: 架构、数据流、风险模型、扩展路线和审计标准。
- `references/scenespec.md`: SceneSpec JSON 契约、连续性锚点、输入资格账本和多人物示例。
- `references/performance-direction.md`: 情绪判断、对白表演节拍、Prompt 编译与验收规则。
- `references/grok-capability-matrix.md`: 已验证的 Grok 能力边界和降级规则。
- `references/provider-profile.md`: host/profile/model 级能力证据、客户端字段、网关传输和上游契约的分层规则。
- `references/reference-upload.md`: I2V 参考图预检、Uguu 上传和公网 URL 验证流程。
- `scripts/compile_prompt.py`: 确定性编译器和 CLI。
- `scripts/upload_reference.py`: 固定的本地图片上传和 URL 验证工具。
- `scripts/test_compile_prompt.py`: 无网络单元测试。

## CLI

运行脚本前，先把 `<skill-directory>` 解析成该 Skill 的绝对目录，再从绝对路径启动 Python；不要依赖当前工作目录。

```powershell
python <skill-directory>\scripts\compile_prompt.py --input scenespec.json --output report.json
python <skill-directory>\scripts\compile_prompt.py --example complex-outfit --output report.json
python <skill-directory>\scripts\test_compile_prompt.py
python <skill-directory>\scripts\upload_reference.py --input C:\path\to\reference.jpg --output reference-url.json
```

自然语言到 `SceneSpec` 的抽取由 Codex/ChatGPT 完成。编译器不访问网络；只有用户明确要求进行视频生成且需要 URL 时，才运行上传工具。上传工具固定使用 Uguu、`trust_env=False`，并在返回 URL 后重新下载校验。
