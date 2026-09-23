# Codex 原生模型目录接入统一网关开发文档

**文档版本：** 1.2  
**状态：** `IMPLEMENTED_PENDING_USB_GATEWAY_FIELD_TEST`  
**关联任务：** `G39.5-R7-FIELD-REPAIR-v1`  
**日期：** 2026-09-22  
**复杂度门禁：** `ESCALATE_REQUIRED`

## 0.1 2026-09-23 根因修复记录

现场“模型列表暂时无法刷新”并不是重试次数不足。项目统一使用
`reqwest` 的 `rustls-no-provider` 特性，而原生模型目录模块在创建 HTTPS
客户端前没有安装 Rustls Provider；在首次刷新早于账户模块发起网络请求时，
`reqwest::Client::builder().build()` 会直接触发
`No rustls crypto provider is configured`，请求因此根本不会到达 404token。

已在 `src-tauri/src/app/model_catalog.rs` 增加独立的网关客户端初始化，先安装
`rustls::crypto::ring` Provider，再创建客户端。回归证据包括：客户端构建测试、
本地 HTTP fixture（验证 Bearer 头和 `data[]` 响应解析）、临时 Codex 目录原子写入
测试，以及 `codex debug models` 读取生成目录测试。真实签名 U 盘上的有效 API Key
鉴权仍需在现场完成一次；没有密钥时对生产端点得到 401 是预期边界，不再被误判为
本地刷新故障。

## 1. 目标

让用户在 Codex 官方原生页面的模型选择菜单中看到当前 `unified-api-internal` 网关实际提供的模型，而不是只看到少数旧 GPT 模型。

目标模型来源为当前 provider 的 OpenAI 兼容接口：

```text
GET <provider.base_url>/models
```

在当前 404token 配置中，实际地址为：

```text
https://404token.xyz/v1/models
```

模型目录必须满足：

- 不限制 GPT、DeepSeek、GLM、Qwen 或其他厂商；
- 上游新增模型无需重新硬编码白名单；
- 只合并明确属于同一模型的发布别名；
- `mini`、`pro`、`reasoner`、`coder`、`vision`、`instruct`、`thinking` 等能力后缀保持独立；
- 展示名称和实际请求 ID 分离；
- Codex 版本变化后，目录桥接失效时必须安全降级，不能阻断启动。

## 2. 当前事实与非解法

### 2.1 当前已经存在的能力

CCSwitch 的 Codex provider 编辑器已经可以：

1. 使用当前 provider 的 API Key 请求 `/v1/models`；
2. 按 `owned_by` 分组；
3. 合并 `latest`、`stable` 和有效日期后缀；
4. 在选择时保留原始模型 ID。

涉及文件：

- `CCSwitch/CCSwitch/src/lib/api/model-fetch.ts`；
- `CCSwitch/CCSwitch/src/components/providers/forms/CodexFormFields.tsx`；
- `CCSwitch/CCSwitch/src/components/providers/forms/shared/ModelInputWithFetch.tsx`；
- `CCSwitch/CCSwitch/src/lib/model-aliases.ts`。

### 2.2 当前缺口

以上链路只影响 CCSwitch 的 provider 编辑器。它不会自动修改：

- 官方 Codex 页面自己的模型菜单；
- Codex 内部的模型目录缓存；
- Codex UI 与 `resources/codex.exe` 之间的模型能力列表；
- Codex 官方页面中的默认模型显示。

当前 provider bootstrap 只写入一个默认值，例如：

```toml
model = "gpt-5.4"
```

修改这个默认值只能改变当前默认模型，不能生成完整下拉菜单。因此以下做法均不视为解决方案：

- 只修改 `model = "gpt-5.4"`；
- 只刷新 CCSwitch provider 编辑器；
- 只更新 `installer-pack/model-catalog.json`；
- 把网关模型名硬编码进安装器；
- 把所有日期后缀模型粗暴删除；
- 修改 API Key 注入逻辑来期待模型菜单自动变化。

