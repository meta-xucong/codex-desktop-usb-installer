# Codex 原生模型目录 M0 探测记录

**日期：** 2026-09-22  
**对象：** 当前发行包中的 `resources/codex.exe`  0.151.0-alpha.7.1  
**目的：** 确认可以使用官方配置契约注入动态模型目录，不猜测内部缓存文件或修改 Codex 二进制。

## 结论

M0 通过，接入级别为 A：当前 Codex CLI 接受官方 `model_catalog_json` 配置字段，并能从该字段指定的 JSON 文件读取模型目录。临时探测使用独立 `CODEX_HOME`，没有改写用户的 `C:\Users\T14S\.codex` 配置，也没有向模型目录写入 API Key。

## 可复现证据

1. 从发行包 `payload/OpenAI.Codex_x64.msix` 只读提取 `app/resources/codex.exe` 到临时目录。
2. 使用临时 `CODEX_HOME` 执行 `codex --version`，结果为 `codex-cli 0.151.0-alpha.7.1`。
3. `codex --help` 暴露 `debug models` 命令；执行该命令可以读取正常的模型目录。
4. 在临时 `config.toml` 写入：

   ```toml
   model_catalog_json = "<临时目录>/catalog.json"
   ```

5. 用一个包含真实模型字段的最小目录执行 `codex debug models`，返回成功并显示目录中的模型；缺少 `support_verbosity` 时会被运行时明确拒绝，补齐该字段后通过。这确认了字段契约和必需字段，而不是只确认文件名存在。

## 实现边界

- 管理器通过已签名 U 盘绑定的 404token 用户 Key 请求 `https://404token.xyz/v1/models`；Key 只存在于请求内存中；
- 目录写入当前用户 Codex 目录下的 `codex-usb-native-model-catalog.json`；配置只增加带 begin/end 标记的受管 `model_catalog_json` 块；
- 网络失败、空列表和配置冲突不会覆盖最近一次成功目录；
- 归一化只合并 `-stable`、`-latest` 和有效日期发布别名，`owned_by` 是去重边界；DeepSeek、GLM、Qwen、罕见厂商以及新模型家族都会保留；
- 卸载时只删除身份字段为 `404token-unified-api-internal` 的本工具目录和受管配置块，其他 provider 配置保留。

## 尚待现场验收

M0 证明了官方本地接入契约；以下项目仍需要真实签名 U 盘和统一网关返回完成：

- `/v1/models` 的实际多厂商列表是否完整到达；
- Codex 页面重新加载后是否显示全部归一化项目；
- 选择 DeepSeek、GLM、Qwen 等模型后抓包/服务端日志中的实际 `rawId`；
- Codex 更新后重新运行本探测并记录新的 app/package 版本。

