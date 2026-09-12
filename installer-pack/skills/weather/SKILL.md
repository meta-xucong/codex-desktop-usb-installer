---
name: weather
description: 使用公开天气服务查询指定地点的当前天气、降雨、温度和短期预报。
---

# 天气查询

需要城市、地区、机场代码或经纬度。当前天气是实时信息，必须写明查询时间和数据来源；优先使用当前 Codex
提供的原生天气或网页能力。涉及预警、航空、海事或官方决策时，改查当地官方气象服务并明确其发布时间。

原生能力不可用、但当前执行环境明确允许外网请求时，Windows 才使用系统自带的 `curl.exe` 作为回退：

```powershell
curl.exe "https://wttr.in/London?format=3"
curl.exe "https://wttr.in/London?format=j1"
```

也可以使用 PowerShell 回退：

```powershell
Invoke-RestMethod "https://wttr.in/Beijing?format=j1"
```

所有回退都失败时，说明无法实时核验，不根据季节或常识猜天气。本技能不保存用户位置或查询历史。若需要记忆
城市，请由用户明确指定一个项目内的存储文件。
