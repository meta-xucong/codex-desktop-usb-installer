import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { listen } from "@tauri-apps/api/event";

import { managerApi, type UpdateInfo } from "../../services/managerApi";
import type { OfflinePayloadReport, OfflineStageReport, UsbGuardStatus } from "../../shared/types";
import { I18nProvider } from "../i18n";
import { ThemeProvider } from "../theme";
import { OfflineHome } from "./OfflineHome";

vi.mock("../../services/managerApi", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../services/managerApi")>();
  return {
    ...actual,
    managerApi: {
      offlinePayloadProbe: vi.fn(),
      winCheckInstallPreflight: vi.fn(),
      offlinePayloadStage: vi.fn(),
      offlinePayloadCleanup: vi.fn(),
      winStatus: vi.fn(),
      winAdopt: vi.fn(),
      winInstallHistoricalRelease: vi.fn(),
      winLaunch: vi.fn(),
      checkCodexPayloadUpdate: vi.fn(),
      applyCodexPayloadUpdate: vi.fn(),
      installAndLaunchCcswitch: vi.fn(),
      gatewayModelProbe: vi.fn(),
      cancelGatewayModelProbe: vi.fn(),
      manageNativeModelCatalog: vi.fn(),
      accountStatus: vi.fn(),
      accountLogin: vi.fn(),
      accountRefresh: vi.fn(),
      accountRedeem: vi.fn(),
      capabilityPackStatus: vi.fn(),
      mediaModelCatalog: vi.fn(),
      mediaModelSettings: vi.fn(),
      installCapabilityPack: vi.fn(),
      usbGuardStatus: vi.fn(),
      openLogsDir: vi.fn(),
    },
  };
});

const api = vi.mocked(managerApi);
const listenMock = vi.mocked(listen);

const PAYLOAD: OfflinePayloadReport = {
  ready: true,
  manifestPath: "F:\\manifest.json",
  payloadPath: "F:\\payload\\OpenAI.Codex_x64.msix",
  fileName: "OpenAI.Codex_x64.msix",
  size: 1024,
  sha256: "a".repeat(64),
  hashVerified: true,
  packageName: "OpenAI.Codex",
  packageVersion: "26.602.3474.0",
  codexDesktopVersion: "26.602.40724",
  architecture: "x64",
  minWindowsBuild: "19041",
  appEntry: "app\\Codex.exe",
  appVersionVerified: true,
  releaseTag: "local-signed-26.602.40724",
  authenticode: {
    trusted: true,
    publisherIsOpenai: true,
    status: "Valid",
    statusMessage: "ok",
    subject: "CN=OpenAI",
    issuer: "Microsoft Marketplace CA",
    thumbprint: "ABC",
  },
  warning: null,
};

const STAGE: OfflineStageReport = {
  sourcePath: PAYLOAD.payloadPath,
  stagedPath: "C:\\Users\\test\\AppData\\Local\\Temp\\CodexDesktopInstaller\\OpenAI.Codex_x64.msix",
  size: PAYLOAD.size,
  sha256: PAYLOAD.sha256,
  payload: PAYLOAD,
};

const USB_GUARD: UsbGuardStatus = {
  schemaVersion: 1,
  status: "authorized",
  watchdog: "running",
  removable: true,
  driveRoot: "F:\\",
  bundlePath: "F:\\usb-oneclick",
  bundleRelativePath: "usb-oneclick",
  signed: true,
  codexAllowed: true,
  lastCheckedAtMs: Date.now(),
  missingSinceMs: null,
  countdownSeconds: null,
  managedProcesses: [],
  message: "U 盘已验证，签名许可证有效",
};

const USB_PORTABLE_CODEX = {
  path: "F:\\usb-oneclick\\apps\\Codex",
  version: PAYLOAD.codexDesktopVersion,
  arch: "x64",
  source: "portable",
  packageFamilyName: null,
  packageVolume: null,
};

function renderView(onOpenUninstall = vi.fn()) {
  return render(
    <ThemeProvider>
      <I18nProvider>
        <OfflineHome onOpenSettings={vi.fn()} onOpenUninstall={onOpenUninstall} />
      </I18nProvider>
    </ThemeProvider>,
  );
}

