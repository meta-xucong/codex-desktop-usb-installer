# USB 运行时工具发现与验收

适用：以 usb-onb-progress-1103 为底包的后续发行版；当前接口参考 MSIX 26.1016.8622.0 所带 Computer Use / Browser 技能。本文是验收用例，不替换官方技能、工具声明、用户授权或审批规则。版本或声明不同，先读当前技能，停止照搬不匹配的示例。

## 先确认规则加载

管理器沿现有 USB 启动入口，将 `USB-TOOLS-v1` 通用规则合并到实际受管 `CODEX_HOME/AGENTS.md` 的既有专属块；保留块外用户文本及原有中文/媒体指引。不写普通用户 HOME，不更改 `AGENTS.override.md`、模型、权限或 MCP 注册。

安装后由用户从当前包正常启动，并手工新建普通会话。请该会话：**不要读取磁盘，说明本轮已收到的“运行时工具发现与验收（USB-TOOLS-v1）”指令，并解释顶层无媒体工具及测试代码语法错误应如何判断。** 仅模型复述不构成完整加载证明；同时优先记录该新线程启动返回的 `instructionSources`，或在获准的本轮记录中只确认规则标题存在，不导出会话正文。文件存在不能替代加载证据。若非空全局 `AGENTS.override.md` 生效、指令被上层覆盖/截断或宿主未提供来源，分别如实记录，不移除用户覆盖文件，也不提高长度限制来凑通过。

开发端离线 `thread/start` 检查只证明随包 app-server 在隔离 HOME 的加载行为，不替代最终 USB Desktop 普通新会话验收，不证明模型一定遵循。

## 工具发现、文件和媒体

只有当轮 `exec` 声明确实提供 `ALL_TOOLS` / `tools` / `text` 时，才用下面的发现片段。不预设媒体或 Node 的命名空间，不把 CUA 应用清单、浏览器标签清单当成完整工具表。发现结果、实际调用和端到端结果分别记录。

<!-- snippet: discovery -->
```js
const matches = ALL_TOOLS.filter(tool =>
  /media_capabilities|exec_command|read_file|read_multiple_files|node_repl/i.test(
    tool.name + "\n" + tool.description
  )
);
text(matches);
```

阅读匹配项的完整声明后，按返回注册名调用。没有专用读取工具但存在 shell 时，允许范围内只读取本包 README 前五行；路径来自实际安装位置，不能套用开发机盘符。结果仅证明该路径可读。

媒体只调用声明为只读且允许空参数的 `media_capabilities({})`；不调用 refresh、prepare、submit。下列片段仅解析已取得的 `result`，不会发起调用。成功不等于模型、目录或付费生成就绪。只保留布尔值、预定义状态及数量；不输出响应正文、模型名、URL、凭据或环境值。

<!-- snippet: media-summary -->
```js
let payload = result.structuredContent;
if (!payload || typeof payload !== "object") {
  payload = undefined;
  for (const part of result.content ?? []) {
    if (part.type !== "text") continue;
    try {
      const parsed = JSON.parse(part.text);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        payload = parsed;
        break;
      }
    } catch { /* Text may be a diagnostic rather than JSON. */ }
  }
}
const states = new Set(["fresh", "stale", "not_checked"]);
text({
  toolError: result.isError === true,
  payloadParsed: payload !== undefined,
  businessSuccess: typeof payload?.success === "boolean" ? payload.success : null,
  catalogStatus: states.has(payload?.gateway_catalog?.status)
    ? payload.gateway_catalog.status : null,
  catalogModelCount: Number.isSafeInteger(payload?.gateway_catalog?.model_count)
    ? payload.gateway_catalog.model_count : null,
  configured: Object.fromEntries(
    ["image-2", "grok-image", "grok-video", "seedance-video", "minimax-video"]
      .map(name => [name, typeof payload?.providers?.[name]?.configured === "boolean"
        ? payload.providers[name].configured : null])
  ),
  generationTested: payload?.generation_capability === "not_tested" ? false : null
});
```

解析不到状态写“未知”，不要把 `null` 写成未配置。`isError=true` 或 `success=false` 必须保留为错误，不能被返回了 JSON 掩盖。

## 媒体阶段诊断专项验收（phase-trace-v2-catalog-v1）

