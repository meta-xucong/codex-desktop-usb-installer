import { useState } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { managerApi } from "../../services/managerApi";
import type { GatewayModelProbeReport, GatewayReasoningEffortResult } from "../../shared/types";
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
const ready = {
  status: "ready",
  modelIds: ["gpt-5"],
  restartRequired: false,
  message: "ready",
  connectionFingerprint: "binding-fixture",
  credentialFingerprint: "sha256:fixture",
};
const added = {
  ...ready,
  status: "applied_restart_pending",
  modelIds: ["gpt-5", "vendor/model-latest"],
  restartRequired: true,
  message: "added",
};

function passedProbe(
  results: GatewayReasoningEffortResult[] = [
    { effort: "low", status: "accepted_by_route", errorCode: null },
    { effort: "medium", status: "accepted_by_route", errorCode: null },
  ],
  modelId = "vendor/model-latest",
): GatewayModelProbeReport {
  return {
    ...listing,
    status: "passed" as const,
    generatedRequests: 4,
    reasoningModelId: modelId,
    reasoningProofId: "proof-single-click",
    connectivityProofId: "proof-single-click",
    reasoningResults: results,
    checkedAt: Math.floor(Date.now() / 1000),
    connectivityProofExpiresAt: Math.floor(Date.now() / 1000) + 300,
  };
}

function Harness() {
  const [open, setOpen] = useState(true);
  return (
    <>
      {!open ? <button type="button" onClick={() => setOpen(true)}>重新打开</button> : null}
      <UpstreamModelManagerSheet open={open} onDismiss={() => setOpen(false)} />
    </>
  );
}

function renderSheet() {
  return render(
    <ThemeProvider>
      <I18nProvider>
        <Harness />
      </I18nProvider>
    </ThemeProvider>,
  );
}

async function clickProbeAndAdd(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "探测并添加" }));
}

