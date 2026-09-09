# Codex USB 更新中继设计

状态：relay 同步服务、独立 443 edge、TLS 和客户端 Relay/GitHub 自动回退均已完成并通过当前阶段审计；公网路由矩阵已从 VPS 和本机直连验证。剩余关键证据是中国大陆测试机插入 U 盘后的真实 check/apply；Debian 11 生命周期和正式 Authenticode 发布证书仍是运维/发布治理事项。

## 1. 目标

让中国大陆测试机上的 USB 版 Codex 在无法稳定直连 GitHub 时，仍可通过一台菲律宾 VPS 获取最新的 Windows Codex payload，并保留现有客户端的完整校验、事务安装和失败回滚能力。

本方案只解决“USB payload 更新”的网络可达性，不改变离线安装、skill/agent/MCP capability pack 的离线安装语义，也不改变普通 Codex 自更新通道。

## 2. 已核验的 VPS 边界

- 系统为 Debian 11。
- 当前日期为 2026-09-09；Debian 官方已宣布 Debian 11 Bullseye 的 LTS 于 2026-08-31 结束。若继续承载公网 relay，必须先完成升级计划或书面接受明确的安全例外。
- 心理测试站位于 `/opt/wzy-xl-psych`，由 Docker Compose 管理 `web`、`app`、`db` 三个容器。
- `web` 容器使用 `nginx:1.27-alpine`，发布宿主机 80 端口；宿主机没有 systemd Nginx 服务。
- 现有 Web Nginx 使用 `server_name _`，已有 `/`、PHP 等路由；其配置文件通过宿主机路径挂载到容器。
- 根盘约 7.9 GB；部署当前 relay release 后可用空间约 1.7 GB。不能无上限缓存 MSIX，也不能在没有空间预算的情况下并行保留多个完整版本。
- 已部署的变更包括 `/srv/codex-update-relay`、`/etc/codex-update-relay`、一个 root-owned 同步脚本、同步 timer、独立 edge 的 `/opt/codex-update-relay-edge` 配置目录、TLS 证书和证书续期 timer；edge 容器已启动，但未修改心理测试容器、Compose、站点文件、数据库或现有 80 监听。
- 现有 Web Nginx 配置来源是宿主机上的 Compose bind mount；不能在容器内直接修改配置，否则重建后会丢失。

这些事实决定：不能安装一个接管 80 端口的宿主机 Nginx，也不能把更新服务混进心理测试应用容器。更新中继使用独立目录、独立同步任务和独立的只读 edge 容器；现有 Web 容器不需要改配置、不需要 reload，也不共享业务 volume。

## 3. 总体架构

```text
测试机 USB Codex
        |
        | HTTPS：update.404token.xyz/__codex_update__/
        v
VPS 独立 edge/nginx 容器（443）
        |
        | 仅匹配 /__codex_update__/
        v
只读静态目录：/srv/codex-update-relay/export/current

VPS 现有心理测试 web 容器（80）保持独立、不变
        ^
        | 原子发布
独立同步脚本 + systemd timer
        |
        | 仅由 VPS 主动访问
        v
GitHub Release API / Release assets
```

首选实现是“静态镜像中继”，不运行新的常驻代理服务：VPS 定时从指定 GitHub 仓库拉取 release 元数据和允许的 Windows 资产，校验后原子发布；独立 Nginx edge 只负责静态 HTTPS 分发。它新增的唯一公网业务端口是 443，不接管现有 80，也没有新的应用端口、数据库、运行时常驻同步进程或跨业务容器网络依赖，故障面最小。

客户端仍把 MSIX Authenticode、SHA-256、release manifest、MSIX identity、版本单调性和事务回滚作为最终信任边界。中继只改善可达性，不成为新的签名根。

需要明确区分“完整性”和“来源真实性”：仅依赖 HTTPS、VPS 生成的 `latest.json` 和 GitHub 返回的 digest，不能在 relay 被攻陷时证明它没有替换为另一份仍带有效 Authenticode 的旧/非目标资产。因此上线前采用以下两档策略之一：

- 推荐：给 release manifest 增加独立的 Ed25519 发布签名，公钥固定编译进客户端；VPS 只转发并校验签名，不能生成新签名。
- 过渡：不增加新的发布签名时，relay 只能视为可用性缓存；客户端仍必须依赖现有 Authenticode、MSIX identity、版本单调性和 manifest/checksum 校验，并在产品文档中明确“不防 relay 被攻陷后的合法签名包替换”。这不应被描述成新的密码学信任根。

