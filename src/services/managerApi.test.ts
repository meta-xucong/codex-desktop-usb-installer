import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_SETTINGS, type AppSettings } from "../shared/types";
import {
  isNetworkError,
  managerApi,
  skillRepoAttempts,
  SETTINGS_CHANGED_EVENT,
  type UpdateInfo,
} from "./managerApi";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const invokeMock = vi.mocked(invoke);

const CODEX_PAYLOAD_INFO: UpdateInfo = {
  channel: "codex",
  currentVersion: "26.825.51511",
  latestVersion: "26.826.51511",
  releaseNotes: null,
  sizeBytes: 1234,
  requiresRestart: false,
  payload: {
    repo: "owner/repo",
    releaseId: 42,
  },
};

beforeEach(() => {
  invokeMock.mockReset();
  vi.stubGlobal("window", {
    open: vi.fn(),
    __TAURI_INTERNALS__: undefined,
    setTimeout: (...args: Parameters<typeof setTimeout>) => setTimeout(...args),
    clearTimeout: (handle: ReturnType<typeof setTimeout>) => clearTimeout(handle),
  });
  localStorage.clear();
});

describe("update check watchdog", () => {
  it("returns unavailable when the manager update bridge never settles", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockReturnValue(new Promise<never>(() => {}));
    vi.useFakeTimers();
    try {
      const pending = managerApi.checkManagerUpdate();
      await vi.advanceTimersByTimeAsync(35_000);
      await expect(pending).resolves.toEqual({ kind: "unavailable" });
    } finally {
      vi.useRealTimers();
    }
  });

  it("rejects the Windows update plan when the native bridge never settles", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockReturnValue(new Promise<never>(() => {}));
    vi.useFakeTimers();
    try {
      const pending = managerApi.winPlanUpdate();
      const rejected = expect(pending).rejects.toMatchObject({
        code: "timeout",
        detail: "stage=ui-check-watchdog; timeout_ms=150000",
      });
      await vi.advanceTimersByTimeAsync(150_000);
      await rejected;
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("isNetworkError", () => {
  it("classifies transport and TLS failures as connectivity errors", () => {
    expect(
      isNetworkError(
        "update engine error: io error: curl failed for host=codexapp.agentsmirror.com exit=35: stderr='curl: (35) schannel: failed to receive handshake, SSL/TLS connection failed'",
      ),
    ).toBe(true);
    expect(
      isNetworkError(
        "curl: (6) Could not resolve host: codexapp.agentsmirror.com",
      ),
    ).toBe(true);
    expect(
      isNetworkError("curl: (28) Operation timed out after 20000 milliseconds"),
    ).toBe(true);
  });

  it("classifies the macOS auto-source fallback failure as connectivity", () => {
    expect(
      isNetworkError(
        "both the mirror and OpenAI official appcast are unreachable",
      ),
    ).toBe(true);
  });

  it("does not treat server responses or verification failures as connectivity", () => {
    expect(
      isNetworkError(
        "update engine error: curl failed for https://example.test/appcast.xml: curl: (22) The requested URL returned error: 404",
      ),
    ).toBe(false);
    expect(isNetworkError("appcast enclosure missing edSignature")).toBe(false);
    expect(isNetworkError("EdDSA signature does not match")).toBe(false);
  });
});

describe("settings API", () => {
  it("migrates legacy browser settings into startup and periodic checks", async () => {
    localStorage.setItem(
      "cam.settings",
      JSON.stringify({
        source: "mirror",
        customUrl: "",
        autoCheck: false,
        askBefore: true,
        signedOnly: true,
      }),
    );

    const settings = await managerApi.getSettings();

    expect(settings.source).toBe("mirror");
    expect(settings.autoCheck).toBe(false);
    expect(settings.checkOnStartup).toBe(false);
    expect(settings.periodicCheck).toBe(false);
    expect(settings.periodicCheckIntervalSeconds).toBe(15 * 60);
    expect(settings.disableCodexSelfUpdates).toBe(false);
    expect(settings.codexPayloadSourceKind).toBe("auto");
    expect(settings.codexPayloadBaseUrl).toBe(
      "https://codexapp.agentsmirror.com/",
    );
    expect(settings.codexPayloadRepo).toBe("");
  });

  it("normalizes a persisted legacy Codex mirror source to auto", async () => {
    localStorage.setItem(
      "cam.settings",
      JSON.stringify({
        ...DEFAULT_SETTINGS,
        codexPayloadSourceKind: "mirror",
      }),
    );

    const settings = await managerApi.getSettings();

    expect(settings.codexPayloadSourceKind).toBe("auto");
  });

  it("normalizes and broadcasts browser settings writes", async () => {
    const dispatchEvent = vi.fn();
    vi.stubGlobal("window", {
      open: vi.fn(),
      __TAURI_INTERNALS__: undefined,
      dispatchEvent,
    });

    const saved = await managerApi.setSettings({
      ...DEFAULT_SETTINGS,
      periodicCheckIntervalSeconds: 0,
      disableCodexSelfUpdates: true,
    });

    expect(saved.periodicCheckIntervalSeconds).toBe(60);
    expect(saved.disableCodexSelfUpdates).toBe(true);
    expect(dispatchEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        type: SETTINGS_CHANGED_EVENT,
        detail: saved,
      }),
    );
  });

  it("coerces empty custom source and proxy modes to real defaults", async () => {
    const dispatchEvent = vi.fn();
    vi.stubGlobal("window", {
      open: vi.fn(),
      __TAURI_INTERNALS__: undefined,
      dispatchEvent,
    });

    const saved = await managerApi.setSettings({
      ...DEFAULT_SETTINGS,
      source: "custom",
      customUrl: "  ",
      proxyMode: "custom",
      customProxyUrl: "",
    });

    expect(saved.source).toBe("auto");
    expect(saved.customUrl).toBe("");
    expect(saved.proxyMode).toBe("system");
    expect(saved.customProxyUrl).toBe("");
  });

  it("trims the dedicated Codex payload repo and preserves blank as the default sentinel", async () => {
    vi.stubGlobal("window", {
      open: vi.fn(),
      __TAURI_INTERNALS__: undefined,
      dispatchEvent: vi.fn(),
    });

    const saved = await managerApi.setSettings({
      ...DEFAULT_SETTINGS,
      source: "custom",
      customUrl: "https://mirror.example.test/appcast.xml",
      codexPayloadRepo: "  owner/repo  ",
    });

    expect(saved.codexPayloadRepo).toBe("owner/repo");
    expect(saved.source).toBe("custom");
    expect(saved.customUrl).toBe("https://mirror.example.test/appcast.xml");

    const reset = await managerApi.setSettings({
      ...saved,
      codexPayloadRepo: "   ",
    });

    expect(reset.codexPayloadRepo).toBe("");
    expect(reset.source).toBe("custom");
    expect(reset.customUrl).toBe("https://mirror.example.test/appcast.xml");
  });

  it("normalizes the Codex payload source kind and Relay URL", async () => {
    vi.stubGlobal("window", {
      open: vi.fn(),
      __TAURI_INTERNALS__: undefined,
      dispatchEvent: vi.fn(),
    });

    const relay = await managerApi.setSettings({
      ...DEFAULT_SETTINGS,
      codexPayloadSourceKind: "relay",
      codexPayloadBaseUrl: "  https://relay.example.test/codex/  ",
    });
    expect(relay.codexPayloadSourceKind).toBe("relay");
    expect(relay.codexPayloadBaseUrl).toBe("https://relay.example.test/codex/");

    const fallback = await managerApi.setSettings({
      ...DEFAULT_SETTINGS,
      codexPayloadSourceKind: "invalid" as AppSettings["codexPayloadSourceKind"],
      codexPayloadBaseUrl: "   ",
    });
    expect(fallback.codexPayloadSourceKind).toBe("auto");
    expect(fallback.codexPayloadBaseUrl).toBe(
      DEFAULT_SETTINGS.codexPayloadBaseUrl,
    );
  });
});

describe("Windows launch locale", () => {
  it("passes the selected Chinese locale to the native launcher", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue(undefined);

    await managerApi.winLaunch("zh-CN");

    expect(invokeMock).toHaveBeenCalledWith("win_launch_codex", {
      locale: "zh-CN",
    });
  });

  it("keeps the legacy no-argument launch payload for the default language", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue(undefined);

    await managerApi.winLaunch();

    expect(invokeMock).toHaveBeenCalledWith("win_launch_codex");
  });
});

