---
name: weather-forecast
description: 记录用户明确指定的默认城市，并通过公开天气服务查询当前天气和预报。
---

# 天气预报与城市记忆

## 脚本路径

先把 `<skill-directory>` 解析为本 `SKILL.md` 所在目录。执行示例中的 `scripts/...` 时使用其绝对路径，
不要假设当前工作目录是仓库根目录。

这是带本地城市偏好的天气工作流。城市记忆只写入当前项目的本地数据目录，不读取平台配置，
也不保存账号、Cookie 或完整对话历史。实时结果要标注查询时间和来源。

## 首次使用

```powershell
powershell -ExecutionPolicy Bypass -File scripts/weather_db.ps1 has_city
powershell -ExecutionPolicy Bypass -File scripts/weather_db.ps1 set_city "北京"
powershell -ExecutionPolicy Bypass -File scripts/weather_db.ps1 get_city
```

查询天气时优先使用当前 Codex 提供的原生天气或网页能力。原生能力不可用、但执行环境明确允许外网请求时，
才使用公开服务回退：

```powershell
curl.exe "https://wttr.in/Beijing?format=3"
```

如果需要机器可读的 JSON，可调用 Open-Meteo 等明确配置的公开接口；所有路径失败时说明网络或服务状态，
不得根据城市、季节或历史天气猜测实时结果。

## 存储位置

默认数据文件位于当前工作区的 `codex-data/weather-forecast/`；可用 `WEATHER_DATA_DIR` 指定目录。
不要把该目录中的个人偏好、缓存或运行结果提交到公开仓库。

城市记忆脚本使用 Windows PowerShell 的 JSON 能力，不依赖 Python；天气查询本身仍需要网络。
