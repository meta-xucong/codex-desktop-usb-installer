---
name: daily-reflection
description: 日复盘助手，帮助用户记录每日反思、追踪目标进度、管理情绪。支持每日三问、周月复盘、习惯追踪，促进个人成长。
---

# Daily Reflection - 日复盘助手

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行示例中的 `scripts/...` 时使用其绝对路径，
不要假设当前工作目录是仓库根目录。

## Overview

这位技能帮助用户建立每日反思习惯，通过结构化的问题引导，记录成长轨迹，追踪目标进度，管理情绪变化。温柔陪伴，不强求，让反思成为自然的生活方式。

## When to Use This Skill

- 每日睡前/早晨进行日复盘
- 记录感恩时刻和成就
- 追踪目标完成进度
- 管理情绪和压力
- 周/月复盘总结
- 建立和追踪习惯
- 重要决策前的思考整理
- **制定和管理阅读计划**
- **获取课程推荐和学习路径**
- **心理建设和情绪调节**

## Workflow

### Step 1: Check for Existing Profile

检查用户是否已有复盘档案：

```bash
python scripts/reflection_db.py is_initialized
```

If "false", proceed to Step 2 (Setup). If "true", proceed to Step 3 (Daily Reflection).

### Step 2: Initial Profile Collection

收集用户复盘偏好：

**基础信息：**
- 希望每天什么时间复盘？（睡前/早晨/其他）
- 偏好的复盘形式？（文字/要点/语音转文字）
- 复盘提醒方式？（不提醒/温和提醒/严格提醒）

**目标设定：**
- 短期目标（1-3个月）：如"读完3本书"、"养成早起习惯"
- 中期目标（3-12个月）：如"减重5kg"、"通过某考试"
- 长期愿景（1年以上）：如"成为某个领域的专家"

**习惯追踪：**
- 想要建立的习惯（最多5个，避免贪多）
- 每个习惯的频率（每天/每周几次）
- 习惯完成的标准

**保存档案：**

```python
import sys
sys.path.append('[SKILL_DIR]/scripts')
from reflection_db import save_profile

profile = {
    "preferred_time": "21:00",
    "reflection_style": "structured",  # structured/free
    "reminder_style": "gentle",  # none/gentle/strict
    "goals": {
        "short_term": [
            {"id": "1", "text": "读完《原子习惯》", "deadline": "2025-05-15", "status": "active"}
        ],
        "medium_term": [
            {"id": "2", "text": "减重5kg", "deadline": "2025-08-01", "status": "active"}
        ],
        "long_term": [
            {"id": "3", "text": "成为健身达人", "deadline": "2026-12-31", "status": "active"}
        ]
    },
    "habits": [
        {"id": "h1", "name": "早起", "frequency": "daily", "target": "06:30"},
        {"id": "h2", "name": "阅读", "frequency": "daily", "target": "30分钟"},
        {"id": "h3", "name": "运动", "frequency": "4x_week", "target": "45分钟"}
    ]
}

save_profile(profile)
```

### Step 3: Daily Reflection

**每日三问模板：**

```
📅 2025年4月16日 星期三
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

🌟 今天做得好的（至少1件）
□ 

💡 今天学到的/可以改进的
□ 

❤️ 今天感恩的（人/事/物）
□ 

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

📊 今日评分
整体感受：😊 😐 😔 （选择）
能量水平：1-10 分：____
专注程度：1-10 分：____

🎯 目标进度
□ 读完《原子习惯》 - 进度：65/300页
□ 减重5kg - 当前：75kg（已减2kg）

✅ 习惯打卡
□ 早起 06:30 ✓
□ 阅读 30分钟 ✓
□ 运动 45分钟 ✗（原因：加班太晚）

📝 自由记录
（任何想记录的想法、感受、事件）

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**记录复盘：**

```python
from reflection_db import log_reflection