describe("diagnostics API", () => {
  it("returns browser fallbacks without invoking Tauri", async () => {
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => {});
    const diagnostics = await managerApi.getDiagnostics();

    expect(diagnostics.os).toBe("browser");
    await expect(managerApi.getHostArchitecture()).resolves.toBe("x86_64");
    await expect(managerApi.openLogsDir()).resolves.toBeUndefined();
    await expect(managerApi.openCodexHome()).resolves.toBeUndefined();
    await expect(
      managerApi.frontendReady("en", 1, "browser-token"),
    ).resolves.toBeUndefined();
    await expect(
      managerApi.reportFrontendError({
        kind: "test",
        message: "boom",
        stack: null,
        componentStack: null,
      }),
    ).resolves.toBeUndefined();
    expect(invokeMock).not.toHaveBeenCalled();
    expect(consoleError).toHaveBeenCalledWith(
      "[frontend]",
      expect.objectContaining({ kind: "test", message: "boom" }),
    );
    consoleError.mockRestore();
  });

  it("reports frontend readiness and application language through IPC", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue(undefined);

    await expect(
      managerApi.frontendReady("zh-TW", 7, "generation-token"),
    ).resolves.toBeUndefined();

    expect(invokeMock).toHaveBeenCalledWith("frontend_ready", {
      lang: "zh-TW",
      generation: 7,
      token: "generation-token",
    });
  });

  it("invokes diagnostics commands inside Tauri", async () => {
    window.__TAURI_INTERNALS__ = {};
    const diagnostics = {
      appVersion: "0.1.17",
      os: "macos",
      arch: "aarch64",
      locale: null,
      updateSource: "auto",
      customSourceHost: null,
      windowsInstallMode: null,
      installStatus: "macos status=none",
      configHealth: {
        settingsStatus: "ok",
        provenanceStatus: "ok",
        unknownSource: null,
        detail: null,
        settingsBackupAvailable: false,
        provenanceBackupAvailable: false,
      },
      logsDir: "/tmp/logs",
      recentErrors: [],
      logTail: "",
      generatedAtUnix: 1,
    };
    invokeMock
      .mockResolvedValueOnce(diagnostics)
      .mockResolvedValueOnce("aarch64")
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce(undefined);

    await expect(managerApi.getDiagnostics()).resolves.toEqual(diagnostics);
    await expect(managerApi.getHostArchitecture()).resolves.toBe("aarch64");
    await expect(managerApi.openLogsDir()).resolves.toBeUndefined();
    await expect(managerApi.openCodexHome()).resolves.toBeUndefined();
    await expect(
      managerApi.reportFrontendError({
        kind: "test",
        message: "boom",
        stack: null,
        componentStack: null,
      }),
    ).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenNthCalledWith(1, "get_diagnostics");
    expect(invokeMock).toHaveBeenNthCalledWith(2, "get_host_architecture");
    expect(invokeMock).toHaveBeenNthCalledWith(3, "open_logs_dir");
    expect(invokeMock).toHaveBeenNthCalledWith(4, "open_codex_home");
    expect(invokeMock).toHaveBeenNthCalledWith(5, "log_frontend_error", {
      payload: {
        kind: "test",
        message: "boom",
        stack: null,
        componentStack: null,
      },
    });
  });
});

