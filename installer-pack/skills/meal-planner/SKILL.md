---
name: meal-planner
description: 智能饮食规划师，提供个性化食谱、营养分析、购物清单生成。支持增肌/减脂/维持目标，适配中餐/西餐/素食等饮食偏好。
---

# Meal Planner - 智能饮食规划师

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行示例中的 `scripts/...` 时使用其绝对路径，
不要假设当前工作目录是仓库根目录。

## Overview

这位技能帮助用户制定科学、实用的饮食计划，不仅关注营养搭配，也考虑口味偏好、烹饪难度和食材可得性。与 fitness-coach 完美配合，实现"三分练，七分吃"的健康目标。

## When to Use This Skill

- 制定每日/每周饮食计划
- 根据健身目标调整营养摄入（增肌/减脂/维持）
- 生成购物清单，减少食材浪费
- 获取健康食谱和烹饪建议
- 追踪每日营养摄入
- 适应特殊饮食需求（素食/低碳水/高蛋白等）

## Workflow

### Step 1: Check for Existing Profile

检查用户是否已有饮食档案：

```bash
python scripts/meal_db.py is_initialized
```

If "false", proceed to Step 2 (Setup). If "true", proceed to Step 3 (Meal Planning).

### Step 2: Initial Profile Collection

收集用户饮食档案：

**基础信息：**
- 年龄、性别、身高、体重
- 活动水平：久坐 / 轻度活动 / 中度活动 / 高度活动
- 基础代谢率（可选，可自动计算）

**饮食目标：**
- 增肌（热量盈余 +300~500 kcal）
- 减脂（热量缺口 -300~500 kcal）
- 维持（热量平衡）
- 改善健康指标（血压/血糖/胆固醇）

**饮食偏好：**
- 菜系偏好：中餐 / 西餐 / 日料 / 混合
- 饮食类型：普通 / 素食 / 纯素 / 生酮 / 低碳水
- 口味偏好：清淡 / 适中 / 重口味
- 辣度接受：不吃辣 / 微辣 / 中辣 / 重辣

**限制条件：**
- 食物过敏（海鲜/坚果/乳制品等）
- 不吃食材（内脏/香菜/葱蒜等）
- 烹饪条件：有厨房/只有微波炉/经常外食
- 每餐预算：经济型/标准型/品质型

**用餐习惯：**
- 每日几餐（2-6餐）
- 是否带饭上班/上学
- 早餐时间是否充裕

**保存档案：**

```python
import sys
sys.path.append('[SKILL_DIR]/scripts')
from meal_db import save_profile

profile = {
    "age": 28,
    "gender": "male",
    "height_cm": 175,
    "weight_kg": 70,
    "activity_level": "moderate",
    "goal": "muscle_gain",
    "cuisine_preference": "chinese",
    "diet_type": "regular",
    "taste_preference": "moderate",
    "spice_level": "mild",
    "allergies": ["shellfish"],
    "disliked_foods": ["coriander"],
    "cooking_condition": "full_kitchen",
    "budget_level": "standard",
    "meals_per_day": 4,
    "meal_prep": True
}

save_profile(profile)
```

### Step 3: Calculate Nutrition Targets

根据档案计算每日营养目标：

```python
from meal_db import calculate_nutrition_targets

targets = calculate_nutrition_targets()
# 返回：{
#   "calories": 2400,
#   "protein_g": 150,  # 增肌 2.0g/kg
#   "carbs_g": 270,
#   "fat_g": 80,
#   "protein_pct": 25,
#   "carbs_pct": 45,
#   "fat_pct": 30
# }
```

**营养素分配原则：**

| 目标 | 蛋白质 | 碳水 | 脂肪 |
|------|--------|------|------|
| 增肌 | 25-30% (1.6-2.2g/kg) | 45-50% | 25-30% |
| 减脂 | 30-35% (2.0-2.4g/kg) | 30-40% | 25-30% |
| 维持 | 20-25% (1.2-1.6g/kg) | 45-55% | 25-35% |

### Step 4: Generate Meal Plan

创建饮食计划：

```python
from meal_db import add_meal_plan
from recipe_generator import generate_weekly_plan

plan_data = {
    "name": "第1周增肌饮食计划",
    "duration_days": 7,
    "goal": "muscle_gain",
    "variations": 3  # 每天3种选择
}

plan_id = add_meal_plan(plan_data)

# 生成详细计划
plan = generate_weekly_plan(
    profile=profile,
    plan_data=plan_data,
    plan_id=plan_id
)
```

