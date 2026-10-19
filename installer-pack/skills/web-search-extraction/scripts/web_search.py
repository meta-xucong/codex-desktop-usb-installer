"""Optional DashScope-compatible web-search adapter.

The adapter never reads platform configuration files. Credentials must be supplied
through DASHSCOPE_API_KEY and the endpoint can be changed with DASHSCOPE_BASE_URL.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request


DEFAULT_BASE_URL = "https://dashscope.aliyuncs.com/compatible-mode/v1"
MAX_RESPONSE_BYTES = 5 * 1024 * 1024


def _error_result(message: str, api_key: str = "") -> dict:
    """Return a safe error string with raw and JSON-escaped credential forms removed."""
    safe_message = str(message)
    if api_key:
        secret_forms = {api_key}
        try:
            secret_forms.add(json.dumps(api_key, ensure_ascii=False)[1:-1])
        except (TypeError, ValueError):
            pass
        for secret in sorted((item for item in secret_forms if item), key=len, reverse=True):
            safe_message = safe_message.replace(secret, "[REDACTED]")
    return {"error": safe_message}


def validate_base_url(value: str) -> str:
    normalized = str(value or "").strip().rstrip("/")
    parsed = urllib.parse.urlparse(normalized)
    if parsed.scheme != "https" or not parsed.hostname:
        raise ValueError("DASHSCOPE_BASE_URL 必须是带主机名的 HTTPS URL。")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("DASHSCOPE_BASE_URL 不得包含凭据、查询参数或 fragment。")
    if parsed.path.rstrip("/") != "/compatible-mode/v1":
        raise ValueError("DASHSCOPE_BASE_URL 必须指向 compatible-mode/v1 根路径。")
    return normalized


def web_search(query: str, model: str | None = None, max_words: int = 300) -> dict:
    api_key = os.environ.get("DASHSCOPE_API_KEY", "").strip()
    normalized_query = str(query or "").strip()
    if not normalized_query:
        return _error_result("搜索问题不能为空。", api_key)
    # bool subclasses int in Python, but True/False are not meaningful word limits.
    if isinstance(max_words, bool) or not isinstance(max_words, int) or not 1 <= max_words <= 5000:
        return _error_result("max_words 必须是 1 到 5000 的整数。", api_key)
    if not api_key:
        return _error_result("缺少 DASHSCOPE_API_KEY；请通过当前用户的环境变量显式配置凭据。")
    selected_model = str(model or os.environ.get("DASHSCOPE_MODEL") or "").strip()
    if not selected_model:
        return _error_result("缺少 DASHSCOPE_MODEL；请显式配置服务当前支持的模型，不使用猜测默认值。", api_key)
    try:
        base_url = validate_base_url(os.environ.get("DASHSCOPE_BASE_URL", DEFAULT_BASE_URL))
    except ValueError as error:
        return _error_result(str(error), api_key)
    endpoint = f"{base_url}/chat/completions"
    prompt = (
        f"请使用可用的联网搜索能力查询，并根据检索到的信息进行总结。\n"
        f"总结不超过{max_words}字，写明来源名称和链接。\n用户问题：{normalized_query}"
    )
    payload = {
        "model": selected_model,
        "messages": [{"role": "user", "content": prompt}],
        "enable_search": True,
        "search_options": {"search_strategy": "turbo"},
    }
    request = urllib.request.Request(
        endpoint,
        data=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {api_key}"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            raw = response.read(MAX_RESPONSE_BYTES + 1)
            if len(raw) > MAX_RESPONSE_BYTES:
                return _error_result(f"接口响应超过 {MAX_RESPONSE_BYTES} bytes，已停止解析。", api_key)
            try:
                decoded = raw.decode("utf-8")
                body = json.loads(decoded)
            except (UnicodeDecodeError, ValueError, RecursionError):
                return _error_result("请求或响应解析失败；上游错误详情已隐藏。", api_key)
        if not isinstance(body, dict):
            return _error_result("接口返回格式无效：choices/message/content 类型不符合要求。", api_key)
        choices = body.get("choices")
        if choices is None or choices == []:
            return _error_result("接口返回为空：未找到 message.content", api_key)
        if not isinstance(choices, list):
            return _error_result("接口返回格式无效：choices/message/content 类型不符合要求。", api_key)
        choice = choices[0]
        if not isinstance(choice, dict):
            return _error_result("接口返回格式无效：choices/message/content 类型不符合要求。", api_key)
        message = choice.get("message")
        if message is None:
            return _error_result("接口返回为空：未找到 message.content", api_key)
        if not isinstance(message, dict):
            return _error_result("接口返回格式无效：choices/message/content 类型不符合要求。", api_key)
        content = message.get("content")
        if content is None or (isinstance(content, str) and not content.strip()):
            return _error_result("接口返回为空：未找到 message.content", api_key)
        if not isinstance(content, str):
            return _error_result("接口返回格式无效：choices/message/content 类型不符合要求。", api_key)
        return {"success": True, "content": content}
    except urllib.error.HTTPError as error:
        # Do not surface upstream bodies: they may reflect Authorization headers or
        # include other sensitive request data. The status code is sufficient here.
        return _error_result(f"HTTP {error.code}: 上游错误正文已隐藏。", api_key)
    except (urllib.error.URLError, TimeoutError):
        return _error_result("网络请求失败：无法连接上游服务。", api_key)
    except OSError:
        return _error_result("请求或响应解析失败；上游错误详情已隐藏。", api_key)


def main() -> int:
    parser = argparse.ArgumentParser(description="Optional web search adapter")
    parser.add_argument("query")
    parser.add_argument("--model", default=None, help="模型名；也可通过 DASHSCOPE_MODEL 配置")
    parser.add_argument("--max-words", type=int, default=200)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    result = web_search(args.query, args.model, args.max_words)
    if args.json:
        print(json.dumps(result, ensure_ascii=False))
    elif result.get("success"):
        print(result["content"])
    else:
        print(f"错误：{result.get('error', '未知错误')}", file=sys.stderr)
    return 0 if result.get("success") else 1


if __name__ == "__main__":
    raise SystemExit(main())
