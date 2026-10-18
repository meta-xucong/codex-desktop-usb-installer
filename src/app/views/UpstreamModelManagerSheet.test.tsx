import { useState } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { managerApi } from "../../services/managerApi";
import type { NativeModelCatalogActionReport } from "../../shared/types";
import { I18nProvider } from "../i18n";
import { ThemeProvider } from "../theme";
import { UpstreamModelManagerSheet } from "./UpstreamModelManagerSheet";

vi.mock("../../services/managerApi", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../services/managerApi")>();
  return {
    ...actual,
    managerApi: {
      ...actual.managerApi,
      gatewayModelProbe: vi.fn(),
      cancelGatewayModelProbe: vi.fn(),
      manageNativeModelCatalog: vi.fn(),
    },
  };
});

const api = vi.mocked(managerApi);
const listing = {
  status: "connected" as const,
  endpoint: "https://404token.xyz/v1",
  credentialFingerprint: "sha256:fixture",
  connectionFingerprint: "binding-fixture",
  checkedAt: 1,
  models: [{ id: "vendor/model-latest", ownedBy: "fixture", selectable: true }],
  results: [],
  generatedRequests: 0,
  errorCode: null,
};

const catalogState = {
  status: "ok", modelIds: [], restartRequired: false, message: "state loaded",
  connectionFingerprint: "binding-fixture", credentialFingerprint: "sha256:fixture",
};

function Harness({ onDismiss }: { onDismiss?: () => void }) {
  const [open, setOpen] = useState(true);
  return (
    <>
      {!open ? <button type="button" onClick={() => setOpen(true)}>重新打开</button> : null}
      <UpstreamModelManagerSheet open={open} onDismiss={() => { setOpen(false); onDismiss?.(); }} />
    </>
  );
}

function renderSheet(onDismiss?: () => void) {
  return render(
    <ThemeProvider>
      <I18nProvider>
        <Harness onDismiss={onDismiss} />
      </I18nProvider>
    </ThemeProvider>,
  );
}

async function runReasoningProbe(user: ReturnType<typeof userEvent.setup>) {
  vi.spyOn(window, "confirm").mockReturnValue(true);
  await user.click(screen.getByRole("button", { name: /自动检测|重新检测/ }));
}

async function confirmAddModel(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "确认添加到 Codex" }));
}