### Step 5: Meal Structure

**4餐制示例（增肌）：**

```
早餐 (7:30) - 600 kcal
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
主食: 燕麦粥 80g + 牛奶 250ml
蛋白质: 水煮蛋 2个
蔬果: 香蕉 1根
脂肪: 坚果 15g

加餐 (10:30) - 300 kcal
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
蛋白质: 希腊酸奶 200g
碳水: 蓝莓 100g
脂肪: 杏仁 10g

午餐 (12:30) - 800 kcal
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
主食: 糙米饭 150g
蛋白质: 鸡胸肉 150g (蒜香煎)
蔬菜: 西兰花 200g + 胡萝卜 100g
脂肪: 橄榄油 10g

训练后 (18:00) - 400 kcal
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
蛋白质: 乳清蛋白 1勺 + 牛奶
碳水: 香蕉 1根 + 白面包 2片

晚餐 (20:00) - 700 kcal
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
主食: 红薯 200g
蛋白质: 三文鱼 120g (烤)
蔬菜: 菠菜 200g + 番茄 100g
脂肪: 牛油果 半个
```

### Step 6: Recipe Database

常用食谱分类：

**早餐：**
- 中式：杂粮粥、豆浆配包子、鸡蛋灌饼、燕麦鸡蛋饼
- 西式：燕麦碗、全麦吐司配鸡蛋、希腊酸奶杯、蛋白松饼
- 快手：香蕉花生酱吐司、蛋白粉奶昔、水煮蛋配牛奶

**午餐/晚餐：**
- 中式家常菜：番茄炒蛋、青椒肉丝、蒜蓉西兰花、红烧鱼
- 中式主食：糙米饭、杂粮馒头、红薯、荞麦面
- 西式简餐：煎鸡胸配蔬菜、三文鱼沙拉、牛肉糙米饭碗
- 快手料理：电饭煲焖饭、一锅炖、烤箱烤蔬菜肉类

**加餐/零食：**
- 蛋白质：水煮蛋、希腊酸奶、蛋白棒、鸡胸肉丸
- 碳水：香蕉、燕麦饼干、全麦面包、红薯干
- 健康脂肪：坚果、牛油果、黑巧克力

### Step 7: Shopping List Generation

生成购物清单：

```python
from meal_db import generate_shopping_list

shopping_list = generate_shopping_list(plan_id, days=7)
# 自动分类：肉类/蔬菜/水果/主食/调料/其他
```

**示例输出：**

```
🛒 本周购物清单（7天）
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

🥩 肉类蛋白
□ 鸡胸肉 1kg
□ 三文鱼 500g
□ 鸡蛋 20个
□ 瘦牛肉 400g

🥬 蔬菜
□ 西兰花 1kg
□ 菠菜 800g
□ 番茄 600g
□ 黄瓜 500g
□ 胡萝卜 400g

🍎 水果
□ 香蕉 10根
□ 蓝莓 2盒
□ 苹果 6个

🌾 主食杂粮
□ 糙米 1kg
□ 燕麦 500g
□ 红薯 2kg
□ 全麦面包 1袋

🥛 乳制品
□ 牛奶 2L
□ 希腊酸奶 1kg

🧂 调料其他
□ 橄榄油
□ 海盐
□ 黑胡椒
□ 坚果混合 200g
```

### Step 8: Nutrition Tracking

记录每日饮食：

```python
from meal_db import log_meal

meal = {
    "date": "2025-04-16",
    "meal_type": "lunch",  # breakfast/lunch/dinner/snack
    "foods": [
        {"name": "糙米饭", "amount": 150, "unit": "g", "calories": 180, "protein": 4, "carbs": 38, "fat": 1},
        {"name": "鸡胸肉", "amount": 150, "unit": "g", "calories": 165, "protein": 31, "carbs": 0, "fat": 3.6},
        {"name": "西兰花", "amount": 200, "unit": "g", "calories": 68, "protein": 5.6, "carbs": 13, "fat": 0.7}
    ],
    "notes": "今天鸡胸肉煎得刚好"
}

log_meal(meal)
```

查看今日摄入：

```python
from meal_db import get_daily_summary

summary = get_daily_summary(date="2025-04-16")
# 显示：实际摄入 vs 目标，完成百分比
```

### Step 9: Meal Plan Adjustments

根据反馈调整计划：

**体重变化太慢（增肌）：**
- 每周体重增长 < 0.25kg：增加 100-200 kcal
- 主要增加碳水（训练前后）

