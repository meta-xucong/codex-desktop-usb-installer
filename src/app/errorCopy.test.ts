import { describe, expect, it } from "vitest";

import {
  isConnectivityFailure,
  messageFailure,
  resolveFailure,
  sanitizeDiagnosticDetail,
  userErrorMessage,
} from "./errorCopy";
import type { TKey } from "./i18n";

const copy: Partial<Record<TKey, string>> = {
  "error.install": "安装未完成。已保留现有 Codex，请关闭 Codex 后重试。",
  "error.busy": "已有操作正在进行，请稍后再试。",
  "error.network": "无法连接更新服务器。请检查网络后重试。",
  "error.timeout": "连接超时。请检查网络后重试。",
  "error.disk_space": "磁盘空间不足，请清理后重试。",
  "error.disk_write": "无法写入磁盘。请检查磁盘权限与可用空间后重试。",
  "error.permission": "权限不足。请以合适的权限重试，或更换安装位置。",
  "error.usbAuthorization": "U 盘授权尚未就绪。请保持 U 盘连接并稍后重试；当前 Codex 未改动。",
  "error.signature": "更新包签名校验失败。已保留现有 Codex，请稍后重试或更换更新源。",
  "error.bundle_integrity": "USB 包完整性校验失败。已保留现有 Codex，请展开详情检查具体文件后重试。",
  "error.artifact": "更新包无效或已过期。请稍后重试，或在设置中更换更新源。",
  "error.incompatible": "当前系统环境不支持此安装方式。请尝试其他安装选项。",
  "error.unsupported": "当前平台暂不支持此操作。",
  "error.generic": "操作未完成。请稍后重试；若持续失败，可复制诊断信息反馈。",
  "skillRepo.allSourcesFailed": "更新源暂时不可用，未修改本地能力包。请重试。",
  "skillRepo.sourceInvalid": "更新来源或确认版本无效。请重新检查；展开详情可查看具体原因。",
  "skillRepo.baseResourceInvalid": "安装器基础资源不可用，请重新复制完整安装包后重试。",
  "home.stale.rechecked": "安装状态已变化,已重新检查——请再次确认。",
  "progress.cancelled": "下载已取消。",
};

const t = (key: TKey) => copy[key] ?? key;

describe("userErrorMessage / resolveFailure", () => {
  it("uses the stable backend code instead of leaking raw install errors", () => {
    expect(
      userErrorMessage(
        {
          code: "install",
          message:
            "update engine error: install error: target Codex process is still running",
        },
        t,
      ),
    ).toBe("安装未完成。已保留现有 Codex，请关闭 Codex 后重试。");
  });

  it("never leaks raw engine detail as the primary message", () => {
    const failure = resolveFailure({ code: "engine_error", message: "raw detail" }, t);
    expect(failure.message).toBe(
      "操作未完成。请稍后重试；若持续失败，可复制诊断信息反馈。",
    );
    expect(failure.detail).toBe("raw detail");
    expect(failure.code).toBe("engine_error");
    expect(failure.recoverable).toBe(true);
  });

  it("maps every classified backend kind to localized copy", () => {
    const cases: Array<[string, string]> = [
      ["network", "无法连接更新服务器。请检查网络后重试。"],
      ["timeout", "连接超时。请检查网络后重试。"],
      ["disk_space", "磁盘空间不足，请清理后重试。"],
      ["disk_write", "无法写入磁盘。请检查磁盘权限与可用空间后重试。"],
      ["permission", "权限不足。请以合适的权限重试，或更换安装位置。"],
      ["usb_authorization", "U 盘授权尚未就绪。请保持 U 盘连接并稍后重试；当前 Codex 未改动。"],
      ["signature", "更新包签名校验失败。已保留现有 Codex，请稍后重试或更换更新源。"],
      ["bundle_integrity", "USB 包完整性校验失败。已保留现有 Codex，请展开详情检查具体文件后重试。"],
      ["artifact", "更新包无效或已过期。请稍后重试，或在设置中更换更新源。"],
      ["incompatible", "当前系统环境不支持此安装方式。请尝试其他安装选项。"],
      ["cancelled", "下载已取消。"],
      ["operation_busy", "已有操作正在进行，请稍后再试。"],
      ["stale_expectation", "安装状态已变化,已重新检查——请再次确认。"],
      ["post_commit_cleanup", "操作未完成。请稍后重试；若持续失败，可复制诊断信息反馈。"],
      ["unsupported_platform", "当前平台暂不支持此操作。"],
      ["internal_error", "操作未完成。请稍后重试；若持续失败，可复制诊断信息反馈。"],
      ["contract_mismatch", "操作未完成。请稍后重试；若持续失败，可复制诊断信息反馈。"],
      ["SKILL_REPO_SOURCE_UNAVAILABLE", "无法连接更新服务器。请检查网络后重试。"],
      ["SKILL_REPO_ALL_SOURCES_FAILED", "更新源暂时不可用，未修改本地能力包。请重试。"],
      ["SKILL_REPO_SOURCE_INVALID", "更新来源或确认版本无效。请重新检查；展开详情可查看具体原因。"],
      ["SKILL_REPO_BASE_RESOURCE_INVALID", "安装器基础资源不可用，请重新复制完整安装包后重试。"],
      ["SKILL_REPO_LOCAL_IO", "无法写入磁盘。请检查磁盘权限与可用空间后重试。"],
    ];
    for (const [code, message] of cases) {
      expect(userErrorMessage({ code, message: `opaque ${code}` }, t)).toBe(message);
    }
  });

  it("treats unknown throwables as a generic recoverable failure", () => {
    const failure = resolveFailure(new Error("boom from render"), t);
    expect(failure.code).toBe("unknown");
    expect(failure.message).toBe(
      "操作未完成。请稍后重试；若持续失败，可复制诊断信息反馈。",
    );
    expect(failure.detail).toBe("boom from render");
    expect(failure.recoverable).toBe(true);
  });

  it("classifies connectivity codes for hero presentation", () => {
    expect(isConnectivityFailure({ code: "network", message: "x" })).toBe(true);
    expect(isConnectivityFailure({ code: "timeout", message: "x" })).toBe(true);
    expect(isConnectivityFailure({ code: "disk_write", message: "x" })).toBe(false);
  });

  it("builds a message-only surface without raw detail", () => {
    expect(messageFailure("already localized", "cancelled")).toEqual({
      code: "cancelled",
      message: "already localized",
      detail: null,
      recoverable: true,
    });
  });

  it("redacts transport URLs, local paths, and credential-like values in details", () => {
    expect(
      sanitizeDiagnosticDetail(
        "GET https://example.test/archive.zip failed at C:\\Users\\secret\\skills with api_key=super-secret",
      ),
    ).toBe("GET [remote source] failed at [local path] with api_key=[redacted]");
  });

  it("maps Skill Hub conflict and rollback codes to a stable artifact message", () => {
    expect(userErrorMessage({ code: "SKILL_REPO_USER_CONFLICT", message: "raw" }, t)).toBe(
      "更新包无效或已过期。请稍后重试，或在设置中更换更新源。",
    );
    expect(userErrorMessage({ code: "SKILL_REPO_ROLLBACK_FAILED", message: "raw" }, t)).toBe(
      "更新包无效或已过期。请稍后重试，或在设置中更换更新源。",
    );
  });
});