## 3. 总体方案：模型目录桥接层

实现分为“目录获取与归一化”和“Codex 原生目录适配”两层：

```text
404token /v1/models
        ↓
GatewayModelCatalog
        ↓
ModelNormalizer
        ↓
CodexNativeCatalogAdapter
        ↓
Codex 原生模型选择菜单
```

CCSwitch 的编辑器刷新功能可以复用获取和归一化逻辑，但不得把编辑器的 React state 当作 Codex 原生目录的持久化接口。

## 4. 第一阶段：确认 Codex 原生接入点

这是实现前的硬门禁。当前发行包已完成只读探测，确认了 Codex 官方 `model_catalog_json` 配置字段，因此本实现使用 A 级接入，不猜测内部缓存文件、IPC 命令或修改 Codex 二进制。完整证据见 [`CODEX-NATIVE-MODEL-CATALOG-M0-PROBE-20260922.md`](CODEX-NATIVE-MODEL-CATALOG-M0-PROBE-20260922.md)。

### 4.1 运行时探测内容

针对当前发行包中的 `ChatGPT.exe`、前端资源和 `resources/codex.exe`，只读记录以下证据：

1. Codex 页面加载时是否请求模型目录 HTTP 接口；
2. 请求是否经过配置的 `model_providers.<name>.base_url`；
3. 模型目录是否来自本地文件、IPC、后端命令或远端服务；
4. 页面选择模型时最终写入的是哪个配置字段；
5. 模型 ID 是否经过能力白名单校验；
6. Codex 版本升级后上述路径和 schema 是否变化。

当前探测结果：`resources/codex.exe --version` 为 `codex-cli 0.151.0-alpha.7.1`；`debug models` 能读取由临时 `config.toml` 指向的模型目录。临时探测使用独立 `CODEX_HOME`，未修改用户目录。

### 4.2 接入点分级

| 级别 | 发现的接口 | 处理方式 |
|---|---|---|
| A | 官方配置或公开 provider `/models` 机制 | 优先直接接入，不修改 Codex 文件 |
| B | 稳定本地目录/IPC，且可通过版本探测确认 schema | 由桥接层生成目录，带版本门禁 |
| C | 仅能通过版本特定前端资源适配 | 单独生成版本适配包，禁止静默覆盖原始文件 |
| D | 没有安全、稳定、可验证的接入点 | 本功能阻断，不伪称已完成 |

只有 A 或 B 可以进入常规实现。C 必须经过单独的供应链、签名和升级评审；D 必须保留 CCSwitch 编辑器刷新作为独立能力。

## 5. GatewayModelCatalog 数据契约

桥接层内部使用以下逻辑结构，不直接把 provider 配置和 API Key复制到目录文件：

```ts
type GatewayModel = {
  rawId: string;
  displayName: string;
  ownedBy: string | null;
  canonicalId: string;
};

type GatewayModelCatalog = {
  schemaVersion: 1;
  providerId: "404token-sub2api";
  providerBaseUrl: string;
  codexVersion: string;
  fetchedAt: string;
  models: GatewayModel[];
};
```

约束：

- `rawId` 是真正发给上游的模型 ID；
- `displayName` 只用于菜单展示；
- `canonicalId` 只用于同类发布别名合并；
- 不保存 API Key、Authorization header 或账号密码；
- provider 切换或 base URL 改变时，目录必须按 provider 重新隔离；
- 目录读取失败时保留最近一次成功目录，并展示更新时间和“目录可能过期”状态；
- 空列表不能覆盖已有可用目录，除非用户明确执行清空操作。

## 6. 模型归一化规则

### 6.1 规范化

只处理明确的发布别名尾缀：

- `-latest`；
- `-stable`；
- 有效日期 `-YYYY-MM-DD`；
- 有效日期 `-YYYYMMDD`。