describe("Windows perform API", () => {
  it("invokes the guarded combined Codex + CCSwitch uninstall", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValueOnce("bundle-uninstall-token").mockResolvedValueOnce({
      success: true,
    });

    await managerApi.winUninstallBundle(true, false, false);

    expect(invokeMock).toHaveBeenNthCalledWith(1, "arm_destructive", {
      kind: "uninstall",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "win_uninstall_bundle", {
      confirm: true,
      token: "bundle-uninstall-token",
      purgeUserData: false,
      purgeCcswitchData: false,
      forceCleanup: false,
      removeUsbMedia: false,
    });
  });

  it("uses the dedicated command to acknowledge a pending uninstall handoff", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue(undefined);

    await managerApi.completeUninstallHandoff("handoff-test-id");

    expect(invokeMock).toHaveBeenCalledWith("complete_uninstall_handoff", {
      handoffId: "handoff-test-id",
    });
  });

  it("sends the renderer resume mode and one-shot install root to the backend", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue({ success: true });

    await managerApi.winPerformUpdate(
      true,
      {
        currentVersion: null,
        latestVersion: "2.0.0",
        packageMoniker: "Codex_2.0.0_x64",
        route: "msix-sideload",
      },
      "D:\\Selected\\Codex",
      "resume-token",
      "install",
    );

    expect(invokeMock).toHaveBeenCalledWith("win_perform_update", {
      confirm: true,
      token: "resume-token",
      installRoot: "D:\\Selected\\Codex",
      expected: {
        currentVersion: null,
        latestVersion: "2.0.0",
        packageMoniker: "Codex_2.0.0_x64",
        route: "msix-sideload",
      },
      resumeKind: "install",
    });
  });

  it("marks the offline install as USB AppXVolume-only without changing normal callers", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue({ success: true });

    await managerApi.winInstallHistoricalRelease(
      {
        releaseTag: "local-signed-26.602.40724",
        version: "26.602.40724",
        assetName: "OpenAI.Codex_x64.msix",
        architecture: "x64",
        format: "msix",
        packageVersion: "26.602.3474.0",
        localPath: "C:\\Temp\\CodexDesktopInstaller\\OpenAI.Codex_x64.msix",
        localFileName: "OpenAI.Codex_x64.msix",
      },
      true,
      { currentPath: null, currentVersion: null, currentSource: null },
      "offline-token",
      undefined,
      true,
      true,
    );

    expect(invokeMock).toHaveBeenCalledWith("win_install_historical_release", {
      confirm: true,
      token: "offline-token",
      releaseTag: "local-signed-26.602.40724",
      assetName: "OpenAI.Codex_x64.msix",
      localPath: "C:\\Temp\\CodexDesktopInstaller\\OpenAI.Codex_x64.msix",
      architecture: "x64",
      blockUpdates: true,
      installRoot: undefined,
      expected: { currentPath: null, currentVersion: null, currentSource: null },
      forceMsix: true,
      installToUsb: true,
    });
  });
});

