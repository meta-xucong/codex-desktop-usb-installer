import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";

import {
  errorCode,
  managerApi,
  type CodexPayloadCheck,
  type UpdateInfo,
} from "../../services/managerApi";
import type {
  CapabilityPackInstallOptions,
  CapabilityPackReport,
  HistoricalInstallSelection,
  OfflinePayloadReport,
  OfflineStageReport,
  UsbGuardStatus,
  WinInstallStatus,
} from "../../shared/types";
import {
  contextualFailure,
  resolveFailure,
  type FailureSurface,
} from "../errorCopy";
import { Icon, CodexGlyph } from "../icons";
import { useI18n, type TFn } from "../i18n";
import { FailureBanner, Ring, StatusBanner, Tooltip, TopBar } from "../components";
import { mib } from "../format";
import { Sheet } from "../Sheet";
import { OfflineComponentsSheet } from "./OfflineComponentsSheet";
import { SkillRepoUpdateCard } from "./SkillRepoUpdateCard";

type OfflinePhase = "loading" | "ready" | "installed" | "staging" | "installing" | "error";

const USB_GUARD_STARTUP_FAST_POLL_LIMIT = 60;
const USB_GUARD_STARTUP_FAST_POLL_MS = 250;
const USB_GUARD_NORMAL_POLL_MS = 1_000;

const DEFAULT_CAPABILITY_PACK_OPTIONS: CapabilityPackInstallOptions = {
  enableFeishu: false,
  persistConnections: false,
  feishuAppId: "",
  feishuAppSecret: "",
  feishuUserAccessToken: "",
  feishuDomain: "https://open.feishu.cn",
  feishuAuthMode: "tenant",
  enabledApiServices: [],
  dashscopeApiKey: "",
  dashscopeBaseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
  dashscopeModel: "",
  image2ApiKey: "",
  image2BaseUrl: "",
  image2Model: "",
  image2AgentId: "",
  seedanceApiKey: "",
  seedanceBaseUrl: "",
  seedanceModel: "",
  seedanceAgentId: "",
  seedancePrechargePoints: "",
  seedanceOfficialLinkTtlHours: "",
  seedanceRefundRule: "",
};

function versionParts(value: string): number[] {
  return value
    .split(/[.-]/)
    .map((part) => Number.parseInt(part, 10))
    .filter((part) => Number.isFinite(part));
}

function compareVersions(left: string, right: string): number {
  const a = versionParts(left);
  const b = versionParts(right);
  const length = Math.max(a.length, b.length);
  for (let index = 0; index < length; index += 1) {
    const delta = (a[index] ?? 0) - (b[index] ?? 0);
    if (delta !== 0) return delta;
  }
  return 0;
}

function selectionFromStage(stage: OfflineStageReport): HistoricalInstallSelection {
  const payload = stage.payload;
  return {
    releaseTag: payload.releaseTag,
    version: payload.codexDesktopVersion,
    assetName: payload.fileName,
    architecture: payload.architecture === "arm64" ? "arm64" : "x64",
    format: "msix",
    packageVersion: payload.packageVersion,
    localPath: stage.stagedPath,
    localFileName: payload.fileName,
  };
}

function expectedFromStatus(status: WinInstallStatus | null) {
  // Preserve the fresh snapshot for the backend's TOCTOU check. The USB path
  // deliberately rejects an existing external or non-current installation;
  // this is not an adoption/migration expectation.
  const installed = status?.installed ?? null;
  return {
    currentPath: installed?.path ?? null,
    currentVersion: installed?.version ?? null,
    currentSource: installed?.source ?? null,
  };
}

function canUpdateCodexPayload(guard: UsbGuardStatus | null): boolean {
  return Boolean(
    guard?.status === "authorized" &&
      guard.watchdog === "running" &&
      guard.signed &&
      guard.codexAllowed !== false &&
      guard.removable &&
      guard.bundlePath,
  );
}

function codexReadyOnAuthorizedUsb(guard: UsbGuardStatus | null): boolean {
  return Boolean(
    guard?.status === "authorized" &&
      guard.watchdog === "running" &&
      guard.signed &&
      guard.codexAllowed === true,
  );
}

function codexActionsAllowedOnCurrentRuntime(
  guard: UsbGuardStatus | null,
  guardQueryFailed = false,
): boolean {
  // A null guard is the ordinary installed-manager path. Once the USB guard
  // is present, every Codex action must wait for live signed authorization.
  return !guardQueryFailed && (guard === null || codexReadyOnAuthorizedUsb(guard));
}