不处理：

- 厂商名称；
- `mini`、`pro`、`reasoner`、`coder` 等能力后缀；
- 不符合有效日期的数字尾缀；
- 小众厂商或看起来罕见的模型；
- 未知的新模型家族。

### 6.2 去重边界

去重键必须包含 `owned_by`：

```text
caseFold(trim(ownedBy or "Other")) + "\u001f" + caseFold(canonicalId)
```

因此：

- `deepseek-chat`、`deepseek-chat-latest`、`deepseek-chat-20260101` 合并；
- `deepseek-chat` 与 `deepseek-reasoner` 保持独立；
- `glm-5` 与 `glm-5-pro` 保持独立；
- `gpt-6` 新出现时自动进入目录；
- 不同 provider 或不同 `owned_by` 的同名模型不跨边界合并。

### 6.3 原始 ID 选择

同一展示项存在多个原始 ID 时，优先级为：

1. 精确基础 ID；
2. `-stable`；
3. `-latest`；
4. 有效日期中最新者；
5. 大小写不敏感的字典序。

用户手工输入的模型 ID不能被刷新动作静默覆盖。刷新只更新菜单和目录，不擅自替换用户当前模型。

## 7. CodexNativeCatalogAdapter

### 7.1 适配器职责

适配器只负责把已经归一化的目录交给 Codex 原生界面，不负责：

- 账号注册；
- API Key 生成；
- 余额或额度；
- 模型可用性保证；
- 改写上游请求内容。

### 7.2 首选实现

如果第一阶段找到 A/B 级接口，按以下顺序实现：

1. 管理器启动并确认 provider 已启用；
2. CCSwitch/管理器获取 `/v1/models`；
3. 归一化并校验模型目录；
4. 通过已确认的官方配置或 IPC 接口提交目录；
5. Codex 页面重新加载模型菜单；
6. 用户选择模型后，验证实际请求使用 `rawId`；
7. 记录 Codex 版本、目录版本和 provider 标识，不记录密钥。

当前实现已完成上述 1–5 项：`src-tauri/src/app/model_catalog.rs` 获取并归一化 404token `/v1/models`，生成官方目录 JSON，原子写入受管配置块；管理器安装页后台刷新一次，并提供手动刷新按钮。目录项的 `slug` 保留真实 `rawId`，展示名独立生成。

### 7.3 刷新时机

保持最小、可理解的三种入口：

1. 首次启动 provider 后后台刷新一次；
2. CCSwitch/管理器提供明确的“刷新模型”按钮；
3. 目录超过固定 TTL 后，在下次启动后台刷新一次。

刷新失败不能阻断 Codex 启动，也不能清空最近一次成功目录。用户必须能看到“上次成功刷新时间”和“重试”入口。

### 7.4 版本门禁

适配器必须保存并检查：

- Codex app version；
- MSIX package version；
- 目录 schema version；
- provider 配置身份。

当前 A 级实现不修改版本特定资源，目录文件带有 `schema_version`、`client_version` 和 provider `identity`；管理器只在目录为空、过期或用户强制刷新时请求网关，失败时保留旧目录。Codex 更新后，发布验收仍必须重新运行 M0 探测并记录版本；若官方字段或必需字段发生变化，必须先停用目录写入并回退默认菜单。

发现 Codex 版本变化且适配器尚未验证时：

- 不写入未知文件；
- 不修改官方资源；
- 保留旧目录但标记为兼容性未知；
- 回退到 Codex 默认模型菜单；
- 日志明确写出 `native_model_catalog_unsupported`。

## 8. C 级兼容适配的限制

如果只能通过版本特定的 Codex 前端资源适配，必须另行满足：