reflection = {
    "date": "2025-04-16",
    "wins": [
        "完成了健身计划第3周的训练",
        "主动和同事沟通，解决了误会"
    ],
    "learnings": [
        "发现下午3点后效率下降，需要调整工作安排"
    ],
    "gratitude": [
        "感谢朋友推荐的这本书",
        "感谢今天的好天气"
    ],
    "mood": "happy",  # happy/neutral/sad/excited/tired
    "energy_level": 7,
    "focus_level": 6,
    "goal_progress": [
        {"goal_id": "1", "progress_update": "读了20页，共65页"},
        {"goal_id": "2", "progress_update": "体重75kg，比上周减0.5kg"}
    ],
    "habits": [
        {"habit_id": "h1", "completed": True, "note": "06:15自然醒"},
        {"habit_id": "h2", "completed": True, "note": "午休时读了30分钟"},
        {"habit_id": "h3", "completed": False, "note": "加班到22点，太累了"}
    ],
    "free_notes": "今天虽然没运动，但饮食控制得很好。明天要早点下班补上。"
}

log_reflection(reflection)
```

### Step 4: Weekly Review

**周复盘模板（每周日晚）：**

```
📅 第16周复盘（4月14日-4月20日）
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

📊 本周数据
- 复盘完成：6/7 天
- 平均心情：😊
- 平均能量：7.2/10
- 习惯完成率：
  • 早起：7/7 ✓
  • 阅读：6/7
  • 运动：4/7

🎯 目标进展
□ 读完《原子习惯》
  当前：150/300页（+85页本周）
  预计完成：5月10日（提前5天）
  
□ 减重5kg
  当前：74.5kg（本周-0.5kg）
  累计：-2.5kg
  预计完成：7月15日

💭 本周主题词
（用一个词总结这周）
→ "突破"

🌟 本周高光时刻
1. 完成了健身计划的第4周，力量有明显提升
2. 主动承担了一个新项目，获得领导认可

⚠️ 本周挑战
1. 周三加班导致没运动，需要调整工作节奏
2. 周末两天饮食控制不够好

📅 下周重点
1. 读完《原子习惯》
2. 保持运动4次以上
3. 尝试新的时间管理法

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**生成本周复盘：**

```python
from reflection_db import generate_weekly_review

review = generate_weekly_review(week_start="2025-04-14")
# 自动汇总本周数据、习惯完成率、目标进度
```

### Step 5: Monthly Review

**月复盘模板：**

```
📅 2025年4月复盘
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

📊 本月概览
- 复盘天数：26/30
- 心情趋势：→→↗↗→↘→
- 平均能量：7.0/10
- 习惯平均完成率：78%

🎯 目标完成度
✓ 读完2本书（完成）
→ 减重3kg（完成2.5kg，83%）
✗ 学会Python基础（未开始）

💭 本月主题
"调整与适应"

🌟 本月成就
1. 建立了稳定的早起习惯
2. 完成了健身计划的第一个周期
3. 读完2本期待已久的书

📈 成长轨迹
（与上月对比）
- 复盘完成率：65% → 87% ↑
- 运动频率：3次/周 → 4次/周 ↑
- 阅读时长：15分钟/天 → 35分钟/天 ↑

⚠️ 需要改进
1. 工作日晚睡问题
2. 周末饮食控制
3. 新技能学习进度慢

🎯 下月目标
1. 读完3本书
2. 减重至73kg
3. 开始Python学习（每天至少30分钟）

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### Step 6: Habit Tracking

**习惯追踪视图：**

```
📊 习惯追踪（最近7天）
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

早起 06:30
日 一 二 三 四 五 六
✓  ✓  ✓  ✓  ✓  ✓  ✓  7/7

阅读 30分钟
日 一 二 三 四 五 六
✓  ✓  ✗  ✓  ✓  ✓  ✓  6/7

运动 45分钟
日 一 二 三 四 五 六
✓  ✗  ✓  ✓  ✗  ✓  ✓  5/7

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
连续打卡：早起 21天 🔥
```

**记录习惯：**

```python
from reflection_db import log_habit

# 单独记录习惯（不在日复盘中）
log_habit(
    habit_id="h1",
    date="2025-04-16",
    completed=True,
    note="06:15自然醒，感觉很好"
)
```

### Step 7: Goal Tracking

**目标进度视图：**

```
🎯 目标进度看板
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

短期目标（1-3个月）
─────────────────────
📖 读完《原子习惯》
[████████░░░░░░░░░░░░] 50%
预计完成：5月10日

🏃 每周运动4次
[████████████████████] 100%
连续达标：3周