function programIsOnCurrentUsb(
  status: WinInstallStatus | null,
  guard: UsbGuardStatus | null,
): boolean {
  const installPath = status?.installed?.path;
  const bundlePath = guard?.bundlePath;
  if (!installPath || !bundlePath || status?.installed?.source !== "portable") return false;
  const normalize = (value: string) => value.replace(/\//g, "\\").replace(/[\\]+$/, "").toLowerCase();
  return normalize(installPath) === `${normalize(bundlePath)}\\apps\\codex`;
}

function usbInstallBlockedByExistingCodex(
  status: WinInstallStatus | null,
  guard: UsbGuardStatus | null,
): boolean {
  if (!status) return false;
  if (status.status === "external") return true;
  return Boolean(status.installed && !programIsOnCurrentUsb(status, guard));
}

function configNeedsAttention(report: CapabilityPackReport): boolean {
  return (
    report.coreReady &&
    report.runtimeReady &&
    !report.success &&
    ["partial", "failed", "conflict"].includes(report.configStatus)
  );
}

function usbGuardLabel(status: string, t: TFn): string {
  switch (status) {
    case "authorized": return t("offline.usbGuard.authorized");
    case "grace": return t("offline.usbGuard.grace");
    case "missing": return t("offline.usbGuard.missing");
    case "mismatch": return t("offline.usbGuard.mismatch");
    case "unsupported": return t("offline.usbGuard.unsupported");
    case "starting": return t("offline.usbGuard.starting");
    case "disabled": return t("offline.usbGuard.disabled");
    case "stopped": return t("offline.usbGuard.stopped");
    default: return status;
  }
}

function usbGuardStartupPending(guard: UsbGuardStatus | null): boolean {
  return guard?.status === "starting";
}

/**
 * Minimal USB portable Windows screen. The shipped payload remains the
 * default/offline source, while a signed USB bundle can explicitly check for a
 * newer payload and then reuse the same guarded install path to deploy it.
 */
export function OfflineHome({
  onOpenSettings,
  onOpenUninstall,
}: {
  onOpenSettings: () => void;
  onOpenUninstall?: () => void;
}) {
  const { t } = useI18n();
  const [phase, setPhase] = useState<OfflinePhase>("loading");
  const [payload, setPayload] = useState<OfflinePayloadReport | null>(null);
  const [payloadLoading, setPayloadLoading] = useState(false);
  const [status, setStatus] = useState<WinInstallStatus | null>(null);
  const [usbGuard, setUsbGuard] = useState<UsbGuardStatus | null>(null);
  const [usbGuardQueryFailed, setUsbGuardQueryFailed] = useState(false);
  const [failure, setFailure] = useState<FailureSurface | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [installationDetail, setInstallationDetail] = useState<string | null>(null);
  const [componentsOpen, setComponentsOpen] = useState(false);
  const [pendingCodexUpdate, setPendingCodexUpdate] = useState<UpdateInfo | null>(null);
  const [codexUpdateConfirmOpen, setCodexUpdateConfirmOpen] = useState(false);
  const [busyAction, setBusyAction] = useState<
    "install" | "update" | "launch" | "ccswitch" | "logs" | null
  >(null);
  const updateTitleId = useId();
  const updateBodyId = useId();
  const loadGeneration = useRef(0);
  const usbGuardStartupPolls = useRef(0);

  const load = useCallback(async () => {
    const generation = loadGeneration.current + 1;
    loadGeneration.current = generation;
    setPhase("loading");
    setPayload(null);
    setPayloadLoading(true);
    setFailure(null);
    setNotice(null);
    setInstallationDetail(null);
    setUsbGuardQueryFailed(false);
    try {
      // The payload probe hashes the complete MSIX and may also inspect its
      // Authenticode chain. Do not make that expensive local read the first
      // visible response to a double-click; render the local status shell as
      // soon as the cheap manager/guard probes finish, then fill the payload
      // details in the background.
      const [nextStatus, nextUsbGuardResult] = await Promise.all([
        managerApi.winStatus(),
        managerApi.usbGuardStatus()
          .then((value) => ({ value, failed: false }))
          .catch(() => ({ value: null, failed: true })),
      ]);
      if (loadGeneration.current !== generation) return;
      const nextUsbGuard = nextUsbGuardResult.value;
      setStatus(nextStatus);
      setUsbGuard(nextUsbGuard);
      setUsbGuardQueryFailed(nextUsbGuardResult.failed);
      if (nextStatus.recoveryPending) {
        setNotice(t("offline.recoveryPending"));
      } else if (usbInstallBlockedByExistingCodex(nextStatus, nextUsbGuard)) {
        setNotice(
          nextStatus.status === "external"
            ? t("offline.external")
            : t("offline.existingOutsideUsb"),
        );
      }
      // Let the user see the shell and the current guard state while the
      // immutable package probe continues. Actions remain disabled until the
      // payload is available.
      setPhase("ready");
      const nextPayload = await managerApi.offlinePayloadProbe();
      if (loadGeneration.current !== generation) return;

      // The initial guard snapshot can legitimately be `starting`: the
      // complete bundle/AuthentiCode walk runs in a background startup worker.
      // Refresh once after the expensive payload probe so the action gates use
      // a live authorization result instead of the stale startup snapshot.
      let settledUsbGuard = nextUsbGuard;
      if (!nextUsbGuardResult.failed) {
        try {
          settledUsbGuard = await managerApi.usbGuardStatus();
          if (loadGeneration.current !== generation) return;
          setUsbGuard(settledUsbGuard);
          setUsbGuardQueryFailed(false);
        } catch {
          if (loadGeneration.current !== generation) return;
          setUsbGuardQueryFailed(true);
        }
      }
      setPayload(nextPayload);
      setPayloadLoading(false);
      if (
        nextStatus.installed &&
        compareVersions(nextStatus.installed.version, nextPayload.codexDesktopVersion) > 0
      ) {
        setNotice(t("offline.newerVersion"));
      }
      const installed = nextStatus.installed;
      const sameVersion = Boolean(
        installed &&
          compareVersions(installed.version, nextPayload.codexDesktopVersion) === 0,
      );
      if (
        codexReadyOnAuthorizedUsb(settledUsbGuard) &&
        installed &&
          nextStatus.status === "managed" &&
        !nextStatus.recoveryPending &&
        sameVersion &&
        programIsOnCurrentUsb(nextStatus, settledUsbGuard)
      ) {
        setPhase("installed");
      } else {
        setPhase("ready");
      }
    } catch (cause) {
      if (loadGeneration.current !== generation) return;
      setPayload(null);
      setPayloadLoading(false);
      setStatus(null);
      setPhase("error");
      setFailure(resolveFailure(cause, t));
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  const usbGuardStatus = usbGuard?.status;

  // The helper writes a fresh status roughly once per second. While the
  // startup integrity worker is active, use a short-lived fast poll so the
  // controls unlock immediately after authorization becomes live. The normal
  // one-second cadence remains the long-lived fallback and keeps the visible
  // countdown useful without adding a WebSocket or privileged service.
  useEffect(() => {
    // The modal owns a long, scrollable form. The watchdog countdown is not
    // useful while it is open, and updating it would needlessly re-render the
    // whole page behind the modal once per second.
    if (componentsOpen || !usbGuardStatus) return;
    let active = true;
    let timer: number | null = null;
    if (usbGuardStatus !== "starting") {
      usbGuardStartupPolls.current = 0;
    }
    const poll = () => {
      void managerApi
        .usbGuardStatus()
        .then((next) => {
          if (!active) return;
          setUsbGuard(next);
          setUsbGuardQueryFailed(false);
          if (usbGuardStartupPending(next)) {
            usbGuardStartupPolls.current += 1;
          } else {
            usbGuardStartupPolls.current = 0;
          }
          const fastPoll =
            usbGuardStartupPending(next) &&
            usbGuardStartupPolls.current <= USB_GUARD_STARTUP_FAST_POLL_LIMIT;
          timer = window.setTimeout(
            poll,
            fastPoll ? USB_GUARD_STARTUP_FAST_POLL_MS : USB_GUARD_NORMAL_POLL_MS,
          );
        })
        .catch(() => {
          if (!active) return;
          setUsbGuardQueryFailed(true);
          timer = window.setTimeout(poll, USB_GUARD_NORMAL_POLL_MS);
        });
    };
    timer = window.setTimeout(
      poll,
      usbGuardStatus === "starting" ? 0 : USB_GUARD_NORMAL_POLL_MS,
    );
    return () => {
      active = false;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [componentsOpen, usbGuardStatus]);

  const newerInstalled = useMemo(() => {
    if (!payload || !status?.installed) return false;
    return compareVersions(status.installed.version, payload.codexDesktopVersion) > 0;
  }, [payload, status]);

  const performInstall = useCallback(async (sourcePayload: OfflinePayloadReport) => {
    setPayload(sourcePayload);
    setPhase("staging");
    setFailure(null);
    setNotice(null);
    setInstallationDetail(null);
    let stage: OfflineStageReport | null = null;
    let pendingRecovery = false;
    try {
      stage = await managerApi.offlinePayloadStage();
      setPhase("installing");
      const current = await managerApi.winStatus();
      const currentGuardResult = await managerApi.usbGuardStatus()
        .then((value) => ({ value, failed: false }))
        .catch(() => ({ value: null, failed: true }));
      const currentGuard = currentGuardResult.value;
      if (!codexActionsAllowedOnCurrentRuntime(currentGuard, currentGuardResult.failed)) {
        if (currentGuardResult.failed) setUsbGuardQueryFailed(true);
        throw new Error(t("offline.updateRequiresSigned"));
      }
      if (usbInstallBlockedByExistingCodex(current, currentGuard)) {
        throw new Error(
          current.status === "external"
            ? t("offline.external")
            : t("offline.existingOutsideUsb"),
        );
      }
      if (
        current.installed &&
        compareVersions(current.installed.version, stage.payload.codexDesktopVersion) > 0
      ) {
        throw new Error(t("offline.newerVersion"));
      }
      const result = await managerApi.winInstallHistoricalRelease(
        selectionFromStage(stage),
        // Keep the shipped desktop build fixed for the USB edition. The backend
        // extracts the verified program tree below this bundle while the normal
        // user profile on the local system keeps mutable data.
        true,
        expectedFromStatus(current),
        undefined,
        undefined,
        true,
        true,
      );
      if (!result.success) {
        throw new Error(result.message || t("error.install"));
      }
      // Preserve the recovery/backup locations even when the subsequent
      // configurator succeeds. A CCSwitch notice must not hide this evidence.
      setInstallationDetail(result.message);
      const installed = await managerApi.winStatus();
      if (installed.recoveryPending) {
        pendingRecovery = true;
        setStatus(installed);
        throw new Error(t("offline.recoveryPending"));
      }
      if (
        installed.status !== "managed" ||
        !installed.installed ||
        compareVersions(installed.installed.version, stage.payload.codexDesktopVersion) !== 0
      ) {
        throw new Error(t("offline.verifyAfterInstall"));
      }
      const verifiedUsbGuardResult = await managerApi.usbGuardStatus()
        .then((value) => ({ value, failed: false }))
        .catch(() => ({ value: null, failed: true }));
      const verifiedUsbGuard = verifiedUsbGuardResult.value;
      if (
        !codexActionsAllowedOnCurrentRuntime(verifiedUsbGuard, verifiedUsbGuardResult.failed) ||
        !programIsOnCurrentUsb(installed, verifiedUsbGuard)
      ) {
        if (verifiedUsbGuardResult.failed) setUsbGuardQueryFailed(true);
        const installPath = installed.installed.path || "未知";
        const bundlePath = verifiedUsbGuard?.bundlePath || "未知";
        throw new Error(`无法确认 Codex 程序位于当前 U 盘套件（程序 ${installPath}，套件 ${bundlePath}），已保留暂存文件，请查看日志后重试`);
      }
      setUsbGuard(verifiedUsbGuard);
      setPayload(stage.payload);
      // Keep diagnostic staging until all installed-state/placement checks
      // pass. A verification failure must not claim a deleted file was kept.
      await managerApi.offlinePayloadCleanup(stage.stagedPath).catch(() => undefined);
      // Codex is now confirmed on disk. Install the complete local capability
      // pack first, then hand off to CCSwitch. Either auxiliary component is
      // best-effort: a failure never rolls back a verified Codex installation
      // and remains retryable from the quiet Components entry below.
      let capabilityPackReport: CapabilityPackReport | null = null;
      try {
        capabilityPackReport = await managerApi.installCapabilityPack({
          ...DEFAULT_CAPABILITY_PACK_OPTIONS,
        });
      } catch (capabilityError) {
        console.warn("[capability-pack] automatic install failed", capabilityError);
      }
      let ccswitchReport = null;
      try {
        ccswitchReport = await managerApi.installAndLaunchCcswitch();
      } catch (ccswitchError) {
        console.warn("[ccswitch] automatic launch failed", ccswitchError);
      }
      setStatus(installed);
      setPhase("installed");
      const notices: string[] = [];
      if (capabilityPackReport?.coreReady && capabilityPackReport.success) {
        notices.push(
          t("offline.capabilityInstalled", {
            skills: capabilityPackReport.skills,
            agents: capabilityPackReport.agents,
          }),
        );
      } else if (capabilityPackReport?.coreReady && configNeedsAttention(capabilityPackReport)) {
        notices.push(
          t("offline.capabilityConfigConflict", {
            skills: capabilityPackReport.skills,
            agents: capabilityPackReport.agents,
          }),
        );
      } else if (capabilityPackReport?.coreReady) {
        notices.push(
          t("offline.capabilityPartial", {
            skills: capabilityPackReport.skills,
            agents: capabilityPackReport.agents,
          }),
        );
      } else {
        notices.push(t("offline.capabilityFailed"));
      }
      if (ccswitchReport?.launched) {
        notices.push(t("offline.ccswitchLaunched"));
      } else if (ccswitchReport && !ccswitchReport.available) {
        notices.push(t("offline.ccswitchUnavailable"));
      } else {
        notices.push(t("offline.ccswitchFailed"));
      }
      setNotice(notices.join(" "));
      return true;
    } catch (cause) {
      setPhase("ready");
      setFailure(resolveFailure(cause, t));
      if (pendingRecovery) {
        setNotice(t("offline.recoveryPending"));
      } else if (stage) {
        setNotice("暂存文件已保留，便于查看日志和重试。");
      }
      return false;
    }
  }, [t]);

  const install = useCallback(async () => {
    if (!payload || busyAction) return;
    if (!codexActionsAllowedOnCurrentRuntime(usbGuard, usbGuardQueryFailed)) {
      setNotice(t("offline.updateRequiresSigned"));
      return;
    }
    if (newerInstalled) {
      setNotice(t("offline.newerVersion"));
      return;
    }
    setBusyAction("install");
    try {
      await performInstall(payload);
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, newerInstalled, payload, performInstall, t, usbGuard, usbGuardQueryFailed]);

  const codexPayloadAvailable = !usbGuardQueryFailed && canUpdateCodexPayload(usbGuard);

  const checkCodexPayload = useCallback(async () => {
    if (busyAction) return;
    if (!codexPayloadAvailable) {
      setNotice(t("offline.updateRequiresSigned"));
      return;
    }
    setBusyAction("update");
    setFailure(null);
    setNotice(null);
    setPendingCodexUpdate(null);
    try {
      const result: CodexPayloadCheck = await managerApi.checkCodexPayloadUpdate();
      if (result.kind === "available") {
        setPendingCodexUpdate(result.info);
        setCodexUpdateConfirmOpen(true);
      } else if (result.kind === "none") {
        setNotice(t("offline.updateUpToDate"));
      } else if (result.kind === "development") {
        setNotice(t("about.mgrDev"));
      } else {
        setFailure(
          contextualFailure(
            result.message || t("offline.updateUnavailable"),
            t,
            t("offline.updateUnavailable"),
            "network",
          ),
        );
      }
    } catch (cause) {
      setFailure(resolveFailure(cause, t));
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, codexPayloadAvailable, t]);

  const applyCodexPayload = useCallback(async () => {
    if (!pendingCodexUpdate || busyAction) return;
    setBusyAction("update");
    setFailure(null);
    setNotice(null);
    try {
      await managerApi.applyCodexPayloadUpdate(pendingCodexUpdate);
      const nextPayload = await managerApi.offlinePayloadProbe();
      setPayload(nextPayload);
      setPendingCodexUpdate(null);
      setCodexUpdateConfirmOpen(false);
      const installed = await performInstall(nextPayload);
      if (installed) {
        setNotice(t("offline.updateApplied"));
      } else {
        setNotice(t("offline.updatePayloadAppliedRetry"));
      }
    } catch (cause) {
      if (errorCode(cause) === "stale_expectation") {
        setPendingCodexUpdate(null);
        setCodexUpdateConfirmOpen(false);
        setNotice(t("offline.updateStale"));
      } else {
        setFailure(resolveFailure(cause, t));
        setCodexUpdateConfirmOpen(false);
      }
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, pendingCodexUpdate, performInstall, t]);

  const launchCcswitch = useCallback(async () => {
    if (busyAction) return;
    setBusyAction("ccswitch");
    setFailure(null);
    try {
      const report = await managerApi.installAndLaunchCcswitch();
      if (report.launched) {
        setNotice(t("offline.ccswitchLaunched"));
      } else if (!report.available) {
        setNotice(t("offline.ccswitchUnavailable"));
      } else {
        setNotice(report.message || t("offline.ccswitchFailed"));
      }
    } catch (cause) {
      setFailure(resolveFailure(cause, t));
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, t]);

  const launch = useCallback(async () => {
    if (busyAction) return;
    setBusyAction("launch");
    setFailure(null);
    try {
      await managerApi.winLaunch();
    } catch (cause) {
      setFailure(resolveFailure(cause, t));
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, t]);

  const openLogs = useCallback(async () => {
    if (busyAction) return;
    setBusyAction("logs");
    try {
      await managerApi.openLogsDir();
    } catch (cause) {
      setFailure(resolveFailure(cause, t));
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, t]);

  const installed = status?.installed;
  const codexReady = !usbGuardQueryFailed && codexReadyOnAuthorizedUsb(usbGuard);
  const codexActionsAllowed = codexActionsAllowedOnCurrentRuntime(usbGuard, usbGuardQueryFailed);
  const usbGuardIsStarting = usbGuardStartupPending(usbGuard);
  const showInstalled = phase === "installed" && codexReady;
  const displayInstalled = codexReady ? installed : null;
  const disabled =
    busyAction !== null ||
    payloadLoading ||
    phase === "loading" ||
    phase === "staging" ||
    phase === "installing";

  return (
    <div className="pop">
      <TopBar>
        <button
          className="iconbtn"
          data-page-focus
          title={t("nav.settings")}
          onClick={onOpenSettings}
        >
          <Icon name="gear" />
        </button>
      </TopBar>
      <div className="scroll">
        {failure ? <FailureBanner failure={failure} /> : null}
        {notice ? <StatusBanner tone="info">{notice}</StatusBanner> : null}
        {usbGuardIsStarting ? (
          <StatusBanner tone="info">{t("offline.usbGuard.startingHint")}</StatusBanner>
        ) : null}
        {installationDetail ? (
          <StatusBanner tone="info">
            <details>
              <summary>{t("offline.installDetails")}</summary>
              <div style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{installationDetail}</div>
            </details>
          </StatusBanner>
        ) : null}
        {usbGuard ? (
          <section
            className={`usb-guard-status usb-guard-${usbGuard.status}`}
            data-testid="usb-guard-status"
          >
            <div className="usb-guard-status-head">
              <span className="usb-guard-status-icon"><Icon name={usbGuard.status === "authorized" ? "shield" : "alert"} /></span>
              <span className="usb-guard-status-title">{usbGuardLabel(usbGuard.status, t)}</span>
              {usbGuard.countdownSeconds !== null ? (
                <span className="usb-guard-countdown">{usbGuard.countdownSeconds}s</span>
              ) : null}
              <Tooltip
                content={[
                  usbGuard.message,
                  usbGuard.codexBlockReason,
                  usbGuard.driveRoot
                    ? `${usbGuard.driveRoot}${usbGuard.bundleRelativePath ? ` · ${usbGuard.bundleRelativePath}` : ""}`
                    : null,
                  !usbGuard.signed && usbGuard.status === "authorized"
                    ? t("offline.usbGuard.unsigned")
                    : null,
                ]
                  .filter(Boolean)
                  .join("\n")}
              >
                <span className="tooltip-icon" aria-label={t("themes.details")}>
                  <Icon name="info" />
                </span>
              </Tooltip>
            </div>
          </section>
        ) : null}
        <section className="hero" data-testid="offline-hero">
          {phase === "loading" || payloadLoading ? (
            <>
              <Ring icon="loader" spin />
              <div className="headline">{t("offline.checking")}</div>
            </>
          ) : phase === "error" ? (
            <>
              <Ring icon="alert" variant="danger" />
              <div className="headline">{t("offline.checkFailed")}</div>
            </>
          ) : showInstalled ? (
            <>
              <Ring icon="check" variant="success" />
              <div className="headline">{t("offline.installed")}</div>
              <div className="microcue"><Icon name="shield" />{t("offline.hashVerified")}</div>
            </>
          ) : usbInstallBlockedByExistingCodex(status, usbGuard) ? (
            <>
              <Ring icon="shield" variant="amber" />
              <div className="headline">{t("home.external.title")}</div>
              <div className="prov"><span className="dot external" />{t("prov.external")}</div>
              <Tooltip
                content={
                  status?.status === "external"
                    ? t("offline.external")
                    : t("offline.existingOutsideUsb")
                }
              >
                <span className="tooltip-icon hero-info" aria-label={t("themes.details")}>
                  <Icon name="info" />
                </span>
              </Tooltip>
            </>
          ) : (
            <>
              <Ring icon="download" className={phase === "staging" || phase === "installing" ? "spin" : ""} />
              <div className="headline">
                {phase === "staging" || phase === "installing" ? t("offline.installing") : t("offline.ready")}
              </div>
              <Tooltip content={t("offline.description")}>
                <span className="tooltip-icon hero-info" aria-label={t("themes.details")}>
                  <Icon name="info" />
                </span>
              </Tooltip>
            </>
          )}
        </section>

        {payload ? (
          <div className="list meta">
            <div className="row">
              <span className="rtext"><span className="rtitle">{t("offline.version")}</span></span>
              <span className="rval version">{payload.codexDesktopVersion}</span>
            </div>
            <div className="row">
              <span className="rtext"><span className="rtitle">{t("offline.package")}</span></span>
              <span className="rval path" title={payload.fileName}>{payload.fileName}</span>
            </div>
            <div className="row">
              <span className="rtext"><span className="rtitle">{t("offline.arch")}</span></span>
              <span className="rval">{payload.architecture}</span>
            </div>
            <div className="row">
              <span className="rtext"><span className="rtitle">{t("offline.hash")}</span></span>
              <span className="rval path" title={payload.sha256}>{payload.sha256.slice(0, 12)}…</span>
            </div>
            <div className="row">
              <span className="rtext"><span className="rtitle">{t("offline.signature")}</span></span>
              <span className="rval">{payload.authenticode?.trusted ? t("offline.hashVerified") : t("offline.signaturePending")}</span>
            </div>
            <div className="row">
              <span className="rtext"><span className="rtitle">{t("offline.size")}</span></span>
              <span className="rval">{mib(payload.size)}</span>
            </div>
          </div>
        ) : null}

        <div className="actions">
          {phase === "error" ? (
            <button className="btn primary big" onClick={() => void load()} disabled={disabled}>
              <Icon name="refresh" />{t("offline.recheck")}
            </button>
          ) : showInstalled ? (
            <>
              <button className="btn primary big" onClick={() => void launch()} disabled={disabled}>
                {busyAction === "launch" ? <Icon name="loader" className="spinicon" /> : <CodexGlyph />}
                {busyAction === "launch" ? t("offline.launching") : t("offline.launch")}
              </button>
              <button className="btn ghost" onClick={() => void launchCcswitch()} disabled={disabled}>
                {busyAction === "ccswitch" ? <Icon name="loader" className="spinicon" /> : <Icon name="gear" />}
                {busyAction === "ccswitch" ? t("offline.ccswitchLaunching") : t("offline.ccswitchLaunch")}
              </button>
              <button className="btn ghost" onClick={() => void load()} disabled={disabled}><Icon name="refresh" />{t("offline.recheck")}</button>
              <button className="btn ghost" onClick={() => void install()} disabled={disabled}>
                <Icon name="refresh" />{t("offline.repair")}
              </button>
              <Tooltip content={t("offline.repairHint")}>
                <span className="tooltip-icon action-info" aria-label={t("themes.details")}>
                  <Icon name="info" />
                </span>
              </Tooltip>
              {onOpenUninstall ? (
                <button className="btn ghost danger" onClick={onOpenUninstall} disabled={disabled}>
                  <Icon name="trash" />{t("offline.uninstallBundle")}
                </button>
              ) : null}
            </>
          ) : usbInstallBlockedByExistingCodex(status, usbGuard) ? (
            <>
              <div className="status-copy">
                {status?.status === "external"
                  ? t("offline.external")
                  : t("offline.existingOutsideUsb")}
              </div>
              <button className="btn ghost" onClick={() => void load()} disabled={disabled}><Icon name="refresh" />{t("offline.recheck")}</button>
            </>
          ) : (
            <>
              <button className="btn primary big" onClick={() => void install()} disabled={disabled || newerInstalled || !codexActionsAllowed}>
                {busyAction === "install" ? <Icon name="loader" className="spinicon" /> : <Icon name="download" />}
                {busyAction === "install" ? t("offline.installing") : status?.recoveryPending ? t("offline.resumeInstall") : t("offline.install")}
              </button>
              <button className="btn ghost" onClick={() => void load()} disabled={disabled}><Icon name="refresh" />{t("offline.recheck")}</button>
            </>
          )}
          {phase !== "error" ? (
            <span className="update-entry-wrap">
              <Tooltip
                content={
                  codexPayloadAvailable
                    ? t("offline.updateHint")
                    : t("offline.updateRequiresSigned")
                }
              >
                <button
                  className="btn ghost update-entry"
                  onClick={() => void checkCodexPayload()}
                  disabled={disabled || !codexPayloadAvailable}
                  data-testid="codex-payload-update"
                >
                  {busyAction === "update" ? (
                    <Icon name="loader" className="spinicon" />
                  ) : (
                    <Icon name="download" />
                  )}
                  {busyAction === "update"
                    ? t("offline.updateChecking")
                    : t("offline.updateEntry")}
                </button>
              </Tooltip>
            </span>
          ) : null}
          {!showInstalled && onOpenUninstall ? (
            <button className="btn ghost danger" onClick={onOpenUninstall} disabled={disabled}>
              <Icon name="trash" />{t("offline.uninstallBundle")}
            </button>
          ) : null}
              <button className="btn ghost" onClick={() => void openLogs()} disabled={disabled}>
            {busyAction === "logs" ? <Icon name="loader" className="spinicon" /> : <Icon name="folder" />}
            {t("offline.openLogs")}
          </button>
          <button
            className="linkbtn component-entry"
            onClick={() => setComponentsOpen(true)}
            disabled={disabled}
          >
            <Icon name="sliders" />{t("offline.components.entry")}
          </button>
        </div>

        <SkillRepoUpdateCard />

        {displayInstalled ? (
          <Tooltip
            content={[
              `${t("home.installLocation")}: ${displayInstalled.path}`,
              displayInstalled.packageVolume
                ? `${t("offline.packageVolume")}: ${displayInstalled.packageVolume}`
                : null,
              t("offline.keepUsbConnected"),
            ]
              .filter(Boolean)
              .join("\n")}
          >
            <span
              className="foot foot-evidence"
              aria-label={t("offline.installLocationSummary")}
            >
              <Icon name="info" />
              <span>{t("offline.installLocationSummary")}</span>
            </span>
          </Tooltip>
        ) : null}
      </div>
      <Sheet
        open={codexUpdateConfirmOpen && Boolean(pendingCodexUpdate)}
        onDismiss={() => {
          if (busyAction === "update") return;
          setCodexUpdateConfirmOpen(false);
          setPendingCodexUpdate(null);
        }}
        dismissable={busyAction !== "update"}
        labelledBy={updateTitleId}
        describedBy={updateBodyId}
        initialFocus="dismiss"
      >
        <Ring icon="download" />
        <h3 id={updateTitleId}>
          {pendingCodexUpdate
            ? t("offline.updateConfirmTitle", {
                version: pendingCodexUpdate.latestVersion,
              })
            : ""}
        </h3>
        <p id={updateBodyId}>
          {pendingCodexUpdate
            ? t("offline.updateConfirmBody", {
                current: pendingCodexUpdate.currentVersion,
                latest: pendingCodexUpdate.latestVersion,
              })
            : ""}
        </p>
        <div className="row2 sheet-actions">
          <button
            className="btn ghost"
            onClick={() => {
              setCodexUpdateConfirmOpen(false);
              setPendingCodexUpdate(null);
            }}
            disabled={busyAction === "update"}
          >
            {t("confirm.cancel")}
          </button>
          <button className="btn primary" onClick={() => void applyCodexPayload()} disabled={busyAction === "update"}>
            {busyAction === "update" ? t("offline.updateInstalling") : t("confirm.ok")}
          </button>
        </div>
      </Sheet>
      <OfflineComponentsSheet
        open={componentsOpen}
        onDismiss={() => setComponentsOpen(false)}
      />
    </div>
  );
}
