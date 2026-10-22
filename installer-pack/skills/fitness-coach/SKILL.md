---
name: fitness-coach
description: 智能健身私教，提供个性化训练计划、动作指导、周期化训练和进度追踪。支持增肌、减脂、力量、耐力等多种目标，适配家庭和健身房场景。
---

# Fitness Coach - 智能健身私教

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行示例中的 `scripts/...` 时使用其绝对路径，
不要假设当前工作目录是仓库根目录。

## Overview

这位技能将 AI 打造成你的专属健身私教，不仅制定科学训练计划，还提供动作指导、进度追踪和周期化训练管理。融合 "教练" 的个性化指导和 "计划设计师" 的科学周期规划。

## When to Use This Skill

- 制定健身训练计划（增肌/减脂/力量/耐力/柔韧性）
- 获取动作指导和形式纠正建议
- 设计周期化训练（4-12周计划）
- 管理训练进度和进阶
- 调整计划以适应设备/时间限制
- 安排休息日、减载周避免过度训练

## Workflow

### Step 1: Check for Existing Profile

检查用户是否已有健身档案：

```bash
python scripts/fitness_db.py is_initialized
```

If "false", proceed to Step 2 (Setup). If "true", proceed to Step 3 (Program Creation).

### Step 2: Initial Profile Collection

收集用户健身档案：

**基础信息：**
- 年龄、性别、身高、体重
- 健身经验：新手(0-6月) / 中级(6月-2年) / 进阶(2年+)
- 每周可训练天数：2-6天
- 每次训练时间：30/45/60/90分钟

**健身目标（可多选）：**
- 增肌（Hypertrophy）
- 减脂（Fat Loss）
- 力量提升（Strength）
- 耐力改善（Endurance）
- 柔韧性/灵活性（Mobility）
- 体态矫正（Posture）
- 运动表现（Performance）

**可用器械：**
- 无器械（徒手训练）
- 哑铃
- 杠铃
- 弹力带
- 健身房（全套器械）
- 有氧器械（跑步机/单车/划船机）

**身体情况：**
- 伤病史或限制
- 疼痛部位
- 需要避免的动作

**保存档案：**

```python
import sys
sys.path.append('[SKILL_DIR]/scripts')
from fitness_db import save_profile

profile = {
    "age": 28,
    "gender": "male",
    "height_cm": 175,
    "weight_kg": 70,
    "experience_level": "intermediate",
    "days_per_week": 4,
    "session_duration_minutes": 60,
    "goals": ["hypertrophy", "strength"],
    "equipment": ["dumbbells", "barbell", "gym"],
    "limitations": ["shoulder impingement"],
    "avoid_exercises": ["overhead_press"]
}

save_profile(profile)
```

### Step 3: Create Training Program

**收集计划参数：**

1. **计划周期**：4周 / 8周 / 12周
2. **训练分割方式**：
   - 全身训练（Full Body）- 适合2-3天/周
   - 上下肢分化（Upper/Lower）- 适合4天/周
   - 推拉腿（Push/Pull/Legs）- 适合5-6天/周
   - 身体部位分化（Bro Split）- 适合5-6天/周
3. **重点肌群**：如有特别想加强的部位

**创建计划：**

```python
from fitness_db import add_program
from program_generator import generate_program

program_data = {
    "name": "12周增肌计划",
    "goal": "hypertrophy",
    "duration_weeks": 12,
    "split_type": "upper_lower",
    "days_per_week": 4,
    "focus_areas": ["chest", "back"]
}

program_id = add_program(program_data)

# 生成详细计划
program = generate_program(
    profile=profile,
    program_data=program_data,
    program_id=program_id
)
```

### Step 4: Program Structure

生成的计划包含以下周期：

**阶段一：适应期（第1-2周）**
- 中等重量，高次数（12-15次）
- 重点学习动作模式
- 建立训练习惯
- 容量：每肌群每周6-8组

**阶段二：积累期（第3-6周）**
- 逐步增加重量
- 次数范围8-12次
- 容量递增：每肌群每周10-16组
- 引入渐进超负荷

