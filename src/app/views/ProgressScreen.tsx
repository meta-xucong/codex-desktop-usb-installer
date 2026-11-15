import { useEffect, type RefObject } from "react";

import type {
  DownloadProgress,
  HistoricalInstallSelection,
  HistoricalResumeExpectation,
  OperationPhase,
  OperationResumeContext,
} from "../../shared/types";
import { WINDOWS_PORTABLE_EXTRACTION_SOURCE } from "../../shared/types";
import type { FailureSurface } from "../errorCopy";
import { Icon } from "../icons";
import { useI18n } from "../i18n";
import { FailureBanner, Ring, TopBar } from "../components";
import { mib } from "../format";
import { acquireNavLock } from "../navLock";

export type DownloadStopIntent = "pause" | "cancel";

export interface PausedDownload {
  kind: "perform" | "install";
  dl: DownloadProgress | null;
  /** Normal Windows portable install destination (historical keeps it below). */
  installRoot?: string;
  /** Frozen ordinary latest-channel target. */
  resume?: OperationResumeContext;
  historical?: {
    selection: HistoricalInstallSelection;
    blockUpdates: boolean;
    expectation: HistoricalResumeExpectation;
    installRoot?: string | null;
  };
}

/** The full-screen download/install progress view, shared by the Mac and
 *  Windows homes (byte-for-byte identical before this extraction). Owns the
 *  progressbar accessibility semantics so assistive tech can follow a
 *  destructive install/update: `role="progressbar"` + aria-value*, and an
 *  aria-live phase line ("preparing" → "downloading" → "finishing"). */