中期目标（3-12个月）
─────────────────────
⚖️ 减重5kg
[████████████░░░░░░░░] 60%
当前：75kg → 目标：70kg
已减：3kg

长期愿景（1年以上）
─────────────────────
💪 成为健身达人
[████░░░░░░░░░░░░░░░░] 20%
里程碑：完成第一个训练计划

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**更新目标进度：**

```python
from reflection_db import update_goal_progress

update_goal_progress(
    goal_id="1",
    progress_text="150/300页",
    percentage=50
)
```

### Step 8: Mood & Emotion Tracking

**情绪趋势图（文字描述）：**

```
📈 最近30天情绪趋势
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

心情分布：
😊 开心：45%
😐 平静：35%
😔 低落：12%
😤 焦虑：8%

能量水平趋势：
第1周：6.5/10
第2周：7.0/10
第3周：7.2/10
第4周：7.5/10
→ 整体呈上升趋势

常见情绪触发：
- 开心：完成目标、运动、与朋友聚会
- 低落：工作压力大、睡眠不足
- 焦虑：deadline临近、计划被打乱

建议：
基于你的数据，周三和周五能量通常较低，
建议这两天安排轻松一些的任务。
```

---

## 📚 阅读计划 (Reading Plan)

### 创建阅读计划

```python
from reflection_db import add_reading_plan

reading_plan = {
    "title": "2025年个人成长书单",
    "books": [
        {
            "id": "b1",
            "title": "原子习惯",
            "author": "James Clear",
            "category": "习惯养成",
            "total_pages": 300,
            "status": "reading",  # unread/reading/completed
            "start_date": "2025-04-01",
            "target_date": "2025-04-30",
            "daily_target_pages": 10,
            "notes": ""
        },
        {
            "id": "b2", 
            "title": "深度工作",
            "author": "Cal Newport",
            "category": "效率提升",
            "total_pages": 280,
            "status": "unread",
            "target_date": "2025-05-31",
            "daily_target_pages": 10,
            "notes": ""
        }
    ],
    "reading_time": "21:00-21:30",  # 固定阅读时间
    "reading_days": ["周一", "周二", "周三", "周四", "周五"]  # 阅读日
}

plan_id = add_reading_plan(reading_plan)
```

### 记录阅读进度

```python
from reflection_db import log_reading_progress

progress = {
    "book_id": "b1",
    "date": "2025-04-16",
    "pages_read": 25,
    "total_progress": 150,  # 累计已读
    "notes": "关于习惯堆叠的章节很有启发",
    "rating": 5  # 今日阅读体验 1-5
}

log_reading_progress(progress)
```

### 阅读统计

```python
from reflection_db import get_reading_stats

stats = get_reading_stats()
# 返回：
# - 本月阅读页数
# - 阅读天数
# - 平均每日阅读时长
# - 完成书籍数
# - 阅读 streak
```

### 阅读计划视图

```
📚 我的阅读计划
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

📖 正在阅读 (1/5)
─────────────────────
原子习惯 - James Clear
[████████████░░░░░░░░] 150/300页 (50%)
预计完成：4月30日（还有14天）
今日需读：10页

📚 待读 (4)
─────────────────────
□ 深度工作 - Cal Newport
□ 思考，快与慢 - Daniel Kahneman
□ 非暴力沟通 - Marshall Rosenberg
□ 心流 - Mihaly Csikszentmihalyi

📊 本月阅读数据
─────────────────────
- 阅读天数：12/16天
- 总页数：320页
- 平均每日：27页
- 连续阅读：5天 🔥
```

---

## 🎓 课程推荐 (Course Recommendation)

### 学习领域评估

```python
from reflection_db import assess_learning_needs

# 基于用户目标和当前状态推荐课程
needs = assess_learning_needs()
# 分析：
# - 当前技能缺口
# - 职业发展需求
# - 兴趣爱好方向
```

### 课程库

内置推荐课程分类：

**效率与习惯**
- 《高效能人士的七个习惯》- 基础自我管理
- 《Getting Things Done》- 任务管理系统
- 《深度工作》- 专注力训练

**沟通与人际**
- 《非暴力沟通》- 沟通技巧
- 《关键对话》- 高难度沟通
- 《影响力》- 说服心理学