describe("Codex payload API", () => {
  it("does not invoke the destructive command outside the Tauri runtime", async () => {
    await expect(managerApi.applyCodexPayloadUpdate(CODEX_PAYLOAD_INFO)).rejects.toThrow(
      "payload update requires the desktop app",
    );
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("keeps the payload command and UpdateInfo round trip inside Tauri", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue(undefined);

    await managerApi.applyCodexPayloadUpdate(CODEX_PAYLOAD_INFO);

    expect(invokeMock).toHaveBeenCalledWith("payload_apply_codex", {
      info: CODEX_PAYLOAD_INFO,
    });
  });
});

describe("Skill Hub API", () => {
  it("keeps only the safe source-attempt fields at the IPC boundary", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue({
      attempts: [
        {
          sourceId: "cn-mirror-1",
          sourceKind: "domestic",
          phase: "metadata",
          result: "timeout",
          httpStatus: null,
          elapsedMs: 8_001.4,
          url: "https://secret.example/source",
        },
        {
          sourceId: "https://account.example/source",
          sourceKind: "domestic",
          phase: "metadata",
          result: "ok",
          httpStatus: 200,
          elapsedMs: 1,
        },
      ],
    });

    const info = await managerApi.skillRepoCheckUpdate();

    expect(info.attempts).toEqual([
      {
        sourceId: "cn-mirror-1",
        sourceKind: "domestic",
        phase: "metadata",
        result: "timeout",
        httpStatus: null,
        elapsedMs: 8_001,
      },
    ]);
    expect(invokeMock).toHaveBeenCalledWith("skill_repo_check_update");
  });

  it("reads structured error attempts without exposing untrusted source values", () => {
    const attempts = [
      {
        sourceId: "cn-mirror-2",
        sourceKind: "domestic",
        phase: "archive",
        result: "http",
        httpStatus: 503,
        elapsedMs: 2_000,
      },
      {
        sourceId: "https://secret.example/source",
        sourceKind: "domestic",
        phase: "archive",
        result: "ok",
        httpStatus: 200,
        elapsedMs: 1,
      },
    ];
    const expected = [
      {
        sourceId: "cn-mirror-2",
        sourceKind: "domestic",
        phase: "archive",
        result: "http",
        httpStatus: 503,
        elapsedMs: 2_000,
      },
    ];

    expect(skillRepoAttempts({ attempts })).toEqual(expected);
    expect(skillRepoAttempts({ detail: JSON.stringify({ attempts }) })).toEqual(expected);
  });
});