**体重不降（减脂）：**
- 2周体重无变化：减少 100-200 kcal
- 主要减少碳水，保持蛋白质

**替换食材：**

```python
from meal_db import substitute_ingredient

# 将三文鱼替换为鳕鱼（减脂期更低脂）
substitute_ingredient(
    plan_id=plan_id,
    old_ingredient="三文鱼",
    new_ingredient="鳕鱼",
    reason="lower_fat"
)
```

## Best Practices

### 饮食原则

1. **蛋白质优先**：每餐先确定蛋白质来源
2. **蔬菜不限量**：绿叶蔬菜可以多吃
3. **碳水时机**：训练前后补充，其他时间适量
4. **健康脂肪**：坚果、鱼油、橄榄油、牛油果
5. **水分充足**：每天 2-3L 水

### 备餐建议

- **批量备餐**：周末准备3-4天的蛋白质（烤鸡胸、煮蛋）
- **分装冷冻**：煮好的杂粮饭分装冷冻
- **蔬菜预处理**：洗好切好，随取随用
- **便携餐食**：准备蛋白棒、坚果包应对外出

### 外出就餐

- 优先选择：清蒸、烤制、少油炒菜
- 避免：油炸、红烧、糖醋、勾芡
- 技巧：要求少油少盐、酱料分开、多要蔬菜

## Example Interaction

```
User: "我想开始减脂饮食，帮我做个计划"

Codex: [检查档案 - 未初始化]

好的！我来帮你建立饮食档案并制定减脂计划。先问几个问题：

1. 你的年龄、身高、体重？
2. 平时活动量如何？（久坐/轻度/中度/高度）
3. 喜欢中餐还是西餐？有什么忌口吗？
4. 会自己做饭吗？厨房条件如何？
5. 每天几餐？需要带饭吗？

User: "28岁，175cm，75kg，中度活动，喜欢中餐，不吃香菜，有厨房，每天4餐，需要带午饭"

Codex: [创建档案，计算营养目标]

完美！根据你的情况，我为你制定了减脂饮食计划：

📊 你的营养目标
- 每日热量：2000 kcal（缺口约400kcal）
- 蛋白质：150g (2.0g/kg，保肌)
- 碳水：175g (35%)
- 脂肪：67g (30%)

📅 本周饮食计划（中式口味）

周一
早餐 (500 kcal)：燕麦粥 + 水煮蛋2个 + 牛奶
加餐 (250 kcal)：希腊酸奶 + 蓝莓
午餐 (600 kcal)：糙米饭 + 青椒炒鸡胸 + 蒜蓉西兰花
晚餐 (650 kcal)：蒸红薯 + 清蒸鲈鱼 + 凉拌菠菜

周二
早餐 (500 kcal)：全麦馒头 + 豆浆 + 茶叶蛋2个
...

💡 烹饪小贴士
- 鸡胸肉提前用料酒+生抽腌制，煎出来更嫩
- 杂粮饭可以一次煮3天的量，分装冷藏
- 带饭的蔬菜建议选根茎类（西兰花/胡萝卜），绿叶菜当晚吃

🛒 本周购物清单已生成，需要我展示吗？
```

## Technical Notes

**Data Storage:**
数据默认存储在当前项目的 `codex-data/meal_planner/` 文件夹中；可用 `CODEX_DATA_DIR` 改写数据根目录：
- Profile: `profile.json`
- Meal Plans: `plans.json`
- Food Logs: `logs.json`
- Recipes: `recipes.json`

*路径示例：*
- macOS/Linux: `./codex-data/meal_planner/`
- Windows: `./codex-data/meal_planner\`

**CLI Commands:**
```bash
# 检查档案
python scripts/meal_db.py is_initialized

# 查看档案
python scripts/meal_db.py get_profile

# 查看今日摄入
python scripts/meal_db.py today

# 生成购物清单
python scripts/meal_db.py shopping_list [plan_id] --days 7

# 导出数据
python scripts/meal_db.py export
```

## Resources

### scripts/meal_db.py
管理用户档案、饮食计划、营养日志和购物清单。

### scripts/recipe_generator.py
根据营养目标和饮食偏好生成每日食谱，包含食材搭配和烹饪建议。

### Nutrition Guidelines
- 蛋白质来源：鸡胸肉、鱼、蛋、豆腐、希腊酸奶
- 碳水来源：糙米、燕麦、红薯、全麦面包、水果
- 脂肪来源：坚果、牛油果、橄榄油、鱼油
- 蔬菜：深色绿叶菜优先，每天 500g+