describe("UpstreamModelManagerSheet", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    localStorage.setItem("cam.lang", "zh-CN");
    localStorage.removeItem("codex-manager.upstream-model-probe.v1");
    api.manageNativeModelCatalog.mockResolvedValue(catalogState);
    api.gatewayModelProbe.mockResolvedValue(listing);
    api.cancelGatewayModelProbe.mockResolvedValue(true);
  });

  it("shows clear return and refresh controls, and refreshes the model list on demand", async () => {
    const user = userEvent.setup();
    const onDismiss = vi.fn();
    renderSheet(onDismiss);

    await screen.findByText("vendor/model-latest");
    const refresh = screen.getByRole("button", { name: "刷新模型列表" });
    await waitFor(() => expect(refresh).toBeEnabled());
    await user.click(refresh);
    await waitFor(() => expect(api.gatewayModelProbe).toHaveBeenCalledTimes(2));
    expect(api.gatewayModelProbe.mock.calls[1]?.[0]).toMatchObject({
      selectedModelIds: [], costConsent: false,
    });

    await user.click(screen.getByRole("button", { name: "返回首页" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("shows one model action and automatically checks settings without asking users to choose", async () => {
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderSheet();
    const row = await screen.findByTestId("upstream-model-vendor/model-latest");
    expect(within(row).getByRole("button", { name: "自动检测" })).toBeInTheDocument();
    expect(within(row).getAllByRole("button")).toHaveLength(1);
    expect(within(row).queryByRole("button", { name: "检查设置" })).not.toBeInTheDocument();
    expect(within(row).queryByRole("button", { name: "添加到 Codex" })).not.toBeInTheDocument();
    expect(within(row).queryByRole("button", { name: /测档位|Check efforts/ })).not.toBeInTheDocument();
    expect(within(row).queryByRole("checkbox")).not.toBeInTheDocument();

    await user.click(within(row).getByRole("button", { name: "自动检测" }));
    expect(await within(row).findByText("安装器会自动检测这个模型并筛选可用设置；可能发送多次请求并产生费用。")).toBeInTheDocument();
    expect(within(row).queryByRole("checkbox")).not.toBeInTheDocument();
    expect(within(row).queryByText(/low|medium|high/)).not.toBeInTheDocument();
    expect(within(row).getByRole("button", { name: "重新检测" })).toBeEnabled();
  });

  it("retries a failed first model check with a fresh request and unlocks confirmation after success", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "failed",
        errorCode: "upstream-unavailable",
        reasoningModelId: "vendor/model-latest",
        reasoningResults: [{ effort: "medium", status: "unknown", errorCode: "upstream-unavailable" }],
        checkedAt: Math.floor(Date.now() / 1000),
      })
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-after-retry",
        reasoningModelId: "vendor/model-latest",
        reasoningProofId: "proof-after-retry",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{ modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null }],
      });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    await user.click(within(row).getByRole("button", { name: "自动检测" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("无法连接到上游");
    expect(within(row).getByRole("button", { name: "重新检测" })).toBeEnabled();
    expect(within(row).queryByRole("button", { name: "确认添加到 Codex" })).not.toBeInTheDocument();

    await user.click(within(row).getByRole("button", { name: "重新检测" }));
    expect(await screen.findByRole("button", { name: "确认添加到 Codex" })).toBeEnabled();
    expect(api.gatewayModelProbe).toHaveBeenCalledTimes(3);
    expect(api.gatewayModelProbe.mock.calls[1]?.[0].requestId)
      .not.toBe(api.gatewayModelProbe.mock.calls[2]?.[0].requestId);
  });

  it.each([
    ["upstream-auth", "账号或网关授权未通过"],
    ["upstream-http", "仅凭此诊断码无法确定具体原因"],
    ["upstream-http-400", "HTTP 400"],
    ["upstream-store-unsupported", "不接受 Responses 的 store 参数"],
    ["effort-rejected", "明确不接受所选设置"],
    ["upstream-unavailable", "无法连接到上游"],
    ["probe-timeout", "上游响应超时"],
    ["upstream-response", "上游已响应，但返回格式"],
    ["response-too-large", "超过探测上限"],
  ])("explains model test failure category %s and retains its safe code", async (errorCode, expectedReason) => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "failed",
        errorCode,
        connectivityModelId: "vendor/model-latest",
        results: [{ modelId: "vendor/model-latest", passed: false, generatedRequests: 1,
          checks: [{ name: "text", passed: false }], errorCode }],
      });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await user.click(screen.getByRole("button", { name: "自动检测" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(expectedReason);
    expect(screen.getByRole("alert")).toHaveTextContent(errorCode);
    const cached = JSON.parse(localStorage.getItem("codex-manager.upstream-model-probe.v1") ?? "null");
    expect(cached.failureCodeById["vendor/model-latest"]).toBe(errorCode);
  });

  it("reports local invocation failures without exposing the raw error text", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockRejectedValueOnce(new Error("engine_error: private fixture diagnostic with secret"));

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await user.click(screen.getByRole("button", { name: "自动检测" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("安装器未能完成本地测试流程");
    expect(screen.getByRole("alert")).toHaveTextContent("engine_error");
    expect(screen.getByRole("alert")).not.toHaveTextContent("private fixture");
    expect(screen.getByRole("alert")).not.toHaveTextContent("secret");
  });

  it("preserves bulk HTTP failure codes across reopening and keeps the wording neutral", async () => {
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "failed",
        errorCode: "upstream-http",
        connectivityModelId: "vendor/model-latest",
        results: [{ modelId: "vendor/model-latest", passed: false, generatedRequests: 1,
          checks: [{ name: "text", passed: false }], errorCode: "upstream-http" }],
      });

    const view = renderSheet();
    await screen.findByText("vendor/model-latest");
    await user.click(screen.getByRole("button", { name: "测所有模型可用性" }));
    expect(await screen.findByText("HTTP错误")).toBeInTheDocument();
    expect(screen.getByText("检测完成：0 个可用，1 个未通过。")).toBeInTheDocument();
    const cached = JSON.parse(localStorage.getItem("codex-manager.upstream-model-probe.v1") ?? "null");
    expect(cached.failureCodeById["vendor/model-latest"]).toBe("upstream-http");

    view.unmount();
    renderSheet();
    expect(await screen.findByText("HTTP错误")).toBeInTheDocument();
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    expect(within(row).getByText("HTTP错误")).toHaveAttribute(
      "title",
      expect.stringContaining("仅凭此诊断码无法确定具体原因"),
    );
  });

  it("keeps full-probe results across leaving and reopening, and replaces them only after manual refresh", async () => {
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        results: [{
          modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null,
        }],
      })
      .mockResolvedValueOnce(listing);

    const view = renderSheet();
    await screen.findByText("vendor/model-latest");
    await user.click(screen.getByRole("button", { name: "测所有模型可用性" }));
    expect(await screen.findByText("文字测试可用")).toBeInTheDocument();
    await waitFor(() => expect(api.gatewayModelProbe).toHaveBeenCalledTimes(2));

    const cached = JSON.parse(localStorage.getItem("codex-manager.upstream-model-probe.v1") ?? "null");
    cached.updatedAtMs = Date.now() - 49 * 60 * 60 * 1000;
    localStorage.setItem("codex-manager.upstream-model-probe.v1", JSON.stringify(cached));
    view.unmount();

    renderSheet();
    expect(await screen.findByText("文字测试可用")).toBeInTheDocument();
    expect(screen.getByTestId("upstream-model-cache-notice")).toHaveTextContent("保留 48 小时");
    expect(api.gatewayModelProbe).toHaveBeenCalledTimes(2);

    await user.click(screen.getByRole("button", { name: "刷新模型列表" }));
    await waitFor(() => expect(api.gatewayModelProbe).toHaveBeenCalledTimes(3));
    await waitFor(() => expect(screen.getByText("还没测试")).toBeInTheDocument());
  });

  it("keeps locally configured models missing from the current directory in a collapsed section", async () => {
    const localOnlyModel = "legacy/claude-3-5-haiku";
    api.manageNativeModelCatalog.mockResolvedValue({ ...catalogState, modelIds: [localOnlyModel] });

    renderSheet();
    await screen.findByText("vendor/model-latest");

    const candidates = screen.getByTestId("upstream-model-candidate-list");
    expect(within(candidates).getByText("vendor/model-latest")).toBeInTheDocument();
    expect(within(candidates).queryByText(localOnlyModel)).not.toBeInTheDocument();

    const sectionTitle = screen.getByText("本机已配置，但本次上游目录未返回");
    const section = sectionTitle.closest("details");
    expect(section).not.toHaveAttribute("open");
    expect(within(section!).getByText(localOnlyModel)).toBeInTheDocument();
    expect(within(section!).getByText(/本机记录不代表上游当前支持/)).toBeInTheDocument();
    expect(within(section!).queryByRole("button")).not.toBeInTheDocument();
  });

  it("discovers read-only and only offers Add after the exact model's reasoning efforts pass", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-fixture",
        reasoningModelId: "vendor/model-latest",
        reasoningProofId: "proof-fixture",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{
          modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null,
        }],
      });
    api.manageNativeModelCatalog
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce({ status: "applied_restart_pending", modelIds: ["vendor/model-latest"], restartRequired: true, message: "added" })
      .mockResolvedValueOnce({ ...catalogState, modelIds: ["vendor/model-latest"] });

    renderSheet();
    expect(await screen.findByText("vendor/model-latest")).toBeInTheDocument();
    expect(screen.getByText("还没测试")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "添加到 Codex" })).not.toBeInTheDocument();
    expect(api.gatewayModelProbe).toHaveBeenCalledTimes(1);
    expect(api.gatewayModelProbe.mock.calls[0]?.[0]).toMatchObject({
      selectedModelIds: [], costConsent: false,
    });

    await runReasoningProbe(user);
    await screen.findByRole("button", { name: "确认添加到 Codex" });
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    expect(within(row).queryByRole("button", { name: "添加到 Codex" })).not.toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "确认添加到 Codex" })).toHaveLength(1);
    expect(screen.getByText("检测完成：1 项设置可用，其他已跳过。")).toBeInTheDocument();
    const modelRow = screen.getByTestId("upstream-model-vendor/model-latest");
    await user.click(within(modelRow).getByText("查看检查结果"));
    expect(within(modelRow).getByText("可用")).toBeInTheDocument();
    expect(within(modelRow).getByRole("list", { name: "查看检查结果" })).toBeInTheDocument();
    expect(api.gatewayModelProbe.mock.calls[1]?.[0]).toMatchObject({
      selectedModelIds: [], reasoningModelId: "vendor/model-latest",
      reasoningEfforts: ["none", "minimal", "low", "medium", "high", "xhigh", "max"], reasoningCostConsent: true,
    });

    await confirmAddModel(user);
    await waitFor(() => expect(api.manageNativeModelCatalog).toHaveBeenCalledTimes(4));
    expect(api.manageNativeModelCatalog.mock.calls[2]?.[0]).toMatchObject({
      operation: "add", modelId: "vendor/model-latest", connectivityProofId: "proof-fixture",
    });
    expect(api.manageNativeModelCatalog.mock.calls[2]?.[0].requestId)
      .not.toBe(api.gatewayModelProbe.mock.calls[1]?.[0].requestId);
    expect(await screen.findByText("已添加到 Codex。重启 Codex 后即可使用。")).toBeInTheDocument();
    expect(within(screen.getByTestId("upstream-model-vendor/model-latest"))
      .getByText("已添加到 Codex")).toBeInTheDocument();
    expect(within(screen.getByTestId("upstream-model-vendor/model-latest"))
      .queryByRole("button", { name: "添加到 Codex" })).not.toBeInTheDocument();
  });

  it("automatically checks settings and shows accepted, rejected, and unknown results only on demand", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        reasoningModelId: "vendor/model-latest",
        reasoningProofId: "proof-efforts",
        connectivityProofId: "proof-efforts",
        reasoningResults: [
          { effort: "low", status: "accepted_by_route", errorCode: null },
          { effort: "medium", status: "rejected_by_route", errorCode: "unsupported_effort" },
          { effort: "high", status: "unknown", errorCode: "network_error" },
          { effort: "xhigh", status: "unknown", errorCode: "effort-probe-incomplete" },
        ],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
      });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);
    expect(api.gatewayModelProbe.mock.calls[1]?.[0].reasoningEfforts).toEqual([
      "none", "minimal", "low", "medium", "high", "xhigh", "max",
    ]);
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    expect(within(row).getByText("查看检查结果").closest("details")).not.toHaveAttribute("open");
    await user.click(within(row).getByText("查看检查结果"));
    expect(within(row).getByText("可用")).toBeInTheDocument();
    expect(within(row).getByText("上游不接受")).toBeInTheDocument();
    expect(within(row).getAllByText("未能确认")).toHaveLength(2);
    expect(within(row).getAllByText("未能确认").some((node) =>
      node.closest("span")?.getAttribute("title") === "在输出上限内未完成，不能判断该档位是否受支持。",
    )).toBe(true);
  });

  it("skips a model automatically when none of its settings pass", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "failed",
        errorCode: "no-effort-accepted",
        reasoningModelId: "vendor/model-latest",
        reasoningResults: [{ effort: "medium", status: "rejected_by_route", errorCode: "unsupported_effort" }],
        checkedAt: Math.floor(Date.now() / 1000),
        results: [],
      });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);

    expect(await screen.findByRole("alert")).toHaveTextContent("当前线路没有可添加的设置，这个模型未添加");
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    expect(within(row).queryByRole("button", { name: "添加到 Codex" })).not.toBeInTheDocument();
    expect(within(row).getByText("查看检查结果")).toBeInTheDocument();
    expect(api.manageNativeModelCatalog.mock.calls.some(([request]) => request.operation === "add")).toBe(false);
  });

  it("does not report Add as successful when the post-write catalog reread omits the model", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-reread-mismatch",
        reasoningProofId: "proof-reread-mismatch",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{ modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null }],
      });
    api.manageNativeModelCatalog
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce({
        status: "applied_restart_pending", modelIds: ["vendor/model-latest"],
        restartRequired: true, message: "added",
      })
      .mockResolvedValueOnce(catalogState);

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);
    await confirmAddModel(user);

    expect(await screen.findByRole("alert")).toHaveTextContent("还不能确认这个模型是否添加成功。请刷新列表查看状态。");
    expect(screen.queryByText("已添加到 Codex。重启 Codex 后即可使用。")).not.toBeInTheDocument();
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    expect(within(row).queryByText("已添加到 Codex")).not.toBeInTheDocument();
    expect(within(row).queryByText("文字测试可用")).not.toBeInTheDocument();
    expect(within(row).getByRole("button", { name: "重新检测" })).toBeInTheDocument();
  });

  it("clears the old availability badge when Add outcome is unconfirmed and state reread fails", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-state-read-fails",
        reasoningProofId: "proof-state-read-fails",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{ modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null }],
      });
    let addStarted = false;
    api.manageNativeModelCatalog.mockImplementation(({ operation }) => {
      if (operation === "add") {
        addStarted = true;
        return Promise.resolve({
          status: "outcome_unconfirmed", modelIds: [], restartRequired: true,
          message: "unconfirmed",
        });
      }
      if (addStarted) return Promise.reject(new Error("state read failed"));
      return Promise.resolve(catalogState);
    });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);
    await confirmAddModel(user);

    expect(await screen.findByText("模型设置暂不可用。请刷新列表，或恢复 Codex 默认模型后再试。"))
      .toBeInTheDocument();
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    expect(within(row).getByText("还没测试")).toBeInTheDocument();
    expect(within(row).queryByText("文字测试可用")).not.toBeInTheDocument();
    expect(within(row).queryByText("已添加到 Codex")).not.toBeInTheDocument();
  });

  it("never offers Add after a failed quick test and revokes a passed ticket on close", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "failed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        results: [{
          modelId: "vendor/model-latest", passed: false, generatedRequests: 1,
          checks: [{ name: "text", passed: false }], errorCode: "upstream-http",
        }],
        errorCode: "upstream-http",
      })
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-close",
        reasoningProofId: "proof-close",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{
          modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null,
        }],
      });
    renderSheet();
    await screen.findByText("vendor/model-latest");
    await user.click(screen.getByRole("button", { name: "自动检测" }));
    await screen.findByRole("alert");
    expect(screen.queryByRole("button", { name: "添加到 Codex" })).not.toBeInTheDocument();

    await runReasoningProbe(user);
    await screen.findByRole("button", { name: "确认添加到 Codex" });
    const successfulRequestId = api.gatewayModelProbe.mock.calls[2]?.[0].requestId;
    await user.click(screen.getByRole("button", { name: "关闭" }));
    await waitFor(() => expect(api.cancelGatewayModelProbe).toHaveBeenCalledWith(successfulRequestId));
  });

  it("cancels outstanding tickets before resetting to the native catalog", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-reset",
        reasoningProofId: "proof-reset",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{
          modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null,
        }],
      });
    api.manageNativeModelCatalog
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce({ status: "applied_restart_pending", modelIds: [], restartRequired: true, message: "native restored" })
      .mockResolvedValueOnce(catalogState);

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);
    await screen.findByRole("button", { name: "确认添加到 Codex" });
    const testedRequestId = api.gatewayModelProbe.mock.calls[1]?.[0].requestId;
    await user.click(screen.getByRole("button", { name: "恢复 Codex 默认模型" }));

    await waitFor(() => expect(api.manageNativeModelCatalog).toHaveBeenCalledTimes(4));
    expect(api.cancelGatewayModelProbe).toHaveBeenCalledWith(testedRequestId);
    expect(api.manageNativeModelCatalog.mock.calls[2]?.[0]).toEqual({ operation: "reset" });
    expect(await screen.findByText("已恢复 Codex 默认模型。重启 Codex 后生效。")).toBeInTheDocument();
  });

  it("keeps Refresh and Restore disabled while Add is pending", async () => {
    const user = userEvent.setup();
    let resolveAdd!: (result: NativeModelCatalogActionReport) => void;
    const pendingAdd = new Promise<NativeModelCatalogActionReport>((resolve) => {
      resolveAdd = resolve;
    });
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-pending-add",
        reasoningProofId: "proof-pending-add",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{ modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null }],
      });
    api.manageNativeModelCatalog.mockImplementation(({ operation }) => (
      operation === "add" ? pendingAdd : Promise.resolve(catalogState)
    ));

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);
    await screen.findByRole("button", { name: "确认添加到 Codex" });

    const reset = screen.getByRole("button", { name: "恢复 Codex 默认模型" });
    expect(reset).toBeEnabled();
    await confirmAddModel(user);
    await waitFor(() => expect(api.manageNativeModelCatalog).toHaveBeenCalledWith(
      expect.objectContaining({ operation: "add", modelId: "vendor/model-latest" }),
    ));

    const refresh = screen.getByRole("button", { name: "刷新模型列表" });
    expect(refresh).toBeDisabled();
    expect(reset).toBeDisabled();
    await user.click(refresh);
    await user.click(reset);
    expect(api.gatewayModelProbe).toHaveBeenCalledTimes(2);
    expect(api.manageNativeModelCatalog.mock.calls.filter(([request]) => request.operation === "reset")).toHaveLength(0);
    expect(api.cancelGatewayModelProbe).not.toHaveBeenCalled();
    expect(screen.getAllByText("正在添加…")).toHaveLength(2);

    resolveAdd({
      status: "applied_restart_pending",
      modelIds: ["vendor/model-latest"],
      restartRequired: true,
      message: "added",
    });
    api.manageNativeModelCatalog.mockResolvedValueOnce({
      ...catalogState,
      modelIds: ["vendor/model-latest"],
      restartRequired: true,
    });
    expect(await screen.findByText("已添加到 Codex。重启 Codex 后即可使用。")).toBeInTheDocument();
  });

  it("keeps a pending Add alive across close and reopen, then refreshes its result", async () => {
    const user = userEvent.setup();
    let resolveAdd!: (result: NativeModelCatalogActionReport) => void;
    let currentCatalogState: NativeModelCatalogActionReport = catalogState;
    const pendingAdd = new Promise<NativeModelCatalogActionReport>((resolve) => {
      resolveAdd = resolve;
    });
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-close-pending-add",
        reasoningProofId: "proof-close-pending-add",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{ modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null }],
      });
    api.manageNativeModelCatalog.mockImplementation(({ operation }) => {
      if (operation === "add") return pendingAdd;
      return Promise.resolve(currentCatalogState);
    });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);
    await screen.findByRole("button", { name: "确认添加到 Codex" });
    await confirmAddModel(user);
    await waitFor(() => expect(api.manageNativeModelCatalog).toHaveBeenCalledWith(
      expect.objectContaining({ operation: "add", modelId: "vendor/model-latest" }),
    ));
    const addRequestId = api.manageNativeModelCatalog.mock.calls.find(
      ([request]) => request.operation === "add",
    )?.[0].requestId;
    expect(addRequestId).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "返回首页" }));
    await user.click(await screen.findByRole("button", { name: "重新打开" }));

    const refresh = screen.getByRole("button", { name: "刷新模型列表" });
    const reset = screen.getByRole("button", { name: "恢复 Codex 默认模型" });
    expect(refresh).toBeDisabled();
    expect(reset).toBeDisabled();
    expect(api.gatewayModelProbe).toHaveBeenCalledTimes(2);
    expect(api.manageNativeModelCatalog.mock.calls.filter(([request]) => request.operation === "reset")).toHaveLength(0);
    expect(api.cancelGatewayModelProbe).not.toHaveBeenCalledWith(addRequestId);

    currentCatalogState = { ...catalogState, modelIds: ["vendor/model-latest"] };
    resolveAdd({
      status: "applied_restart_pending",
      modelIds: ["vendor/model-latest"],
      restartRequired: true,
      message: "added",
    });

    expect(await screen.findByText("已添加到 Codex。重启 Codex 后即可使用。")).toBeInTheDocument();
    expect(screen.queryByText("共找到 1 个模型，已添加 1 个")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(refresh).toBeEnabled();
      expect(reset).toBeEnabled();
    });
    expect(api.cancelGatewayModelProbe).not.toHaveBeenCalledWith(addRequestId);
  });

  it("clears the Add ticket when the account or key fingerprint changes", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-stale-binding",
        reasoningProofId: "proof-stale-binding",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{ modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null }],
      });
    api.manageNativeModelCatalog
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce({ ...catalogState, connectionFingerprint: "changed-binding" });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);
    await waitFor(() => expect(screen.queryByRole("button", { name: "添加到 Codex" })).not.toBeInTheDocument());
    expect(await screen.findByText(/账号或服务设置已变化/)).toBeInTheDocument();
  });

  it("removes Add when the short-lived proof expires", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-expiring",
        reasoningProofId: "proof-expiring",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 1,
        results: [{ modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null }],
      });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);
    await screen.findByRole("button", { name: "确认添加到 Codex" }, { timeout: 800 });
    await waitFor(() => expect(screen.queryByRole("button", { name: "添加到 Codex" })).not.toBeInTheDocument(), { timeout: 2500 });
  });

  it("blocks discovery actions while an orphaned catalog needs reset", async () => {
    api.manageNativeModelCatalog.mockResolvedValue({
      ...catalogState, status: "orphaned", message: "reset required", modelIds: [],
    });
    renderSheet();
    await screen.findByRole("alert");
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    const testButton = within(row).getByRole("button", { name: "自动检测" });
    expect(testButton).toBeDisabled();
    expect(testButton).toHaveAttribute("aria-describedby");
    expect(screen.getByText("发现未完成的模型设置。请先恢复 Codex 默认模型，再继续。")).toHaveAttribute(
      "id",
      testButton.getAttribute("aria-describedby"),
    );
    expect(screen.getByRole("button", { name: "恢复 Codex 默认模型" })).toBeInTheDocument();
  });

  it("shows why a non-selectable model cannot be tested", async () => {
    api.gatewayModelProbe.mockResolvedValue({
      ...listing,
      models: [{ id: "vendor/model-latest", ownedBy: "fixture", selectable: false }],
    });

    renderSheet();
    const row = await screen.findByTestId("upstream-model-vendor/model-latest");
    const testButton = await within(row).findByRole("button", { name: "自动检测" });
    expect(testButton).toBeDisabled();
    const reasonId = testButton.getAttribute("aria-describedby");
    expect(reasonId).toBeTruthy();
    expect(document.getElementById(reasonId!)).toHaveTextContent("模型重复，不能添加");
  });

  it("refreshes state after resetting an orphaned catalog and enables model testing", async () => {
    const user = userEvent.setup();
    api.manageNativeModelCatalog
      .mockResolvedValueOnce({ ...catalogState, status: "orphaned", message: "reset required" })
      .mockResolvedValueOnce({ status: "applied_restart_pending", modelIds: [], restartRequired: true, message: "native restored" })
      .mockResolvedValueOnce(catalogState);

    renderSheet();
    await screen.findByRole("alert");
    await user.click(screen.getByRole("button", { name: "恢复 Codex 默认模型" }));

    expect(await screen.findByRole("button", { name: "自动检测" })).toBeInTheDocument();
    expect(api.manageNativeModelCatalog.mock.calls[2]?.[0]).toEqual({ operation: "state" });
  });

  it("revokes the original proof request and rereads state after Add fails", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-failed-add",
        reasoningProofId: "proof-failed-add",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{ modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null }],
      });
    api.manageNativeModelCatalog
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce(catalogState)
      .mockRejectedValueOnce(new Error("upstream recheck failed"))
      .mockResolvedValueOnce({ ...catalogState, status: "orphaned", message: "reset required" });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);
    await confirmAddModel(user);

    const testedRequestId = api.gatewayModelProbe.mock.calls[1]?.[0].requestId;
    await waitFor(() => expect(api.cancelGatewayModelProbe).toHaveBeenCalledWith(testedRequestId));
    expect(api.manageNativeModelCatalog.mock.calls[3]?.[0]).toEqual({ operation: "state" });
    expect(screen.getByRole("button", { name: "重新检测" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "恢复 Codex 默认模型" })).toBeInTheDocument();
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    expect(within(row).getByText("还没测试")).toBeInTheDocument();
    expect(within(row).queryByText("文字测试可用")).not.toBeInTheDocument();
  });

  it("keeps Test and Add disabled when a partial Reset rereads as orphaned", async () => {
    const user = userEvent.setup();
    api.manageNativeModelCatalog
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce({
        status: "cleanup_pending", modelIds: [], restartRequired: true,
        message: "Codex native catalog restored; cleanup pending",
      })
      .mockResolvedValueOnce({
        ...catalogState, status: "orphaned", modelIds: [], message: "reset required",
      });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await user.click(screen.getByRole("button", { name: "恢复 Codex 默认模型" }));

    await screen.findByRole("alert");
    expect(screen.getByRole("button", { name: "自动检测" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "添加到 Codex" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "恢复 Codex 默认模型" })).toBeInTheDocument();
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    expect(within(row).getByText("还没测试")).toBeInTheDocument();
    expect(within(row).queryByText("文字测试可用")).not.toBeInTheDocument();
  });

  it("keeps only Reset when a partial Add rereads as orphaned", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({
        ...listing,
        status: "passed",
        generatedRequests: 1,
        connectivityModelId: "vendor/model-latest",
        connectivityProofId: "proof-add-cleanup-pending",
        reasoningProofId: "proof-add-cleanup-pending",
        reasoningResults: [{ effort: "medium", status: "accepted_by_route", errorCode: null }],
        checkedAt: Math.floor(Date.now() / 1000),
        connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
        results: [{ modelId: "vendor/model-latest", passed: true, generatedRequests: 1,
          checks: [{ name: "text", passed: true }], errorCode: null }],
      });
    api.manageNativeModelCatalog
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce(catalogState)
      .mockResolvedValueOnce({
        status: "cleanup_pending", modelIds: [], restartRequired: true,
        message: "Add cleanup pending",
      })
      .mockResolvedValueOnce({
        ...catalogState, status: "orphaned", modelIds: [], message: "reset required",
      });

    renderSheet();
    await screen.findByText("vendor/model-latest");
    await runReasoningProbe(user);
    await confirmAddModel(user);

    await screen.findByText("发现未完成的模型设置。请先恢复 Codex 默认模型，再继续。");
    expect(screen.getByRole("button", { name: "重新检测" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "添加到 Codex" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "恢复 Codex 默认模型" })).toBeInTheDocument();
    const row = screen.getByTestId("upstream-model-vendor/model-latest");
    expect(within(row).getByText("还没测试")).toBeInTheDocument();
    expect(within(row).queryByText("文字测试可用")).not.toBeInTheDocument();
  });
});
