---
name: web-content-fetcher
description: 把指定网页提取为干净 Markdown，保留标题、链接、图片、列表、代码块和引用结构。
---

# 网页正文提取

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行示例中的 `scripts/...` 时使用其绝对路径，
不要假设当前工作目录是仓库根目录。

给定一个 URL 后，优先使用当前 Codex 提供的网页读取能力；需要批处理或离线复用时，使用本目录
的 `scripts/fetch.py`。脚本不读取本机平台配置，也不把 Cookie 或账号信息写入输出。

Codex 原生网页读取是本技能的核心路径，不需要 Python。只有需要批处理、复杂 HTML 选择器或本地复用
时才启用 Python 适配器；适配器缺失时必须明确降级，不得把“未抓到”写成“没有内容”。

## Python 脚本

```powershell
python scripts/fetch.py "https://example.com/article" 30000
python scripts/fetch.py "https://mp.weixin.qq.com/s/xxx" 30000
```

脚本入口只用标准库解析参数，但真正抓取需要可选依赖 `scrapling[fetchers]` 和 `html2text`。本目录 `requirements.txt` 只声明最低版本范围，不是锁定文件，因此依赖解析不保证可复现。当前脚本使用基于 HTTP 的 `Fetcher`，无需安装浏览器；只有改用 `StealthyFetcher` 等浏览器型 Fetcher 时才需要在获批的运行时准备阶段执行 `scrapling install`。不得在一次抓取请求中自动安装或下载依赖。缺少 Python 包时，脚本会在发起页面请求前失败并说明缺失依赖；这时优先使用 Codex 原生网页读取能力，或向用户说明批处理适配器不可用。不得把未抓取写成“没有内容”。`max_chars` 必须是正整数；CLI 和 Python API 都会在导入可选依赖或发起网络请求前校验，非法 CLI 参数会显示简短用法错误。有效的正整数会按指定字符数截断输出。
抓取失败时返回原因，不把未验证的页面内容当成事实。

## 输出约定

- 保留原始 URL 和抓取时间。
- 区分页面正文、导航、广告和脚本噪声。
- 页面不可访问、需要登录或内容被截断时明确标注。
- 微信、社交平台和付费页面遵守站点权限与版权边界。
