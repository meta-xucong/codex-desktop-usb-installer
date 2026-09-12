import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { managerApi } from "../../services/managerApi";
import type { CapabilityPackReport, CcswitchLaunchReport } from "../../shared/types";
import { I18nProvider } from "../i18n";
import { ThemeProvider } from "../theme";
import { OfflineComponentsSheet } from "./OfflineComponentsSheet";

vi.mock("../../services/managerApi", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../services/managerApi")>();
  return {
    ...actual,
    managerApi: {
      capabilityPackStatus: vi.fn(),
      installCapabilityPack: vi.fn(),
      installAndLaunchCcswitch: vi.fn(),
    },
  };
});

const api = vi.mocked(managerApi);

const PACK_REPORT: CapabilityPackReport = {
  available: true,
  installed: false,
  success: false,
  coreReady: false,
  overallStatus: "failed",
  sourcePath: "F:\\content\\codex-starter-kit",
  packVersion: "1.2.1",
  skills: 50,
  agents: 7,
  runtimeReady: false,
  runtimeStatus: "unknown",
  configStatus: "unknown",
  diagnosticsPath: null,
  feishuEnabled: false,
  enabledApiServices: [],
  message: "能力包可用",
};

const CCSWITCH_REPORT: CcswitchLaunchReport = {
  available: true,
  installed: true,
  launched: true,
  sourcePath: "F:\\ccswitch\\CodexSwitch.exe",
  installedPath: "F:\\ccswitch\\CodexSwitch.exe",
  copiedFiles: 0,
  launchMode: "usb",
  dataScope: "user-profile",
  usableWindowConfirmed: true,
  graphicsMode: "software-rendering",
  message: "Codex Switch 已从 U 盘启动",
};

function SheetView({ open = true }: { open?: boolean }) {
  return (
    <ThemeProvider>
      <I18nProvider>
        <OfflineComponentsSheet open={open} onDismiss={vi.fn()} />
      </I18nProvider>
    </ThemeProvider>
  );
}

