Codex U 盘签名授权工具
======================

使用方法
--------

1. 在签名电脑上双击 CodexUsbLicenseSigner.exe。
2. 插入已经复制完整发行包的可移动 NTFS U 盘。
3. 等待工具自动扫描；必须只发现一个目标 Bundle。
4. 签名器发行目录已经携带加密便携签名库；换电脑使用时只需输入签名密码。
5. 检查通过后点击“注册账号并签名”。
6. 等待签名器完成本地回读验签，并确认 VPS 授权显示“已激活”后再安全弹出 U 盘。
   如果网络暂时不可用，签名仍保留在 U 盘，但激活显示“待同步”；重新扫描后点击
   “同步/补发并激活”即可重试。VPS 中已经作废的授权不会被重试恢复。

可选的 404token 一键注册
------------------------

若要同时给该 U 盘绑定 404token 账号，API 地址固定为 `https://404token.xyz`；在窗口底部确认短邮箱域名和预充值金额，
然后点击“注册账号并签名”。管理员 Key 已由受信任部署编译进本次签名器，窗口内只读显示；
目标 CCSwitch 分组固定为 `unified-api-internal`，不再填写分组 ID，也不按模型拆分 API Key。
工具会自动顺延 `USB-0001` 这类编号，创建账号、生成用户 API Key、签名并回读；
新建账号统一使用初始密码 12345678；U 盘编号和卷序列号会显示在检查摘要及成功结果中，已有账号和中断恢复账号不会被重置密码。
签名并回读验证成功后，签名器会自动向 `https://license.404token.xyz` 上传最小化的签名登记凭证，
并在 VPS 建立 active 授权记录。凭证只包含公开的 U 盘编号（如 `USB-0010`），
不会发送完整账号绑定、邮箱、Sub2API 用户 ID 或 API Key；旧凭证可通过“同步/补发并激活”补齐编号，且不会重签许可证。
已有绑定只复用原账号，不修改原密码；只有回读成功后才执行首次预充值。
初始密码不写入 U 盘、
日志或报告。管理员 Key 按以下优先级自动加载：

  运行时 CODEX_404TOKEN_ADMIN_API_KEY / SUB2API_ADMIN_API_KEY
  本次生产 EXE 编译时嵌入的管理员 Key
  （不接受手工输入，也不接受发布目录旁置的管理员 Key 文件）

API 地址和邮箱域名等非敏感偏好会保存到签名工具本机配置区：

  %LOCALAPPDATA%\CodexUsbLicenseSigner\sub2api-config.json

管理员 Key 不进入运行期 U 盘包、许可证或 CCSwitch。已有账号绑定会复用原绑定，重复点击不会隐式
再次预充值。若建号后签名或网络中断，工具会将账号密码和已生成 Key 写入当前 Windows 用户
可解密的 DPAPI 加密恢复记录；下一次对同一物理 Bundle 会续作原账号，不会盲目创建第二个
账号或第二把 Key。当前版本已完成一次明确授权的 404token 测试联调；正式生产操作
仍应先用 0 金额验证接口配置。

恢复记录只用于当前 Windows 用户/当前机器，永远不复制到 U 盘或签名器发布目录；签名成功
并回读验证后自动删除。若恢复记录损坏或当前 Bundle 不匹配，工具会停止并要求人工核对。

如果签名和回读已经成功，但预充值请求失败，工具会在
%LOCALAPPDATA%\CodexUsbLicenseSigner\pending\USB-xxxx.json 保存不含密钥、密码和邮箱的
待处理记录，并启用“重试预充值”。重试只使用原幂等键调用余额接口，不会重新创建账号或
生成 API Key；成功后自动清除记录。若后台不保证 Idempotency-Key 语义，请先在后台确认
余额状态再重试。

签名器 GUI 同时只允许打开一个窗口。若提示已有窗口，请先关闭旧窗口，再从最新发布目录
重新双击 EXE；不要依据旧窗口上的历史公钥摘要判断当前包。

请使用最新发布目录中的 EXE。若旧版本双击没有窗口，直接换用修复后的版本；启动
诊断记录在：

  %TEMP%\CodexUsbLicenseSigner-startup.log