本节是针对新诊断版本的独立验收。上面的默认能力发现检查仍不联网。只有用户明确要求目录连接测试时，才按实际声明调用一次 `media_refresh_models({})`；该操作向网关发只读 GET，并沿原逻辑更新本地模型目录缓存，不生成媒体、不改网关或模型路由、不自动重试。

1. 正常退出并重新启动 Codex 后，在主会话发现实际媒体工具，调用 `media_capabilities({})`。兼容解析 structuredContent 或 content 内 JSON，仅记录 transport_diagnostics 的 revision、base_revision、loaded_pid、python_executable、catalog_get_success_trace。预期 revision 为 `phase-trace-v2-catalog-v1`、base_revision 为 `phase-trace-v2`；必须来自本次工具调用，不能用磁盘文件版本替代。旧进程继续运行不算完成更新。
2. 获准进行目录 GET 时，仅提交一次刷新调用，从其返回值的 transport_diagnostic（失败时可能在 details 内）提取：revision、request_attempted、delivery_evidence、异常类型/errno/winerror、HTTP 状态，以及 phase_trace 中 DNS 地址族和候选地址、实际 TCP 目标、事件的 phase/elapsed_ms/duration_ms、failure_phase、process 的 PID/Python 路径。成功和失败都兼容解析 content；不打印完整工具返回、请求头、提示词或配置。没有观测到的阶段记未执行/未知；启用代理时 TCP 目标可能是代理，不能宣称它就是网关地址。
3. 成功时预期 failure_phase=null、last_phase=completed、HTTP 状态已确认；同时分别记录目录刷新结果。api_request_sent/request_attempted 仅为尝试标记，不能当作网关收件或扣费证明。HTTP 响应、目录成功和生图成功是三个不同结论。没有网关请求 ID 时不做 ID 级关联，也不称此前 WinError 10060 已修复。

此验收不调用 prepare/submit，不需要付费生图。开发机模拟测试只证明诊断代码与本地协议行为；测试机加载版本和一次真实目录 GET 应独立记录，无法完成则标未测试。

升级边界：仅换包或重启不等于已经应用能力包更新。现有安装器按归属记录保护手工改过的文件；测试机现场改成 phase-trace-v2 的脚本可能被保留。若更新后仍返回旧 revision，记录安装日志是否为 `Skipped locally modified mcp-server` 或未执行能力包更新，交开发端核对。不要删除归属记录、改校验值、强制覆盖脚本或放宽保护来凑通过。

## Computer Use：先读技能，再分步验收

从本轮技能目录定位 computer-use，读完整 SKILL.md 及其要求的 guidance、api、confirmations；不要硬编码缓存根。原生 node_repl 与 node_repl_local 各自做 `1 + 1`，不能替代对方。指定原生 CUA 时必须使用原生入口。

在新鲜原生 JS 会话内按官方初始化：

<!-- snippet: cua-init -->
```js
if (!globalThis.sky) {
  const { sky } = await import("@oai/sky");
  globalThis.sky = sky;
}
```

下一次调用只枚举，检查实际返回，不把链式代码堆成一大段：

<!-- snippet: cua-list -->
```js
globalThis.usbTestWindows = await sky.list_windows();
nodeRepl.write(JSON.stringify(usbTestWindows));
```

用实际 `list_apps()` 返回的记事本 app id 按 `launch_app({app})` 打开记事本，再枚举；启动可能复用窗口，不能因此宣称已新建空白文档。按实际界面新建空白页/窗口，不能唯一识别或不能确认空白则停止，必要时请用户建立空白窗口。只操作该新空白文档，不操作 Codex 自身、不保存文件。将唯一实际返回的 Window 存为 `usbTestWindow` 后：

<!-- snippet: cua-bind -->
```js
globalThis.usbTestWindow = await sky.get_window({
  id: usbTestWindow.id, app: usbTestWindow.app
});
```

<!-- snippet: cua-activate -->
```js
await sky.activate_window({ window: usbTestWindow });
```

<!-- snippet: cua-observe -->
```js
globalThis.usbTestState = await sky.get_window_state({
  window: usbTestWindow, include_screenshot: true, include_text: true
});
globalThis.usbTestWindow = usbTestState.window;
nodeRepl.write(JSON.stringify(usbTestState.accessibility));
```

