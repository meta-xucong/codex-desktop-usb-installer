# G20：U 盘更新暂存目录被自身完整性校验拒绝

状态：源码修复、回归测试和独立审计完成；本轮不打包、不写入 U 盘。

## 1. 用户目标与范围

用户提供的 G20 报告指出：管理器执行“检查并安装更新”时，下载和解包成功，
但提交前失败；用户要求诊断并修复安装失败问题。

本轮范围：

- 修复 U 盘 bundle 完整性校验对管理器自有便携更新暂存目录的误判；
- 保证提交前校验失败时不会移动或删除已有 Codex；
- 让错误分类与实际的 USB bundle 完整性失败一致；
- 增加回归测试、审计记录和使用边界说明。

本轮非目标：不改变签名算法、不放宽正式 U 盘许可证校验、不执行打包、不写入
U 盘、不处理能力包热更新协议。

## 2. 证据与根因

G20 日志中的稳定错误为：

```text
usb_bundle_invalid phase=portable-usb-precommit:
文件未列入 SHA256SUMS.txt：
apps/.codex-app-manager-staging/portable-<uuid>/payload/<file>.manifest
```

`crates/codex-win-engine/src/portable.rs` 会先把 MSIX 解包到
`apps/.codex-app-manager-staging/portable-<uuid>/`，随后通过
`PortableBoundary::BeforeMoveOld` 进入应用层提交前检查。该目录是管理器拥有的
短生命周期运行期目录，不是发布包的静态内容；但 `usb_bundle::verify_sums` 只
豁免了 `apps/Codex/**` 和 `.staging/**`，于是把暂存文件判定为未列入
`SHA256SUMS.txt` 的未知文件。

当前源码的 `BeforeMoveOld` 检查发生在旧目录移动之前；本轮将该不变量写入
engine 测试，防止已打包旧版本再次出现“先移旧版、再校验”的行为。

## 3. 冻结的修复契约

| 编号 | 要求 | 验收证据 |
| --- | --- | --- |
| C1 | `apps/.codex-app-manager-staging/**` 是受控运行期目录，完整性扫描不把其中的临时文件作为静态清单文件 | `usb_bundle` 单测覆盖目录及嵌套文件 |
| C2 | 运行期豁免只对精确目录生效，近似名称或 bundle 其他位置的未知文件仍被拒绝 | 正/反例路径测试 |
| C3 | USB 提交前校验失败发生在旧安装移动之前 | portable engine observer 失败回归测试 |
| C4 | 更新失败回滚使用既有的受控删除/重命名重试路径，并且没有旧备份时不报告成功回滚 | portable rollback 单测及既有回归测试 |
| C5 | `usb_bundle_*` 错误归类为 bundle 完整性失败，不再显示“更新包签名校验失败” | Rust 分类测试、前端错误表测试 |

## 4. 实施计划

1. 在 `src-tauri/src/app/usb_bundle.rs` 集中补充管理器暂存目录运行期边界。
2. 在 `crates/codex-win-engine/src/portable.rs` 使用已有文件系统重试入口完成
   回滚删除，并增加提交前 observer 失败保持旧目录的测试。
3. 在 `src-tauri/src/errors.rs` 和前端错误文案中新增稳定的 bundle 完整性类别。
4. 更新更新通道说明，明确静态清单与运行期目录的边界。
5. 运行格式化、定向 Rust/前端测试和完整回归；本轮不执行打包。

## 5. 审计要求

审计必须检查：

- 运行期豁免没有扩展到任意 `apps/**` 或任意未知目录；
- 正式许可证、锚点、SHA 清单和 Authenticode 约束仍然存在；
- precommit 失败路径未触碰旧安装目录；
- 新错误 code 已绑定前端文案且仍可重试；
- 失败、成功、回滚和遗留清理测试均绑定到本轮源码版本。

## 6. 当前状态

- 诊断：完成。
- 代码：完成，已通过本契约对应的自动化测试。
- 审计：独立只读复核完成，C1-C5 全部通过。
- 打包/U 盘：明确禁止，等待用户后续与能力包热更新器合并。

## 7. 最终验收记录

### 自动化证据

- `cargo test --manifest-path crates/codex-win-engine/Cargo.toml --lib`：179 passed、1 ignored、0 failed。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`：374 passed、2 ignored、0 failed。
- USB bundle 定向测试：7 passed、1 ignored、0 failed；错误分类定向测试：7 passed、0 failed。
- `npm test`：34 个测试文件、395 passed、0 failed。
- `npm run check`、`npm run lint`、`npm run build`、Rust `fmt --check` 和 `git diff --check`：通过。
- cargo 测试期间存在既有的 `codex-theme-engine` 增量 rmeta 文件复制 `Access denied`
  警告，但 cargo 进程退出码为 0，未造成测试失败或功能改动。

### 独立审计结论

独立只读审计复核 C1-C5：全部 PASS。审计初轮发现的两项问题（暂存根可被伪造成
普通文件、bundle 错误中嵌套权限文本时分类顺序不正确）已修复，并由新增反例测试
和复审确认。

本轮没有执行 Tauri/EXE 打包，没有写入或修改任何 U 盘内容；正式包仍需在后续能力
热更新器合并后重新打包，并在真实签名 U 盘和测试机上做手工更新验证。
