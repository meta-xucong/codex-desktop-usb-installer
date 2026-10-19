# 网页正文提取

将一个网页 URL 转换为便于分析的 Markdown。优先使用当前 Codex 的网页读取能力；
批量处理时可使用本目录脚本。

## 本地运行

```powershell
python scripts/fetch.py "https://example.com/article" 30000
python scripts/fetch.py "https://example.com/article" 30000 > article.md
```

`scrapling[fetchers]` 与 `html2text` 是可选增强依赖，不是 Codex 原生网页读取的必需项。`requirements.txt` 只给出最低版本范围，不是版本锁。批处理环境需在获批的运行时准备阶段安装该文件声明的包；当前使用 HTTP `Fetcher`，无需运行 `scrapling install`。只有改用浏览器型 Fetcher 时才需准备浏览器；抓取时不会自动安装或下载任何依赖。默认不保存账号信息、Cookie 或网页历史。
如果依赖缺失、页面需要登录、被 robots 限制或正文不完整，应在结果中说明，而不是猜测缺失内容。
