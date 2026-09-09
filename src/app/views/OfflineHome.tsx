import { useCallback, useEffect, useMemo, useState } from "react";

import { managerApi } from "../../services/managerApi";
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
  resolveFailure,
  type FailureSurface,
} from "../errorCopy";
import { Icon, CodexGlyph } from "../icons";
import { useI18n, type TFn } from "../i18n";
import { FailureBanner, Ring, StatusBanner, Tooltip, TopBar } from "../components";
import { mib } from "../format";
import { OfflineComponentsSheet } from "./OfflineComponentsSheet";

type OfflinePhase = "loading" | "ready" | "installed" | "staging" | "installing" | "error";

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
  // The USB installer can migrate an existing external MSIX registration to
  // the guarded portable program tree. Preserve the fresh snapshot for the backend's
  // TOCTOU check instead of turning an external package into an artificial
  // "nothing installed" expectation.
  const installed = status?.installed ?? null;
  return {
    currentPath: installed?.path ?? null,
    currentVersion: installed?.version ?? null,
    currentSource: installed?.source ?? null,
  };
}

function canMigrateExternalMsix(status: WinInstallStatus | null): boolean {
  return status?.status === "external" && status.installed?.source === "msix";
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

function usbGuardLabel(status: string, t: TFn): string {
  switch (status) {
    case "authorized": return t("offline.usbGuard.authorized");
    case "grace": return t("offline.usbGuard.grace");
    case "missing": return t("offline.usbGuard.missing");
    case "mismatch": return t("offline.usbGuard.mismatch");
    case "unsupported": return t("offline.usbGuard.unsupported");
    case "disabled": return t("offline.usbGuard.disabled");
    case "stopped": return t("offline.usbGuard.stopped");
    default: return status;
  }
}

/**
 * Minimal USB portable Windows screen. It intentionally has no remote
 * catalog or update state machine: the only source is the fixed payload beside
 * the executable, while the guarded USB portable install command remains the
 * single mutation path.
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
  const [status, setStatus] = useState<WinInstallStatus | null>(null);
  const [usbGuard, setUsbGuard] = useState<UsbGuardStatus | null>(null);
  const [failure, setFailure] = useState<FailureSurface | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [installationDetail, setInstallationDetail] = useState<string | null>(null);
  const [componentsOpen, setComponentsOpen] = useState(false);
  const [busyAction, setBusyAction] = useState<
    "install" | "launch" | "ccswitch" | "logs" | "adopt" | null
  >(null);

  const load = useCallback(async () => {
    setPhase("loading");
    setFailure(null);
    setNotice(null);
    setInstallationDetail(null);
    try {
      const [nextPayload, nextStatus, nextUsbGuard] = await Promise.all([
        managerApi.offlinePayloadProbe(),
        managerApi.winStatus(),
        managerApi.usbGuardStatus().catch(() => null),
      ]);
      setPayload(nextPayload);
      setStatus(nextStatus);
      setUsbGuard(nextUsbGuard);
      const migratableExternalMsix = canMigrateExternalMsix(nextStatus);
      if (nextStatus.recoveryPending) {
        setNotice(t("offline.recoveryPending"));
      } else if (migratableExternalMsix) {
        setNotice(t("offline.externalMsix"));
      } else if (nextStatus.status === "external") {
        setNotice(t("offline.external"));
      } else if (
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
        installed &&
        nextStatus.status === "managed" &&
        !nextStatus.recoveryPending &&
        sameVersion &&
        programIsOnCurrentUsb(nextStatus, nextUsbGuard)
      ) {
        setPhase("installed");
      } else {
        setPhase("ready");
      }
    } catch (cause) {
      setPayload(null);
      setStatus(null);
      setPhase("error");
      setFailure(resolveFailure(cause, t));
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  // The helper writes a fresh status roughly once per second. Polling keeps
  // the visible countdown useful without adding a WebSocket or privileged
  // background service.
  useEffect(() => {
    // The modal owns a long, scrollable form. The watchdog countdown is not
    // useful while it is open, and updating it would needlessly re-render the
    // whole page behind the modal once per second.
    if (componentsOpen) return;
    let active = true;
    const timer = window.setInterval(() => {
      void managerApi
        .usbGuardStatus()
        .then((next) => {
          if (active) setUsbGuard(next);
        })
        .catch(() => undefined);
    }, 1_000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [componentsOpen]);

  const newerInstalled = useMemo(() => {
    if (!payload || !status?.installed) return false;
    return compareVersions(status.installed.version, payload.codexDesktopVersion) > 0;
  }, [payload, status]);

  const install = useCallback(async () => {
    if (!payload || busyAction) return;
    if (newerInstalled) {
      setNotice(t("offline.newerVersion"));
      return;
    }
    setBusyAction("install");
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
      if (current.status === "external" && !canMigrateExternalMsix(current)) {
        throw new Error(t("offline.external"));
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
        compareVersions(installed.installed.version, payload.codexDesktopVersion) !== 0
      ) {
        throw new Error(t("offline.verifyAfterInstall"));
      }
      const verifiedUsbGuard = await managerApi.usbGuardStatus().catch(() => usbGuard);
      if (!programIsOnCurrentUsb(installed, verifiedUsbGuard)) {
        const installPath = installed.installed.path || "未知";
        const bundlePath = verifiedUsbGuard?.bundlePath || "未知";
        throw new Error(`无法确认 Codex 程序位于当前 U 盘套件（程序 ${installPath}，套件 ${bundlePath}），已保留暂存文件，请查看日志后重试`);
      }
      setUsbGuard(verifiedUsbGuard);
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
      if (capabilityPackReport?.success) {
        notices.push(
          t("offline.capabilityInstalled", {
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
    } catch (cause) {
      setPhase("ready");
      setFailure(resolveFailure(cause, t));
      if (pendingRecovery) {
        setNotice(t("offline.recoveryPending"));
      } else if (stage) {
        setNotice("暂存文件已保留，便于查看日志和重试。");
      }
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, newerInstalled, payload, t, usbGuard]);

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

  const adopt = useCallback(async () => {
    if (busyAction || status?.status !== "external") return;
    setBusyAction("adopt");
    setFailure(null);
    try {
      await managerApi.winAdopt();
      await load();
    } catch (cause) {
      setFailure(resolveFailure(cause, t));
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, load, status, t]);

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
  const disabled = busyAction !== null || phase === "loading" || phase === "staging" || phase === "installing";

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
          {phase === "loading" ? (
            <>
              <Ring icon="loader" spin />
              <div className="headline">{t("offline.checking")}</div>
            </>
          ) : phase === "error" ? (
            <>
              <Ring icon="alert" variant="danger" />
              <div className="headline">{t("offline.checkFailed")}</div>
            </>
          ) : phase === "installed" ? (
            <>
              <Ring icon="check" variant="success" />
              <div className="headline">{t("offline.installed")}</div>
              <div className="microcue"><Icon name="shield" />{t("offline.hashVerified")}</div>
            </>
          ) : status?.status === "external" && !canMigrateExternalMsix(status) ? (
            <>
              <Ring icon="shield" variant="amber" />
              <div className="headline">{t("home.external.title")}</div>
              <div className="prov"><span className="dot external" />{t("prov.external")}</div>
              <Tooltip content={t("offline.external")}>
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
          ) : phase === "installed" ? (
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
          ) : status?.status === "external" && !canMigrateExternalMsix(status) ? (
            <>
              <button className="btn primary big" onClick={() => void adopt()} disabled={disabled}>
                {busyAction === "adopt" ? <Icon name="loader" className="spinicon" /> : <Icon name="shield" />}
                {busyAction === "adopt" ? t("offline.adopting") : t("home.external.cta")}
              </button>
              <button className="btn ghost" onClick={() => void load()} disabled={disabled}><Icon name="refresh" />{t("offline.recheck")}</button>
            </>
          ) : (
            <>
              <button className="btn primary big" onClick={() => void install()} disabled={disabled || newerInstalled}>
                {busyAction === "install" ? <Icon name="loader" className="spinicon" /> : <Icon name="download" />}
                {busyAction === "install" ? t("offline.installing") : status?.recoveryPending ? t("offline.resumeInstall") : t("offline.install")}
              </button>
              <button className="btn ghost" onClick={() => void load()} disabled={disabled}><Icon name="refresh" />{t("offline.recheck")}</button>
            </>
          )}
          {phase !== "installed" && onOpenUninstall ? (
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

        {installed ? (
          <div className="foot">
            {installed.path}
            {installed.packageVolume ? (
              <>
                <span>·</span>
                <span title="AppX package volume">AppX {installed.packageVolume}</span>
              </>
            ) : null}
            <span>·</span>
            <button className="gobtn" onClick={onOpenSettings}>{t("nav.settings")}</button>
          </div>
        ) : null}
      </div>
      <OfflineComponentsSheet
        open={componentsOpen}
        onDismiss={() => setComponentsOpen(false)}
      />
    </div>
  );
}