**阶段三：强化期（第7-10周）**
- 较重重量，6-10次
- 容量峰值：每肌群每周16-20组
- 多种强度技术（递减组、超级组）

**阶段四：峰值/减载（第11-12周）**
- 第11周：测试极限或高容量
- 第12周：减载周，容量减少40-50%
- 为下一阶段恢复

### Step 5: Daily Workout Format

每个训练日包含：

```
训练日 A - 上肢推（胸/肩/三头）
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

热身（5-10分钟）
□ 肩关节环绕 2×15
□ 俯卧撑（热身组） 2×10
□ 轻重量卧推 2×15

主训练
┌─────────────────────────────────────┐
│ 1. 杠铃卧推                          │
│    4组 × 8-10次                      │
│    休息：90秒                        │
│    💡 要点：肩胛骨收紧，核心稳定      │
│    📈 进阶：第3周起尝试增加2.5kg      │
└─────────────────────────────────────┘

┌─────────────────────────────────────┐
│ 2. 哑铃上斜推举                      │
│    3组 × 10-12次                     │
│    休息：60秒                        │
│    💡 要点：上斜30度，控制离心        │
└─────────────────────────────────────┘

┌─────────────────────────────────────┐
│ 3. 双杠臂屈伸 / 绳索下压             │
│    3组 × 10-12次                     │
│    休息：60秒                        │
│    💡 三头重点，身体略前倾            │
└─────────────────────────────────────┘

... (更多动作)

冷身（5分钟）
□ 胸部拉伸 30秒×2
□ 肩部拉伸 30秒×2
□ 三头拉伸 30秒×2

预计时间：55分钟
```

### Step 6: Exercise Library

常用动作分类：

**胸部：**
- 杠铃卧推、哑铃卧推
- 上斜推举、下斜推举
- 飞鸟（哑铃/绳索/器械）
- 俯卧撑、双杠臂屈伸

**背部：**
- 引体向上/高位下拉
- 杠铃划船、哑铃划船
- 坐姿划船、T杠划船
- 直臂下压、面拉

**腿部：**
- 深蹲（杠铃/哑铃/高脚杯）
- 硬拉（传统/罗马尼亚）
- 腿举、腿屈伸、腿弯举
- 箭步蹲、保加利亚分腿蹲
- 提踵（站姿/坐姿）

**肩部：**
- 肩推（杠铃/哑铃/器械）
- 侧平举、前平举
- 俯身飞鸟（后束）
- 面拉、耸肩

**手臂：**
- 二头：弯举（杠铃/哑铃/锤式/牧师凳）
- 三头：臂屈伸、下压、仰卧臂屈伸、窄距卧推

**核心：**
- 平板支撑、死虫式
- 卷腹、悬垂举腿
- 俄罗斯转体、Pallof Press

### Step 7: Progress Tracking

记录每次训练：

```python
from fitness_db import log_workout

workout = {
    "program_id": program_id,
    "week": 3,
    "day": "A",
    "exercises": [
        {
            "name": "杠铃卧推",
            "sets": [
                {"reps": 10, "weight": 60, "rpe": 7},
                {"reps": 10, "weight": 60, "rpe": 8},
                {"reps": 9, "weight": 60, "rpe": 9},
                {"reps": 8, "weight": 60, "rpe": 10}
            ]
        },
        # ... 更多动作
    ],
    "duration_minutes": 58,
    "notes": "感觉不错，下次可以尝试62.5kg"
}

log_workout(workout)
```

查看进度：

```python
from fitness_db import get_progress_summary

progress = get_progress_summary(program_id)
# 显示：力量曲线、容量趋势、完成率
```

### Step 8: Program Adjustments

根据反馈调整计划：

**增重信号（需要增加负荷）：**
- 目标次数范围内RPE ≤ 7
- 最后一组还能多做2-3次
- 连续两次训练都轻松完成

**调整动作（因伤病/设备）：**

