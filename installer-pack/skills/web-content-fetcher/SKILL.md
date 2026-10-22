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

脚本依赖标准库即可运行；可选安装 `scrapling[fetchers]` 和 `html2text` 以增强复杂页面处理。
抓取失败时返回原因，不把未验证的页面内容当成事实。

## 输出约定

- 保留原始 URL 和抓取时间。
- 区分页面正文、导航、广告和脚本噪声。
- 页面不可访问、需要登录或内容被截断时明确标注。
- 微信、社交平台和付费页面遵守站点权限与版权边界。
