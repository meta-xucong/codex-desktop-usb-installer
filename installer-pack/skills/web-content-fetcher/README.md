# 网页正文提取

将一个网页 URL 转换为便于分析的 Markdown。优先使用当前 Codex 的网页读取能力；
批量处理时可使用本目录脚本。

## 本地运行

```powershell
python scripts/fetch.py "https://example.com/article" 30000
python scripts/fetch.py "https://example.com/article" 30000 > article.md
```

`scrapling[fetchers]` 与 `html2text` 是可选增强依赖，不是核心调用的必需项。默认不保存账号信息、Cookie 或网页历史。
如果页面需要登录、被 robots 限制或正文不完整，应在结果中说明，而不是猜测缺失内容。