- 适配对象有固定版本、文件 hash 和回滚副本；
- 原始上游包和修改后文件的来源关系可审计；
- 每次 Codex 更新都重新执行探测和生成；
- 适配失败时保留原始 Codex 启动能力；
- 不通过十六进制替换、未记录的注入或全局代理劫持绕过校验；
- 发行清单明确记录适配层版本；
- 签名、SHA256、更新和卸载流程同步更新。

在上述条件没有实现前，不允许采用 C 级方案进入正式 U 盘包。

## 9. 测试与验收矩阵

### M0：运行时探测（已完成）

- 记录当前 Codex 版本和 package version；
- 记录模型目录请求/本地读取/IPC 证据；
- 形成可复现的接口说明；
- 当前证据确认 A 级官方配置接入；没有证据的后续版本仍标记 D 级阻断。

### M1：归一化单元测试（已完成）

输入至少包含：

```text
gpt-6
gpt-6-20260901
deepseek-chat
deepseek-chat-latest
deepseek-reasoner
glm-5
glm-5-pro
qwen-next
罕见厂商模型-20260101
```

验收：

- `gpt-6` 自动出现；
- 日期/`latest` 同类合并；
- DeepSeek、GLM、Qwen 均保留；
- `reasoner`、`pro` 不被合并；
- 罕见厂商不被过滤；
- 发送值仍为真实 `rawId`。

### M2：目录和密钥隔离（已完成源码测试）

- 目录文件不包含 API Key；
- 日志不包含 Authorization header；
- provider 切换不会串用旧目录；
- 空列表或网络失败不会覆盖最近一次成功目录。

### M3：Codex 原生菜单（待真实 U 盘字段验收）

- 官方 Codex 页面显示网关返回的多厂商模型；
- 新增模型无需修改前端硬编码名单即可出现；
- 选择 DeepSeek/GLM/Qwen 后抓取请求，确认实际模型 ID 正确；
- 刷新后用户当前选择不被无故替换；
- Codex 版本不支持适配器时安全降级且仍可启动。

### M4：升级回归（代码和发行包已接入，待现场验收）

- Codex 更新前后目录桥接均可恢复；
- 适配器版本不匹配时不写未知格式；
- 更新失败不损坏旧目录和旧 Codex；
- 卸载时清理本产品生成的目录，但不删除用户其他 provider 数据。

## 10. 允许改动与明确不做

允许改动：

- 独立的模型目录类型、获取、归一化和缓存模块；
- 已确认的 Codex 原生配置/IPC 适配层；
- CCSwitch provider 刷新链路的复用接口；
- 模型菜单、刷新状态和错误提示；
- 对应 Rust/TypeScript 测试、版本探测和发布清单。

明确不做：

- 不硬编码 GPT/DeepSeek/GLM/Qwen 白名单；
- 不把 `installer-pack/model-catalog.json` 冒充运行时动态目录；
- 不把 API Key 写入 U 盘、签名文件或模型目录；
- 不因“看起来罕见”删除模型；
- 不在未确认官方接入点时猜测 `models_cache.json` 等路径；
- 不把 CCSwitch 编辑器刷新验收当作 Codex 原生菜单验收。

## 11. 当前状态与交付门禁

当前状态：


- 目录归一化规则已经明确；
- CCSwitch 编辑器刷新能力已经存在；
- Codex 原生目录接入点已通过 M0 运行时探测；
- 动态桥接、别名归一化、API Key 隔离、受管配置和卸载清理已实现；
- 管理器已编译为 Windows x64 USB 版本，模型刷新按钮和后台刷新入口已进入 `release/usb-oneclick-g39.5-20260922-r8-final`；
- 包内模型桥接标识、CCSwitch 统一网关环境变量注入、SHA256/签名绑定和安装清单验证均已通过；
- 未修改 Codex 发行文件；
- 真实统一网关返回、原生页面菜单和更新后版本回归仍是交付前门禁。

进入发行包前必须保留 M0 探测证据。若未来版本不能证明 A/B 级接入，必须保持阻断，不能用 C 级二进制修改替代证据。