## 4. VPS 隔离设计

### 4.1 文件与权限

建议目录：

```text
/srv/codex-update-relay/
  export/
    current -> releases/<verified-release-id>
    releases/<verified-release-id>/             # Nginx 唯一可见目录
  work/                                         # 下载与 staging
  state/                                        # 状态、锁、发布记录
  backup/                                       # 可恢复配置备份
  logs/                                         # 同步日志，不放 token
```

- 创建专用系统用户 `codexrelay`，同步任务以该用户运行。
- 独立 Nginx edge 容器只读挂载 `/srv/codex-update-relay/export` 到容器内相同的 `/srv/codex-update-relay/export`，不挂载同步配置、`state`、`work`、`backup` 或日志目录；只通过 alias 访问 `export/current`。
- `current` 是指向同一挂载路径内版本目录的相对 symlink，容器内保留相同的 `/srv/codex-update-relay` 绝对路径，确保原子切换后不会固定到旧 bind mount 目标；`releases/` 不直接对外发布。
- 同步脚本、unit 文件和配置由 `root:root` 持有；同步用户不得写脚本、unit、配置或 Docker socket。
- 同步用户无权写 `/opt/wzy-xl-psych`，无权访问心理测试数据库和应用 `.env`。
- 同步脚本使用锁，禁止并发运行；下载超出配额或磁盘安全阈值时先失败，不删除现行版本。
- `current` 和历史 release 位于同一文件系统；用临时 symlink + `os.replace` 原子切换，不能只把 `current` 单独 bind mount 进容器，否则 Docker 可能在挂载创建时固定解析旧目标。
- 默认最多保留当前版本和一个回滚版本；单个大文件下载前检查：下载临时空间 + 当前已发布版本总量 + Docker/日志保留空间 + 固定安全余量。空间不足时必须拒绝发布，不删除现行版本；新版本完整发布并切换 `current` 后才清理最旧的非 current release。
- systemd unit 应启用 `NoNewPrivileges=yes`、`PrivateTmp=yes`、`ProtectSystem=strict`、`ProtectHome=yes`，并只对 `work`、`state` 和 `export` 授予所需写权限；设置网络依赖、超时、随机延迟和失败状态。

### 4.2 Nginx 路由

独立 edge Nginx 只监听 443，并使用独立 server；不修改心理测试 Web Nginx 的配置。目录路由不能只用 `location =`，因此同时处理无斜杠入口和带斜杠目录：

```nginx
location = /__codex_update__ {
    return 308 /__codex_update__/;
}

location ^~ /__codex_update__/releases/ {
    return 404;
}

location ^~ /__codex_update__/ {
    alias /srv/codex-update-relay/export/current/;
    autoindex off;
    add_header Cache-Control "no-store" always;
    add_header X-Content-Type-Options "nosniff" always;
    limit_except GET HEAD { deny all; }
}

location / {
    return 404;
}
```

独立 edge Compose 使用以下只读挂载和端口：

```text
/srv/codex-update-relay/export:/srv/codex-update-relay/export:ro
/opt/codex-update-relay-edge/conf.d:/etc/nginx/conf.d:ro
/srv/codex-update-relay/tls:/etc/nginx/tls:ro
443:443
```

约束：