工具会把 usb-license.json 只写到目标 Bundle 根目录，并在写入后自动回读、验签和
重新检查 Bundle。GUI 只验证 U 盘内部授权签名；PE/MSIX Authenticode 不作为本流程
的签名条件。签名报告保存在本机：

  %LOCALAPPDATA%\CodexUsbLicenseSigner\reports

签名工具正式发行不依赖本机 DPAPI 密钥。历史兼容用的本机 DPAPI 密钥与仅含公钥的状态描述
仅由受控的密钥迁移流程保存在：

  %LOCALAPPDATA%\CodexUsbLicenseSigner\keys

当前版本 release profile 会随签名器目录一起交付，签名器只接受与该 profile 和目标 Bundle
同时匹配的密钥。便携签名库保存在 EXE 同目录：

codex-usb-signing-key.vault

codex-usb-signing-key.vault.public.json  （只含公钥绑定，不含私钥）

正式发行目录必须同时包含 EXE、README、DEVELOPMENT、manifest、SHA256SUMS、当前版本
profile、vault 及其 `.public.json`。本次生产 EXE 已把管理员 Key 编译进 EXE，不需要再
旁置管理员 Key 文件即可开箱注册。把完整目录复制到另一台 Windows x64 电脑，输入同一个
密码即可签名；签名器不会读取或比较运行电脑的 DPAPI 公钥，也不依赖机器指纹。密码不会写入
文件、环境变量、命令行或日志。若 vault 缺失，工具会直接提示“请复制完整签名器目录”，
不会退回本机密钥，也不会修改 U 盘。

安全边界
--------

- 只扫描可移动 NTFS 卷；发现多个 Bundle 时不会签名。
- 便携 vault 是 AES-256-GCM + Argon2id 加密文件，不是私钥明文；私钥不会进入 GUI、U 盘、命令行或日志。
- 本次生产签名器 EXE 含有 404token 管理员凭据；它是受信任生产工具，不能公开分发或上传到公共仓库。
- 不要把密码写进脚本或发给他人。丢失 vault 或密码后不能从公钥恢复私钥，只能按密钥轮换流程重新发行。
- 正式 Bundle 必须声明与当前版本 profile、管理器和 signer manifest 相同的 keyId、公钥摘要和版本标识；绑定不匹配时自动停止，不会写入许可证。
- Bundle 必须先通过结构、清单、SHA256、Payload 和路径安全检查；GUI 会跳过 PE/MSIX
  Authenticode，但不会跳过 Bundle 完整性检查。
- 不要对已经安装过、含有 apps\Codex 运行树的 U 盘目录签名；请从干净发行包重新复制。
- 签名期间不要拔出 U 盘或运行第二个签名事务。

当前 Bundle 为空、结构错误、是已安装运行树、不是正式 SignedRequired 包、vault
缺失/损坏/密码错误，或解密出的密钥与 Bundle 绑定不一致时，工具会停止并保持签名按钮
不可用。这是预期的 fail-closed 行为；仅跳过系统 Authenticode 不会替代内部公钥验签。

正式签名器不再显示“本机公钥与安装包不一致”作为发行门槛。若出现 vault 缺失，请复制
完整签名器目录；若出现密码或密钥绑定错误，请使用同一版本的 vault，不要修改 Bundle。

正式发布前，构建脚本会校验当前版本 profile、Bundle、管理器、签名器 manifest 以及
portable vault 的公共绑定；它不会用构建机或运行机的 DPAPI 公钥阻断便携签名器。历史兼容
诊断命令 `CodexUsbLicenseSigner.exe --diagnose-key <临时 JSON>` 只用于排查本机 DPAPI，
不代表正式便携签名器的签名条件；独立 CLI `init-key` 也不能替代 portable vault。

本工作区生成的 GUI EXE 尚未使用组织代码签名证书签名；正式分发前应完成 Authenticode
签名。Windows 可能因此显示 SmartScreen 或“未签名”提示。

详细开发、发行和联调说明见同目录 DEVELOPMENT.md。