检查工具呈现的截图确实是空白记事本。按最新状态选择编辑区 element_index 或截图坐标，执行一次 `sky.click`，刷新状态并核对焦点；禁止套用固定示例坐标。确认焦点后：

<!-- snippet: cua-type -->
```js
await sky.type_text({ window: usbTestWindow, text: "USB CUA TEST" });
```

重新执行观察片段，保存本次截图与可用无障碍文本，文字实际可见才算通过。按测试授权只读记录实际主 helper 映像和时间，区分 MTA 主 helper 与 Swift 辅助进程；不手工启动 helper。人工输入冲突按官方技能刷新状态；截图超时、geometry unavailable 等按本轮约定停止，不连续重试或杀进程。

`Expected ')'` 若在代码解析阶段出现，记“受阻：harness/test-call 语法错误，目标操作未执行”，不能计为 CUA 产品失败。若任务允许纠正后重测，原错误仍保留，不重置其他正常会话。

## IAB：初始化与可见性分开

定位活动技能列表中的 control-in-app-browser，读完整技能。从它的真实插件根取得 `scripts/browser-client.mjs` 绝对路径；不拼测试机旧目录。以下是当前随包接口示例，导入路径须先替换为实际已确认的绝对路径：

<!-- snippet: iab-init -->
```js
const { setupBrowserRuntime } = await import("<confirmed absolute plugin root>/scripts/browser-client.mjs");
const agent = await setupBrowserRuntime();
```

持久绑定不要放进临时代码块；不要用 globalThis 保存 Browser 绑定。首次读取 documentation 的外层 exec 按当前 Browser 技能要求设置足够输出预算并转发完整结果。

<!-- snippet: iab-docs -->
```js
const iab = await agent.browsers.get("iab");
nodeRepl.write(await iab.documentation());
```

<!-- snippet: iab-tab -->
```js
const usbTestTab = await iab.tabs.new();
```

<!-- snippet: iab-goto -->
```js
await usbTestTab.goto("https://example.com/");
```

需要用户看见页面的验收单独执行当前文档给出的 visibility 能力：

<!-- snippet: iab-show -->
```js
await (await iab.capabilities.get("visibility")).set(true);
```

按刚读取的 documentation 获取最终 URL、标题和少量可见正文；预期 Example Domain。没有 h1 不直接判失败，页面读取与前台显示分别报告。没有前台接口时请用户观察；不使用 CUA 点击 Codex、不用外部浏览器/HTTP/自行启动 Playwright 替代。

## 判定回归用例与报告

| 输入事实 | 必须得到的判定 |
|---|---|
| 顶层只有 exec，完整目录中存在媒体注册名 | 工具已发现；未调用前不得称调用成功或缺失 |
| 目录返回另一命名空间前缀 | 使用目录给出的完整名称，不拼固定前缀 |
| 专用 read_file 缺失，exec_command 读指定 README 成功 | 该路径文件读取通过，不泛化到所有路径 |
| media_capabilities 成功只返回 content 文本 JSON | 解析 content；不能因 structuredContent 缺失判失败 |
| 只返回成功包络、configured 字段缺失 | 调用成功；配置状态未知 |
| isError=true / success=false，即使有 JSON | 保留错误，不判能力通过 |
| CUA 调用前 Expected ')' | 受阻（测试调用错误），产品功能未验证 |
| 磁盘 COMMANDS；没有运行时模式接口 | 静态值已读；运行时模式未知 |
| 只查文本日志，未查 SQLite | 仅该文本来源及时间范围未命中，不称完整日志无错 |
| 当前调用成功，但最早没有快照/调用 | 当前通过；最早状态未知，不能编造自行恢复原因 |

每项记录：通过/失败/受阻/未测试、证据来源、时间、实际入口、最小脱敏错误及覆盖范围。配置、文件、进程、工具返回、截图及用户观察分别列明。新会话与正常重启由用户界面操作完成；不制造孤儿进程，不清缓存、不改权限/账号/模式/模型，不为缺证据重装，不运行付费媒体任务。

自动用例只验证示例语法、模拟发现/返回处理与指令合并；不能冒称代理自然决策、CUA 截图、IAB 或真实生成端到端通过。