- 不改变现有 Web 的 `root`、`location /`、PHP 路由、`proxy_pass`、数据库、应用 volume、Compose 文件或 80 端口。
- 更新路由使用不可能与心理测试业务冲突的保留前缀；所有非该前缀请求的路由必须保持原样。
- edge 配置由 `/opt/codex-update-relay-edge/docker-compose.yml` 和只读 bind-mounted `conf.d/default.conf` 管理，`nginx.conf` 是同一配置的宿主机审计副本；禁止 `docker exec` 临时修改。部署前执行 Compose 配置渲染，取得证书后执行 edge 容器内 `nginx -t`，通过后才启动或重建 edge。
- 不依赖现有 Web 的 default server 或容器内路由；edge 明确绑定 `update.404token.xyz`、IPv4/IPv6 的 443，并在启动前确认 443 仍未被其他服务占用。
- 必须在同一 Nginx 镜像中测试 `/__codex_update__`、`/__codex_update__/`、有效文件、未知文件、编码路径、`..`、POST/PUT 和原有首页/API/静态资源；特别检查 `alias` 与现有正则 location/SPA fallback 的交互。
- 若现有公网入口由 Cloudflare/其他网关终止 TLS，客户端配置使用已有可验证的 HTTPS 域名；不能让生产客户端使用裸 IP 或明文 HTTP 下载更新包。
- 独立域名已确定为 `update.404token.xyz`，A 记录指向 `141.11.138.152`；在证书签发前不能开放 relay，不能为了省事复用心理测试站的明文入口或关闭证书校验。
- 若域名由 CDN 终止 HTTPS，还要确认 CDN 不会缓存 `latest.json` 或把 Range/HEAD 处理成不兼容行为；入口 metadata 使用 `no-store`，大文件可按验证过的策略缓存。

## 5. Relay 文件协议

Relay 对外暴露一个固定 base URL，例如：

```text
https://<独立域名>/__codex_update__/
```

`latest.json` 是唯一入口，结构至少包含：

```json
{
  "protocol": 1,
  "repository": "<现有 payload owner>/<repo>",
  "releaseId": 0,
  "tag": "<tag>",
  "publishedAt": "<RFC3339>",
  "assets": {
    "windows-msix": {
      "path": "<固定文件名>.msix",
      "size": 0,
      "sha256": "<lowercase hex>",
      "githubDigest": "sha256:<hex>"
    },
    "release-manifest": {
      "path": "release-manifest.json",
      "size": 0,
      "sha256": "<lowercase hex>"
    },
    "checksums": {
      "path": "SHA256SUMS-windows.txt",
      "size": 0,
      "sha256": "<lowercase hex>"
    }
  }
}
```

实际字段名应在客户端实现前冻结为 Rust 类型，不允许客户端对未知路径拼接任意 URL。服务器只允许发布目标仓库中已明确列入白名单的 Windows x64 资产；不镜像源码包、日志、任意 URL 或用户上传内容。

若采用推荐的发布签名，`release-manifest.json` 采用固定 canonical serialization，签名覆盖 repository、releaseId/tag、架构、package moniker/version、每个 artifact 的规范路径、size 和 sha256；客户端内置公钥和算法，不接受 manifest 自带公钥作为信任根。签名验证失败时 relay 版本不可见、不可下载、不可安装。

同步发布流程：

1. 获取 GitHub latest release，拒绝 draft、prerelease、缺失 manifest/checksum/MSIX 或仓库不匹配的 release。
2. 在同一文件系统的 `staging/<随机 ID>` 下载到临时文件；支持失败重试和 HTTP Range 续传。
3. 计算本地 SHA-256，核对 GitHub asset digest/发布 manifest/Windows checksum，必要时核对 MSIX identity 和目标架构。
4. 完成所有文件后写 `latest.json.tmp`，再通过同文件系统 `os.replace` 原子切换 `current` 指针；入口元数据随完整版本一起生效。
5. 只有新版本完整发布并切换 `current` 后，才清理超过保留数量的旧版本；失败时保留旧 `current`，清理失败不会删除 current。

GitHub token（如未来需要）只能存在 VPS 的受限 root-owned secret 文件或 systemd credential 中，不下发到客户端、不放入 Nginx、同步日志或 Git 仓库。

服务器端还必须保证：版本目录不可变；已发布版本文件不覆盖；`current` symlink 和其中的 `latest.json` 原子切换；坏版本删除只影响后续检查，不替代客户端本地事务回滚。

## 6. 客户端接入设计

### 6.1 Source 抽象

在现有 GitHub payload updater 外增加独立的 `CodexPayloadSource`，至少有：

- `GitHub`：现有直连逻辑，行为保持不变。
- `Relay`：读取 `latest.json`，把相对路径解析到已配置的 HTTPS base URL。

普通 Codex updater 的 `UpdateSource::Custom` 不复用为 relay 配置，避免普通更新和 USB payload 更新互相污染。

配置只允许：

- 内置 relay URL；或
- 用户显式配置且通过 HTTPS、公网域名、路径前缀和 URL 规范化检查的 relay URL。