**思维与认知**
- 《思考，快与慢》- 决策心理学
- 《模型思维》- 系统思考
- 《批判性思维》- 逻辑分析

**专业技能**
- 根据用户职业方向推荐（需用户补充领域）

### 生成学习路径

```python
from reflection_db import generate_learning_path

path = generate_learning_path(
    goal="提升职场沟通能力",
    time_commitment="每周5小时",
    duration="3个月"
)

# 输出：
# 阶段1（第1月）：基础理论 - 《非暴力沟通》
# 阶段2（第2月）：实践应用 - 每日沟通复盘
# 阶段3（第3月）：高阶技巧 - 《关键对话》
```

### 课程推荐交互示例

```
User: "我想提升沟通能力，有什么推荐？"

Codex: 基于你的复盘记录，我发现你多次提到"和同事沟通有困难"。

📚 为你设计的学习路径：

阶段1（第1-2周）：基础理论
📖 《非暴力沟通》
   - 重点：观察 vs 评价、感受表达、需求识别
   - 实践：每天一次非暴力沟通练习

阶段2（第3-4周）：实践强化  
💡 每日沟通复盘
   - 记录：今天的一次重要对话
   - 分析：用了什么技巧？效果如何？
   - 改进：下次可以怎么做？

阶段3（第5-6周）：高阶应用
📖 《关键对话》
   - 重点：高风险对话、情绪管理、共同目标
   - 实践：主动发起一次困难对话

⏰ 时间投入：每天30分钟阅读 + 日常实践

需要我帮你制定详细的阅读计划吗？
```

---

## 🧘 心理建设 (Mental Wellness)

### 焦虑/压力应对

**当用户表达焦虑时：**

```
User: "最近工作压力很大，感觉很焦虑"

Codex: [检查最近复盘的情绪记录]

我注意到你这周的能量水平都在5分以下，
而且连续提到了3次"焦虑"。让我们一起来看看：

🔍 焦虑来源分析
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

根据你的复盘记录，焦虑主要来自：
1. 项目deadline压力（提到4次）
2. 与上级沟通不畅（提到2次）
3. 工作与生活边界模糊（提到3次）

💡 即时缓解技巧
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

🌬️ 4-7-8呼吸法（现在就可以做）
- 吸气4秒 → 屏息7秒 → 呼气8秒
- 重复3-5次

📝 焦虑外化（写下来）
- 最坏的情况是什么？
- 我能控制的是什么？
- 如果朋友遇到这事，我会怎么建议TA？

🎯 行动计划
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

明天可以做的3件小事：
1. 与上级确认项目优先级，协商deadline
2. 设置工作截止时间，晚上8点后不回复消息
3. 安排一次30分钟的户外散步

需要我陪你做一次呼吸练习，
或者聊聊具体的应对策略吗？
```

### 自我对话模板

**当用户自我批评时：**

```
User: "我又没完成今天的计划，太失败了"

Codex:

我听到了你对自己的批评。让我们换个角度看看：

🔄 转换视角
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

❌ 自我批评："我又失败了"
✅ 自我关怀："今天遇到了什么困难？"

❌ 绝对化："我总是做不到"
✅ 具体化："今天是什么阻碍了计划？"

❌ 苛责："我应该做得更好"
✅ 理解："我在什么情况下表现最好？"

📊 数据说话
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

看看你这周的实际情况：
- 计划完成率：5/7天 = 71%
- 这其实是很好的表现！
- 周二和周四都超额完成了

💡 调整建议
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

不是"失败"，而是"计划需要调整"：
1. 每天少安排1件事
2. 给意外留出缓冲时间
3. 完成比完美更重要

你已经做得很好了。要对自己温柔一点。💚
```

### 心理建设工具库

**1. 情绪调节技巧**
- 4-7-8呼吸法
- 5-4-3-2-1 grounding技巧
- 情绪日记

**2. 认知重构**
- 识别自动化负面思维
- 证据检验法
- 换位思考

**3. 压力管理**
- 压力源分类（可控/不可控）
- 放松训练
- 边界设定

**4. 自我关怀**
- 自我对话转换
- 小确幸记录
- 成就清单

### 心理建设记录

