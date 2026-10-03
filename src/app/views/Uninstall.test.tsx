import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { managerApi } from "../../services/managerApi";
import { emptyOperationOutcome } from "../../shared/types";
import { CATALOG, I18nProvider, type Lang } from "../i18n";
import { Uninstall } from "./Uninstall";

vi.mock("../../services/managerApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../services/managerApi")>()),
  managerApi: {
    macStatus: vi.fn(),
    winStatus: vi.fn(),
    macUninstall: vi.fn(),
    winUninstall: vi.fn(),
    winUninstallBundle: vi.fn(),
    getUninstallHandoff: vi.fn(),
    completeUninstallHandoff: vi.fn(),
    retryAncillary: vi.fn(),
    armDestructive: vi.fn(() => Promise.resolve("token")),
    openCodexHome: vi.fn(() => Promise.resolve()),
  },
}));

const macStatus = vi.mocked(managerApi.macStatus);
const winStatus = vi.mocked(managerApi.winStatus);
const macUninstall = vi.mocked(managerApi.macUninstall);
const winUninstall = vi.mocked(managerApi.winUninstall);
const winUninstallBundle = vi.mocked(managerApi.winUninstallBundle);
const completeUninstallHandoff = vi.mocked(managerApi.completeUninstallHandoff);
const retryAncillary = vi.mocked(managerApi.retryAncillary);
const SAFETY_FLOW_LANGS = ["fr", "ar", "es", "zh-TW"] as const satisfies readonly Lang[];
const CODEX_PURGE_PATHS = [
  "%USERPROFILE%\\.codex", "%USERPROFILE%\\.agents\\skills", "%LOCALAPPDATA%\\Packages\\OpenAI.Codex_2p2nqsd0c76g0",
  "%LOCALAPPDATA%\\OpenAI\\Codex", "%LOCALAPPDATA%\\Codex\\Logs",
  "%APPDATA%\\OpenAI\\Codex", "%APPDATA%\\Codex",
  "%LOCALAPPDATA%\\CodexAppxDataBackups\\OpenAI.Codex_2p2nqsd0c76g0",
];
const CCSWITCH_PURGE_PATHS = [
  "%USERPROFILE%\\.cc-switch", "%LOCALAPPDATA%\\com.ccswitch.desktop", "%APPDATA%\\com.ccswitch.desktop",
];

function setPlatform(platform: string) {
  Object.defineProperty(navigator, "platform", { configurable: true, value: platform });
}

function renderUninstall() {
  return render(
    <I18nProvider>
      <Uninstall onBack={vi.fn()} />
    </I18nProvider>,
  );
}

async function openBundle(user: ReturnType<typeof userEvent.setup>) {
  setPlatform("Win32");
  render(<I18nProvider><Uninstall onBack={vi.fn()} bundleMode /></I18nProvider>);
  await screen.findByText("CCSwitch program to remove");
  await user.click(screen.getByText(CATALOG.en["uninstall.bundle.advanced"]));
}