不能把 GitHub asset 的任意 `browser_download_url` 直接交给 relay，也不能混用不同来源的 metadata、manifest、checksum 和 MSIX。一次 check/apply 事务应锁定一个 source；切换来源时必须重新取得完整 release identity 并重新验证。

`CodexPayloadIdentity` 必须带 source-tagged identity：至少包含 `sourceKind`、规范化后的 `sourceLocator`、release/manifest identity、架构、package moniker、MSIX version、MSIX size、MSIX/checksum/manifest SHA-256。Relay 不伪造 GitHub release/asset ID（其资产 ID 为 `null`）；GitHub 和 relay 使用不同的 identity 分支。这样可以拒绝“从 GitHub check、切到 relay 后 apply 却被误判为同一检查结果”。

Relay 配置只存本机 `settings.json`，不写 USB 根 manifest、不参与 USB anchor 计算；否则单纯切换网络源就会使已授权 bundle 失效。现有 payload IPC 可继续传 opaque identity，不需要新增独立 IPC。

### 6.2 Failover 顺序

推荐顺序：

1. Relay（中国大陆默认优先）。
2. GitHub 直连（relay 不可用时的备用）。
3. 用户明确启用的企业代理/系统代理（沿用现有网络配置）。

公共第三方 GitHub 代理不作为默认信任源；如以后加入，只能作为显式、可禁用的候选，并仍执行全部客户端校验。

失败处理：

- check 阶段可以按候选源顺序尝试，每个源有短超时和冷却时间。
- apply 阶段先重新取得 release identity；必须与用户确认时的目标一致，否则停止并要求重新 check。
- Relay 的 metadata、manifest、checksum 和 MSIX 必须来自同一规范化 base URL/路径；客户端对 Relay 传输明确禁止 HTTP redirect，GitHub 资产仍允许其必要的 CDN redirect；不将任何 redirect 后 URL 写入 identity。
- Relay/GitHub 都失败时，旧 USB bundle 保持不变，界面报告可诊断的网络失败原因。
- 不能因为 relay 返回 200 就跳过 Authenticode、digest、checksum、manifest、MSIX identity 或回滚检查。

### 6.3 版本与安全边界

- Relay 返回的版本号不能覆盖客户端从 manifest/MSIX 解析出的版本。
- 必须拒绝路径穿越、非 HTTPS、跨主机重定向、超大响应和不匹配的 Content-Length（若服务端提供）。
- 下载后继续使用现有 Authenticode 验证；正式证书暂不改变，也不把 relay 签名当作替代。
- 旧版本只有在新版本所有验证通过且原子替换成功后才清理。

## 7. 本地代码改动面

实现阶段按下列顺序修改，避免把 VPS 细节散落在 UI：

1. `src-tauri/src/app/channels/codex.rs`：已完成 payload source、relay descriptor、候选源和同源事务逻辑；覆盖 `resolve_latest`、`check`、`apply`、`same_identity` 和 `ResolvedRelease`。
2. `src-tauri/src/app/settings_store.rs`、`src-tauri/src/state.rs`：已完成 `codexPayloadSourceKind`、`codexPayloadBaseUrl` 的默认值、持久化和旧 settings 兼容迁移；未改变 USB anchor。
3. `src-tauri/src/commands.rs`、`src-tauri/src/app/url_guard.rs`：已完成保存时的 HTTPS、userinfo、控制字符、IP/本地域名和路径规范化检查；Relay 运行时传输使用 HTTPS 且禁止跳转。
4. `src/shared/types.ts`、`src/services/managerApi.ts`：已完成类型、默认值、旧 settings 归一化；renderer 不执行 payload 网络逻辑。
5. `src/app/views/Settings.tsx`、`src/app/i18n.tsx`：已提供 Auto / Relay / GitHub 选择、独立配置字段和中英文文案，不暴露 token 或内部 VPS 信息。
6. `crates/codex-win-engine`：已补充 Relay 禁止重定向下载入口及参数测试；既有 Windows 安全校验未降低。
7. `docs/UPDATE-CHANNELS.md`、`docs/UPDATE-WORKFLOWS.md`、`README.offline.md`：说明 relay 只影响在线 payload 更新，capability pack 仍可离线安装。

部署实现文件：