describe("macOS resumable target API", () => {
  it("sends exact update and fresh-install targets to the backend", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue({});

    await managerApi.macPerformUpdate({
      fromBuild: 100,
      toBuild: 150,
      path: "/Applications/Codex.app",
      fromVersion: "1.0.0",
      toVersion: "1.5.0",
    });
    await managerApi.macInstall({
      targetBuild: 150,
      targetVersion: "1.5.0",
    });

    expect(invokeMock).toHaveBeenNthCalledWith(1, "arm_destructive", {
      kind: "update",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "mac_perform_update", {
      confirm: true,
      token: {},
      expected: {
        fromBuild: 100,
        toBuild: 150,
        path: "/Applications/Codex.app",
        fromVersion: "1.0.0",
        toVersion: "1.5.0",
      },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "mac_install", {
      expectedTargetBuild: 150,
      expectedTargetVersion: "1.5.0",
    });
  });
});

describe("404token account API", () => {
  it("returns unbound status fallback when not in Tauri runtime", async () => {
    const status = await managerApi.accountStatus?.();
    expect(status?.bound).toBe(false);
    expect(status?.message).toContain("account unavailable");
  });

  it("invokes account status command in Tauri", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue({
      bound: true,
      accountName: "USB-0001",
      loggedIn: false,
      balance: null,
      usage: null,
      message: "已绑定,请输入密码登录",
    });

    const status = await managerApi.accountStatus?.();

    expect(invokeMock).toHaveBeenCalledWith("account_status");
    expect(status?.bound).toBe(true);
    expect(status?.accountName).toBe("USB-0001");
    expect(status?.loggedIn).toBe(false);
  });

  it("invokes account login with password", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue({
      bound: true,
      accountName: "USB-0001",
      loggedIn: true,
      balance: 12500,
      usage: { today: 320, thisMonth: 8460 },
      message: "登录成功",
    });

    const status = await managerApi.accountLogin("test-password");

    expect(invokeMock).toHaveBeenCalledWith("account_login", {
      password: "test-password",
    });
    expect(status.loggedIn).toBe(true);
    expect(status.balance).toBe(12500);
  });

  it("invokes account refresh", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue({
      bound: true,
      accountName: "USB-0001",
      loggedIn: true,
      balance: 15000,
      usage: { today: 450 },
      message: "账户信息已刷新",
    });

    const status = await managerApi.accountRefresh();

    expect(invokeMock).toHaveBeenCalledWith("account_refresh");
    expect(status.balance).toBe(15000);
  });

  it("invokes account redeem with code", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue({
      bound: true,
      accountName: "USB-0001",
      loggedIn: true,
      balance: 25000,
      usage: null,
      message: "充值成功,余额已更新",
    });

    const status = await managerApi.accountRedeem("RECHARGE-ABC123");

    expect(invokeMock).toHaveBeenCalledWith("account_redeem", {
      code: "RECHARGE-ABC123",
    });
    expect(status.balance).toBe(25000);
    expect(status.message).toContain("充值成功");
  });

  it("handles account errors gracefully", async () => {
    window.__TAURI_INTERNALS__ = {};
    invokeMock.mockRejectedValue(new Error("登录失败：密码错误或会话已过期,请重新登录"));

    await expect(managerApi.accountLogin("wrong-password")).rejects.toThrow("密码错误");
  });
});


