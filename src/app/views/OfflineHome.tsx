import { useCallback, useEffect, useMemo, useState } from "react";

import { managerApi } from "../../services/managerApi";
import type {
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
import { FailureBanner, Ring, StatusBanner, TopBar } from "../components";
import { mib } from "../format";

type OfflinePhase = "loading" | "ready" | "installed" | "staging" | "installing" | "error";

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
  const installed = status?.status === "managed" ? status.installed : null;
  return {
    currentPath: installed?.path ?? null,
    currentVersion: installed?.version ?? null,
    currentSource: installed?.source ?? null,
  };
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
 * Minimal V1 Windows screen.  It intentionally has no remote catalog or
 * update state machine: the only source is the fixed payload beside the
 * executable, while the existing guarded MSIX install command remains the
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
  const [busyAction, setBusyAction] = useState<
    "install" | "launch" | "ccswitch" | "logs" | "adopt" | null
  >(null);

  const load = useCallback(async () => {
    setPhase("loading");
    setFailure(null);
    setNotice(null);
    try {
      const [nextPayload, nextStatus, nextUsbGuard] = await Promise.all([
        managerApi.offlinePayloadProbe(),
        managerApi.winStatus(),
        managerApi.usbGuardStatus().catch(() => null),
      ]);
      setPayload(nextPayload);
      setStatus(nextStatus);
      setUsbGuard(nextUsbGuard);
      if (nextStatus.status === "external") {
        setNotice(t("offline.external"));
      } else if (
        nextStatus.installed &&
        compareVersions(nextStatus.installed.version, nextPayload.codexDesktopVersion) > 0
      ) {
        setNotice(t("offline.newerVersion"));
      }
      const installed = nextStatus.installed;
      if (
        installed &&
        nextStatus.status === "managed" &&
        compareVersions(installed.version, nextPayload.codexDesktopVersion) === 0
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
  }, []);

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
    let stage: OfflineStageReport | null = null;
    try {
      stage = await managerApi.offlinePayloadStage();
      setPhase("installing");
      const current = await managerApi.winStatus();
      if (current.status === "external") {
        throw new Error(t("offline.external"));
      }
      const result = await managerApi.winInstallHistoricalRelease(
        selectionFromStage(stage),
        // Keep the shipped desktop build fixed for V1.  This only disables
        // Codex's own background self-update flag; it does not patch binaries
        // or block the network needed later for sign-in/model requests.
        true,
        expectedFromStatus(current),
        undefined,
        undefined,
        true,
      );
      if (!result.success) {
        throw new Error(result.message || t("error.install"));
      }
      await managerApi.offlinePayloadCleanup(stage.stagedPath).catch(() => undefined);
      const installed = await managerApi.winStatus();
      if (
        installed.status !== "managed" ||
        !installed.installed ||
        compareVersions(installed.installed.version, payload.codexDesktopVersion) !== 0
      ) {
        throw new Error(t("offline.verifyAfterInstall"));
      }
      // Codex is now confirmed on disk.  Hand off to the bundled CCSwitch
      // configurator; a missing/failed configurator must not turn a successful
      // Codex installation into a failure, so it is shown as a notice instead.
      let ccswitchReport = null;
      try {
        ccswitchReport = await managerApi.installAndLaunchCcswitch();
      } catch (ccswitchError) {
        console.warn("[ccswitch] automatic launch failed", ccswitchError);
      }
      setStatus(installed);
      setPhase("installed");
      if (ccswitchReport?.launched) {
        setNotice(t("offline.ccswitchLaunched"));
      } else if (ccswitchReport && !ccswitchReport.available) {
        setNotice(t("offline.ccswitchUnavailable"));
      } else if (!ccswitchReport) {
        setNotice(t("offline.ccswitchFailed"));
      } else {
        setNotice(ccswitchReport.message || t("offline.doneDetail"));
      }
    } catch (cause) {
      setPhase("ready");
      setFailure(resolveFailure(cause, t));
      if (stage) {
        setNotice("暂存文件已保留，便于查看日志和重试。");
      }
    } finally {
      setBusyAction(null);
    }
  }, [busyAction, newerInstalled, payload, t]);

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
            </div>
            <div className="usb-guard-status-message">{usbGuard.message}</div>
            {usbGuard.driveRoot ? (
              <div className="usb-guard-status-path">{usbGuard.driveRoot}{usbGuard.bundleRelativePath ? ` · ${usbGuard.bundleRelativePath}` : ""}</div>
            ) : null}
            {!usbGuard.signed && usbGuard.status === "authorized" ? (
              <div className="usb-guard-status-note">{t("offline.usbGuard.unsigned")}</div>
            ) : null}
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
              <div className="headline">{t("offline.noPackage")}</div>
            </>
          ) : phase === "installed" ? (
            <>
              <Ring icon="check" variant="success" />
              <div className="headline">{t("offline.installed")}</div>
              <div className="microcue"><Icon name="shield" />{t("offline.hashVerified")}</div>
            </>
          ) : status?.status === "external" ? (
            <>
              <Ring icon="shield" variant="amber" />
              <div className="headline">{t("home.external.title")}</div>
              <div className="prov"><span className="dot external" />{t("prov.external")}</div>
              <div className="desc">{t("offline.external")}</div>
            </>
          ) : (
            <>
              <Ring icon="download" className={phase === "staging" || phase === "installing" ? "spin" : ""} />
              <div className="headline">
                {phase === "staging" || phase === "installing" ? t("offline.installing") : t("offline.ready")}
              </div>
              <div className="desc">{t("offline.description")}</div>
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
              {onOpenUninstall ? (
                <button className="btn ghost danger" onClick={onOpenUninstall} disabled={disabled}>
                  <Icon name="trash" />{t("offline.uninstallBundle")}
                </button>
              ) : null}
            </>
          ) : status?.status === "external" ? (
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
                {busyAction === "install" ? t("offline.installing") : t("offline.install")}
              </button>
              <button className="btn ghost" onClick={() => void load()} disabled={disabled}><Icon name="refresh" />{t("offline.recheck")}</button>
            </>
          )}
          <button className="btn ghost" onClick={() => void openLogs()} disabled={disabled}>
            {busyAction === "logs" ? <Icon name="loader" className="spinicon" /> : <Icon name="folder" />}
            {t("offline.openLogs")}
          </button>
        </div>

        {installed ? (
          <div className="foot">
            {installed.path}
            <span>·</span>
            <button className="gobtn" onClick={onOpenSettings}>{t("nav.settings")}</button>
          </div>
        ) : null}
      </div>
    </div>
  );
}
