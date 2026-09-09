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
  sourcePath: "F:\\content\\codex-starter-kit",
  packVersion: "1.2.1",
  skills: 50,
  agents: 7,
  runtimeReady: false,
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

function renderSheet() {
  return render(
    <ThemeProvider>
      <I18nProvider>
        <OfflineComponentsSheet open onDismiss={vi.fn()} />
      </I18nProvider>
    </ThemeProvider>,
  );
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
      runtimeReady: true,
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
