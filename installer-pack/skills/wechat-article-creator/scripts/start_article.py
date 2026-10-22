#!/usr/bin/env python
"""
Quick start a new WeChat article with template
"""
import argparse
import os
from pathlib import Path
from datetime import datetime


def resolve_article_data_dir() -> Path:
    explicit = str(os.environ.get("WECHAT_ARTICLE_DATA_DIR") or "").strip()
    if explicit:
        return Path(explicit).expanduser().resolve()
    root = Path(os.environ.get("CODEX_DATA_DIR", Path.cwd() / "codex-data"))
    return (root.expanduser() / "wechat-article-creator").resolve()


def safe_filename_component(value: str) -> str:
    safe = []
    for char in str(value or "").strip():
        if char.isalnum() or char in {"-", "_"}:
            safe.append(char)
        elif char.isspace():
            safe.append("_")
    normalized = "".join(safe).strip("._-")[:80]
    return normalized or "article"


def create_article(title: str, topic: str = ""):
    """Create new article file from template"""
    drafts_dir = resolve_article_data_dir() / "drafts"
    drafts_dir.mkdir(parents=True, exist_ok=True)
    
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S_%f")
    filename = f"{timestamp}_{safe_filename_component(title)}.md"
    
    template = f"""# {title}

## 写作计划

**主题：** {topic or "待定"}

**目标读者：** 

**核心观点：** 

**关键数据/案例：**
- 

---

## 文章正文

### 开头 (100-150字)

*用故事、数据或问题勾起兴趣。直白说明文章解决什么问题。*

---

### 主体观点

#### 观点 1

*观点 + 案例 + 解释*

---

#### 观点 2

*观点 + 案例 + 解释*

---

#### 观点 3

*观点 + 案例 + 解释*

---

### 结尾 (100-150字)

*总结核心观点,留下思考或行动建议*

---

## 发布前检查清单

- [ ] 标题是否能引起好奇心或共鸣?
- [ ] 开头 30 秒能抓住读者吗?
- [ ] 有具体故事或数据吗?
- [ ] 避免了大白话或学术语言吗?
- [ ] 段落长度合适吗?
- [ ] 有清晰的结尾吗?
- [ ] 检查了错别字和语病吗?

## 提示

**口语化、故事化、有观点写作风格:**
- ✅ 口语化,像跟朋友聊天
- ✅ 用故事代替说教
- ✅ 有态度和观点
- ✅ 简化复杂的东西
- ✅ 短句 + 多换行

**避免:**
- ❌ AI 八股文
- ❌ 模棱两可
- ❌ 数据堆砌
- ❌ 长句子
- ❌ 空洞的词汇
"""
    
    file_path = drafts_dir / filename
    file_path.write_text(template, encoding='utf-8')
    
    print(f"Article created: {file_path}")
    print("\nNext steps:")
    print(f"   1. Edit the article: {file_path}")
    scripts_dir = Path(__file__).resolve().parent
    print(f"   2. Polish the text: python \"{scripts_dir / 'polish_text.py'}\" \"{file_path}\"")
    print(f"   3. Generate cover: python \"{scripts_dir / 'generate_cover.py'}\" --title <title> --style modern")
    
    return str(file_path)


def main():
    parser = argparse.ArgumentParser(description="Create new WeChat article")
    parser.add_argument("title", help="Article title")
    parser.add_argument("--topic", default="", help="Article topic (optional)")
    
    args = parser.parse_args()
    create_article(args.title, args.topic)


if __name__ == "__main__":
    main()
