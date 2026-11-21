import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { managerApi } from "../../services/managerApi";
import type { CapabilityPackReport, CcswitchLaunchReport } from "../../shared/types";
import { I18nProvider } from "../i18n";
import { ThemeProvider } from "../theme";
import { OfflineComponentsSheet, type OfflineComponentsView } from "./OfflineComponentsSheet";

vi.mock("../../services/managerApi", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../services/managerApi")>();
  return {
    ...actual,
    managerApi: {
      capabilityPackStatus: vi.fn(),
      installCapabilityPack: vi.fn(),
      installAndLaunchCcswitch: vi.fn(),
      mediaModelCatalog: vi.fn(),
      mediaModelSettings: vi.fn(),
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
  packVersion: "1.4.0",
  skills: 55,
  agents: 7,
  runtimeReady: false,
  runtimeStatus: "unknown",
  configStatus: "unknown",
  diagnosticsPath: null,
  feishuEnabled: false,
  enabledApiServices: [],
  mediaGatewayModelCount: 0,
  mediaGatewayCatalogVerifiedAt: null,
  mediaRuntimeStatus: "not-checked",
  mediaProtocolStatus: "not-checked",
  mediaGatewayCatalogStatus: "not-checked",
  mediaGenerationCapability: "not-tested",
  codexRestartRequired: false,
  codexRunning: false,
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

function SheetView({
  open = true,
  view = "connections",
}: {
  open?: boolean;
  view?: OfflineComponentsView;
}) {
  return (
    <ThemeProvider>
      <I18nProvider>
        <OfflineComponentsSheet open={open} onDismiss={vi.fn()} view={view} />
      </I18nProvider>
    </ThemeProvider>
  );
}

function renderSheet(open = true, view: OfflineComponentsView = "connections") {
  return render(
    <SheetView open={open} view={view} />,
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
      feishuConfigured: true,
      feishuRuntimeStatus: "passed",
      message: "能力包安装完成",
    });
    api.installAndLaunchCcswitch.mockResolvedValue(CCSWITCH_REPORT);
    api.mediaModelCatalog.mockResolvedValue({ models: ["gpt-image-2.5"] });
    api.mediaModelSettings.mockResolvedValue({
      licensedCredentialAvailable: true,
      mediaGenerationAgentId: "",
      image2Model: "gpt-image-2.5",
      grokImageModel: "",
      grokVideoModel: "",
      seedanceModel: "",
      minimaxVideoModel: "",
    });
  });

  it("shows two Feishu inputs and enables the installed local runtime after saving", async () => {
    const user = userEvent.setup();
    api.capabilityPackStatus.mockResolvedValue({ ...PACK_REPORT, feishuRuntimeStatus: "passed" });
    renderSheet();

    await screen.findByText("飞书 MCP 已安装。填写应用凭据即可启用。");
    const save = screen.getByRole("button", { name: "保存飞书连接" });
    expect(save).toBeDisabled();
    expect(screen.queryByRole("button", { name: "保存其他连接" })).not.toBeInTheDocument();
    expect(api.mediaModelCatalog).not.toHaveBeenCalled();
    expect(screen.getByLabelText("飞书 App Secret")).toHaveAttribute("type", "password");
    await user.type(screen.getByLabelText("飞书 App ID"), "cli_test");
    await user.type(screen.getByLabelText("飞书 App Secret"), "secret_test");
    await user.click(save);

    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack).toHaveBeenCalledWith(
      expect.objectContaining({
        enableFeishu: true,
        feishuAction: "apply",
        persistConnections: true,
        feishuAppId: "cli_test",
        feishuAppSecret: "secret_test",
        feishuAuthMode: "tenant",
        enabledApiServices: [],
      }),
    );
    expect(await screen.findByText("飞书 MCP 已配置。请重新打开 Codex，再在新会话中使用。")).toBeInTheDocument();
    expect(screen.getByLabelText("飞书 App Secret")).toHaveValue("");
  }, 30_000);

  it("preserves the existing Feishu connection when saving other integrations", async () => {
    const user = userEvent.setup();
    api.capabilityPackStatus.mockResolvedValue({ ...PACK_REPORT, feishuEnabled: true, feishuConfigured: true, feishuRuntimeStatus: "passed" });
    renderSheet();
    await user.click(screen.getByText("其他连接"));
    await screen.findByText("飞书 MCP 已启用，凭据已保存。");
    const save = screen.getByRole("button", { name: "保存其他连接" });
    await waitFor(() => expect(save).toBeEnabled());
    await user.click(save);
    expect(api.installCapabilityPack).toHaveBeenCalledWith(expect.objectContaining({
      feishuAction: "preserve",
      feishuAppId: "",
      feishuAppSecret: "",
    }));
  });

  it("shows Feishu as enabled with blank credentials and confirms live save", async () => {
    const user = userEvent.setup();
    const pending = { ...PACK_REPORT, feishuEnabled: true, feishuConfigured: false, feishuRuntimeStatus: "passed" };
    api.capabilityPackStatus.mockResolvedValue(pending);
    api.installCapabilityPack.mockResolvedValue({
      ...pending, coreReady: false, feishuConfigured: true, feishuSavedLive: true,
    });
    renderSheet();

    await screen.findByText("飞书 MCP 已启用，待连接。填写 App ID 和 App Secret 后保存。");
    const id = screen.getByLabelText("飞书 App ID");
    const secret = screen.getByLabelText("飞书 App Secret");
    expect(id).toHaveValue("");
    expect(secret).toHaveValue("");
    expect(secret).toHaveAttribute("type", "password");
    const save = screen.getByRole("button", { name: "保存飞书连接" });
    expect(save).toBeDisabled();
    await user.type(id, "cli_fixture");
    await user.type(secret, "fixture-secret");
    await user.click(save);

    expect(await screen.findByText("飞书凭据已保存，可回到当前 Codex 会话使用。")).toBeInTheDocument();
    expect(secret).toHaveValue("");
    expect(screen.queryByText(/请重新打开 Codex/)).not.toBeInTheDocument();
  }, 30_000);

  it("shows a Feishu save failure beside the form and keeps both inputs for retry", async () => {
    const user = userEvent.setup();
    api.installCapabilityPack.mockRejectedValueOnce({
      message: "update engine error: 无法从安装包准备飞书运行时。请查看管理器日志",
    });
    renderSheet();
    const id = screen.getByLabelText("飞书 App ID");
    const secret = screen.getByLabelText("飞书 App Secret");
    await user.type(id, "cli_fixture");
    await user.type(secret, "fixture-secret");
    const save = screen.getByRole("button", { name: "保存飞书连接" });
    await user.click(save);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("无法从安装包准备飞书运行时。请查看管理器日志");
    expect(alert).not.toHaveTextContent("update engine error");
    expect(save.closest("form")).toContainElement(alert);
    expect(id).toHaveValue("cli_fixture");
    expect(secret).toHaveValue("fixture-secret");
    expect(api.mediaModelCatalog).not.toHaveBeenCalled();
  });

  it("disables installation while status is pending and allows it after a status failure", async () => {
    const user = userEvent.setup();
    const status = deferred<CapabilityPackReport>();
    api.capabilityPackStatus.mockReturnValue(status.promise);
    renderSheet();
    await user.click(screen.getByText("其他连接"));

    const install = await screen.findByRole("button", { name: "保存其他连接" });
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
    await user.click(screen.getByText("其他连接"));

    const install = await screen.findByRole("button", { name: "保存其他连接" });
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
    renderSheet(true, "maintenance");

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
    renderSheet(true, "maintenance");

    expect(await screen.findByText("核心能力已就绪，运行时待修复")).toBeInTheDocument();
    expect(screen.queryByText("核心能力已就绪，Codex 配置存在冲突/需确认")).not.toBeInTheDocument();
  }, 30_000);

  it("installs core capabilities without optional connections when they are unchecked", async () => {
    const user = userEvent.setup();
    renderSheet();
    await user.click(screen.getByText("其他连接"));

    await user.click(screen.getByRole("checkbox", { name: "图片与视频生成（本地 MCP）" }));

    await user.click(screen.getByRole("button", { name: "保存其他连接" }));

    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack).toHaveBeenCalledWith(
      expect.objectContaining({
        enableFeishu: false,
        persistConnections: true,
        enabledApiServices: [],
        feishuAppId: "",
        feishuAppSecret: "",
        dashscopeApiKey: "",
        enableMediaGeneration: false,
        image2Model: "gpt-image-2.5",
        minimaxVideoModel: "",
      }),
    );
    expect(screen.queryByText("DashScope 的 Key、Base URL 和模型必须填写完整。")).not.toBeInTheDocument();
  }, 30_000);

  it("enables license-backed Image-2 by default without asking for a separate media key", async () => {
    const user = userEvent.setup();
    renderSheet();
    await user.click(screen.getByText("其他连接"));

    expect(screen.getByRole("checkbox", { name: "图片与视频生成（本地 MCP）" })).toBeChecked();
    expect(screen.getByText("MiniMax 视频：暂未开放")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Grok 图片模型 ID"), "grok-imagine-test");
    await user.click(screen.getByRole("button", { name: "保存其他连接" }));

    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    const submitted = api.installCapabilityPack.mock.calls[0][0];
    expect(submitted).toEqual(expect.objectContaining({
      enableMediaGeneration: true,
      image2Model: "gpt-image-2.5",
      minimaxVideoModel: "",
      mediaGatewayCatalogStatus: "verified",
      mediaGatewayModelCount: 1,
      mediaGatewayCatalogVerifiedAt: expect.any(String),
      enabledApiServices: [],
    }));
    expect(submitted).not.toHaveProperty("mediaGenerationApiKey");
    expect(submitted).not.toHaveProperty("mediaGenerationApiBaseUrl");
    expect(submitted).not.toHaveProperty("minimaxApiKey");
    expect(submitted).not.toHaveProperty("minimaxBaseUrl");
  }, 30_000);

  it("does not auto-enable an un-routed Seedance model and retains other discovered routes", async () => {
    const user = userEvent.setup();
    api.mediaModelCatalog.mockResolvedValueOnce({
      models: ["gpt-image-2.5", "seedance-2.0"],
    });
    renderSheet();
    await user.click(screen.getByText("其他连接"));

    await waitFor(() => expect(api.mediaModelCatalog).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("button", { name: "保存其他连接" }));
    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack.mock.calls[0]?.[0]).toMatchObject({
      enableMediaGeneration: true,
      image2Model: "gpt-image-2.5",
      seedanceModel: "",
      mediaGenerationAgentId: "",
    });
  }, 30_000);

  it("restores a configured Seedance model only when its route ID is present", async () => {
    const user = userEvent.setup();
    api.mediaModelCatalog.mockResolvedValueOnce({
      models: ["seedance-2.0"],
      configuredSeedanceModel: "seedance-2.0",
      configuredAgentId: "agent-route-42",
    });
    renderSheet();
    await user.click(screen.getByText("其他连接"));

    await waitFor(() => expect(api.mediaModelCatalog).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("button", { name: "保存其他连接" }));
    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack.mock.calls[0]?.[0]).toMatchObject({
      enableMediaGeneration: true,
      seedanceModel: "seedance-2.0",
      mediaGenerationAgentId: "agent-route-42",
    });
  }, 30_000);

  it("registers media MCP when the upstream has no supported routes", async () => {
    const user = userEvent.setup();
    api.mediaModelCatalog.mockResolvedValueOnce({ models: ["gpt-4o", "claude-opus-4-8"] });
    renderSheet();
    await user.click(screen.getByText("其他连接"));

    await waitFor(() => expect(api.mediaModelCatalog).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("checkbox", { name: "图片与视频生成（本地 MCP）" })).toBeChecked();
    await user.click(screen.getByRole("button", { name: "保存其他连接" }));
    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack.mock.calls[0]?.[0]).toMatchObject({
      enableMediaGeneration: true,
      image2Model: "",
      grokImageModel: "",
      grokVideoModel: "",
      seedanceModel: "",
    });
  }, 30_000);

  it("keeps media MCP registration available when catalog lookup fails", async () => {
    const user = userEvent.setup();
    api.mediaModelCatalog.mockRejectedValueOnce(new Error("offline"));
    renderSheet();
    await user.click(screen.getByText("其他连接"));

    await waitFor(() => expect(api.mediaModelCatalog).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("button", { name: "保存其他连接" }));
    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack.mock.calls[0]?.[0]).toMatchObject({
      enableMediaGeneration: true,
      image2Model: "gpt-image-2.5",
      mediaGatewayCatalogStatus: "failed",
    });
  }, 30_000);

  it("restores saved media routes during maintenance and does not persist empty connections", async () => {
    const user = userEvent.setup();
    api.mediaModelSettings.mockResolvedValueOnce({
      licensedCredentialAvailable: true,
      mediaGenerationAgentId: "agent-route-42",
      image2Model: "gpt-image-2.5",
      grokImageModel: "grok-imagine-image",
      grokVideoModel: "grok-video-2",
      seedanceModel: "seedance-2.0",
      minimaxVideoModel: "",
    });
    renderSheet(true, "maintenance");

    await waitFor(() => expect(api.mediaModelSettings).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("button", { name: "安装 / 修复能力包" }));
    await waitFor(() => expect(api.installCapabilityPack).toHaveBeenCalledTimes(1));
    expect(api.installCapabilityPack.mock.calls[0]?.[0]).toMatchObject({
      persistConnections: false,
      image2Model: "gpt-image-2.5",
      grokImageModel: "grok-imagine-image",
      grokVideoModel: "grok-video-2",
      seedanceModel: "seedance-2.0",
      mediaGenerationAgentId: "agent-route-42",
    });
  }, 30_000);

  it("blocks maintenance if saved media settings cannot be read", async () => {
    api.mediaModelSettings.mockRejectedValueOnce(new Error("local settings unavailable"));
    renderSheet(true, "maintenance");

    expect(await screen.findByRole("alert")).toHaveTextContent("无法读取现有媒体设置");
    expect(screen.getByRole("button", { name: "安装 / 修复能力包" })).toBeDisabled();
  }, 30_000);

  it("rejects an enabled service with missing required fields before IPC", async () => {
    const user = userEvent.setup();
    renderSheet();
    await user.click(screen.getByText("其他连接"));

    await user.click(screen.getByRole("checkbox", { name: "DashScope 联网搜索" }));
    await user.click(screen.getByRole("button", { name: "保存其他连接" }));

    expect(await screen.findByText("DashScope 的 Key、Base URL 和模型必须填写完整。")).toBeInTheDocument();
    expect(api.installCapabilityPack).not.toHaveBeenCalled();
  }, 30_000);

  it("retries CCSwitch independently and renders its success report", async () => {
    const user = userEvent.setup();
    renderSheet(true, "maintenance");

    await user.click(screen.getByRole("button", { name: "单独重试 CCSwitch" }));
    await waitFor(() => expect(api.installAndLaunchCcswitch).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("CCSwitch 已启动")).toBeInTheDocument();
    expect(screen.getByText("Codex Switch 已从 U 盘启动")).toBeInTheDocument();
  }, 30_000);
});
