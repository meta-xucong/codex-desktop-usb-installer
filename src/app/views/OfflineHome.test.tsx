import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { managerApi } from "../../services/managerApi";
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
      offlinePayloadStage: vi.fn(),
      offlinePayloadCleanup: vi.fn(),
      winStatus: vi.fn(),
      winAdopt: vi.fn(),
      winInstallHistoricalRelease: vi.fn(),
      winLaunch: vi.fn(),
      installAndLaunchCcswitch: vi.fn(),
      usbGuardStatus: vi.fn(),
      openLogsDir: vi.fn(),
    },
  };
});

const api = vi.mocked(managerApi);

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
  signed: false,
  lastCheckedAtMs: Date.now(),
  missingSinceMs: null,
  countdownSeconds: null,
  managedProcesses: [],
  message: "U 盘已验证（开发许可证未签名），看门狗运行中",
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
    localStorage.setItem("cam.lang", "zh-CN");
    api.offlinePayloadProbe.mockResolvedValue(PAYLOAD);
    api.offlinePayloadStage.mockResolvedValue(STAGE);
    api.offlinePayloadCleanup.mockResolvedValue();
    api.winStatus.mockResolvedValue({ installed: null, status: "none" });
    api.winAdopt.mockResolvedValue({ installed: null, status: "managed" });
    api.winInstallHistoricalRelease.mockResolvedValue({
      success: true,
      message: "installed",
    } as never);
    api.winLaunch.mockResolvedValue();
    api.installAndLaunchCcswitch.mockResolvedValue({
      available: true,
      installed: true,
      launched: true,
      sourcePath: "F:\\ccswitch\\CodexSwitch.exe",
      installedPath: "C:\\Users\\test\\AppData\\Local\\Programs\\CodexSwitch\\CodexSwitch.exe",
      copiedFiles: 5,
      message: "launched",
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

  it("stages the payload and calls the guarded USB portable install", async () => {
    const user = userEvent.setup();
    api.winStatus
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValueOnce({ installed: null, status: "none" })
      .mockResolvedValue({
        status: "managed",
        installed: USB_PORTABLE_CODEX,
      });
    renderView();
    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "一键安装" }));
    await waitFor(() => expect(api.winInstallHistoricalRelease).toHaveBeenCalled());
    expect(api.offlinePayloadStage).toHaveBeenCalledTimes(1);
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[0]).toMatchObject({
      localPath: STAGE.stagedPath,
      version: PAYLOAD.codexDesktopVersion,
      format: "msix",
    });
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[1]).toBe(true);
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[5]).toBe(true);
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[6]).toBe(true);
    expect(api.offlinePayloadCleanup).toHaveBeenCalledWith(STAGE.stagedPath);
    expect(api.installAndLaunchCcswitch).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByText(/Codex Switch 已从 U 盘启动/)).toBeInTheDocument());
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

  it("does not treat a managed C-drive package as installed in USB mode", async () => {
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
    await waitFor(() => expect(screen.getByRole("button", { name: "一键安装" })).toBeEnabled());
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
        path: "C:\\Program Files\\WindowsApps\\OpenAI.Codex",
        version: "26.700.1",
        arch: "x64",
        source: "msix",
        packageFamilyName: "OpenAI.Codex_2p2nqsd0c76g0",
      },
    });
    renderView();
    await waitFor(() => expect(screen.getByText(/更新版本/)).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "一键安装" })).toBeDisabled();
  });

  it("migrates an external MSIX install to the USB program tree", async () => {
    const user = userEvent.setup();
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
      .mockResolvedValueOnce(external)
      .mockResolvedValueOnce(external)
      .mockResolvedValue({
        status: "managed",
        installed: USB_PORTABLE_CODEX,
      });
    renderView();
    expect(await screen.findByRole("button", { name: "一键安装" })).toBeEnabled();
    expect(screen.getByText(/建立当前 U 盘程序树/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "一键安装" }));
    await waitFor(() => expect(api.winInstallHistoricalRelease).toHaveBeenCalledTimes(1));
    expect(api.winInstallHistoricalRelease.mock.calls[0]?.[2]).toMatchObject({
      currentPath: external.installed.path,
      currentVersion: external.installed.version,
      currentSource: "msix",
    });
  });

  it("offers an explicit adoption step for an external portable Codex install", async () => {
    const user = userEvent.setup();
    api.winStatus
      .mockResolvedValueOnce({
        status: "external",
        installed: {
          path: "C:\\Users\\test\\AppData\\Local\\Programs\\Codex",
          version: "26.500.1",
          arch: "x64",
          source: "portable",
          packageFamilyName: "OpenAI.Codex_2p2nqsd0c76g0",
        },
      })
      .mockResolvedValue({ installed: null, status: "managed" });
    renderView();
    await waitFor(() => expect(screen.getByRole("button", { name: "开始管理" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "开始管理" }));
    await waitFor(() => expect(api.winAdopt).toHaveBeenCalledTimes(1));
  });
});