- `tools/codex-update-relay-sync.py`：VPS 端 GitHub 拉取、资产白名单、大小/SHA-256/digest 校验和原子发布。
- `tools/codex-update-relay-config.json`：当前仓库、架构、空间余量和已发布版本上限。
- `tools/codex-update-relay-sync.service`、`tools/codex-update-relay-sync.timer`：专用用户、systemd 沙箱和六小时同步周期。
- `tools/install-codex-update-relay.ps1`：通过现有加密 SSH wrapper 部署；默认 dry-run，`-RunSync` 才执行实际下载。
- `tools/codex-update-relay-edge.conf`、`tools/codex-update-relay-edge-compose.yml`：独立 443 edge 的精确路径路由、只读挂载和容器硬化。
- `tools/issue-codex-update-relay-cert.sh`、`tools/renew-codex-update-relay-cert.sh`、`tools/codex-update-relay-cert-renew.service`、`tools/codex-update-relay-cert-renew.timer`、`tools/prepare-codex-update-relay-edge.ps1`：使用固定 acme.sh ALPN 签发/续期证书，续期只停止独立 edge，不触碰 80 站点。

## 8. 测试与验收

### 8.1 代码阶段可完成

- relay descriptor 的 schema、URL 解析、主机/路径白名单、路径穿越拒绝。
- source-tagged identity、GitHub→relay 切换后的 stale 拒绝，以及 relay 配置不写 USB manifest/anchor 的回归。
- relay/GitHub source failover、冷却、同源事务和 identity 不一致拒绝。
- metadata 来自 A、asset 来自 B 的混用测试必须失败。
- 断点续传、超大响应、错误 checksum、错误 Authenticode、错误 MSIX identity、旧版本回滚。
- 若启用 Ed25519 manifest 签名，覆盖正确签名、错误签名、错误公钥、签名覆盖字段变化和回退版本拒绝。
- Rust/TS 单元测试、格式化、lint、已有 updater 回归测试。

截至 2026-09-09，已实测通过：

- Relay/GitHub 客户端 Rust 单测：29 项通过。
- Relay 无重定向下载层单测：7 项通过。
- settings_store Rust 单测：10 项通过。
- TypeScript 类型检查、ESLint、前端全量 Vitest：34 个文件、381 项通过。
- `codex-win-engine` 全量回归：175 项通过、0 失败、1 项按真实 MSIX 环境条件忽略；此前的 CRLF 测试夹具误注入问题已修复，并重新跑通。
- `src-tauri` 全量回归：348 项通过、0 失败、1 项忽略；覆盖 capability pack、命令桥及其余后端模块。
- VPS relay 同步已实际拉取并原子发布当前 release；Nginx edge 配置渲染、同镜像 `nginx -t`、隔离容器生命周期和公网路由矩阵均通过。

### 8.2 VPS 部署阶段可完成

- 同步脚本 dry-run：不写现行目录，不触碰心理测试站。
- 错误 checksum 不得发布新 `current`。
- 发布中断不得留下可被客户端选中的半成品。
- 磁盘低于安全阈值时拒绝下载并告警。
- edge Compose 配置渲染、证书到位、容器内 `nginx -t`、精确前缀命中和非前缀回归均已通过；现有 80 站点同时回归通过。
- 证书续期服务实际执行通过；当前证书未到续期窗口时 acme.sh 返回跳过码 2，脚本按“无需续期”成功处理；timer 已启用且 active。
- 更新脚本用户无法读取心理测试 `.env`/数据库目录。
- timer 单实例、日志脱敏、重启后状态可恢复。

### 8.3 必须在中国大陆测试机实测

- U 盘版 Codex 通过真实 HTTPS relay check/apply 成功。
- relay 不可达时 GitHub 直连或用户代理的实际回退表现。
- 真实 MSIX 下载、Authenticode、安装/回滚和重启后的 bundle 一致性。
- 不同网络、DNS、证书链、运营商连接质量下的超时与错误提示。

## 9. 回滚方案

任何 VPS 变更都必须可逆：

1. 停止/禁用 relay timer。
2. 恢复 Web Nginx 配置和 Compose volume 变更。
3. 执行配置校验后 reload；不删除心理测试容器、数据库或业务目录。
4. 保留 relay 目录作为取证材料；只在确认无回滚需求后按精确路径清理。

客户端遇到任何 relay 异常时不得删除旧 bundle，不得改写 capability pack，不得影响离线启动。

## 10. 剩余外部前置条件与治理事项