```python
from reflection_db import log_mental_wellness

wellness_entry = {
    "date": "2025-04-16",
    "mood": "anxious",
    "triggers": ["项目deadline", "会议冲突"],
    "coping_strategies_used": ["深呼吸", "与朋友聊天"],
    "effectiveness": 7,  # 1-10
    "insights": "发现提前准备可以减少焦虑",
    "gratitude": ["同事的帮助", "好天气"]
}

log_mental_wellness(wellness_entry)
```

## Best Practices

### 复盘原则

1. **不评判**：复盘不是为了自责，是为了成长
2. **具体化**：记录具体事件，而非笼统感受
3. **行动导向**：每个反思都指向下一步行动
4. **一致性**：每天5分钟，胜过偶尔1小时
5. **回顾周期**：每周回顾一次，每月深度复盘

### 习惯养成

- **从小开始**：先坚持1个习惯21天
- **环境设计**：让好习惯容易，坏习惯困难
- **奖励机制**：完成习惯后给自己小奖励
- **容错心态**：错过一天没关系，第二天继续

### 目标管理

- **SMART原则**：具体、可衡量、可实现、相关、有时限
- **分解目标**：大目标拆成小里程碑
- **定期回顾**：每月检查目标相关性，及时调整

## Example Interaction

```
User: "我想开始每天复盘"

Codex: [检查档案 - 未初始化]

很高兴你想开始复盘！这是非常好的成长习惯。
让我帮你设置一下：

1. 你希望每天什么时间复盘？（建议睡前或早晨）
2. 有什么正在追求的目标吗？
3. 想要追踪哪些习惯？

User: "睡前复盘，目标是读完一本书和减重，想追踪早起、阅读、运动"

Codex: [创建档案]

太好了！我已经为你设置好复盘档案。

📋 你的复盘配置
- 时间：每天晚上
- 风格：结构化三问
- 目标：
  • 读完一本书（短期）
  • 减重（中期）
- 习惯追踪：
  • 早起
  • 阅读
  • 运动

🌙 今晚就可以开始第一次复盘：

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
📅 今天复盘

🌟 今天做得好的
□ （至少1件，多小都可以）

💡 今天学到的/可以改进的
□ 

❤️ 今天感恩的
□ 

📊 今日评分
整体感受：😊 😐 😔
能量水平：1-10分

✅ 习惯打卡
□ 早起 □ 阅读 □ 运动

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

准备好开始了吗？可以现在就开始今天的复盘！
```

## Technical Notes

**Data Storage:**
数据默认存储在当前项目的 `codex-data/daily_reflection/` 文件夹中；可用 `CODEX_DATA_DIR` 改写数据根目录：
- Profile: `profile.json`
- Reflections: `reflections.json`
- Habits: `habits.json`
- Goals: `goals.json`
- Reading Plans: `reading.json`
- Mental Wellness: `wellness.json`

*路径示例：*
- macOS/Linux: `./codex-data/daily_reflection/`
- Windows: `./codex-data/daily_reflection\`

**CLI Commands:**
```bash
# 检查档案
python scripts/reflection_db.py is_initialized

# 查看今日复盘
python scripts/reflection_db.py today

# 生成本周复盘
python scripts/reflection_db.py weekly

# 查看习惯追踪
python scripts/reflection_db.py habits

# 查看目标进度
python scripts/reflection_db.py goals

# 查看阅读统计
python scripts/reflection_db.py reading_stats [days]

# 查看心理建设统计
python scripts/reflection_db.py wellness_stats [days]

# 导出数据
python scripts/reflection_db.py export
```

## Resources

### scripts/reflection_db.py
管理用户档案、日复盘记录、习惯追踪、目标进度、阅读计划和心理建设。

### 核心功能模块

| 模块 | 功能 | 数据文件 |
|------|------|----------|
| 日复盘 | 每日三问、情绪追踪 | reflections.json |
| 习惯追踪 | 习惯打卡、连续天数 | habits.json |
| 目标管理 | 目标设定、进度更新 | goals.json |
| 阅读计划 | 书单管理、阅读进度 | reading.json |
| 心理建设 | 情绪记录、应对策略 | wellness.json |

### Reflection Guidelines
- 每天5-10分钟，保持简洁
- 关注过程，而非完美
- 定期回顾，看到成长轨迹
- 与 fitness-coach、meal-planner 配合使用效果更佳