function renderSheet(open = true) {
  return render(
    <SheetView open={open} />,
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("OfflineComponentsSheet", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    localStorage.setItem("cam.lang", "zh-CN");
    api.capabilityPackStatus.mockResolvedValue(PACK_REPORT);
    api.installCapabilityPack.mockResolvedValue({
      ...PACK_REPORT,
      installed: true,
      success: true,
      coreReady: true,
      overallStatus: "ready",
      runtimeReady: true,
      runtimeStatus: "ready",
      configStatus: "merged",
      diagnosticsPath: null,
      feishuEnabled: true,
      message: "能力包安装完成",
    });
    api.installAndLaunchCcswitch.mockResolvedValue(CCSWITCH_REPORT);
  });

  it("loads status and submits the selected Feishu configuration", async () => {
    const user = userEvent.setup();
    renderSheet();

    await waitFor(() => expect(api.capabilityPackStatus).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("checkbox", { name: "飞书 / Lark MCP" }));
    await user.type(screen.getByLabelText("飞书 App ID"), "cli_test");
    await user.type(screen.getByLabelText("飞书 App Secret"), "secret_test");
    await user.click(screen.getByRole("button", { name: "安装 / 修复能力包" }));

    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack).toHaveBeenCalledWith(
      expect.objectContaining({
        enableFeishu: true,
        persistConnections: true,
        feishuAppId: "cli_test",
        feishuAppSecret: "secret_test",
        enabledApiServices: [],
      }),
    );
    expect(await screen.findByText("能力包就绪")).toBeInTheDocument();
  }, 30_000);

  it("disables installation while status is pending and allows it after a status failure", async () => {
    const user = userEvent.setup();
    const status = deferred<CapabilityPackReport>();
    api.capabilityPackStatus.mockReturnValue(status.promise);
    renderSheet();

    const install = await screen.findByRole("button", { name: "安装 / 修复能力包" });
    await waitFor(() => expect(api.capabilityPackStatus).toHaveBeenCalledTimes(1));
    expect(install).toBeDisabled();

    status.reject(new Error("status unavailable"));
    await waitFor(() => expect(install).toBeEnabled());
    await user.click(install);
    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(install).toBeEnabled());
  }, 30_000);

  it("does not let a stale status query overwrite the install result", async () => {
    const user = userEvent.setup();
    const staleStatus = deferred<CapabilityPackReport>();
    api.capabilityPackStatus
      .mockReturnValueOnce(staleStatus.promise)
      .mockResolvedValueOnce(PACK_REPORT);
    const rendered = renderSheet();

    await waitFor(() => expect(api.capabilityPackStatus).toHaveBeenCalledTimes(1));
    rendered.rerender(<SheetView open={false} />);
    rendered.rerender(<SheetView open />);
    await waitFor(() => expect(api.capabilityPackStatus).toHaveBeenCalledTimes(2));

    const install = await screen.findByRole("button", { name: "安装 / 修复能力包" });
    await waitFor(() => expect(install).toBeEnabled());
    await user.click(install);
    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("能力包就绪")).toBeInTheDocument();

    staleStatus.resolve(PACK_REPORT);
    await waitFor(() => expect(screen.getByText("能力包就绪")).toBeInTheDocument());
    expect(screen.queryByText("能力包未就绪")).not.toBeInTheDocument();
  }, 30_000);

  it("explains that a config conflict does not mean core capabilities are unavailable", async () => {
    api.capabilityPackStatus.mockResolvedValueOnce({
      ...PACK_REPORT,
      installed: true,
      coreReady: true,
      overallStatus: "partial",
      runtimeReady: true,
      runtimeStatus: "ready",
      configStatus: "conflict",
      message: "Codex 配置冲突",
    });
    renderSheet();

    expect(await screen.findByText("核心能力已就绪，Codex 配置存在冲突/需确认")).toBeInTheDocument();
    expect(screen.queryByText("核心能力已就绪，运行时待修复")).not.toBeInTheDocument();
  }, 30_000);

  it("prioritizes runtime repair when runtime and config are both incomplete", async () => {
    api.capabilityPackStatus.mockResolvedValueOnce({
      ...PACK_REPORT,
      installed: true,
      coreReady: true,
      overallStatus: "partial",
      runtimeReady: false,
      runtimeStatus: "failed",
      configStatus: "conflict",
      message: "运行时和 Codex 配置均需处理",
    });
    renderSheet();

    expect(await screen.findByText("核心能力已就绪，运行时待修复")).toBeInTheDocument();
    expect(screen.queryByText("核心能力已就绪，Codex 配置存在冲突/需确认")).not.toBeInTheDocument();
  }, 30_000);

  it("installs core capabilities without credentials when every optional service is unchecked", async () => {
    const user = userEvent.setup();
    renderSheet();

    await user.click(screen.getByRole("button", { name: "安装 / 修复能力包" }));

    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack).toHaveBeenCalledWith(
      expect.objectContaining({
        enableFeishu: false,
        persistConnections: true,
        enabledApiServices: [],
        feishuAppId: "",
        feishuAppSecret: "",
        dashscopeApiKey: "",
        image2ApiKey: "",
        seedanceApiKey: "",
      }),
    );
    expect(screen.queryByText("DashScope 的 Key、Base URL 和模型必须填写完整。")).not.toBeInTheDocument();
  }, 30_000);

  it("rejects an enabled service with missing required fields before IPC", async () => {
    const user = userEvent.setup();
    renderSheet();

    await user.click(screen.getByRole("checkbox", { name: "DashScope 联网搜索" }));
    await user.click(screen.getByRole("button", { name: "安装 / 修复能力包" }));

    expect(await screen.findByText("DashScope 的 Key、Base URL 和模型必须填写完整。")).toBeInTheDocument();
    expect(api.installCapabilityPack).not.toHaveBeenCalled();
  }, 30_000);

  it("retries CCSwitch independently and renders its success report", async () => {
    const user = userEvent.setup();
    renderSheet();

    await user.click(screen.getByRole("button", { name: "单独重试 CCSwitch" }));
    await waitFor(() => expect(api.installAndLaunchCcswitch).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("CCSwitch 已启动")).toBeInTheDocument();
    expect(screen.getByText("Codex Switch 已从 U 盘启动")).toBeInTheDocument();
  }, 30_000);
});
