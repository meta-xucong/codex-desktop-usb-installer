# G17：能力包 readiness JSON 的 UTF-8 BOM 误报修复

状态：已落地代码修复，待新包在另一台 Windows 测试机复测。

## 1. 现象与结论

测试机报告显示：能力包文件实际已完整安装（50 个 Skills、7 个 Agents、运行时健康），但管理器提示：

```text
能力包核心文件已返回，但 readiness 校验未通过
(Skills: 0, Agents: 0, runtime: unknown, config: unknown)
```

报告确认 `readiness.json` 前三个字节为 `EF BB BF`。这是 Windows PowerShell 5.1 的
`Set-Content -Encoding UTF8` 默认行为：写入 UTF-8 BOM。Rust `serde_json` 直接读取时拒绝该前缀，
解析失败后使用了 0/unknown 的安全回退值，因此这是校验器假阴性，不是 Skills 或 Agents 没有安装。

## 2. 修复策略

### 2.1 写入端根治

能力包安装脚本统一通过 `Write-JsonNoBom` 使用：

```powershell
[System.Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText($path, $json, $utf8NoBom)
```

已覆盖：

- `readiness.json`
- `runtime-state.json`
- `connection-status.json`
- `install-manifest.json`
- 连接向导的 `connection-ownership.json`
- 脱敏诊断文本也改为无 BOM 写入

### 2.2 管理器防御性兼容

Rust 读取 `readiness.json` 和包根 `pack.json` 前统一去除可选 UTF-8 BOM。
这样旧 U 盘或旧安装器已经产生的 readiness 文件也能被识别，不需要用户手工重存文件。

### 2.3 回归测试

新增 Rust 测试覆盖带 BOM 的 readiness JSON：必须识别包身份、50/7 计数、运行时和配置状态。
同时保留无 BOM 输入测试，确保普通 JSON 行为不变。PowerShell 三个修改脚本均通过 Windows PowerShell
语法解析。

## 3. 包清单要求

安装脚本本身属于能力包受管文件。修改后必须重新生成：

- `installer-pack/file-manifest.json`
- `installer-pack/checksums.sha256`

否则安装器会在执行实际复制前拒绝能力包，不能只替换脚本文件。

## 4. 验收标准

1. 新安装生成的 JSON 文件均不以 `EF BB BF` 开头；
2. 旧版带 BOM 的 readiness 仍能被管理器读取；
3. 完整安装结果显示 50 Skills、7 Agents，而不是 0/0/unknown；
4. 未填写可选凭据仍只显示 disabled/skipped/partial，不被解释成核心安装失败；
5. 新 USB 包通过能力包哈希、运行时介质和入口验证；
6. 不写入 U 盘中的用户凭据，也不改变卸载和 U 盘保护边界。