async function confirmDefaultBundle(user: ReturnType<typeof userEvent.setup>) {
  await openBundle(user);
  await user.click(screen.getByRole("button", { name: "Uninstall Codex + CCSwitch" }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  const purge = screen.queryByRole("button", { name: "Erase data & uninstall" });
  if (purge) await user.click(purge);
}

describe("Uninstall", () => {
  beforeEach(() => {
    localStorage.setItem("cam.lang", "en");
    macStatus.mockResolvedValue({ status: "managed", installed: null });
    winStatus.mockResolvedValue({ status: "managed", installed: null });
    macUninstall.mockResolvedValue({
      removed: true,
      keptCodexHome: true,
      message: "removed",
      outcome: emptyOperationOutcome({
        primaryOk: true,
        appState: "absent",
        installClass: "none",
      }),
    });
    winUninstall.mockResolvedValue({
      success: true,
      action: "remove-portable",
      message: "removed",
      installedBefore: null,
      msix: null,
      portable: null,
      purgedUserData: false,
      notes: [],
      outcome: emptyOperationOutcome({
        primaryOk: true,
        appState: "absent",
        installClass: "none",
      }),
    });
    winUninstallBundle.mockResolvedValue({
      success: true,
      codex: null,
      codexError: null,
      ccswitch: {
        success: true,
        installedBefore: true,
        removedInstall: true,
        removedAutostart: false,
        purgedUserData: false,
        installDir: "%LOCALAPPDATA%\\Programs\\CodexSwitch",
        dataDirs: ["%USERPROFILE%\\.cc-switch"],
        message: "bundle removed",
        notes: [],
      },
      ccswitchError: null,
      message: "Codex and CCSwitch removed",
      notes: [],
      purgedUserData: false,
      purgedCcswitchData: false,
      outcome: emptyOperationOutcome({
        primaryOk: true,
        appState: "absent",
        installClass: "none",
      }),
    });
    completeUninstallHandoff.mockResolvedValue(undefined);
    vi.mocked(managerApi.getUninstallHandoff).mockResolvedValue(null);
    retryAncillary.mockResolvedValue({
      message: "cleanup ok",
      outcome: emptyOperationOutcome({
        primaryOk: true,
        appState: "absent",
        installClass: "none",
      }),
    });
  });

  it("shows and copies the Windows Codex data path", async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue(undefined);
    setPlatform("Win32");
    renderUninstall();

    expect(await screen.findByText("Data location")).toBeInTheDocument();
    expect(screen.getByText("%USERPROFILE%\\.codex")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Copy path" }));

    expect(writeText).toHaveBeenCalledWith("%USERPROFILE%\\.codex");
    expect(screen.getByRole("button", { name: "Path copied" })).toBeInTheDocument();
    writeText.mockRestore();
  });

  it("runs the combined Windows cleanup and purges managed data by default", async () => {
    const user = userEvent.setup();
    setPlatform("Win32");
    render(<I18nProvider><Uninstall onBack={vi.fn()} bundleMode /></I18nProvider>);

    expect(await screen.findByText("CCSwitch program to remove")).toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "Keep my data" })).not.toBeInTheDocument();
    await user.click(screen.getByText(CATALOG.en["uninstall.bundle.advanced"]));
    expect(screen.getByRole("switch", { name: CATALOG.en["uninstall.bundle.purgeCodex"] })).toBeChecked();
    expect(screen.getByRole("switch", { name: CATALOG.en["uninstall.bundle.purgeCcswitch"] })).toBeChecked();
    expect(screen.getByRole("switch", { name: CATALOG.en["uninstall.bundle.purgeManager"] })).toBeChecked();
    await user.click(screen.getByRole("button", { name: "Uninstall Codex + CCSwitch" }));
    const first = screen.getByRole("dialog", { name: "Uninstall Codex + CCSwitch?" });
    expect(within(first).getByText(CATALOG.en["uninstall.bundle.codexErase"])).toBeInTheDocument();
    expect(within(first).getByText(CATALOG.en["uninstall.bundle.ccswitchErase"])).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(screen.getByRole("dialog", { name: CATALOG.en["uninstall.bundle.confirm2.title"] })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Erase data & uninstall" }));

    await waitFor(() => expect(winUninstallBundle).toHaveBeenCalledWith(true, true, true, true));
    expect(winUninstall).not.toHaveBeenCalled();
  });

  it("allows the USB bundle cleanup to remove an untracked AppX residual", async () => {
    const user = userEvent.setup();
    setPlatform("Win32");
    winStatus.mockResolvedValue({
      status: "external",
      installed: {
        path: "C:\\Program Files\\WindowsApps\\OpenAI.Codex_26.825.5331.0_x64__2p2nqsd0c76g0",
        version: "26.825.5331.0",
        arch: "x64",
        source: "msix",
        packageFamilyName: "OpenAI.Codex_2p2nqsd0c76g0",
        packageVolume: "C:\\",
        installedAt: null,
      },
    });
    render(<I18nProvider><Uninstall onBack={vi.fn()} bundleMode /></I18nProvider>);

    expect(await screen.findByText(/No managed Codex record was found/i)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Uninstall Codex + CCSwitch" }));
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(screen.getByRole("button", { name: "Erase data & uninstall" }));

    await waitFor(() => expect(winUninstallBundle).toHaveBeenCalledWith(true, true, true, true));
  });

  it.each([[true, false], [false, true], [true, true]])("independently confirms shared data: Codex=%s CCSwitch=%s", async (purgeCodex, purgeCcswitch) => {
    const user = userEvent.setup();
    setPlatform("Win32");
    render(<I18nProvider><Uninstall onBack={vi.fn()} bundleMode /></I18nProvider>);

    await screen.findByText("CCSwitch program to remove");
    await user.click(screen.getByText(CATALOG.en["uninstall.bundle.advanced"]));
    // The USB bundle defaults to a complete cleanup. Toggle a switch only when
    // this case asks to retain that data, so the parameters describe the final
    // state sent to the backend.
    if (!purgeCodex) await user.click(screen.getByRole("switch", { name: CATALOG.en["uninstall.bundle.purgeCodex"] }));
    if (!purgeCcswitch) await user.click(screen.getByRole("switch", { name: CATALOG.en["uninstall.bundle.purgeCcswitch"] }));
    await user.click(screen.getByRole("button", { name: "Uninstall Codex + CCSwitch" }));
    const first = screen.getByRole("dialog", { name: "Uninstall Codex + CCSwitch?" });
    expect(within(first).getByText(CATALOG.en[purgeCodex ? "uninstall.bundle.codexErase" : "uninstall.bundle.codexKeep"])).toBeInTheDocument();
    expect(within(first).getByText(CATALOG.en[purgeCcswitch ? "uninstall.bundle.ccswitchErase" : "uninstall.bundle.ccswitchKeep"])).toBeInTheDocument();
    for (const path of [...CODEX_PURGE_PATHS, ...CCSWITCH_PURGE_PATHS]) expect(first).toHaveTextContent(path);
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(winUninstallBundle).not.toHaveBeenCalled();
    const second = screen.getByRole("dialog", { name: CATALOG.en["uninstall.bundle.confirm2.title"] });
    for (const path of [...CODEX_PURGE_PATHS, ...CCSWITCH_PURGE_PATHS]) expect(second).toHaveTextContent(path);
    expect(within(second).getByText(CATALOG.en[purgeCodex ? "uninstall.bundle.codexErase" : "uninstall.bundle.codexKeep"])).toBeInTheDocument();
    expect(within(second).getByText(CATALOG.en[purgeCcswitch ? "uninstall.bundle.ccswitchErase" : "uninstall.bundle.ccswitchKeep"])).toBeInTheDocument();
    expect(within(second).queryByText(CATALOG.en[purgeCodex ? "uninstall.bundle.codexKeep" : "uninstall.bundle.codexErase"])).not.toBeInTheDocument();
    expect(within(second).queryByText(CATALOG.en[purgeCcswitch ? "uninstall.bundle.ccswitchKeep" : "uninstall.bundle.ccswitchErase"])).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Erase data & uninstall" }));
    await waitFor(() => expect(winUninstallBundle).toHaveBeenCalledWith(true, purgeCodex, purgeCcswitch, true));
  });

  it("keeps the USB installation kit available for future reinstalls", async () => {
    const user = userEvent.setup();
    await openBundle(user);
    expect(screen.queryByRole("switch", { name: "Also erase this USB installation kit" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Uninstall Codex + CCSwitch" }));
    const first = screen.getByRole("dialog", { name: "Uninstall Codex + CCSwitch?" });
    expect(first).toHaveTextContent(CATALOG.en["uninstall.bundle.mediaKeep"]);
    await user.click(within(first).getByRole("button", { name: "Continue" }));
    await user.click(screen.getByRole("button", { name: "Erase data & uninstall" }));
    await waitFor(() => expect(winUninstallBundle).toHaveBeenCalledWith(true, true, true, true));
  });

  it.each([1, 2])("cancelling confirmation %s never starts program or data removal", async (step) => {
    const user = userEvent.setup();
    await openBundle(user);
    await user.click(screen.getByRole("switch", { name: CATALOG.en["uninstall.bundle.purgeCodex"] }));
    await user.click(screen.getByRole("button", { name: "Uninstall Codex + CCSwitch" }));
    if (step === 2) await user.click(screen.getByRole("button", { name: "Continue" }));
    const dialog = screen.getByRole("dialog", { name: step === 1 ? "Uninstall Codex + CCSwitch?" : CATALOG.en["uninstall.bundle.confirm2.title"] });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(winUninstallBundle).not.toHaveBeenCalled();
    expect(winUninstall).not.toHaveBeenCalled();
  });

  it("keeps missing-auth and small-directory heuristics out of data deletion authorization", async () => {
    const user = userEvent.setup();
    await openBundle(user);
    expect(screen.getByText(CATALOG.en["uninstall.bundle.sharedRisk"])).toHaveTextContent(/permanently delete the current user's Codex, CCSwitch, and manager data/i);
    expect(screen.getByRole("switch", { name: CATALOG.en["uninstall.bundle.purgeCodex"] })).toBeChecked();
    expect(screen.getByRole("switch", { name: CATALOG.en["uninstall.bundle.purgeCcswitch"] })).toBeChecked();
    expect(screen.getByRole("switch", { name: CATALOG.en["uninstall.bundle.purgeManager"] })).toBeChecked();
  });

  it("preserves raw thrown backend errors in a details disclosure", async () => {
    const user = userEvent.setup();
    const raw = "Remove-AppxPackage HRESULT 0x80073CFA: AC backup verification failed; C:\\Users\\test\\AppData\\Local\\Packages\\OpenAI.Codex_2p2nqsd0c76g0";
    winUninstallBundle.mockRejectedValue({ code: "permission", message: raw });
    await confirmDefaultBundle(user);
    expect(await screen.findByText(CATALOG.en["error.permission"])).toBeInTheDocument();
    await user.click(screen.getByText(CATALOG.en["home.error.details"]));
    expect(screen.getByText(raw, { selector: "pre.errdetails" })).toBeVisible();
    expect(document.querySelector(".ring.success")).not.toBeInTheDocument();
    expect(screen.queryByText(CATALOG.en["uninstall.bundle.exiting"])).not.toBeInTheDocument();
  });

  it("does not let cleanupPending mask success=false and nested backend errors", async () => {
    const user = userEvent.setup();
    const original = await winUninstallBundle.getMockImplementation()!(true, false, false);
    const raw = "Windows could not remove exact package: HRESULT 0x80073CFA";
    winUninstallBundle.mockResolvedValue({ ...original, success: false, cleanupPending: true,
      message: "Uninstall stopped", codexError: raw,
      outcome: emptyOperationOutcome({ primaryOk: false, appState: "unknown" }),
    });
    await confirmDefaultBundle(user);
    expect(await screen.findByText(CATALOG.en["uninstall.failed"])).toBeInTheDocument();
    await user.click(screen.getByText(CATALOG.en["home.error.details"]));
    expect(screen.getByText(/Windows could not remove exact package/, { selector: "pre.errdetails" })).toHaveTextContent(raw);
    expect(screen.queryByText(CATALOG.en["uninstall.bundle.exiting"])).not.toBeInTheDocument();
    expect(document.querySelector(".ring.success")).not.toBeInTheDocument();
  });

  it("keeps ancillary failures actionable instead of entering pending-exit success", async () => {
    const user = userEvent.setup();
    const original = await winUninstallBundle.getMockImplementation()!(true, false, false);
    winUninstallBundle.mockResolvedValue({ ...original, cleanupPending: true, message: "helper was not started: metadata access denied",
      outcome: emptyOperationOutcome({ primaryOk: true, appState: "absent",
        cleanup: { state: "failed", detail: "metadata access denied" }, recoveryActions: ["cleanup_metadata"] }),
    });
    await confirmDefaultBundle(user);
    expect(await screen.findByRole("button", { name: /Retry cleanup only/ })).toBeEnabled();
    expect(screen.queryByText(CATALOG.en["uninstall.bundle.exiting"])).not.toBeInTheDocument();
    expect(document.querySelector(".ring.success")).not.toBeInTheDocument();
  });

  it("reports primary USB uninstall success when shared recovery cleanup is safely deferred", async () => {
    const user = userEvent.setup();
    setPlatform("Win32");
    const original = await winUninstallBundle.getMockImplementation()!(true, false, false);
    winUninstallBundle.mockResolvedValue({
      ...original,
      cleanupDeferred: true,
      message: "USB apps removed; another install's recovery record was retained",
      outcome: emptyOperationOutcome({ primaryOk: true, appState: "absent",
        cleanup: { state: "skipped", detail: "shared cleanup retained" } }),
    });
    await confirmDefaultBundle(user);
    expect(await screen.findByText(CATALOG.en["uninstall.bundle.sharedCleanupDeferred"]))
      .toBeInTheDocument();
    expect(screen.getByText(CATALOG.en["uninstall.bundle.sharedCleanupDeferredNote"]))
      .toBeInTheDocument();
    await user.click(screen.getByText(CATALOG.en["home.error.details"]));
    expect(screen.getByText("USB apps removed; another install's recovery record was retained", {
      selector: "pre.errdetails",
    })).toBeVisible();
    expect(document.querySelector(".ring.success")).not.toBeInTheDocument();
  });

  it("does not show success for an unfinished backend cleanup state", async () => {
    const user = userEvent.setup();
    const original = await winUninstallBundle.getMockImplementation()!(true, false, false);
    winUninstallBundle.mockResolvedValue({ ...original, message: "Cleanup still pending",
      outcome: emptyOperationOutcome({ primaryOk: true, appState: "absent", cleanup: { state: "pending", detail: "helper pending" } }),
    });
    await confirmDefaultBundle(user);
    expect(await screen.findByText(CATALOG.en["uninstall.partial.summary"])).toBeInTheDocument();
    expect(document.querySelector(".ring.success")).not.toBeInTheDocument();
  });

  it("uninstalls after one confirmation when keeping data", async () => {
    const user = userEvent.setup();
    setPlatform("MacIntel");
    renderUninstall();

    await screen.findByText("Data location");
    await user.click(screen.getByRole("button", { name: "Uninstall" }));
    expect(screen.getByRole("dialog", { name: "Uninstall Codex?" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Continue" }));

    await waitFor(() => expect(macUninstall).toHaveBeenCalledWith(true));
  });

  it("shows pending cleanup rather than final success during the exit handoff", async () => {
    const user = userEvent.setup();
    setPlatform("Win32");
    const original = await winUninstallBundle.getMockImplementation()!(true, false, false);
    winUninstallBundle.mockResolvedValue({
      ...original,
      cleanupPending: true,
      cleanupHandoffId: "handoff-test-id",
      cleanupReportPath: "C:\\Users\\test\\Desktop\\Codex-cleanup.txt",
      message: "Waiting for process exit",
    });
    render(<I18nProvider><Uninstall onBack={vi.fn()} bundleMode /></I18nProvider>);
    await screen.findByText("CCSwitch program to remove");
    await user.click(screen.getByRole("button", { name: "Uninstall Codex + CCSwitch" }));
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(screen.getByRole("button", { name: "Erase data & uninstall" }));
    expect(await screen.findByText(CATALOG.en["uninstall.bundle.exiting"])).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Finish and exit" })).toBeEnabled();
    expect(completeUninstallHandoff).not.toHaveBeenCalled();
    expect(screen.getByText("Back", { selector: "button.btn" })).toBeDisabled();
    expect(screen.getByText("Back", { selector: "button.navback" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Finish and exit" }));
    await waitFor(() => expect(completeUninstallHandoff).toHaveBeenCalledTimes(1));
    expect(completeUninstallHandoff).toHaveBeenCalledWith("handoff-test-id");
  });

  it("recovers the pending exit action after the renderer is remounted", async () => {
    const user = userEvent.setup();
    setPlatform("Win32");
    vi.mocked(managerApi.getUninstallHandoff).mockResolvedValueOnce({
      id: "recovered-handoff-id",
      reportPath: "C:\\Users\\test\\AppData\\Local\\CodexAppManager\\logs\\cleanup.json",
      message: "Final cleanup is waiting for manager exit",
    });

    render(<I18nProvider><Uninstall onBack={vi.fn()} bundleMode /></I18nProvider>);

    expect(await screen.findByText(CATALOG.en["uninstall.bundle.exiting"])).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Finish and exit" })).toBeEnabled();
    expect(winUninstallBundle).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Finish and exit" }));
    await waitFor(() => expect(completeUninstallHandoff).toHaveBeenCalledTimes(1));
    expect(completeUninstallHandoff).toHaveBeenCalledWith("recovered-handoff-id");
  });

  it("returns bundle ancillary recovery to the confirmed final cleanup flow", async () => {
    const user = userEvent.setup();
    setPlatform("Win32");
    const original = await winUninstallBundle.getMockImplementation()!(true, false, false);
    winUninstallBundle.mockResolvedValueOnce({
      ...original,
      outcome: emptyOperationOutcome({
        primaryOk: true, appState: "absent", installClass: "none",
        cleanup: { state: "failed", detail: "shortcut busy" },
        recoveryActions: ["cleanup_metadata"],
      }),
    }).mockResolvedValue({ ...original, cleanupPending: true, cleanupHandoffId: "handoff-retry-id" });
    render(<I18nProvider><Uninstall onBack={vi.fn()} bundleMode /></I18nProvider>);
    await screen.findByText("CCSwitch program to remove");
    await user.click(screen.getByRole("button", { name: "Uninstall Codex + CCSwitch" }));
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(screen.getByRole("button", { name: "Erase data & uninstall" }));
    await user.click(await screen.findByRole("button", { name: /Retry cleanup only/i }));
    expect(await screen.findByText(/Manager caches still need final cleanup/)).toBeInTheDocument();
    expect(winUninstallBundle).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "Continue final cleanup" }));
    expect(winUninstallBundle).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(screen.getByRole("button", { name: "Erase data & uninstall" }));
    expect(await screen.findByText(CATALOG.en["uninstall.bundle.exiting"])).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Finish and exit" })).toBeInTheDocument();
    expect(winUninstallBundle).toHaveBeenCalledTimes(2);
  });

  it("requires a second confirmation before purging data", async () => {
    const user = userEvent.setup();
    setPlatform("MacIntel");
    renderUninstall();

    await screen.findByText("Data location");
    await user.click(screen.getByRole("switch", { name: "Keep my data" }));
    await user.click(screen.getByRole("button", { name: "Uninstall" }));
    await user.click(screen.getByRole("button", { name: "Continue" }));

    expect(macUninstall).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Erase all data?" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Erase & uninstall" }));

    await waitFor(() => expect(macUninstall).toHaveBeenCalledWith(false));
  });

  it("does not mislabel a status-probe failure as external install", async () => {
    setPlatform("MacIntel");
    macStatus.mockRejectedValue(new Error("status probe failed"));
    renderUninstall();

    expect(
      await screen.findByText(/Could not confirm install status \(probe failed\)/i),
    ).toBeInTheDocument();
    expect(screen.queryByText(/external Codex/i)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Uninstall" })).toBeDisabled();
  });

  it("offers ancillary-only cleanup retry after partial uninstall", async () => {
    const user = userEvent.setup();
    setPlatform("Win32");
    winUninstall.mockResolvedValue({
      success: true,
      action: "remove-portable",
      message: "removed with cleanup warnings",
      installedBefore: null,
      msix: null,
      portable: {
        success: true,
        partial: true,
        installRoot: "C:\\Codex",
        removedFiles: true,
        removedShortcut: false,
        removedUninstallEntry: false,
        purgedUserData: false,
        message: "partial",
        notes: ["Start Menu shortcut cleanup failed: access denied"],
      },
      purgedUserData: false,
      notes: ["Start Menu shortcut cleanup failed: access denied"],
      outcome: emptyOperationOutcome({
        primaryOk: true,
        appState: "absent",
        installClass: "none",
        cleanup: { state: "failed", detail: "shortcut cleanup failed" },
        recoveryActions: ["cleanup_metadata"],
        warnings: ["Start Menu shortcut cleanup failed: access denied"],
      }),
    });
    renderUninstall();

    await screen.findByText("Data location");
    await user.click(screen.getByRole("button", { name: "Uninstall" }));
    await user.click(screen.getByRole("button", { name: "Continue" }));

    expect(
      await screen.findByText(/App removed, but some cleanup did not finish/i),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: /Retry cleanup only/i }),
    );
    await waitFor(() =>
      expect(retryAncillary).toHaveBeenCalledWith(
        expect.objectContaining({ actions: ["cleanup_metadata"] }),
      ),
    );
  });

  it("requires a second confirm before retrying purge_user_data", async () => {
    const user = userEvent.setup();
    setPlatform("MacIntel");
    macUninstall.mockResolvedValue({
      removed: true,
      keptCodexHome: true,
      message: "removed but purge failed",
      outcome: emptyOperationOutcome({
        primaryOk: true,
        appState: "absent",
        installClass: "none",
        path: "/Applications/Codex.app",
        cleanup: { state: "failed", detail: "purge failed" },
        recoveryActions: ["purge_user_data"],
        warnings: ["~/.codex purge failed"],
      }),
    });
    renderUninstall();

    await screen.findByText("Data location");
    await user.click(screen.getByRole("button", { name: "Uninstall" }));
    await user.click(screen.getByRole("button", { name: "Continue" }));

    expect(
      await screen.findByText(/App removed, but some cleanup did not finish/i),
    ).toBeInTheDocument();
    // Clicking the purge retry CTA opens a confirm sheet — no API call yet.
    await user.click(
      screen.getByRole("button", { name: /Retry purging user data only/i }),
    );
    expect(retryAncillary).not.toHaveBeenCalled();
    const purgeDialog = screen.getByRole("dialog", { name: "Erase all data?" });
    expect(purgeDialog).toBeInTheDocument();

    await user.click(
      within(purgeDialog).getByRole("button", {
        name: /Retry purging user data only/i,
      }),
    );
    await waitFor(() =>
      expect(retryAncillary).toHaveBeenCalledWith(
        expect.objectContaining({
          actions: ["purge_user_data"],
          purgeUserData: true,
          path: "/Applications/Codex.app",
        }),
      ),
    );
  });

  it("requires independent Codex-only purge retry confirmation with every actual Windows target", async () => {
    const user = userEvent.setup();
    const original = await winUninstallBundle.getMockImplementation()!(true, false, false);
    winUninstallBundle.mockResolvedValue({ ...original, message: "Codex removed but selected data purge failed",
      outcome: emptyOperationOutcome({ primaryOk: true, appState: "absent", path: "D:\\WindowsApps\\OpenAI.Codex",
        cleanup: { state: "failed", detail: "purge failed: file locked" }, recoveryActions: ["purge_user_data"] }),
    });
    await openBundle(user);
    await user.click(screen.getByRole("switch", { name: CATALOG.en["uninstall.bundle.purgeCcswitch"] }));
    await user.click(screen.getByRole("button", { name: "Uninstall Codex + CCSwitch" }));
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(screen.getByRole("button", { name: "Erase data & uninstall" }));
    await user.click(await screen.findByRole("button", { name: /Retry purging user data only/i }));
    const dialog = screen.getByRole("dialog", { name: CATALOG.en["uninstall.bundle.retryPurgeTitle"] });
    for (const path of CODEX_PURGE_PATHS) expect(dialog).toHaveTextContent(path);
    for (const path of CCSWITCH_PURGE_PATHS) expect(dialog).not.toHaveTextContent(path);
    expect(dialog).toHaveTextContent(CATALOG.en["uninstall.bundle.retryPurgeScope"]);
    expect(retryAncillary).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(retryAncillary).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole("dialog", { name: CATALOG.en["uninstall.bundle.retryPurgeTitle"] })).not.toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: /Retry purging user data only/i }));
    await user.click(within(screen.getByRole("dialog", { name: CATALOG.en["uninstall.bundle.retryPurgeTitle"] })).getByRole("button", { name: /Retry purging user data only/i }));
    await waitFor(() => expect(retryAncillary).toHaveBeenCalledWith({ actions: ["purge_user_data"], purgeUserData: true, path: "D:\\WindowsApps\\OpenAI.Codex" }));
    expect(winUninstallBundle).toHaveBeenCalledTimes(1);
    expect(winUninstallBundle).toHaveBeenCalledWith(true, true, false, true);
  });

  it("preserves raw retry failure without claiming ancillary completion", async () => {
    const user = userEvent.setup();
    const original = await winUninstallBundle.getMockImplementation()!(true, false, false);
    winUninstallBundle.mockResolvedValue({ ...original,
      outcome: emptyOperationOutcome({ primaryOk: true, appState: "absent",
        cleanup: { state: "failed", detail: "metadata pending" }, recoveryActions: ["cleanup_metadata"] }),
    });
    retryAncillary.mockRejectedValue({ code: "permission", message: "cleanup_metadata: Win32 error 5 at exact uninstall entry" });
    await confirmDefaultBundle(user);
    await user.click(await screen.findByRole("button", { name: /Retry cleanup only/ }));
    expect(await screen.findByText("cleanup_metadata: Win32 error 5 at exact uninstall entry", { selector: "pre.errdetails" })).toBeInTheDocument();
    expect(screen.queryByText(CATALOG.en["uninstall.bundle.retryContinue"])).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Retry cleanup only/ })).toBeEnabled();
    expect(document.querySelector(".ring.success")).not.toBeInTheDocument();
  });

  it("keeps a primary-failed ancillary retry unresolved even if step states say ok", async () => {
    const user = userEvent.setup();
    const original = await winUninstallBundle.getMockImplementation()!(true, false, false);
    winUninstallBundle.mockResolvedValue({ ...original,
      outcome: emptyOperationOutcome({ primaryOk: true, appState: "absent", cleanup: { state: "failed", detail: "cleanup failed" }, recoveryActions: ["cleanup_metadata"] }),
    });
    retryAncillary.mockResolvedValue({ message: "retry could not establish completion", outcome: emptyOperationOutcome({ primaryOk: false, appState: "unknown" }) });
    await confirmDefaultBundle(user);
    await user.click(await screen.findByRole("button", { name: /Retry cleanup only/ }));
    expect(await screen.findByText(CATALOG.en["uninstall.partial.retryPending"])).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Retry cleanup only/ })).toBeEnabled();
    expect(document.querySelector(".ring.success")).not.toBeInTheDocument();
  });

  it("retains the normal Windows data toggle and uninstall API outside bundle mode", async () => {
    const user = userEvent.setup();
    setPlatform("Win32"); renderUninstall();
    await screen.findByText("Data location");
    expect(screen.queryByText(CATALOG.en["uninstall.bundle.advanced"])).not.toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: "Keep my data" }));
    await user.click(screen.getByRole("button", { name: "Uninstall" }));
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(winUninstall).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Erase & uninstall" }));
    await waitFor(() => expect(winUninstall).toHaveBeenCalledWith(true, true));
    expect(winUninstallBundle).not.toHaveBeenCalled();
  });

  it("keeps both an unattempted action and the attempted action when retry still fails", async () => {
    const user = userEvent.setup();
    setPlatform("MacIntel");
    macUninstall.mockResolvedValue({
      removed: true,
      keptCodexHome: true,
      message: "removed with two ancillary failures",
      outcome: emptyOperationOutcome({
        primaryOk: true,
        appState: "absent",
        installClass: "none",
        provenance: { state: "failed", detail: "record failed" },
        cleanup: { state: "failed", detail: "cleanup failed" },
        recoveryActions: ["cleanup_metadata", "record_provenance"],
      }),
    });
    // Deliberately omit recoveryActions: the failed step status must still keep
    // the attempted CTA, while the untouched cleanup action is merged back in.
    retryAncillary.mockResolvedValue({
      message: "record still failed",
      outcome: emptyOperationOutcome({
        primaryOk: true,
        appState: "present",
        installClass: "external",
        provenance: { state: "failed", detail: "disk full" },
        recoveryActions: [],
      }),
    });

    renderUninstall();
    await screen.findByText("Data location");
    await user.click(screen.getByRole("button", { name: "Uninstall" }));
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(
      screen.getByRole("button", { name: "Retry writing managed record only" }),
    );

    expect(
      await screen.findByText(/some recovery steps are still incomplete/i),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Retry cleanup only (shortcuts / uninstall entry)" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Retry writing managed record only" }),
    ).toBeInTheDocument();
  });

  it.each(SAFETY_FLOW_LANGS)(
    "preserves unattempted recovery actions after one successful retry in %s",
    async (lang) => {
      const user = userEvent.setup();
      localStorage.setItem("cam.lang", lang);
      setPlatform("MacIntel");
      macUninstall.mockResolvedValue({
        removed: true,
        keptCodexHome: true,
        message: "removed with recovery actions",
        outcome: emptyOperationOutcome({
          primaryOk: true,
          appState: "absent",
          installClass: "none",
          cleanup: { state: "failed", detail: "ancillary cleanup failed" },
          recoveryActions: [
            "cleanup_metadata",
            "clear_provenance",
            "purge_user_data",
            "record_provenance",
          ],
          warnings: ["ancillary cleanup failed"],
        }),
      });

      renderUninstall();

      await screen.findByText(CATALOG[lang]["uninstall.dataPath"]);
      await user.click(screen.getByRole("button", { name: CATALOG[lang]["uninstall.confirm"] }));
      await user.click(screen.getByRole("button", { name: CATALOG[lang]["uninstall.continue"] }));

      expect(
        await screen.findByText(CATALOG[lang]["uninstall.partial.title"]),
      ).toBeInTheDocument();
      expect(
        screen.getByText(CATALOG[lang]["uninstall.partial.summary"], { selector: ".desc" }),
      ).toBeInTheDocument();
      expect(
        screen.getByText("removed with recovery actions", { selector: ".errdetails" }),
      ).toBeInTheDocument();
      const recoveryKeys = [
        "uninstall.partial.retryCleanup",
        "uninstall.partial.retryProvenance",
        "uninstall.partial.retryPurge",
        "uninstall.partial.retryRecord",
      ] as const;
      for (const key of recoveryKeys) {
        expect(screen.getByRole("button", { name: CATALOG[lang][key] })).toBeInTheDocument();
      }

      await user.click(
        screen.getByRole("button", {
          name: CATALOG[lang]["uninstall.partial.retryProvenance"],
        }),
      );
      await waitFor(() =>
        expect(retryAncillary).toHaveBeenCalledWith(
          expect.objectContaining({ actions: ["clear_provenance"], purgeUserData: false }),
        ),
      );
      expect(
        await screen.findByText(CATALOG[lang]["uninstall.partial.retryPending"], {
          selector: ".desc",
        }),
      ).toBeInTheDocument();
      expect(
        screen.queryByRole("button", {
          name: CATALOG[lang]["uninstall.partial.retryProvenance"],
        }),
      ).not.toBeInTheDocument();
      for (const key of [
        "uninstall.partial.retryCleanup",
        "uninstall.partial.retryPurge",
        "uninstall.partial.retryRecord",
      ] as const) {
        expect(screen.getByRole("button", { name: CATALOG[lang][key] })).toBeInTheDocument();
      }
      expect(screen.getByText("cleanup ok", { selector: ".errdetails" })).toBeInTheDocument();
      expect(document.documentElement.dir).toBe(lang === "ar" ? "rtl" : "ltr");
    },
  );
});