describe("OfflineHome", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    localStorage.removeItem("codex-usb-ccswitch-injection-issue-v1");
    localStorage.setItem("cam.lang", "zh-CN");
    api.offlinePayloadProbe.mockResolvedValue(PAYLOAD);
    api.winCheckInstallPreflight.mockResolvedValue({
      internetAvailable: true,
      apiKeyReady: true,
    });
    api.offlinePayloadStage.mockResolvedValue(STAGE);
    api.offlinePayloadCleanup.mockResolvedValue();
    api.winStatus.mockResolvedValue({ installed: null, status: "none" });
    api.winAdopt.mockResolvedValue({ installed: null, status: "managed" });
    api.winInstallHistoricalRelease.mockResolvedValue({
      success: true,
      message: "installed",
    } as never);
    api.winLaunch.mockResolvedValue();
    api.checkCodexPayloadUpdate.mockResolvedValue({ kind: "none" });
    api.applyCodexPayloadUpdate.mockResolvedValue();
    api.installAndLaunchCcswitch.mockResolvedValue({
      available: true,
      installed: true,
      launched: true,
      sourcePath: "F:\\ccswitch\\CodexSwitch.exe",
      installedPath: "C:\\Users\\test\\AppData\\Local\\Programs\\CodexSwitch\\CodexSwitch.exe",
      copiedFiles: 5,
      accountInjected: true,
      accountInjectionStatus: "verified",
      message: "launched",
    });
    api.cancelGatewayModelProbe.mockResolvedValue(true);
    api.manageNativeModelCatalog.mockResolvedValue({
      status: "ok", modelIds: [], restartRequired: false, message: "native list",
    });
    api.gatewayModelProbe.mockResolvedValue({
      status: "connected", endpoint: "https://404token.xyz/v1", credentialFingerprint: "sha256:fixture", connectionFingerprint: "binding-fixture",
      checkedAt: 1, models: [], results: [], generatedRequests: 0, errorCode: null,
    });
    api.accountStatus.mockResolvedValue({
      bound: false,
      accountName: null,
      loggedIn: false,
      balance: null,
      usage: null,
      message: "此 U 盘尚未绑定 404token 账号",
    });
    api.capabilityPackStatus.mockResolvedValue({
      available: true,
      installed: false,
      success: false,
      coreReady: false,
      overallStatus: "failed",
      sourcePath: "F:\\content\\codex-starter-kit",
      packVersion: "1.4.0",
      skills: 0,
      agents: 0,
      runtimeReady: false,
      runtimeStatus: "unknown",
      configStatus: "unknown",
      diagnosticsPath: null,
      feishuEnabled: false,
      enabledApiServices: [],
      mediaRuntimeStatus: "unknown",
      mediaProtocolStatus: "unknown",
      mediaGatewayCatalogStatus: "not-checked",
      mediaGatewayModelCount: 0,
      mediaGatewayCatalogVerifiedAt: null,
      mediaGenerationCapability: "unknown",
      codexRestartRequired: false,
      codexRunning: false,
      message: "ready",
    });
    api.installCapabilityPack.mockResolvedValue({
      available: true,
      installed: true,
      success: true,
      coreReady: true,
      overallStatus: "ready",
      sourcePath: "F:\\content\\codex-starter-kit",
      packVersion: "1.4.0",
      skills: 55,
      agents: 7,
      runtimeReady: true,
      runtimeStatus: "ready",
      configStatus: "merged",
      diagnosticsPath: null,
      feishuEnabled: false,
      enabledApiServices: [],
      mediaRuntimeStatus: "ready",
      mediaProtocolStatus: "ready",
      mediaGatewayCatalogStatus: "verified",
      mediaGatewayModelCount: 3,
      mediaGatewayCatalogVerifiedAt: "2026-10-13T00:00:00Z",
      mediaGenerationCapability: "available",
      codexRestartRequired: false,
      codexRunning: false,
      message: "installed",
    });
    api.mediaModelCatalog.mockResolvedValue({
      models: ["gpt-image-2", "gpt-image-2.5", "grok-imagine-image", "grok-video-2", "seedance-2.0"],
      configuredSeedanceModel: null,
      configuredAgentId: null,
    });
    api.mediaModelSettings.mockResolvedValue({
      licensedCredentialAvailable: false,
      mediaGenerationAgentId: "",
      image2Model: "",
      grokImageModel: "",
      grokVideoModel: "",
      seedanceModel: "",
      minimaxVideoModel: "",
    });
    api.openLogsDir.mockResolvedValue();
    api.usbGuardStatus.mockResolvedValue(USB_GUARD);
  });

  it("loads and displays the fixed package evidence", async () => {
    renderView();
    await waitFor(() => expect(screen.getByText("26.602.40724")).toBeInTheDocument());
    expect(screen.getByText("OpenAI.Codex_x64.msix")).toBeInTheDocument();
    expect(screen.getByText("校验通过")).toBeInTheDocument();
    expect(screen.getByTestId("usb-guard-status")).toHaveTextContent("U 盘绑定已启用");
  });

  it("renders the local shell before the slow payload hash completes", async () => {
    let resolveProbe: (value: OfflinePayloadReport) => void = () => undefined;
    api.offlinePayloadProbe.mockReturnValue(
      new Promise<OfflinePayloadReport>((resolve) => {
        resolveProbe = resolve;
      }),
    );
    renderView();

    expect(await screen.findByTestId("usb-guard-status")).toHaveTextContent("U 盘绑定已启用");
    expect(screen.getByText("正在校验 U 盘安装包…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "一键安装" })).toBeDisabled();

    resolveProbe(PAYLOAD);
    await waitFor(() => expect(screen.getByText("26.602.40724")).toBeInTheDocument());
  });

  it("shows a spinning readiness indicator while USB authorization settles", async () => {
    let resolveProbe: (value: OfflinePayloadReport) => void = () => undefined;
    api.offlinePayloadProbe.mockReturnValue(
      new Promise<OfflinePayloadReport>((resolve) => {
        resolveProbe = resolve;
      }),
    );
    api.usbGuardStatus.mockResolvedValue({
      ...USB_GUARD,
      status: "starting",
      watchdog: "not-running",
      codexAllowed: false,
      message: "正在校验 U 盘能力包",
    });

    const { container } = renderView();

    await waitFor(() => {
      expect(screen.getByText("正在完成启动校验，完成后会自动开放安装和更新；请保持 U 盘连接。")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "一键安装" })).toBeDisabled();
    });
    expect(container.querySelector("[data-testid=offline-hero] .ring.spin")).not.toBeNull();

    resolveProbe(PAYLOAD);
    await waitFor(() => expect(screen.getByText("26.602.40724")).toBeInTheDocument());
    expect(container.querySelector("[data-testid=offline-hero] .ring.spin")).not.toBeNull();
  });

  it("refreshes startup authorization after the initial guard snapshot", async () => {
    api.usbGuardStatus
      .mockResolvedValueOnce({
        ...USB_GUARD,
        status: "starting",
        watchdog: "not-running",
        codexAllowed: false,
        message: "正在校验 U 盘能力包",
      })
      .mockResolvedValue(USB_GUARD);

    renderView();

    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeEnabled());
    expect(screen.getByRole("button", { name: "检查并安装更新" })).toBeEnabled();
    expect(api.usbGuardStatus.mock.calls.length).toBeGreaterThanOrEqual(2);
  });

  it("re-reads an installation after USB authorization so a stale external snapshot cannot block it", async () => {
    api.winStatus
      .mockResolvedValueOnce({ status: "external", installed: USB_PORTABLE_CODEX })
      .mockResolvedValue({ status: "managed", installed: USB_PORTABLE_CODEX });
    api.usbGuardStatus
      .mockResolvedValueOnce({
        ...USB_GUARD,
        status: "starting",
        watchdog: "not-running",
        codexAllowed: false,
      })
      .mockResolvedValue(USB_GUARD);

    renderView();

    await waitFor(() => expect(screen.getByText("此版本已经安装")).toBeInTheDocument());
    expect(screen.queryByText(/Codex 已安装在其他位置/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "检查并安装更新" })).toBeEnabled();
  });

  it("does not claim Codex is installed until signed authorization is live", async () => {
    api.winStatus.mockResolvedValue({
      status: "managed",
      installed: USB_PORTABLE_CODEX,
    });
    api.usbGuardStatus.mockResolvedValue({
      ...USB_GUARD,
      signed: true,
      codexAllowed: false,
      codexBlockReason: "授权看门狗未运行",
    });

    renderView();
    await waitFor(() => expect(screen.getByText("安装包已准备好")).toBeInTheDocument());
    expect(screen.queryByText("此版本已经安装")).not.toBeInTheDocument();
    expect(screen.queryByText("安装位置")).not.toBeInTheDocument();
  });

  it("keeps install and launch available without an upstream model proof", async () => {
    api.winStatus.mockResolvedValue({ status: "managed", installed: USB_PORTABLE_CODEX });
    renderView();

    expect(await screen.findByRole("button", { name: "启动 Codex" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "打开 Codex Switch" })).toBeEnabled();
    expect(api.gatewayModelProbe).not.toHaveBeenCalled();
    expect(api.manageNativeModelCatalog).not.toHaveBeenCalled();
  });

  it("opens model management from the home page only after an explicit click", async () => {
    const user = userEvent.setup();
    api.winStatus.mockResolvedValue({ status: "managed", installed: USB_PORTABLE_CODEX });
    renderView();
    await screen.findByRole("button", { name: "启动 Codex" });
    expect(api.gatewayModelProbe).not.toHaveBeenCalled();
    expect(api.manageNativeModelCatalog).not.toHaveBeenCalled();

    const manageModels = screen.getByRole("button", { name: "管理模型" });
    const uninstall = screen.getByRole("button", { name: "卸载 Codex 与 CCSwitch" });
    expect(manageModels.compareDocumentPosition(uninstall) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    await user.click(manageModels);
    await screen.findByRole("heading", { name: "管理模型" });
    await waitFor(() => expect(api.gatewayModelProbe).toHaveBeenCalledTimes(1));
    expect(api.gatewayModelProbe.mock.calls[0]?.[0].selectedModelIds).toEqual([]);
    expect(api.manageNativeModelCatalog).toHaveBeenCalledWith({ operation: "state" });
  });

  it("disables the initial install action for an unsigned USB", async () => {
    api.usbGuardStatus.mockResolvedValue({
      ...USB_GUARD,
      signed: false,
      codexAllowed: false,
      signatureState: "unsigned-development",
    });

    renderView();
    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeDisabled());
  });

  it("fails closed when the initial USB Guard status query fails", async () => {
    api.usbGuardStatus.mockRejectedValue(new Error("status unavailable"));

    renderView();
    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeDisabled());
    expect(api.winInstallHistoricalRelease).not.toHaveBeenCalled();
  });

  it("shows the unplug grace countdown reported by the helper", async () => {
    api.usbGuardStatus.mockResolvedValue({
      ...USB_GUARD,
      status: "grace",
      countdownSeconds: 4,
      message: "暂时未检测到 U 盘，正在等待重新连接",
    });
    renderView();
    await waitFor(() => expect(screen.getByTestId("usb-guard-status")).toHaveTextContent("4s"));
    expect(screen.getByTestId("usb-guard-status")).toHaveTextContent("正在等待 U 盘重新连接");
  });

  it("exposes the signed USB update flow and redeploys the refreshed payload", async () => {
    const user = userEvent.setup();
    const nextPayload: OfflinePayloadReport = {
      ...PAYLOAD,
      codexDesktopVersion: "26.700.1",
      packageVersion: "26.700.1.0",
      releaseTag: "local-signed-26.700.1",
    };
    const nextStage: OfflineStageReport = {
      ...STAGE,
      payload: nextPayload,
      sha256: "b".repeat(64),
      stagedPath: "C:\\Temp\\CodexDesktopInstaller\\OpenAI.Codex_updated.msix",
    };
    const updateInfo: UpdateInfo = {
      channel: "codex",
      currentVersion: PAYLOAD.codexDesktopVersion,
      latestVersion: nextPayload.codexDesktopVersion,
      releaseNotes: null,
      sizeBytes: nextPayload.size,
      requiresRestart: false,
      payload: {},
    };
    api.usbGuardStatus.mockResolvedValue({ ...USB_GUARD, signed: true });
    api.checkCodexPayloadUpdate.mockResolvedValue({ kind: "available", info: updateInfo });
    api.offlinePayloadProbe.mockResolvedValueOnce(PAYLOAD).mockResolvedValue(nextPayload);
    api.offlinePayloadStage.mockResolvedValue(nextStage);
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({
        status: "managed",
        installed: { ...USB_PORTABLE_CODEX, version: nextPayload.codexDesktopVersion },
      });

    renderView();
    const update = await screen.findByRole("button", { name: "检查并安装更新" });
    expect(update).toBeEnabled();
    await user.click(update);
    expect(await screen.findByRole("heading", { name: "更新 Codex 到 26.700.1？" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^更新$/ }));

    await waitFor(() => expect(api.checkCodexPayloadUpdate).toHaveBeenCalledTimes(1));
    expect(api.applyCodexPayloadUpdate).toHaveBeenCalledWith(updateInfo);
    await waitFor(() => expect(api.winInstallHistoricalRelease).toHaveBeenCalledTimes(1));
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[0]).toMatchObject({
      version: nextPayload.codexDesktopVersion,
      localPath: nextStage.stagedPath,
    });
    expect(await screen.findByText("Codex 已更新到最新版。")).toBeInTheDocument();
  });

  it("uses the same full-screen progress view while the USB update payload downloads", async () => {
    const user = userEvent.setup();
    let payloadProgressHandler: ((event: { payload: unknown }) => void) | null = null;
    listenMock.mockImplementation((event: string, handler: unknown) => {
      if (event === "payload-update-progress") {
        payloadProgressHandler = handler as (event: { payload: unknown }) => void;
      }
      return Promise.resolve(() => {});
    });
    const updateInfo: UpdateInfo = {
      channel: "codex",
      currentVersion: PAYLOAD.codexDesktopVersion,
      latestVersion: "26.700.1",
      releaseNotes: null,
      sizeBytes: 4096,
      requiresRestart: false,
      payload: {},
    };
    api.usbGuardStatus.mockResolvedValue({ ...USB_GUARD, signed: true });
    api.checkCodexPayloadUpdate.mockResolvedValue({ kind: "available", info: updateInfo });
    api.applyCodexPayloadUpdate.mockImplementation(() => new Promise(() => {}));

    renderView();
    await user.click(await screen.findByRole("button", { name: "检查并安装更新" }));
    await user.click(await screen.findByRole("button", { name: /^更新$/ }));

    await waitFor(() => {
      expect(screen.getByTestId("install-progress-bar")).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "暂停" })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "取消" })).not.toBeInTheDocument();
    });
    expect(payloadProgressHandler).not.toBeNull();
    const emitPayloadProgress = payloadProgressHandler as unknown as (
      event: { payload: unknown },
    ) => void;
    emitPayloadProgress({
      payload: { phase: "download", bytesDone: 2048, bytesTotal: 4096 },
    });
    await waitFor(() =>
      expect(screen.getByTestId("install-progress-bar")).toHaveAttribute(
        "aria-valuenow",
        "50",
      ),
    );
  });

  it("rechecks once when the confirmed Codex update becomes stale", async () => {
    const user = userEvent.setup();
    const firstInfo: UpdateInfo = {
      channel: "codex",
      currentVersion: PAYLOAD.codexDesktopVersion,
      latestVersion: "26.700.1",
      releaseNotes: null,
      sizeBytes: PAYLOAD.size,
      requiresRestart: false,
      payload: {},
    };
    const refreshedInfo: UpdateInfo = {
      ...firstInfo,
      latestVersion: "26.700.2",
    };
    api.usbGuardStatus.mockResolvedValue({ ...USB_GUARD, signed: true });
    api.checkCodexPayloadUpdate
      .mockResolvedValueOnce({ kind: "available", info: firstInfo })
      .mockResolvedValueOnce({ kind: "available", info: refreshedInfo });
    api.applyCodexPayloadUpdate.mockRejectedValueOnce({
      code: "stale_expectation",
      message: "远端 Codex 更新信息已变化（发布版本身份已变化），请重新检查后再确认。",
    });

    renderView();
    await user.click(await screen.findByRole("button", { name: "检查并安装更新" }));
    await user.click(await screen.findByRole("button", { name: /^更新$/ }));

    await waitFor(() => expect(api.checkCodexPayloadUpdate).toHaveBeenCalledTimes(2));
    expect(api.applyCodexPayloadUpdate).toHaveBeenCalledTimes(1);
    expect(
      await screen.findByRole("heading", { name: "更新 Codex 到 26.700.2？" }),
    ).toBeInTheDocument();
    expect(await screen.findByText(/已刷新到 26\.700\.2/)).toBeInTheDocument();
  });

  it("keeps the update entry visible but disabled for an unsigned development USB", async () => {
    api.usbGuardStatus.mockResolvedValue({
      ...USB_GUARD,
      signed: false,
      codexAllowed: false,
      signatureState: "unsigned-development",
      codexBlockReason: "未签名开发许可证不能执行 Codex 操作",
      message: "U 盘已验证（开发许可证未签名），看门狗运行中",
    });
    renderView();
    const update = await screen.findByRole("button", { name: "检查并安装更新" });
    expect(update).toBeDisabled();
    expect(screen.getByText(/需要已配置签名许可证/)).toBeInTheDocument();
  });

  it("blocks the USB install before staging when 404token is unreachable", async () => {
    const user = userEvent.setup();
    api.winCheckInstallPreflight.mockResolvedValue({
      internetAvailable: false,
      apiKeyReady: true,
    });
    renderView();

    await user.click(await screen.findByRole("button", { name: "一键安装" }));

    expect(await screen.findByText(/安装需要连接互联网并能访问 404token/)).toBeInTheDocument();
    expect(api.offlinePayloadStage).not.toHaveBeenCalled();
    expect(api.winInstallHistoricalRelease).not.toHaveBeenCalled();
    expect(api.installCapabilityPack).not.toHaveBeenCalled();
  });

  it("blocks the USB install before staging when the signed API key binding is missing", async () => {
    const user = userEvent.setup();
    api.winCheckInstallPreflight.mockResolvedValue({
      internetAvailable: true,
      apiKeyReady: false,
    });
    renderView();

    await user.click(await screen.findByRole("button", { name: "一键安装" }));

    expect(await screen.findByText(/签名 U 盘没有可自动导入的 404token API Key/)).toBeInTheDocument();
    expect(api.offlinePayloadStage).not.toHaveBeenCalled();
    expect(api.winInstallHistoricalRelease).not.toHaveBeenCalled();
    expect(api.installCapabilityPack).not.toHaveBeenCalled();
  });

  it("stages the payload and launches CCSwitch without running model probes", async () => {
    const user = userEvent.setup();
    const setupOrder: string[] = [];
    localStorage.setItem("codex-usb-ccswitch-injection-issue-v1", "launch-failed");
    api.installAndLaunchCcswitch.mockImplementationOnce(async () => {
      setupOrder.push("ccswitch-launch");
      return {
        available: true,
        installed: true,
        launched: true,
        sourcePath: "F:\\ccswitch\\CodexSwitch.exe",
        installedPath: "C:\\Users\\test\\AppData\\Local\\Programs\\CodexSwitch\\CodexSwitch.exe",
        copiedFiles: 5,
        accountInjected: true,
        accountInjectionStatus: "verified",
        message: "launched",
      };
    });
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({ status: "managed", installed: USB_PORTABLE_CODEX });
    renderView();
    api.offlinePayloadStage.mockResolvedValue({
      ...STAGE,
      payload: { ...STAGE.payload, releaseTag: "upstream-26.602.40724" },
    });
    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "一键安装" }));
    await waitFor(() => expect(api.winInstallHistoricalRelease).toHaveBeenCalled());
    expect(api.offlinePayloadStage).toHaveBeenCalledTimes(1);
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[0]).toMatchObject({
      localPath: STAGE.stagedPath,
      version: PAYLOAD.codexDesktopVersion,
      format: "msix",
      releaseTag: "local-signed-" + PAYLOAD.codexDesktopVersion,
    });
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[1]).toBe(true);
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[5]).toBe(true);
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[6]).toBe(true);
    expect(api.offlinePayloadCleanup).toHaveBeenCalledWith(STAGE.stagedPath);
    expect(api.installCapabilityPack).toHaveBeenCalledTimes(1);
    expect(api.installCapabilityPack.mock.calls[0]?.[0]).toMatchObject({
      enableFeishu: false,
      enableMediaGeneration: true,
      persistConnections: true,
      image2Model: "gpt-image-2.5",
      grokImageModel: "grok-imagine-image",
      grokVideoModel: "grok-video-2",
      seedanceModel: "",
      mediaGatewayCatalogStatus: "verified",
      mediaGatewayModelCount: 5,
      enabledApiServices: [],
    });
    await waitFor(() => expect(api.installAndLaunchCcswitch).toHaveBeenCalledTimes(1));
    expect(setupOrder).toEqual(["ccswitch-launch"]);
    expect(api.gatewayModelProbe).not.toHaveBeenCalled();
    expect(screen.queryByText(/404token API Key 自动/)).not.toBeInTheDocument();
    expect(localStorage.getItem("codex-usb-ccswitch-injection-issue-v1")).toBeNull();
    expect(await screen.findByRole("button", { name: "启动 Codex" })).toBeEnabled();
  });

  it("keeps the capability pack installable when the media model catalog has no supported routes", async () => {
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({ status: "managed", installed: USB_PORTABLE_CODEX });
    api.mediaModelCatalog.mockResolvedValueOnce({ models: ["gpt-4o", "claude-opus-4-8"] });
    renderView();
    api.offlinePayloadStage.mockResolvedValue({
      ...STAGE,
      payload: { ...STAGE.payload, releaseTag: "upstream-26.602.40724" },
    });
    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeEnabled());
    await userEvent.setup().click(screen.getByRole("button", { name: "一键安装" }));

    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack.mock.calls[0]?.[0]).toMatchObject({
      persistConnections: true,
      enableMediaGeneration: false,
      mediaGatewayCatalogStatus: "verified",
      mediaGatewayModelCount: 2,
      image2Model: "",
      grokImageModel: "",
      grokVideoModel: "",
      seedanceModel: "",
    });
    expect(await screen.findByText(/媒体生成暂未启用/)).toBeInTheDocument();
  });

  it("preserves saved media selections when the catalog request fails", async () => {
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({ status: "managed", installed: USB_PORTABLE_CODEX });
    api.mediaModelCatalog.mockRejectedValueOnce(new Error("offline"));
    renderView();
    api.offlinePayloadStage.mockResolvedValue({
      ...STAGE,
      payload: { ...STAGE.payload, releaseTag: "upstream-26.602.40724" },
    });
    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeEnabled());
    await userEvent.setup().click(screen.getByRole("button", { name: "一键安装" }));

    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack.mock.calls[0]?.[0]).toMatchObject({
      persistConnections: false,
      enableMediaGeneration: false,
      mediaGatewayCatalogStatus: "failed",
      image2Model: "",
    });
    expect(await screen.findByText(/媒体生成暂未启用/)).toBeInTheDocument();
  });

  it("keeps an existing media MCP enabled when the catalog is unavailable but local routes are configured", async () => {
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({ status: "managed", installed: USB_PORTABLE_CODEX });
    api.mediaModelCatalog.mockRejectedValueOnce(new Error("offline"));
    api.mediaModelSettings.mockResolvedValueOnce({
      licensedCredentialAvailable: true,
      mediaGenerationAgentId: "",
      image2Model: "gpt-image-2.5",
      grokImageModel: "",
      grokVideoModel: "",
      seedanceModel: "",
      minimaxVideoModel: "",
    });
    renderView();
    api.offlinePayloadStage.mockResolvedValue({
      ...STAGE,
      payload: { ...STAGE.payload, releaseTag: "upstream-26.602.40724" },
    });
    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeEnabled());
    await userEvent.setup().click(screen.getByRole("button", { name: "一键安装" }));

    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack.mock.calls[0]?.[0]).toMatchObject({
      persistConnections: false,
      enableMediaGeneration: true,
      mediaGatewayCatalogStatus: "failed",
      image2Model: "gpt-image-2.5",
    });
    expect(await screen.findByText(/继续使用已有媒体模型设置/)).toBeInTheDocument();
  });

  it("keeps a still-listed Seedance route already configured by the user", async () => {
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({ status: "managed", installed: USB_PORTABLE_CODEX });
    api.mediaModelCatalog.mockResolvedValueOnce({
      models: ["seedance-2.0"],
      configuredSeedanceModel: "seedance-2.0",
      configuredAgentId: "agent-route-42",
    });
    renderView();
    api.offlinePayloadStage.mockResolvedValue({
      ...STAGE,
      payload: { ...STAGE.payload, releaseTag: "upstream-26.602.40724" },
    });
    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeEnabled());
    await userEvent.setup().click(screen.getByRole("button", { name: "一键安装" }));

    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack.mock.calls[0]?.[0]).toMatchObject({
      persistConnections: true,
      enableMediaGeneration: true,
      mediaGenerationAgentId: "agent-route-42",
      seedanceModel: "seedance-2.0",
    });
  });

  it("keeps a warning when CCSwitch receives a key but does not confirm activation", async () => {
    const user = userEvent.setup();
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({ status: "managed", installed: USB_PORTABLE_CODEX });
    api.installAndLaunchCcswitch.mockResolvedValueOnce({
      available: true,
      installed: true,
      launched: true,
      sourcePath: "F:\\ccswitch\\CodexSwitch.exe",
      installedPath: "F:\\ccswitch\\CodexSwitch.exe",
      copiedFiles: 0,
      accountInjected: true,
      accountInjectionStatus: "injected",
      message: "launched without activation acknowledgement",
    });
    renderView();
    api.offlinePayloadStage.mockResolvedValue({
      ...STAGE,
      payload: { ...STAGE.payload, releaseTag: "upstream-26.602.40724" },
    });
    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "一键安装" }));

    await waitFor(() => expect(api.installAndLaunchCcswitch).toHaveBeenCalledTimes(1));
    expect(await screen.findByText(/404token 自动配置状态无法确认/)).toBeInTheDocument();
    expect(localStorage.getItem("codex-usb-ccswitch-injection-issue-v1")).toBe("unconfirmed");
  });

  it("does not expose the old model diagnostics in Advanced", async () => {
    const user = userEvent.setup();
    api.winStatus.mockResolvedValue({ status: "managed", installed: USB_PORTABLE_CODEX });
    renderView();
    await screen.findByRole("button", { name: "启动 Codex" });
    await user.click(screen.getByText("高级功能"));
    expect(screen.queryByRole("button", { name: "只检查模型目录（不生成）" })).not.toBeInTheDocument();
    expect(screen.queryByTestId("gateway-model-probe")).not.toBeInTheDocument();
  });
  it("does not render an unplanned account password field in the USB home", async () => {
    renderView();
    await screen.findByText("26.602.40724");
    expect(screen.queryByPlaceholderText("输入密码")).not.toBeInTheDocument();
  });

  it("redeems a code from the compact 404token account card", async () => {
    const user = userEvent.setup();
    api.accountStatus.mockResolvedValue({
      bound: true,
      accountName: "USB-0001",
      loggedIn: true,
      balance: 100,
      usage: null,
      message: "账户信息已刷新",
    });
    api.accountRedeem.mockResolvedValue({
      bound: true,
      accountName: "USB-0001",
      loggedIn: true,
      balance: 200,
      usage: null,
      message: "充值成功,余额已更新",
    });

    renderView();
    const input = await screen.findByPlaceholderText("输入充值码");
    await waitFor(() => expect(input).toBeEnabled());
    await user.type(input, "RECHARGE-ABCD1234");
    await user.click(screen.getByRole("button", { name: "一键充值" }));

    expect(api.accountRedeem).toHaveBeenCalledWith("RECHARGE-ABCD1234");
    expect(await screen.findByText("充值成功,余额已更新")).toBeInTheDocument();
  });

  it("places the first-login password field before the recharge code", async () => {
    api.accountStatus.mockResolvedValue({
      bound: true,
      accountName: "USB-0001",
      loggedIn: false,
      balance: null,
      usage: null,
      message: "已绑定,请输入密码登录",
    });

    renderView();
    const password = await screen.findByPlaceholderText("输入 404token 密码");
    const code = await screen.findByPlaceholderText("输入充值码");
    expect(password.compareDocumentPosition(code) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByRole("button", { name: "登录账户" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "一键充值" })).toBeDisabled();
  });

  it("stops before the USB install when the pre-install Guard recheck fails", async () => {
    const user = userEvent.setup();
    api.usbGuardStatus
      .mockResolvedValueOnce(USB_GUARD)
      .mockRejectedValueOnce(new Error("status unavailable"));

    renderView();
    await user.click(await screen.findByRole("button", { name: "一键安装" }));

    await waitFor(() => expect(api.winInstallHistoricalRelease).not.toHaveBeenCalled());
    expect(api.installCapabilityPack).not.toHaveBeenCalled();
  });

  it("rechecks USB authorization before staging and reports a readiness issue", async () => {
    const user = userEvent.setup();
    // First call: initial shell; second call: post-probe refresh; third call:
    // the action-boundary preflight intentionally fails.
    api.usbGuardStatus.mockReset();
    api.usbGuardStatus
      .mockResolvedValueOnce(USB_GUARD)
      .mockResolvedValueOnce(USB_GUARD)
      .mockRejectedValueOnce(new Error("status unavailable"))
      .mockResolvedValue(USB_GUARD);

    renderView();
    await user.click(await screen.findByRole("button", { name: "一键安装" }));

    await waitFor(() =>
      expect(screen.getByText("U 盘授权尚未就绪，请保持 U 盘连接并稍后重试。")).toBeInTheDocument(),
    );
    expect(api.offlinePayloadStage).not.toHaveBeenCalled();
    expect(api.winInstallHistoricalRelease).not.toHaveBeenCalled();
  });

  it("reports a Codex configuration conflict separately from core capability readiness", async () => {
    const user = userEvent.setup();
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({
        status: "managed",
        installed: USB_PORTABLE_CODEX,
      });
    api.installCapabilityPack.mockResolvedValueOnce({
      available: true,
      installed: true,
      success: false,
      coreReady: true,
      overallStatus: "partial",
      sourcePath: "F:\\content\\codex-starter-kit",
      packVersion: "1.4.0",
      skills: 55,
      agents: 7,
      runtimeReady: true,
      runtimeStatus: "ready",
      configStatus: "conflict",
      diagnosticsPath: null,
      feishuEnabled: false,
      enabledApiServices: [],
      mediaRuntimeStatus: "ready",
      mediaProtocolStatus: "ready",
      mediaGatewayCatalogStatus: "verified",
      mediaGatewayModelCount: 3,
      mediaGatewayCatalogVerifiedAt: "2026-10-13T00:00:00Z",
      mediaGenerationCapability: "available",
      codexRestartRequired: false,
      codexRunning: false,
      message: "Codex 配置冲突",
    });

    renderView();
    await user.click(await screen.findByRole("button", { name: "一键安装" }));

    expect(
      await screen.findByText(
        /核心能力已就绪（55 个 Skills、7 个 Agents），但 Codex 配置存在冲突\/需确认。/,
      ),
    ).toBeInTheDocument();
  });

  it("reports runtime repair when runtime and configuration both need attention", async () => {
    const user = userEvent.setup();
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({
        status: "managed",
        installed: USB_PORTABLE_CODEX,
      });
    api.installCapabilityPack.mockResolvedValueOnce({
      available: true,
      installed: true,
      success: false,
      coreReady: true,
      overallStatus: "partial",
      sourcePath: "F:\\content\\codex-starter-kit",
      packVersion: "1.4.0",
      skills: 55,
      agents: 7,
      runtimeReady: false,
      runtimeStatus: "failed",
      configStatus: "conflict",
      diagnosticsPath: null,
      feishuEnabled: false,
      enabledApiServices: [],
      mediaRuntimeStatus: "failed",
      mediaProtocolStatus: "unknown",
      mediaGatewayCatalogStatus: "verified",
      mediaGatewayModelCount: 3,
      mediaGatewayCatalogVerifiedAt: "2026-10-13T00:00:00Z",
      mediaGenerationCapability: "unavailable",
      codexRestartRequired: false,
      codexRunning: false,
      message: "运行时和 Codex 配置均需处理",
    });

    renderView();
    await user.click(await screen.findByRole("button", { name: "一键安装" }));

    expect(await screen.findByText(/部分运行时待修复/)).toBeInTheDocument();
    expect(screen.queryByText(/Codex 配置存在冲突\/需确认/)).not.toBeInTheDocument();
  });

  it("keeps a quiet entry for capability repair and an independent CCSwitch retry", async () => {
    const user = userEvent.setup();
    renderView();
    const entry = await screen.findByRole("button", { name: "组件修复" });
    await user.click(entry);
    expect(await screen.findByRole("heading", { name: "组件修复" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "单独重试 CCSwitch" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "安装 / 修复能力包" })).toBeInTheDocument();
  });

  it("shows the structured CCSwitch command error instead of [object Object]", async () => {
    const user = userEvent.setup();
    api.installAndLaunchCcswitch.mockRejectedValueOnce({
      code: "permission",
      message: "USB 看门狗：未检测到有效 U 盘租约",
    });
    renderView();
    await user.click(await screen.findByRole("button", { name: "组件修复" }));
    await user.click(screen.getByRole("button", { name: "单独重试 CCSwitch" }));

    expect(await screen.findByText("USB 看门狗：未检测到有效 U 盘租约")).toBeInTheDocument();
    expect(screen.queryByText("[object Object]")).not.toBeInTheDocument();
  });

  it("offers a manual CCSwitch retry after Codex is already installed", async () => {
    const user = userEvent.setup();
    api.winStatus.mockResolvedValue({
      status: "managed",
      installed: USB_PORTABLE_CODEX,
    });
    renderView();
    await waitFor(() => expect(screen.getByRole("button", { name: "打开 Codex Switch" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "打开 Codex Switch" }));
    await waitFor(() => expect(api.installAndLaunchCcswitch).toHaveBeenCalledTimes(1));
  });

  it("continues pending recovery instead of claiming an already registered package is ready", async () => {
    const user = userEvent.setup();
    const current = {
      status: "managed" as const,
      recoveryPending: true,
      installed: USB_PORTABLE_CODEX,
    };
    api.winStatus.mockResolvedValueOnce(current).mockResolvedValueOnce(current)
      .mockResolvedValue({ ...current, recoveryPending: false });
    renderView();
    const resume = await screen.findByRole("button", { name: "继续完成安装" });
    expect(screen.queryByRole("button", { name: "启动 Codex" })).not.toBeInTheDocument();
    expect(screen.queryByText("此版本已经安装")).not.toBeInTheDocument();
    await user.click(resume);
    await waitFor(() => expect(api.installAndLaunchCcswitch).toHaveBeenCalledTimes(1));
    expect(api.winInstallHistoricalRelease).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole("button", { name: "启动 Codex" })).toBeEnabled();
  });

  it("retains staging and skips configuration if recovery is still pending after deployment", async () => {
    const user = userEvent.setup();
    api.winStatus.mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({
        status: "managed", recoveryPending: true,
        installed: USB_PORTABLE_CODEX,
      });
    renderView();
    await user.click(await screen.findByRole("button", { name: "一键安装" }));
    await waitFor(() => expect(screen.getByText(/上次安装或数据恢复尚未完成/)).toBeInTheDocument());
    expect(api.offlinePayloadCleanup).not.toHaveBeenCalled();
    expect(api.installAndLaunchCcswitch).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "启动 Codex" })).not.toBeInTheDocument();
  });

  it("blocks a managed Codex outside the current USB bundle", async () => {
    api.winStatus.mockResolvedValue({
      status: "managed",
      installed: {
        path: "C:\\Program Files\\WindowsApps\\OpenAI.Codex",
        version: PAYLOAD.codexDesktopVersion,
        arch: "x64",
        source: "msix",
        packageFamilyName: "OpenAI.Codex_2p2nqsd0c76g0",
        packageVolume: "C:\\",
      },
    });
    renderView();
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(/Codex 已安装在其他位置/));
    expect(screen.queryByRole("button", { name: "一键安装" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "启动 Codex" })).not.toBeInTheDocument();
  });

  it("can repair the same USB version and keeps recovery evidence after CCSwitch starts", async () => {
    const user = userEvent.setup();
    const current = {
      status: "managed" as const,
      installed: USB_PORTABLE_CODEX,
    };
    api.winStatus.mockResolvedValue(current);
    api.winInstallHistoricalRelease.mockResolvedValue({
      success: true,
      message: "原数据已恢复；新注册数据备份保留在 C:\\Recovery\\new-state",
    } as never);
    renderView();
    const repair = await screen.findByRole("button", { name: "重新校验并修复程序" });
    await user.click(repair);
    await waitFor(() => expect(api.winInstallHistoricalRelease).toHaveBeenCalledTimes(1));
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[2]).toMatchObject({
      currentPath: current.installed.path,
      currentVersion: PAYLOAD.codexDesktopVersion,
      currentSource: "portable",
    });
    await waitFor(() => expect(api.installAndLaunchCcswitch).toHaveBeenCalledTimes(1));
    await user.click(screen.getByText("高级功能"));
    await user.click(screen.getByText("安装／修复详情与备份位置"));
    expect(screen.getByText(/新注册数据备份保留在 C:/)).toBeVisible();
  });

  it("surfaces a missing or invalid bundle without offering install", async () => {
    api.offlinePayloadProbe.mockRejectedValue(new Error("manifest missing"));
    renderView();
    await waitFor(() => expect(screen.getByText(/安装前检查未通过/)).toBeInTheDocument());
    expect(screen.queryByRole("button", { name: "一键安装" })).not.toBeInTheDocument();
  });

  it("keeps cleanup accessible after a status probe fails without claiming the payload is missing", async () => {
    const user = userEvent.setup();
    const uninstall = vi.fn();
    api.winStatus.mockRejectedValueOnce(new Error("AppX query access denied"));
    renderView(uninstall);
    await screen.findByText(/安装前检查未通过/);
    expect(screen.queryByText(/未找到离线载荷/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /卸载 Codex 与 CCSwitch/ }));
    expect(uninstall).toHaveBeenCalledTimes(1);
  });

  it("retains staged evidence if successful deployment returns a non-bundle path", async () => {
    const user = userEvent.setup();
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({ status: "managed", installed: {
        path: "C:\\Program Files\\WindowsApps\\OpenAI.Codex",
        version: PAYLOAD.codexDesktopVersion, source: "msix", arch: "x64",
        packageFamilyName: "OpenAI.Codex_2p2nqsd0c76g0", packageVolume: "C:\\",
      } });
    renderView();
    await user.click(await screen.findByRole("button", { name: "一键安装" }));
    await screen.findByText(/暂存文件已保留/);
    expect(api.offlinePayloadCleanup).not.toHaveBeenCalled();
    expect(api.installAndLaunchCcswitch).not.toHaveBeenCalled();
  });

  it("does not downgrade a newer managed installation", async () => {
    api.winStatus.mockResolvedValue({
      status: "managed",
      installed: {
        path: USB_PORTABLE_CODEX.path,
        version: "26.700.1",
        arch: "x64",
        source: "portable",
        packageFamilyName: "OpenAI.Codex_2p2nqsd0c76g0",
      },
    });
    renderView();
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(/更新版本/));
    expect(screen.getByRole("button", { name: "一键安装" })).toBeDisabled();
  });

  it("blocks an external MSIX install without mutating it", async () => {
    const external = {
      status: "external" as const,
      installed: {
        path: "C:\\Program Files\\WindowsApps\\OpenAI.Codex",
        version: PAYLOAD.codexDesktopVersion,
        arch: "x64" as const,
        source: "msix" as const,
        packageFamilyName: "OpenAI.Codex_2p2nqsd0c76g0",
        packageVolume: "C:\\",
      },
    };
    api.winStatus
      .mockResolvedValue(external);
    renderView();
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(/其他来源已安装的 Codex/));
    expect(screen.queryByRole("button", { name: "一键安装" })).not.toBeInTheDocument();
    expect(api.winInstallHistoricalRelease).not.toHaveBeenCalled();
  });

  it("blocks an external portable Codex install without an adoption action", async () => {
    api.winStatus.mockResolvedValue({
        status: "external",
        installed: {
          path: "C:\\Users\\test\\AppData\\Local\\Programs\\Codex",
          version: "26.500.1",
          arch: "x64",
          source: "portable",
          packageFamilyName: "OpenAI.Codex_2p2nqsd0c76g0",
        },
      });
    renderView();
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(/其他来源已安装的 Codex/));
    expect(screen.queryByRole("button", { name: "开始管理" })).not.toBeInTheDocument();
    expect(api.winAdopt).not.toHaveBeenCalled();
  });

});
