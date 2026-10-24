# Skill Repo Relay Fallback 测试 flaky 修复

- 日期：2026-10-24
- 范围：`src-tauri/src/app/skill_repo_update.rs` 内单个测试及其专用 fixture（范围自包含）
- 关联：本机全量 `cargo test --lib` 复跑中唯一失败项；与模型添加兼容修复（MODEL-ADD-FLOW-SIMPLIFICATION-20261023）为不同 bug 类别，故独立记录

## 问题现象

全量 `cargo test --lib` 报：

```
app::skill_repo_update::tests::apply_download_timeout_falls_back_to_relay_for_the_same_archive FAILED
```

relay 回退成功后，verify 阶段连续超时 2 次。日志中所有 attempt 的 `elapsed_ms`（74/65/61/62ms）全部紧贴客户端 60ms 超时阈值。

## 根因

测试设计的时序脆弱性，非产品逻辑缺陷：

1. `skill_hub_fixture_server` 对主源 `cn-mirror-1/archive.zip` 故意 `sleep(180ms)` 制造下载超时（`skill_repo_update.rs` fixture 段）。
2. 测试构造的 reqwest client 超时仅为 **60ms**。
3. 180ms 与 60ms 之间没有安全边际。781 个测试在并行负载 + U 盘 I/O 下，连 relay 的正常请求也被拖过 60ms，verify 连续超时，relay 回退判定失败。

旁证：该测试单独运行 3 次全部 `1 passed`（约 0.2s），证明 relay 回退逻辑本身正确。

## 修复

拉大"触发超时"与"client 超时"之间的安全边际，只动该测试专用路径：

- fixture 主源 archive 延迟 `180ms → 900ms`
- 测试 client 超时 `60ms → 300ms`

效果：主源仍必然超时（900 ≫ 300，回退触发保持确定性）；relay 正常响应为毫秒级，对 300ms 有充足余量，可抵御并行负载抖动。

约束遵守：未新增任何依赖（C2 约束不动）；改动不波及任何其他测试。

## 验证

- 单独运行（修复后）：3/3 通过
- 全量 `cargo test --lib`（781 测试并行）：`775 passed; 0 failed; 6 ignored`

## 本机环境备注（非代码问题）

- 沙箱 PATH 缺 Rust toolchain bin，`autocfg` 构建脚本 spawn 不到 `rustc`（`program not found`）；显式 `RUSTC` + 补 PATH 后解决。
- D 盘 652G 100% 满导致 `os error 112`；E 盘写入报 `os error 31`（设备故障迹象）。复跑使用 C 盘临时 `CARGO_TARGET_DIR`。磁盘本身状态需另行处理。