describe("advanced gateway diagnostic API", () => {
  it("passes the selected model IDs and one-run cost consent through IPC", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    invokeMock.mockResolvedValue({});
    await managerApi.gatewayModelProbe({
      requestId: "gateway-probe-fixture",
      selectedModelIds: ["glm-5.2"],
      costConsent: true,
    });
    expect(invokeMock).toHaveBeenCalledWith("gateway_model_probe", {
      request: {
        requestId: "gateway-probe-fixture",
        selectedModelIds: ["glm-5.2"],
        costConsent: true,
      },
    });
  });
  it("uses the request ID to cancel the active diagnostic", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    invokeMock.mockResolvedValue(true);
    await managerApi.cancelGatewayModelProbe("fixture-run");
    expect(invokeMock).toHaveBeenCalledWith("cancel_gateway_model_probe", { requestId: "fixture-run" });
  });
  it("passes the quick-test model ID without cost-matrix consent", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    invokeMock.mockResolvedValue({
      status: "passed", connectionFingerprint: "binding-fixture",
      connectivityModelId: "vendor/model-latest", connectivityProofId: "proof-fixture",
      connectivityProofExpiresAt: 2_000_000_000,
    });
    await managerApi.gatewayModelProbe({
      requestId: "quick-test-fixture",
      selectedModelIds: [],
      costConsent: false,
      connectivityModelId: "vendor/model-latest",
    });
    expect(invokeMock).toHaveBeenCalledWith("gateway_model_probe", {
      request: {
        requestId: "quick-test-fixture",
        selectedModelIds: [],
        costConsent: false,
        connectivityModelId: "vendor/model-latest",
      },
    });
  });
  it("maps catalog actions to the one manager command", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    invokeMock.mockResolvedValue({ status: "ok", modelIds: ["vendor/model-latest"], restartRequired: true, message: "done" });
    const request = {
      operation: "add" as const,
      requestId: "add-fixture",
      modelId: "vendor/model-latest",
      connectivityProofId: "proof-fixture",
    };
    await managerApi.manageNativeModelCatalog(request);
    expect(invokeMock).toHaveBeenCalledWith("manage_native_model_catalog", { request });
    await managerApi.manageNativeModelCatalog({ operation: "state" });
    await managerApi.manageNativeModelCatalog({ operation: "reset" });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "manage_native_model_catalog", {
      request: { operation: "state" },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "manage_native_model_catalog", {
      request: { operation: "reset" },
    });
  });
  it("browser preview cannot probe or issue model requests", async () => {
    const report = await managerApi.gatewayModelProbe({
      requestId: "fixture-browser", selectedModelIds: [], costConsent: false,
    });
    expect(report.status).toBe("skipped");
    expect(report).toMatchObject({ endpoint: "", models: [], results: [], generatedRequests: 0 });
    expect(await managerApi.cancelGatewayModelProbe("fixture-browser")).toBe(false);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