```python
from fitness_db import substitute_exercise

# 将杠铃卧推替换为哑铃卧推
substitute_exercise(
    program_id=program_id,
    old_exercise="杠铃卧推",
    new_exercise="哑铃卧推",
    reason="shoulder discomfort"
)
```

**减载信号：**
- 持续疲劳、睡眠质量下降
- 力量下降或停滞2周以上
- 关节疼痛
- 主动安排减载周

## Best Practices

### 训练原则

1. **渐进超负荷**：逐步增加重量、次数或组数
2. **动作质量优先**：宁轻勿假，全程控制
3. **充分休息**：大肌群48-72小时恢复
4. **睡眠与营养**：训练只是刺激，恢复才能成长
5. **倾听身体**：区分"努力感"和"疼痛"

### 安全准则

- 每次训练必须热身和冷身
- 大重量动作需要保护或安全架
- 出现锐痛立即停止
- 呼吸节奏：用力时呼气
- 脱水会影响表现，保持水分

### 营养配合

**增肌期：**
- 热量盈余：+300-500 kcal/天
- 蛋白质：1.6-2.2g/kg体重
- 碳水：训练前后补充

**减脂期：**
- 热量缺口：-300-500 kcal/天
- 蛋白质：2.0-2.4g/kg体重（保肌）
- 力量训练维持强度

## Example Interaction

```
User: "我想开始健身，目标是增肌，在家练，有哑铃"

Codex: [检查档案 - 未初始化]

好的！我来帮你建立健身档案并制定训练计划。先问几个问题：

1. 你的年龄、身高、体重？
2. 之前有健身经验吗？
3. 每周能练几天？每次大概多久？
4. 有没有伤病或需要避免的动作？

User: "28岁，175cm，70kg，新手，每周4天，每次1小时，没有伤病"

Codex: [创建档案，生成计划]

完美！我为你设计了一个12周居家哑铃增肌计划：

📋 计划概览
- 训练分割：上肢/下肢分化（4天/周）
- 周期：12周，分4个阶段
- 重点：全身均衡发展，胸背腿核心

📅 每周安排
- 周一：上肢推（胸/肩/三头）
- 周二：下肢（腿/臀）
- 周四：上肢拉（背/二头）
- 周五：全身+核心

💪 第1周示例（适应期）

周一 - 上肢推
□ 热身：肩关节环绕、俯卧撑热身组
□ 哑铃卧推 3×12
□ 哑铃上斜推举 3×12
□ 哑铃肩推 3×12
□ 侧平举 3×15
□ 俯身臂屈伸 3×12
□ 冷身：胸部/肩部拉伸

📈 进阶策略
- 第1-2周：适应动作，找到合适重量
- 第3周起：每周尝试增加重量或次数
- 第6周：评估进度，调整容量
- 第12周：减载恢复

需要我详细展示某一周的训练内容吗？
```

## Technical Notes

**Data Storage:**
数据默认存储在当前项目的 `codex-data/fitness_coach/` 文件夹中；可用 `CODEX_DATA_DIR` 改写数据根目录：
- Profile: `profile.json`
- Programs: `programs.json`
- Workouts: `workouts.json`

*路径示例：*
- macOS/Linux: `./codex-data/fitness_coach/`
- Windows: `./codex-data/fitness_coach\`

**CLI Commands:**
```bash
# 检查档案
python scripts/fitness_db.py is_initialized

# 查看档案
python scripts/fitness_db.py get_profile

# 查看计划列表
python scripts/fitness_db.py get_programs

# 查看进度
python scripts/fitness_db.py progress [program_id]

# 导出数据
python scripts/fitness_db.py export
```

## Resources

### scripts/fitness_db.py
管理用户档案、训练计划、训练日志和进度统计。

### scripts/program_generator.py
生成周期化训练计划，包含动作选择、组数次数安排、进阶策略。

### Exercise Guidelines
- 复合动作优先（深蹲、硬拉、卧推、划船、肩推）
- 单关节动作补充
- 核心训练每2-3天一次
- 有氧根据目标安排