export function ProgressScreen({
  scene,
  scopeRef,
  paused,
  dl,
  downloadProgress,
  installProgress,
  dlPct,
  dlBytes,
  dlSpeed,
  installing,
  splitWindowsInstall = false,
  localSetup = false,
  downloadStop,
  downloadStopBusy,
  failure,
  activity,
  onResume,
  onPause,
  onCancel,
  showControls = true,
}: {
  scene: string;
  scopeRef: RefObject<HTMLDivElement | null>;
  paused: PausedDownload | null;
  /** Live progress (null when not transferring). */
  dl: DownloadProgress | null;
  /** Windows download bytes retained separately from later install-phase packets. */
  downloadProgress?: DownloadProgress | null;
  /** Windows install bytes; only portable extraction currently has a real total. */
  installProgress?: DownloadProgress | null;
  /** Eased display figures (useCountUp), so the readouts glide. */
  dlPct: number;
  dlBytes: number;
  dlSpeed: number;
  /** Whether the operation is a fresh install (vs an in-place update). */
  installing: boolean;
  /** Keep Windows package download and install progress in separate tracks. */
  splitWindowsInstall?: boolean;
  /** Host-only setup resumes the fourth stage without downloading the program. */
  localSetup?: boolean;
  downloadStop: DownloadStopIntent | null;
  downloadStopBusy: boolean;
  failure: FailureSurface | null;
  /** Specific post-install work performed while the overall install remains active. */
  activity?: string | null;
  onResume: () => void;
  onPause: () => void;
  onCancel: () => void;
  /** Hide pause/cancel for update channels that expose progress but no stop API. */
  showControls?: boolean;
}) {
  const { t } = useI18n();

  // This screen is the operation's isolation chamber: while it is mounted,
  // navigation elsewhere (the expanded rail) must not offer an exit that could
  // start a concurrent operation or unmount the progress listeners.
  useEffect(() => acquireNavLock(), []);

  // Paused reads from its captured snapshot; live runs from the eased `dl`.
  const snap = paused ? paused.dl : dl;
  const source = (snap?.source ?? "").trim();
  const known = Boolean(snap && snap.total > 0);
  const snapPct =
    snap && snap.total > 0
      ? Math.min(100, (snap.downloaded / snap.total) * 100)
      : 0;
  const pct = known ? Math.round(paused ? snapPct : dlPct) : null;
  const barPct = paused ? snapPct : dlPct;
  const phaseCode: OperationPhase =
    snap?.phase ??
    (!paused && snap && snap.total > 0 && snap.downloaded >= snap.total
      ? "finishing"
      : snap
        ? "downloading"
        : "preparing");
  const installStarted = ["verifying", "applying", "committing", "finishing"].includes(
    phaseCode,
  );
  const showSplitWindowsInstall = splitWindowsInstall && !paused;
  const installStepCount =
    phaseCode === "verifying"
      ? 1
      : phaseCode === "applying"
        ? 2
        : phaseCode === "committing" || phaseCode === "finishing"
          ? 3
          : 0;
  const completedInstallSteps =
    phaseCode === "applying"
      ? 1
      : phaseCode === "committing"
        ? 2
        : phaseCode === "finishing"
          ? 3
          : 0;
  const activeInstallStep =
    phaseCode === "finishing"
      ? 3
      : Math.max(0, Math.min(installStepCount - 1, 3));
  const downloadKnown = Boolean(downloadProgress && downloadProgress.total > 0);
  const downloadPercent =
    downloadProgress && downloadProgress.total > 0
      ? Math.min(100, (downloadProgress.downloaded / downloadProgress.total) * 100)
      : 0;
  const extractionKnown = Boolean(
    installProgress &&
      installProgress.source === WINDOWS_PORTABLE_EXTRACTION_SOURCE &&
      installProgress.phase === "applying" &&
      installProgress.total > 0,
  );
  const extractionPercent =
    extractionKnown && installProgress
      ? Math.min(100, (installProgress.downloaded / installProgress.total) * 100)
      : 0;
  const primaryProgress = splitWindowsInstall && !paused ? downloadProgress : snap;
  const primaryKnown = Boolean(primaryProgress && primaryProgress.total > 0);
  const primaryPercent =
    splitWindowsInstall && !paused
      ? downloadKnown
        ? Math.round(downloadPercent)
        : null
      : pct;
  const primaryBarPercent =
    splitWindowsInstall && !paused ? downloadPercent : barPct;
  // Bytes are in → the uninterruptible install phase (gate/quit/atomic swap on
  // mac, sideload/extract on Windows). Say so and drop the dead buttons rather
  // than leave them greyed for no visible reason.
  const finishing =
    !paused &&
    (installStarted || Boolean(snap && snap.total > 0 && snap.downloaded >= snap.total));
  const uninterruptible = failure?.code === "download_stop_uninterruptible";
  // Pause only makes sense mid-transfer; cancel is the "abandon" out and works
  // through the preparing phase too (a backend abort checkpoint honors it), but
  // not once the install has begun.
  const canPause =
    !paused &&
    !uninterruptible &&
    Boolean(dl && dl.total > 0 && dl.downloaded < dl.total) &&
    !installStarted &&
    !downloadStopBusy;
  const canCancel =
    !paused && !finishing && !installStarted && !uninterruptible && !downloadStopBusy;

  const phase = paused
    ? t("progress.paused.title")
    : localSetup
      ? t("progress.localSetup")
      : phaseCode === "downloading" && snap
        ? source
          ? t("progress.downloadingFrom", { source })
          : t("progress.downloading")
        : phaseCode === "preparing" || phaseCode === "idle"
          ? t("progress.preparing")
          : phaseCode === "verifying"
            ? t("progress.verifying")
            : phaseCode === "applying"
              ? t("progress.applying")
              : phaseCode === "committing"
                ? t("progress.committing")
                : t("progress.finishing");

  return (
    <div className="pop">
      <TopBar />
      <div className="scroll" ref={scopeRef}>
        {failure ? <FailureBanner failure={failure} /> : null}
        <div className="hero" style={{ marginTop: 24 }} key={scene}>
          <Ring
            icon={paused ? "pause" : "loader"}
            spin={!paused}
            className="glow"
          />
          <div className={`headline${paused ? "" : " shimmer"}`}>
            {paused
              ? t("progress.paused.title")
              : installing
                ? t("progress.installing")
                : t("progress.title")}
          </div>
          {!paused ? (
            <div className="sub" aria-live="polite">
              {phase}
            </div>
          ) : null}
          {!paused && activity ? (
            <div className="sub" aria-live="polite" aria-atomic="true">
              {activity}
            </div>
          ) : null}
          {!showSplitWindowsInstall && primaryPercent !== null ? (
            <div className="pctbig">
              {primaryPercent}
              <span className="pctsign">%</span>
            </div>
          ) : null}
          {!showSplitWindowsInstall ? (
            <div
              className="bar"
              role="progressbar"
              aria-label={installing ? t("progress.installing") : t("progress.title")}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={primaryPercent ?? undefined}
              aria-valuetext={
                primaryPercent !== null
                  ? `${primaryPercent}% · ${activity ?? phase}`
                  : activity ?? phase
              }
              data-testid="install-progress-bar"
            >
              <div
                className={`bar-fill${primaryPercent === null ? " indeterminate" : ""}`}
                style={primaryPercent === null ? undefined : { width: `${primaryBarPercent}%` }}
              />
            </div>
          ) : null}
          {showSplitWindowsInstall ? (
            <div className="progress-tracks">
              {!localSetup ? <div className="progress-track">
                <div className="dlmeta">
                  <strong>{t("progress.downloadStage")}: </strong>
                  {downloadKnown && downloadProgress
                    ? downloadProgress.downloaded >= downloadProgress.total
                      ? t("progress.downloadComplete")
                      : downloadProgress.source
                        ? t("progress.downloadingFrom", { source: downloadProgress.source })
                        : t("progress.downloading")
                    : phaseCode === "preparing" || phaseCode === "idle"
                      ? t("progress.preparing")
                      : t("progress.downloading")}
                </div>
                <div
                  className="bar"
                  role="progressbar"
                  aria-label={t("progress.downloadStage")}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={downloadKnown ? Math.round(downloadPercent) : undefined}
                  aria-valuetext={
                    downloadKnown
                      ? `${Math.round(downloadPercent)}% · ${t("progress.downloadStage")}`
                      : t("progress.preparing")
                  }
                  data-testid="download-progress-bar"
                >
                  <div
                    className={`bar-fill${downloadKnown ? "" : " indeterminate"}`}
                    style={downloadKnown ? { width: `${downloadPercent}%` } : undefined}
                  />
                </div>
                {downloadKnown && downloadProgress ? (
                  <div className="dlmeta">
                    {mib(downloadProgress.downloaded)} / {mib(downloadProgress.total)}
                  </div>
                ) : null}
              </div> : null}
              <div className="progress-track">
                <div className="dlmeta">
                  {t("progress.installStage")} · {installStepCount}/4 · {phase}
                </div>
                <div
                  className="install-stage-track"
                  role="progressbar"
                  aria-label={t("progress.installStage")}
                  aria-valuemin={0}
                  aria-valuemax={4}
                  aria-valuenow={installStepCount}
                  aria-valuetext={
                    extractionKnown
                      ? `${installStepCount}/4 · ${t("progress.extracting")} ${Math.round(extractionPercent)}%`
                      : `${installStepCount}/4 · ${phase}`
                  }
                  data-testid="install-progress-bar"
                >
                  {Array.from({ length: 4 }, (_, index) => {
                    const complete = index < completedInstallSteps;
                    const active = index === activeInstallStep;
                    const indeterminate = active && !extractionKnown;
                    const width = complete
                      ? "100%"
                      : active && extractionKnown
                        ? `${extractionPercent}%`
                        : "0%";
                    return (
                      <span
                        key={index}
                        className="install-stage-segment"
                        data-state={complete ? "complete" : active ? "active" : "pending"}
                      >
                        <span
                          className={`install-stage-fill${indeterminate ? " indeterminate" : ""}`}
                          style={indeterminate ? undefined : { width }}
                        />
                      </span>
                    );
                  })}
                </div>
                {extractionKnown && installProgress ? (
                  <div className="dlmeta">
                    {mib(installProgress.downloaded)} / {mib(installProgress.total)}
                  </div>
                ) : null}
              </div>
            </div>
          ) : null}
          {!showSplitWindowsInstall && primaryKnown && primaryProgress ? (
            <div className="dlmeta">
              {mib(
                splitWindowsInstall && !paused
                  ? primaryProgress.downloaded
                  : paused
                    ? primaryProgress.downloaded
                    : dlBytes,
              )} / {mib(primaryProgress.total)}
              {!paused && dlSpeed > 0 ? ` · ${mib(dlSpeed)}/s` : ""}
            </div>
          ) : null}
          {showControls ? (
            <div className="progress-actions">
              {paused ? (
                <button
                  className="btn primary"
                  onClick={onResume}
                  disabled={downloadStopBusy}
                >
                  <Icon name="play" />
                  {t("progress.resume")}
                </button>
              ) : (
                <button
                  className="btn ghost"
                  onClick={onPause}
                  disabled={!canPause}
                >
                  <Icon name="pause" />
                  {downloadStop === "pause"
                    ? t("progress.pausePending")
                    : t("progress.pause")}
                </button>
              )}
              <button
                className="btn danger"
                onClick={onCancel}
                disabled={downloadStopBusy || (!paused && !canCancel)}
              >
                <Icon name="close" />
                {downloadStop === "cancel"
                  ? t("progress.cancelPending")
                  : t("progress.cancel")}
              </button>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
