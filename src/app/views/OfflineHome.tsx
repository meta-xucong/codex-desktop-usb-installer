import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  errorCode,
  managerApi,
  type CodexPayloadCheck,
  type UpdateInfo,
} from "../../services/managerApi";
import type {
  CapabilityPackInstallOptions,
  CapabilityPackReport,
  DownloadProgress,
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
import { AccountCard } from "./AccountCard";
import { SkillRepoUpdateCard } from "./SkillRepoUpdateCard";
import { ProgressScreen } from "./ProgressScreen";
import { useDownloadProgress } from "./useDownloadProgress";

type OfflinePhase = "loading" | "ready" | "installed" | "staging" | "installing" | "error";

type PayloadUpdateProgress = {
  phase: "preparing" | "download" | "done";
  bytesDone?: number;
  bytesTotal?: number;
};

type NativeModelNotice = {
  message: string;
  tone: "err" | "info" | "ok" | "warn";
};

const USB_GUARD_STARTUP_FAST_POLL_LIMIT = 60;
const USB_GUARD_STARTUP_FAST_POLL_MS = 250;
const USB_GUARD_NORMAL_POLL_MS = 1_000;
// The native command performs a bounded three-attempt upstream refresh. Keep
// the renderer watchdog longer than that budget so a slow 404token response
// is reported with the backend's upstream diagnosis instead of being turned
// into a misleading local timeout.
// Three 20-second upstream attempts plus the 500ms/1s backoff need 61.5s in
// the worst case. Keep a margin so the renderer does not turn the backend's
// explicit 404token-upstream diagnosis into a generic local timeout.
const NATIVE_MODEL_REFRESH_TIMEOUT_MS = 75_000;
const NATIVE_MODEL_REFRESH_START_DELAY_MS = 250;

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
    // A USB payload's manifest tag describes provenance and may be rewritten
    // by the hot-update channel. The local MSIX installer uses the canonical
    // local-signed namespace, so never feed an upstream tag into its stale
    // expectation check.
    releaseTag: `local-signed-${payload.codexDesktopVersion}`,
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

function ccswitchUserMessage(message: string | null | undefined, fallback: string): string {
  const value = message?.trim();
  // The native command may return a short machine/test status (for example
  // "launched") or an actionable Chinese diagnostic. Keep the translated
  // user-facing status for the former while preserving useful diagnostics for
  // the latter.
  if (!value || value.length < 4 || !/[\u4e00-\u9fff]|Codex\s*Switch|CCSwitch|404token/i.test(value)) {
    return fallback;
  }
  return value;
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
  const normalize = (value: string) => value
    .trim()
    .replace(/^\\\\[?.]\\/i, "")
    .replace(/\//g, "\\")
    .replace(/[\\]+$/, "")
    .toLowerCase();
  return normalize(installPath) === `${normalize(bundlePath)}\\apps\\codex`;
}

function usbInstallBlockedByExistingCodex(
  status: WinInstallStatus | null,
  guard: UsbGuardStatus | null,
): boolean {
  if (!status) return false;
  // On a fresh launch win_status can be read before the USB watchdog has
  // re-established its lease.  A stale `external` classification must not
  // hide a portable tree that is visibly inside the current verified bundle.
  if (programIsOnCurrentUsb(status, guard)) return false;
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
  const { t, lang } = useI18n();
  const [phase, setPhase] = useState<OfflinePhase>("loading");
  const [payload, setPayload] = useState<OfflinePayloadReport | null>(null);
  const [payloadLoading, setPayloadLoading] = useState(false);
  const [status, setStatus] = useState<WinInstallStatus | null>(null);
  const [usbGuard, setUsbGuard] = useState<UsbGuardStatus | null>(null);
  const [usbGuardQueryFailed, setUsbGuardQueryFailed] = useState(false);
  const [failure, setFailure] = useState<FailureSurface | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [nativeModelNotice, setNativeModelNotice] = useState<NativeModelNotice | null>(null);
  const [installationDetail, setInstallationDetail] = useState<string | null>(null);
  const [componentsOpen, setComponentsOpen] = useState(false);
  const [pendingCodexUpdate, setPendingCodexUpdate] = useState<UpdateInfo | null>(null);
  const [codexUpdateConfirmOpen, setCodexUpdateConfirmOpen] = useState(false);
  const [payloadUpdateProgress, setPayloadUpdateProgress] =
    useState<PayloadUpdateProgress | null>(null);
  const [nativeModelBusy, setNativeModelBusy] = useState(false);
  const [busyAction, setBusyAction] = useState<
    "install" | "update" | "launch" | "ccswitch" | "logs" | null
  >(null);
  const updateTitleId = useId();
  const updateBodyId = useId();
  const loadGeneration = useRef(0);
  const usbGuardStartupPolls = useRef(0);
  const progressScopeRef = useRef<HTMLDivElement | null>(null);
  const nativeModelRefreshStarted = useRef(false);
  const usbGuardWasReady = useRef(false);

  const {
    dl,
    dlPct,
    dlBytes,
    dlSpeed,
    downloadStop,
    downloadStopBusy,
    startDlListen,
    requestDownloadStop,
    resetStop,
  } = useDownloadProgress({
    eventName: "win://download-progress",
    pauseDownload: (operationId) => managerApi.winPauseDownload(operationId),
    cancelDownload: (operationId) => managerApi.winCancelDownload(operationId),
    getOperationSnapshot: () => managerApi.getOperationSnapshot(),
    onError: setFailure,
  });

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    void startDlListen().then((cleanup) => {
      if (active) unlisten = cleanup;
      else cleanup();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [startDlListen]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<PayloadUpdateProgress>("payload-update-progress", (event) => {
      if (active) setPayloadUpdateProgress(event.payload);
    }).then((cleanup) => {
      if (active) unlisten = cleanup;
      else cleanup();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  const load = useCallback(async () => {
    const generation = loadGeneration.current + 1;
    loadGeneration.current = generation;
    setPhase("loading");
    setPayload(null);
    setPayloadLoading(true);
    setFailure(null);
    setNotice(null);
    setNativeModelNotice(null);
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
      // The initial status probe can race the background USB lease startup.
      // Re-read it once after the guard has settled so a current USB install is
      // not left with the pre-authorization `external` snapshot.
      let settledStatus = nextStatus;
      if (settledUsbGuard && codexReadyOnAuthorizedUsb(settledUsbGuard)) {
        try {
          settledStatus = await managerApi.winStatus();
          if (loadGeneration.current !== generation) return;
          setStatus(settledStatus);
        } catch {
          // Keep the already-rendered status if the second read is transient.
        }
      }
      setPayload(nextPayload);
      setPayloadLoading(false);
      if (
        settledStatus.installed &&
        compareVersions(settledStatus.installed.version, nextPayload.codexDesktopVersion) > 0
      ) {
        setNotice(t("offline.newerVersion"));
      }
      const installed = settledStatus.installed;
      const sameVersion = Boolean(
        installed &&
          compareVersions(installed.version, nextPayload.codexDesktopVersion) === 0,
      );
      if (
        codexReadyOnAuthorizedUsb(settledUsbGuard) &&
        installed &&
        settledStatus.status === "managed" &&
        !settledStatus.recoveryPending &&
        sameVersion &&
        programIsOnCurrentUsb(settledStatus, settledUsbGuard)
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
          const ready = codexReadyOnAuthorizedUsb(next);
          const becameReady = ready && !usbGuardWasReady.current;
          usbGuardWasReady.current = ready;
          if (becameReady) {
            void managerApi.winStatus().then(setStatus).catch(() => undefined);
          }
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
          usbGuardWasReady.current = false;
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

  const refreshNativeModels = useCallback(async (automatic = false) => {
    if (nativeModelBusy) return;
    setNativeModelBusy(true);
    setNativeModelNotice({
      message: t("offline.nativeModelsRefreshing"),
      tone: "info",
    });
    try {
      const result = await new Promise<Awaited<ReturnType<typeof managerApi.refreshNativeModelCatalog>>>((resolve, reject) => {
        const timer = window.setTimeout(
          () => reject(new Error("native model catalog refresh timed out")),
          NATIVE_MODEL_REFRESH_TIMEOUT_MS,
        );
        void managerApi.refreshNativeModelCatalog(true).then(
          (value) => {
            window.clearTimeout(timer);
            resolve(value);
          },
          (cause) => {
            window.clearTimeout(timer);
            reject(cause);
          },
        );
      });
      const message =
        result.status === "ready" || result.status === "fresh"
          ? t("offline.nativeModelsReady", { count: result.modelCount })
          : result.message || t("offline.nativeModelsUnavailable");
      setNativeModelNotice({
        message,
        tone: result.status === "ready" || result.status === "fresh" ? "ok" : "warn",
      });
    } catch (cause) {
      const detail = cause instanceof Error ? cause.message : String(cause ?? "");
      const likelyUpstream = /404token|gateway|network|timeout|unavailable|连接|上游/i.test(detail);
      setNativeModelNotice({
        message: likelyUpstream
          ? t("offline.nativeModelsUpstreamUnavailable")
          : t("offline.nativeModelsUnavailable"),
        // Automatic refresh is best-effort after installation. Keep the
        // completed install visible and distinguish a retryable upstream
        // problem from a local installation failure. Manual refresh remains
        // a hard-error surface so the user can investigate it explicitly.
        tone: automatic ? "warn" : "err",
      });
      if (!automatic) {
        setFailure(resolveFailure(cause, t));
      }
    } finally {
      setNativeModelBusy(false);
    }
  }, [nativeModelBusy, t]);

  // Keep the delayed startup refresh scheduled even when a user clicks the
  // manual button in the meantime.  `refreshNativeModels` changes identity
  // while its busy flag flips; storing the latest callback in a ref prevents
  // that normal state transition from cancelling the one-shot startup timer.
  const refreshNativeModelsRef = useRef(refreshNativeModels);
  useEffect(() => {
    refreshNativeModelsRef.current = refreshNativeModels;
  }, [refreshNativeModels]);

  const performInstall = useCallback(async (sourcePayload: OfflinePayloadReport) => {
    setPayload(sourcePayload);
    setPhase("staging");
    setFailure(null);
    setNotice(null);
    setNativeModelNotice(null);
    setInstallationDetail(null);
    let stage: OfflineStageReport | null = null;
    let pendingRecovery = false;
    try {
      // The initial button state is only a snapshot. Re-read the live guard
      // before staging the MSIX so a stale `authorized/running` status can
      // never enter the backend stage path and turn a lease race into a
      // misleading signature failure. The backend repeats this gate at its
      // own mutating boundary as the final authority.
      const preflightGuardResult = await managerApi.usbGuardStatus()
        .then((value) => ({ value, failed: false }))
        .catch(() => ({ value: null, failed: true }));
      const preflightGuard = preflightGuardResult.value;
      if (!codexActionsAllowedOnCurrentRuntime(preflightGuard, preflightGuardResult.failed)) {
        if (preflightGuardResult.failed) setUsbGuardQueryFailed(true);
        if (preflightGuard) setUsbGuard(preflightGuard);
        setPhase("ready");
        setNotice(t("offline.installRequiresAuthorized"));
        return false;
      }
      setUsbGuard(preflightGuard);
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
      // A historical USB install owns the same progress lease as the normal
      // Windows flow. Reset the previous event owner before the new operation
      // so a late packet from an earlier install cannot paint this one.
      resetStop();
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
      let ccswitchErrorDetail: string | null = null;
      try {
        ccswitchReport = await managerApi.installAndLaunchCcswitch();
      } catch (ccswitchError) {
        ccswitchErrorDetail =
          ccswitchError instanceof Error
            ? ccswitchError.message
            : String(ccswitchError ?? "");
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
        notices.push(ccswitchUserMessage(ccswitchReport.message, t("offline.ccswitchLaunched")));
      } else if (ccswitchReport && !ccswitchReport.available) {
        notices.push(t("offline.ccswitchUnavailable"));
      } else if (ccswitchErrorDetail) {
        notices.push(`${t("offline.ccswitchFailed")} ${ccswitchErrorDetail}`);
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
  }, [resetStop, t]);

  const install = useCallback(async () => {
    if (!payload || busyAction) return;
    if (!codexActionsAllowedOnCurrentRuntime(usbGuard, usbGuardQueryFailed)) {
      setNotice(t("offline.installRequiresAuthorized"));
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

  const checkCodexPayload = useCallback(async (allowWhileBusy = false) => {
    if (busyAction && !allowWhileBusy) return;
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
        if (allowWhileBusy) {
          setNotice(
            t("offline.updateStaleRefreshed", {
              version: result.info.latestVersion,
            }),
          );
        }
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
    setPayloadUpdateProgress({ phase: "preparing", bytesDone: 0, bytesTotal: 0 });
    try {
      await managerApi.applyCodexPayloadUpdate(pendingCodexUpdate);
      setPayloadUpdateProgress(null);
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
      setPayloadUpdateProgress(null);
      if (errorCode(cause) === "stale_expectation") {
        setPendingCodexUpdate(null);
        setCodexUpdateConfirmOpen(false);
        // The backend deliberately rejects the old confirmation. Re-check
        // once against the same signed USB instead of leaving the user with a
        // dead-end banner; the fresh candidate still needs an explicit click.
        await checkCodexPayload(true);
      } else {
        setFailure(resolveFailure(cause, t));
        setCodexUpdateConfirmOpen(false);
      }
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, checkCodexPayload, pendingCodexUpdate, performInstall, t]);

  const launchCcswitch = useCallback(async () => {
    if (busyAction) return;
    setBusyAction("ccswitch");
    setFailure(null);
    try {
      const report = await managerApi.installAndLaunchCcswitch();
      if (report.launched) {
        setNotice(ccswitchUserMessage(report.message, t("offline.ccswitchLaunched")));
      } else if (!report.available) {
        setNotice(t("offline.ccswitchUnavailable"));
      } else {
        setNotice(ccswitchUserMessage(report.message, t("offline.ccswitchFailed")));
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
      const normalizedLang = lang.toLowerCase();
      const codexLocale = normalizedLang.startsWith("zh")
        ? normalizedLang.startsWith("zh-tw")
          ? "zh-TW"
          : "zh-CN"
        : undefined;
      await managerApi.winLaunch(codexLocale);
    } catch (cause) {
      setFailure(resolveFailure(cause, t));
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, lang, t]);

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
  const startupReadinessPending =
    payloadLoading || phase === "loading" || usbGuardIsStarting;
  const disabled =
    busyAction !== null ||
    startupReadinessPending ||
    phase === "staging" ||
    phase === "installing";
  const payloadBytesDone = payloadUpdateProgress?.bytesDone ?? 0;
  const payloadBytesTotal = payloadUpdateProgress?.bytesTotal ?? 0;

  // Native Codex accepts the catalog through its official model_catalog_json
  // setting. Refresh once in the background after the USB install is known to
  // be usable; the explicit button below remains available for retry.
  useEffect(() => {
    if (!showInstalled) {
      nativeModelRefreshStarted.current = false;
      return;
    }
    if (!codexActionsAllowed || nativeModelRefreshStarted.current) return;
    nativeModelRefreshStarted.current = true;
    const timer = window.setTimeout(() => {
      void refreshNativeModelsRef.current(true);
    }, NATIVE_MODEL_REFRESH_START_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [codexActionsAllowed, showInstalled]);

  const payloadProgressDownload: DownloadProgress | null = payloadUpdateProgress
    ? {
        downloaded:
          payloadUpdateProgress.phase === "done"
            ? payloadUpdateProgress.bytesTotal ?? payloadUpdateProgress.bytesDone ?? 0
            : payloadUpdateProgress.bytesDone ?? 0,
        total: payloadUpdateProgress.bytesTotal ?? 0,
        source: "USB 更新源",
        phase:
          payloadUpdateProgress.phase === "download"
            ? "downloading"
            : payloadUpdateProgress.phase === "done"
              ? "finishing"
              : "preparing",
      }
    : null;
  const payloadProgressTotal = payloadProgressDownload?.total ?? 0;
  const payloadProgressDone = payloadProgressDownload?.downloaded ?? 0;
  const payloadProgressPct =
    payloadProgressTotal > 0
      ? Math.min(100, (payloadProgressDone / payloadProgressTotal) * 100)
      : 0;

  // Keep the main-screen phase union intact below; the progress screen is a
  // temporary view and the page still renders the normal phase copy after it
  // unmounts.
  if (String(phase) === "installing") {
    return (
      <ProgressScreen
        scene="offline-install"
        scopeRef={progressScopeRef}
        paused={null}
        dl={dl}
        dlPct={dlPct}
        dlBytes={dlBytes}
        dlSpeed={dlSpeed}
        installing
        downloadStop={downloadStop}
        downloadStopBusy={downloadStopBusy}
        failure={failure}
        onResume={() => undefined}
        onPause={() => void requestDownloadStop("pause")}
        onCancel={() => void requestDownloadStop("cancel")}
      />
    );
  }

  if (payloadProgressDownload) {
    return (
      <ProgressScreen
        scene="offline-payload-update"
        scopeRef={progressScopeRef}
        paused={null}
        dl={payloadProgressDownload}
        dlPct={payloadProgressPct}
        dlBytes={payloadProgressDone}
        dlSpeed={0}
        installing
        downloadStop={null}
        downloadStopBusy
        failure={failure}
        onResume={() => undefined}
        onPause={() => undefined}
        onCancel={() => undefined}
        showControls={false}
      />
    );
  }

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
        {nativeModelNotice ? (
          <StatusBanner tone={nativeModelNotice.tone}>{nativeModelNotice.message}</StatusBanner>
        ) : null}
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
        {payloadUpdateProgress ? (
          <section className="payload-update-progress" data-testid="payload-update-progress">
            <div className="sub">
              {payloadUpdateProgress.phase === "download"
                ? t("progress.downloadingFrom", { source: "USB 更新源" })
                : t("progress.finishing")}
            </div>
            <div
              className="bar"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={
                payloadBytesTotal > 0
                  ? Math.min(
                      100,
                      (payloadBytesDone / payloadBytesTotal) * 100,
                    )
                  : undefined
              }
            >
              <div
                className={`bar-fill${payloadBytesTotal > 0 ? "" : " indeterminate"}`}
                style={
                  payloadBytesTotal > 0
                    ? {
                        width: `${Math.min(
                          100,
                          (payloadBytesDone / payloadBytesTotal) * 100,
                        )}%`,
                      }
                    : undefined
                }
              />
            </div>
            {payloadBytesTotal > 0 ? (
              <div className="dlmeta">
                {mib(payloadBytesDone)} / {mib(payloadBytesTotal)}
              </div>
            ) : null}
          </section>
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
          {startupReadinessPending ? (
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
              <button
                className="btn ghost"
                onClick={() => void refreshNativeModels()}
                disabled={disabled || nativeModelBusy}
                data-testid="refresh-native-models"
              >
                {nativeModelBusy ? <Icon name="loader" className="spinicon" /> : <Icon name="refresh" />}
                {nativeModelBusy ? t("offline.nativeModelsRefreshing") : t("offline.nativeModelsRefresh")}
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
        <AccountCard />
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
