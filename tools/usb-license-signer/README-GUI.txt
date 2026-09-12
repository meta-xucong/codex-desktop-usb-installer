Codex U 盘签名授权工具
======================

使用方法
--------

1. 在签名电脑上双击 CodexUsbLicenseSigner.exe。
2. 插入已经复制完整发行包的可移动 NTFS U 盘。
3. 等待工具自动扫描；必须只发现一个目标 Bundle。
4. 工具自动找到或初始化本机 DPAPI 签名环境，不需要选择文件或填写 KeyId。
5. 检查通过后点击“一键签名 U 盘”。
6. 等待“签名完成，自动复核通过”，再安全弹出 U 盘。

请使用最新发布目录中的 EXE。若旧版本双击没有窗口，直接换用修复后的版本；启动
诊断记录在：

  %TEMP%\CodexUsbLicenseSigner-startup.log

工具会把 usb-license.json 只写到目标 Bundle 根目录，并在写入后自动回读、验签和
重新检查 Bundle。GUI 只验证 U 盘内部授权签名；PE/MSIX Authenticode 不作为本流程
的签名条件。签名报告保存在本机：

  %LOCALAPPDATA%\CodexUsbLicenseSigner\reports

签名密钥与仅含公钥的状态描述自动保存在：

  %LOCALAPPDATA%\CodexUsbLicenseSigner\keys

安全边界
--------

- 只扫描可移动 NTFS 卷；发现多个 Bundle 时不会签名。
- DPAPI Blob 必须位于本机固定磁盘；私钥不会进入 GUI、U 盘或命令行。
- 工具首次在空的签名环境中自动生成当前用户绑定的 DPAPI 密钥；不会覆盖已有密钥，也不会把私钥打包进发布目录。
- 正式 Bundle 必须声明与管理器构建时公钥相同的 keyId 和公钥摘要；绑定不匹配时自动停止，不会写入许可证。
- Bundle 必须先通过结构、清单、SHA256、Payload 和路径安全检查；GUI 会跳过 PE/MSIX
  Authenticode，但不会跳过 Bundle 完整性检查。
- 不要对已经安装过、含有 apps\Codex 运行树的 U 盘目录签名；请从干净发行包重新复制。
- 签名期间不要拔出 U 盘或运行第二个签名事务。

当前 Bundle 为空、结构错误、是已安装运行树、不是正式 SignedRequired 包，或本机自动
生成/找到的密钥与 Bundle 绑定不一致时，工具会停止并保持签名按钮不可用。这是预期
的 fail-closed 行为；仅跳过系统 Authenticode 不会替代内部公钥验签。

如果出现“本机公钥摘要与安装包不一致”，不要修改 Bundle 或删除 DPAPI 文件绕过检查。
这表示打包时使用了另一套发行公钥；应废弃旧包，使用项目发行 profile 重新打包，再在
持有匹配 DPAPI 私钥的签名电脑上重新复制和签名。

本工作区生成的 GUI EXE 尚未使用组织代码签名证书签名；正式分发前应完成 Authenticode
签名。Windows 可能因此显示 SmartScreen 或“未签名”提示。

详细开发、发行和联调说明见同目录 DEVELOPMENT.md。