relay 同步服务已经部署并成功发布当前 release，公网 HTTPS relay 也已开放。以下是仍需在长期生产化前处理的事项：

- 独立子域名已选定为 `update.404token.xyz`，A 记录指向 `141.11.138.152`；当前从 VPS、1.1.1.1、8.8.8.8 和 9.9.9.9 均解析正确。
- TLS 已由固定版本的 acme.sh 使用 TLS-ALPN-01 签发，当前证书有效期为 2026-09-09 至 2026-12-08；未登记通知邮箱，因此续期 timer 可自动续期，但不会提供到期邮件提醒。首次签发/续期脚本会只协调独立 edge 的 443，不停止心理测试站 80。
- 公开 relay base URL 固定为 `https://update.404token.xyz/__codex_update__/`，仍需在中国大陆测试机验证可达性。
- Debian 11 升级到受支持版本的窗口、备份和回滚方案；在升级或安全例外未确认前，不将该 VPS 作为长期公网更新源。
- 约 1.7 GB 可用空间下的 MSIX 实际大小、`/var/lib/docker`/日志/inode 占用和保留策略。硬门槛应满足：`可用空间 >= 下载临时副本 + 当前版本 + 回滚版本 + Docker/日志保留 + 安全余量`；若不够，应扩容或使用对象存储，不能强行缓存。
- 更新源的可信链：推荐固定发布 manifest 的 Ed25519 公钥与签名格式；若暂不增加发布签名，必须接受 relay 被攻陷时无法阻止“另一份合法签名包”替换的风险，并不得把该模式宣传为密码学安全镜像。

公网 relay 已完成对外提供；在长期生产化前仍应处理 Debian 11 生命周期、容量预算和发布签名治理，不能把这些事项误称为客户端代码已解决。

## 11. 当前实施与审计结果

- GitHub 出站连通性：通过；API 和 release 页面返回 200。
- 首次实际同步：通过；MSIX、release manifest、Windows checksum 和 GitHub digest 校验后原子发布。
- 幂等同步：通过；重复触发返回 `already current`，不重复下载。
- 业务隔离：通过；心理测试 Web/App/DB 容器保持运行，Web 容器 ID 和 80 端口未变化；`codexrelay` 无法读取心理测试 `.env`、写入业务目录或读写 Docker socket。
- DNS：通过；VPS 和 1.1.1.1、8.8.8.8、9.9.9.9 均返回 `141.11.138.152`，未发现 AAAA 记录。
- edge 公网部署：通过；独立目录、Nginx 路由、Compose 配置、Let’s Encrypt TLS 证书和续期 timer 已部署，Compose schema 与同镜像 `nginx -t` 通过；edge 容器监听 443，心理测试容器 ID、80 端口和站点根路径回归未变化。
- 客户端 Relay 接入：通过代码审计；支持 Auto（Relay 优先、GitHub 回退）、Relay-only、GitHub-only；Relay 描述符、同源资产、SHA-256、版本、MSIX identity、Authenticode 和事务回滚均保留。
- Relay 跳转边界：通过代码审计；Relay 使用不跟随重定向的 metadata/checksum/MSIX 传输，GitHub 仍允许其 CDN 跳转。
- 代码回归：Relay/GitHub 客户端 29 项、下载层 7 项、设置存储 10 项通过；`src-tauri` 全量 348 项通过、0 失败、1 项忽略；前端全量 34 个测试文件、381 项通过；`codex-win-engine` 全量 175 项通过、0 失败、1 项忽略。此前的 CRLF 测试夹具问题已修复并纳入最终回归。
- 当前 relay 已占用约 760 MB；可用空间约 1.7 GB。最新空间预检已实际运行：下一份约 760 MB release 需要约 2.13 GB 可用空间（新下载 796 MB + 当前已发布 796 MB + 512 MB 安全余量），当前仅约 1.73 GB，因此同步器安全拒绝本次 dry-run；这说明容量是当前公网持续更新的外部运维门槛，不能靠删除现行版本规避。
- 公网 HTTPS/Nginx 路由：已从 VPS 直连和本机绕过代理直连验证；`latest.json=200`、无斜杠入口 `308`、发布目录与根路径 `404`、POST `403`、MSIX Range `206`，响应头包含 HSTS/nosniff/no-store；客户端可开始进行真实测试机 relay check/apply 验证。
