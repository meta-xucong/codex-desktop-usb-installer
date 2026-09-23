# CCSwitch U 盘运行时 Key 补丁记录

## 目的

管理器已经把签名许可证中的设备 API Key 以子进程环境变量传给 CCSwitch。CCSwitch 必须在提取 Codex provider 凭据时读取它，并且不能把它写入本地数据库或 `providers.json`。

## 固定基线

- 上游仓库：CC Switch
- 产品当前基线 commit：`1d44b1ba41987cddeb8096fadba8190fab465cbb`
- 读取环境变量：`CCSWITCH_404TOKEN_DEVICE_KEY`
- 仅对 provider id：`404token-sub2api` 生效
- 没有环境变量时，保留原有手工 API Key 配置

## 必须的源码改动

在 `src-tauri/src/proxy/providers/codex.rs` 的 `CodexAdapter::extract_key` 最前面加入：

```rust
if provider.id == "404token-sub2api" {
    if let Ok(key) = std::env::var("CCSWITCH_404TOKEN_DEVICE_KEY") {
        if !key.trim().is_empty() {
            return Some(key);
        }
    }
}
```

该补丁已经在固定基线源码上通过 `cargo check --locked`。构建前必须同时保留本产品对 404token provider、资源清单和窗口行为的现有改造；不能直接拿未带产品化资源的纯上游构建替换随包 EXE。

## 发行前验收

1. 管理器启动 CCSwitch 时，子进程能看到环境变量；
2. 首次进入 404token provider 不弹 API Key 输入框；
3. `providers.json`、数据库和日志中没有设备 Key；
4. 删除环境变量后，手动配置路径仍可用；
5. 更新 `manifest.json` 的 EXE SHA256，并运行 `verify-product.ps1`；
6. 最后重新生成 U 盘包、签名工具配对版本和签名许可证。

## 当前状态

管理器账户卡片、内存登录/刷新/充值调用和环境注入已落地；旧随包 CCSwitch 仍保持产品静态验收通过。新的 CCSwitch 二进制必须从“现有产品化构建链”重建后才能替换，不能使用只通过上游 `cargo check` 的临时产物。