describe("UpstreamModelManagerSheet direct add flow", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    localStorage.setItem("cam.lang", "zh-CN");
    localStorage.removeItem("codex-manager.upstream-model-probe.v1");
    api.manageNativeModelCatalog.mockImplementation(async (request) => {
      if (request.operation === "prepare") return ready;
      if (request.operation === "add") return added;
      return {
        status: "ok",
        modelIds: [],
        restartRequired: false,
        message: "state loaded",
        connectionFingerprint: "binding-fixture",
        credentialFingerprint: "sha256:fixture",
      };
    });
    api.gatewayModelProbe.mockResolvedValue(listing);
    api.cancelGatewayModelProbe.mockResolvedValue(true);
  });

  it("discovers models without probing or adding one", async () => {
    renderSheet();
    const row = await screen.findByTestId("upstream-model-vendor/model-latest");
    expect(within(row).getByRole("button", { name: "探测并添加" })).toBeInTheDocument();
    expect(api.gatewayModelProbe).toHaveBeenCalledTimes(1);
    expect(api.manageNativeModelCatalog).toHaveBeenCalledWith(expect.objectContaining({ operation: "state" }));
    expect(api.manageNativeModelCatalog).not.toHaveBeenCalledWith(expect.objectContaining({ operation: "add" }));
  });

  it("probes the full supported effort set and automatically adds after any accepted result", async () => {
    const user = userEvent.setup();
    const confirmSpy = vi.spyOn(window, "confirm");
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce(passedProbe());
    renderSheet();

    await clickProbeAndAdd(user);

    await waitFor(() => expect(api.manageNativeModelCatalog).toHaveBeenCalledWith(expect.objectContaining({
      operation: "add",
      modelId: "vendor/model-latest",
      connectivityProofId: "proof-single-click",
    })));
    expect(api.manageNativeModelCatalog.mock.invocationCallOrder[1])
      .toBeLessThan(api.gatewayModelProbe.mock.invocationCallOrder[1]!);
    expect(api.gatewayModelProbe.mock.calls[1]?.[0]).toMatchObject({
      reasoningModelId: "vendor/model-latest",
      reasoningEfforts: ["none", "minimal", "low", "medium", "high", "xhigh", "max"],
      reasoningCostConsent: true,
    });
    expect(confirmSpy).not.toHaveBeenCalled();
    expect(await screen.findByText("已添加到 Codex")).toBeInTheDocument();
    expect(screen.getByTestId("upstream-model-vendor/model-latest")).toBeInTheDocument();
  });

  it("keeps the accepted subset when later effort results are unknown and still adds", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce(passedProbe([
        { effort: "low", status: "accepted_by_route", errorCode: null },
        { effort: "medium", status: "unknown", errorCode: "upstream-timeout" },
      ]));
    renderSheet();

    await clickProbeAndAdd(user);

    await waitFor(() => expect(api.manageNativeModelCatalog).toHaveBeenCalledWith(expect.objectContaining({ operation: "add" })));
    expect(await screen.findByText("已添加到 Codex")).toBeInTheDocument();
  });

  it("omits none for Claude Opus 5.5 before sending the reasoning probe", async () => {
    const user = userEvent.setup();
    const opus = { id: "claude-opus-5-5", ownedBy: "fixture", selectable: true };
    api.gatewayModelProbe
      .mockResolvedValueOnce({ ...listing, models: [opus] })
      .mockResolvedValueOnce(passedProbe(undefined, opus.id));
    renderSheet();

    await user.click(await screen.findByRole("button", { name: "探测并添加" }));

    await waitFor(() => expect(api.gatewayModelProbe).toHaveBeenCalledTimes(2));
    expect(api.gatewayModelProbe.mock.calls[1]?.[0]).toMatchObject({
      reasoningModelId: opus.id,
      reasoningEfforts: ["minimal", "low", "medium", "high", "xhigh", "max"],
    });
    await waitFor(() => expect(api.manageNativeModelCatalog).toHaveBeenCalledWith(expect.objectContaining({ operation: "add" })));
  });

  it("preserves the tested proof after bounded Add retries and reuses it on the next click", async () => {
    const user = userEvent.setup();
    let addAttempts = 0;
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce(passedProbe());
    api.manageNativeModelCatalog.mockImplementation(async (request) => {
      if (request.operation === "prepare") return ready;
      if (request.operation === "add") {
        addAttempts += 1;
        return addAttempts <= 3 ? { ...ready, status: "precommit_retryable", retryable: true } : added;
      }
      return { ...ready, status: "ok", modelIds: [] };
    });
    renderSheet();

    await clickProbeAndAdd(user);

    const retryButton = await screen.findByRole("button", { name: "重试添加" });
    expect(addAttempts).toBe(3);
    expect(api.cancelGatewayModelProbe).not.toHaveBeenCalled();
    await user.click(retryButton);

    await waitFor(() => expect(addAttempts).toBe(4));
    expect(api.gatewayModelProbe).toHaveBeenCalledTimes(2);
    expect(api.manageNativeModelCatalog).toHaveBeenCalledTimes(6);
    expect(await screen.findByText("已添加到 Codex")).toBeInTheDocument();
  });

  it("does not add when no effort was explicitly accepted", async () => {
    const user = userEvent.setup();
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce({ ...listing, status: "failed", errorCode: "no-effort-accepted", reasoningResults: [] });
    renderSheet();

    await clickProbeAndAdd(user);

    await waitFor(() => expect(api.gatewayModelProbe).toHaveBeenCalledTimes(2));
    expect(api.manageNativeModelCatalog).not.toHaveBeenCalledWith(expect.objectContaining({ operation: "add" }));
  });

  it("stops before paid probes when local preparation reports a duplicate", async () => {
    const user = userEvent.setup();
    api.manageNativeModelCatalog.mockImplementation(async (request) => request.operation === "prepare"
      ? { ...ready, status: "already_added", modelIds: ["gpt-5", "vendor/model-latest"] }
      : request.operation === "add" ? added : { ...ready, status: "ok", modelIds: [] });
    renderSheet();

    await clickProbeAndAdd(user);

    expect(api.gatewayModelProbe).toHaveBeenCalledTimes(1);
    expect(api.manageNativeModelCatalog).not.toHaveBeenCalledWith(expect.objectContaining({ operation: "add" }));
  });

  it("treats a detached existing catalog that was reattached during prepare as active", async () => {
    const user = userEvent.setup();
    api.manageNativeModelCatalog.mockImplementation(async (request) => request.operation === "prepare"
      ? { ...ready, status: "applied_restart_pending", modelIds: ["gpt-5", "vendor/model-latest"] }
      : { ...ready, status: "ok", modelIds: [] });
    renderSheet();

    await clickProbeAndAdd(user);

    expect(api.gatewayModelProbe).toHaveBeenCalledTimes(1);
    expect(api.manageNativeModelCatalog).not.toHaveBeenCalledWith(expect.objectContaining({ operation: "add" }));
    expect(await screen.findByText("已添加到 Codex")).toBeInTheDocument();
  });

  it("does not begin paid probes if local prerequisites fail", async () => {
    const user = userEvent.setup();
    api.manageNativeModelCatalog.mockImplementation(async (request) => {
      if (request.operation === "prepare") throw new Error("local catalog conflict");
      return { ...ready, status: "ok", modelIds: [] };
    });
    api.gatewayModelProbe.mockResolvedValue(listing);
    renderSheet();

    await clickProbeAndAdd(user);

    expect(api.gatewayModelProbe).toHaveBeenCalledTimes(1);
    expect(api.manageNativeModelCatalog).not.toHaveBeenCalledWith(expect.objectContaining({ operation: "add" }));
    expect(await screen.findByText("local catalog conflict")).toBeInTheDocument();
  });

  it("reports commit errors instead of claiming an uncommitted model was added", async () => {
    const user = userEvent.setup();
    api.manageNativeModelCatalog.mockImplementation(async (request) => {
      if (request.operation === "prepare") return ready;
      if (request.operation === "state") return { ...ready, status: "ok", modelIds: ["gpt-5"] };
      if (request.operation === "add") return { ...ready, status: "failed", modelIds: [], message: "write blocked" };
      return { ...ready, status: "ok", modelIds: [] };
    });
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce(passedProbe());
    renderSheet();

    await clickProbeAndAdd(user);

    expect(await screen.findByText("write blocked")).toBeInTheDocument();
    expect(screen.queryByText("已添加到 Codex")).not.toBeInTheDocument();
    expect(screen.getByTestId("upstream-model-configured-only-gpt-5")).toBeInTheDocument();
  });

  it("refreshes the catalog after Add throws so a committed model is not shown as missing", async () => {
    const user = userEvent.setup();
    let stateReads = 0;
    api.manageNativeModelCatalog.mockImplementation(async (request) => {
      if (request.operation === "prepare") return ready;
      if (request.operation === "state") {
        stateReads += 1;
        return {
          ...ready,
          status: "ok",
          modelIds: stateReads > 1 ? ["gpt-5", "vendor/model-latest"] : ["gpt-5"],
        };
      }
      if (request.operation === "add") throw new Error("connection lost after write");
      return { ...ready, status: "ok", modelIds: ["gpt-5"] };
    });
    api.gatewayModelProbe
      .mockResolvedValueOnce(listing)
      .mockResolvedValueOnce(passedProbe());
    renderSheet();

    await clickProbeAndAdd(user);

    expect(await screen.findByText("connection lost after write")).toBeInTheDocument();
    await waitFor(() => expect(stateReads).toBe(2));
    const modelRow = screen.getByTestId("upstream-model-vendor/model-latest");
    expect(within(modelRow).getByText("已添加到 Codex")).toBeInTheDocument();
  });
});
