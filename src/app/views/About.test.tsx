import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  managerApi,
  type UpdateInfo,
} from "../../services/managerApi";
import type { UsbGuardStatus } from "../../shared/types";
import { CATALOG, I18nProvider } from "../i18n";
import { ThemeProvider } from "../theme";
import { About } from "./About";

const platformMock = vi.hoisted(() =>
  vi.fn<() => "macos" | "windows" | "other">(() => "other"),
);

vi.mock("../platform", () => ({
  currentPlatform: platformMock,
}));

vi.mock("../../services/managerApi", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../services/managerApi")>();
  return {
    ...actual,
    managerApi: {
      ...actual.managerApi,
      checkManagerUpdate: vi.fn(),
      checkCodexPayloadUpdate: vi.fn(),
      applyCodexPayloadUpdate: vi.fn(),
      usbGuardStatus: vi.fn(),
      openLogsDir: vi.fn(),
      getDiagnostics: vi.fn(),
      openUrl: vi.fn(),
    },
  };
});

const api = vi.mocked(managerApi);

const CODEX_UPDATE: UpdateInfo = {
  channel: "codex",
  currentVersion: "26.825.51511",
  latestVersion: "26.826.51511",
  releaseNotes: null,
  sizeBytes: 1234,
  requiresRestart: false,
  payload: { repo: "owner/repo", releaseId: 42 },
};

const USB_GUARD: UsbGuardStatus = {
  schemaVersion: 1,
  status: "authorized",
  watchdog: "running",
  removable: true,
  driveRoot: "E:\\",
  bundlePath: "E:\\CodexBundle",
  bundleRelativePath: "CodexBundle",
  signed: true,
  lastCheckedAtMs: 1,
  missingSinceMs: null,
  countdownSeconds: null,
  managedProcesses: [],
  message: "authorized",
};

const NO_USB_GUARD: UsbGuardStatus = {
  ...USB_GUARD,
  status: "unsupported",
  watchdog: "not-running",
  removable: false,
  driveRoot: null,
  bundlePath: null,
  bundleRelativePath: null,
  signed: false,
};

function renderAbout() {
  return render(
    <ThemeProvider>
      <I18nProvider>
        <About onBack={vi.fn()} />
      </I18nProvider>
    </ThemeProvider>,
  );
}

describe("About Codex payload update entry", () => {
  beforeEach(() => {
    localStorage.setItem("cam.lang", "en");
    platformMock.mockReset();
    platformMock.mockReturnValue("other");
    api.usbGuardStatus.mockReset();
    api.checkCodexPayloadUpdate.mockReset();
    api.applyCodexPayloadUpdate.mockReset();
    api.usbGuardStatus.mockResolvedValue(NO_USB_GUARD);
    api.checkCodexPayloadUpdate.mockResolvedValue({ kind: "development" });
  });

  it("does not offer a Codex payload update outside Windows", () => {
    renderAbout();

    expect(
      screen.queryByRole("button", { name: CATALOG.en["about.checkCodexPayload"] }),
    ).not.toBeInTheDocument();
    expect(api.usbGuardStatus).not.toHaveBeenCalled();
  });

  it("does not offer the update in an ordinary non-USB Windows manager", async () => {
    platformMock.mockReturnValue("windows");
    renderAbout();

    await waitFor(() => expect(api.usbGuardStatus).toHaveBeenCalledTimes(1));
    expect(
      screen.queryByRole("button", { name: CATALOG.en["about.checkCodexPayload"] }),
    ).not.toBeInTheDocument();
  });

  it("rechecks a transient guard state so the update entry survives startup races", async () => {
    platformMock.mockReturnValue("windows");
    api.usbGuardStatus
      .mockResolvedValueOnce({
        ...NO_USB_GUARD,
        status: "stopped",
        message: "starting",
      })
      .mockResolvedValueOnce(USB_GUARD);

    renderAbout();

    expect(
      await screen.findByRole("button", {
        name: CATALOG.en["about.checkCodexPayload"],
      }),
    ).toBeInTheDocument();
    expect(api.usbGuardStatus).toHaveBeenCalledTimes(2);
  });

  it("rechecks the explicit startup state before hiding payload updates", async () => {
    platformMock.mockReturnValue("windows");
    api.usbGuardStatus
      .mockResolvedValueOnce({
        ...NO_USB_GUARD,
        status: "starting",
        removable: true,
        message: "正在校验 U 盘能力包",
      })
      .mockResolvedValueOnce(USB_GUARD);

    renderAbout();

    expect(
      await screen.findByRole("button", {
        name: CATALOG.en["about.checkCodexPayload"],
      }),
    ).toBeInTheDocument();
    expect(api.usbGuardStatus).toHaveBeenCalledTimes(2);
  });

  it("keeps the manual confirmation and clears an expired pending update", async () => {
    const user = userEvent.setup();
    platformMock.mockReturnValue("windows");
    api.usbGuardStatus.mockResolvedValue(USB_GUARD);
    api.checkCodexPayloadUpdate.mockResolvedValue({
      kind: "available",
      info: CODEX_UPDATE,
    });
    api.applyCodexPayloadUpdate.mockRejectedValue({
      code: "stale_expectation",
      message: "expired",
    });
    renderAbout();

    const check = await screen.findByRole("button", {
      name: CATALOG.en["about.checkCodexPayload"],
    });
    await user.click(check);
    await waitFor(() => expect(api.checkCodexPayloadUpdate).toHaveBeenCalledTimes(1));
    expect(api.applyCodexPayloadUpdate).not.toHaveBeenCalled();

    await user.click(check);
    const dialog = screen.getByRole("dialog", {
      name: CATALOG.en["about.codexConfirmTitle"].replace(
        "{version}",
        CODEX_UPDATE.latestVersion,
      ),
    });
    expect(api.applyCodexPayloadUpdate).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: CATALOG.en["confirm.ok"] }));
    await waitFor(() => expect(api.applyCodexPayloadUpdate).toHaveBeenCalledWith(CODEX_UPDATE));
    expect(
      await screen.findByText(CATALOG.en["about.codexStale"]),
    ).toBeInTheDocument();

    await user.click(check);
    await waitFor(() => expect(api.checkCodexPayloadUpdate).toHaveBeenCalledTimes(2));
  });
});
