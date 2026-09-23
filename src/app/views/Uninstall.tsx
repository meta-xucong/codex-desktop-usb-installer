import { useCallback, useEffect, useId, useState } from "react";

import { managerApi } from "../../services/managerApi";
import type { InstallProbeState, OperationOutcome } from "../../shared/types";
import { outcomeIsPartial } from "../../shared/types";
import { resolveFailure } from "../errorCopy";
import { Icon } from "../icons";
import { useI18n } from "../i18n";
import { NavBar, Ring, Toggle, StatusBanner, Tooltip } from "../components";
import { codexHomeDisplay } from "../paths";
import { currentPlatform } from "../platform";
import { Sheet } from "../Sheet";

function hasTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    Boolean((window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__)
  );
}

const PROVENANCE_RECOVERY_ACTIONS = new Set(["record_provenance", "clear_provenance"]);
const CLEANUP_RECOVERY_ACTIONS = new Set(["cleanup_metadata", "purge_user_data"]);
const TERMINAL_STEP_STATES = new Set(["ok", "skipped", "not_applicable"]);

function outcomeNeedsAttention(outcome: OperationOutcome): boolean {
  return !outcome.primaryOk || outcomeIsPartial(outcome) || outcome.recoveryActions.length > 0 ||
    !TERMINAL_STEP_STATES.has(outcome.provenance.state) ||
    !TERMINAL_STEP_STATES.has(outcome.cleanup.state);
}

/** Merge a scoped ancillary retry back into the original partial outcome.
 * The backend reports only the actions attempted in this request; actions the
 * user has not retried yet must remain visible, and a failed attempted action
 * remains available even if a backend forgets to echo its recovery key. */
function mergeAncillaryRetryOutcome(
  previous: OperationOutcome,
  attempted: string[],
  retried: OperationOutcome,
): OperationOutcome {
  const attemptedSet = new Set(attempted);
  const unattempted = previous.recoveryActions.filter((action) => !attemptedSet.has(action));
  const failedAttempted = attempted.filter((action) => {
    if (!retried.primaryOk) return true;
    if (PROVENANCE_RECOVERY_ACTIONS.has(action)) return !TERMINAL_STEP_STATES.has(retried.provenance.state);
    if (CLEANUP_RECOVERY_ACTIONS.has(action)) return !TERMINAL_STEP_STATES.has(retried.cleanup.state);
    return false;
  });
  const recoveryActions = [
    ...new Set([...unattempted, ...retried.recoveryActions, ...failedAttempted]),
  ];
  const hasUnattemptedProvenance = unattempted.some((action) =>
    PROVENANCE_RECOVERY_ACTIONS.has(action),
  );
  const hasUnattemptedCleanup = unattempted.some((action) =>
    CLEANUP_RECOVERY_ACTIONS.has(action),
  );

  return {
    ...retried,
    path: retried.path ?? previous.path,
    provenance: hasUnattemptedProvenance ? previous.provenance : retried.provenance,
    cleanup: hasUnattemptedCleanup ? previous.cleanup : retried.cleanup,
    warnings: [...new Set([...previous.warnings, ...retried.warnings])],
    recoveryActions,
  };
}

const CCSWITCH_INSTALL_DISPLAY = "%LOCALAPPDATA%\\Programs\\CodexSwitch";
const CCSWITCH_DATA_DISPLAY = "%USERPROFILE%\\.cc-switch";
// Mirrors the existing Windows purge lists; these are display strings only,
// not new cleanup targets or ownership-based authorization to erase data.
const CODEX_WINDOWS_DATA_PATHS = [
  "%USERPROFILE%\\.codex",
  "%USERPROFILE%\\.agents\\skills",
  "%LOCALAPPDATA%\\Packages\\OpenAI.Codex_2p2nqsd0c76g0",
  "%LOCALAPPDATA%\\OpenAI\\Codex",
  "%LOCALAPPDATA%\\Codex\\Logs",
  "%APPDATA%\\OpenAI\\Codex",
  "%APPDATA%\\Codex",
  "%LOCALAPPDATA%\\CodexAppxDataBackups\\OpenAI.Codex_2p2nqsd0c76g0",
].join("\n");
const CCSWITCH_DATA_PATHS = [
  CCSWITCH_DATA_DISPLAY,
  "%LOCALAPPDATA%\\com.ccswitch.desktop",
  "%APPDATA%\\com.ccswitch.desktop",
].join("\n");
const DATA_PATH_STYLE = { whiteSpace: "pre-wrap", overflowWrap: "anywhere", display: "block", textAlign: "start" } as const;

export function Uninstall({
  onBack,
  bundleMode = false,
}: {
  onBack: () => void;
  /** Fixed USB build exposes a combined Codex + CCSwitch cleanup flow. */
  bundleMode?: boolean;
}) {
  const { t } = useI18n();
  const platform = currentPlatform();
  const win = platform === "windows";
  const bundledWindows = win && bundleMode;
  const codexHome = codexHomeDisplay(platform);
  // Default to keeping the user's data (~/.codex on mac, %USERPROFILE%\.codex on
  // Windows). Opting out is deliberate.
  const [keepData, setKeepData] = useState(true);
  const [purgeCodexData, setPurgeCodexData] = useState(false);
  const [purgeCcswitchData, setPurgeCcswitchData] = useState(false);
  const [cleanupPending, setCleanupPending] = useState(false);
  const [cleanupHandoffId, setCleanupHandoffId] = useState<string | null>(null);
  const [handoffBusy, setHandoffBusy] = useState(false);
  const [needsFinalCleanup, setNeedsFinalCleanup] = useState(false);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [doneDetail, setDoneDetail] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [errorDetail, setErrorDetail] = useState<string | null>(null);
  const [pathCopied, setPathCopied] = useState(false);
  // Install probe: Loading / Managed / External / None / Error — never treat a
  // status-query failure as "external".
  const [probe, setProbe] = useState<InstallProbeState>("loading");
  const [partialOutcome, setPartialOutcome] = useState<OperationOutcome | null>(null);
  const [retryBusy, setRetryBusy] = useState(false);
  // Confirmation gate: 0 = none, 1 = first confirm, 2 = data-purge confirm
  // (only reached when the user opted out of keeping data — a 3rd tap total).
  // 3 = post-uninstall purge_user_data ancillary retry confirm (destructive).
  const [confirmStep, setConfirmStep] = useState<0 | 1 | 2 | 3>(0);
  const confirm1TitleId = useId();
  const confirm1BodyId = useId();
  const confirm2TitleId = useId();
  const confirm2BodyId = useId();
  const purgeRetryTitleId = useId();
  const purgeRetryBodyId = useId();
  const keepDataTitleId = useId();
  const purgeCodexTitleId = useId();
  const purgeCcswitchTitleId = useId();

  const refreshProbe = useCallback(async () => {
    setProbe("loading");
    try {
      const s = win ? await managerApi.winStatus() : await managerApi.macStatus();
      if (s.status === "managed") setProbe("managed");
      else if (s.status === "external") setProbe("external");
      else setProbe("none");
    } catch {
      setProbe("error");
    }
  }, [win]);

  useEffect(() => {
    void refreshProbe();
  }, [refreshProbe]);

  // The helper handoff lives in the native process, not in this component.
  // Re-query it once so a renderer reload/remount cannot strand the only
  // visible exit action.
  useEffect(() => {
    if (!bundledWindows) return;
    let active = true;
    void managerApi.getUninstallHandoff().then((handoff) => {
      if (!active || !handoff) return;
      setCleanupPending(true);
      setCleanupHandoffId(handoff.id);
      setDone(t("uninstall.bundle.exiting"));
      setDoneDetail(`${handoff.message}\n${handoff.reportPath}`);
    }).catch(() => {
      // The normal uninstall action still reports its own failure. A status
      // probe is only a renderer-recovery convenience and must not replace it.
    });
    return () => {
      active = false;
    };
  }, [bundledWindows, t]);

  const run = async () => {
    setConfirmStep(0);
    setBusy(true);
    setDone(null);
    setError(null);
    setErrorDetail(null);
    setPartialOutcome(null);
    setNeedsFinalCleanup(false);
    setDoneDetail(null);
    setCleanupPending(false);
    setCleanupHandoffId(null);
    setHandoffBusy(false);
    try {
      // mac keeps ~/.codex via keepCodexHome; win purges via purgeUserData (the
      // inverse) — both surface the backend message as the source of truth.
      if (win) {
        const r = bundledWindows
          ? await managerApi.winUninstallBundle(true, purgeCodexData, purgeCcswitchData)
          : await managerApi.winUninstall(true, !keepData);
        if (!r.success || !r.outcome.primaryOk) {
          setError(t("uninstall.failed"));
          setErrorDetail([
            r.message, ...(r.notes ?? []), r.outcome.cleanup.detail, r.outcome.provenance.detail,
            ...r.outcome.warnings, ...("codexError" in r ? [r.codexError, r.ccswitchError] : []),
          ].filter(Boolean).join("\n"));
          return;
        }
        if ("cleanupPending" in r && r.cleanupPending && !outcomeNeedsAttention(r.outcome)) {
          const handoffId = "cleanupHandoffId" in r ? r.cleanupHandoffId?.trim() ?? "" : "";
          if (!handoffId) {
            setError(t("uninstall.bundle.handoffMissing"));
            setErrorDetail([
              r.message,
              "cleanupReportPath" in r ? r.cleanupReportPath : null,
            ].filter(Boolean).join("\n"));
            return;
          }
          setCleanupPending(true);
          setCleanupHandoffId(handoffId);
          setDone(t("uninstall.bundle.exiting"));
          setDoneDetail(`${r.message}\n${"cleanupReportPath" in r ? r.cleanupReportPath ?? "" : ""}`);
          return;
        }
        if (outcomeNeedsAttention(r.outcome)) {
          setNeedsFinalCleanup(bundledWindows);
          setPartialOutcome(r.outcome);
          setDone(t("uninstall.partial.summary"));
          setDoneDetail(r.message);
          return;
        }
        setDone(r.message);
      } else {
        const r = await managerApi.macUninstall(keepData);
        if (r.removed && outcomeIsPartial(r.outcome)) {
          setPartialOutcome(r.outcome);
          setDone(t("uninstall.partial.summary"));
          setDoneDetail(r.message);
          return;
        }
        if (!r.removed) {
          setError(r.message);
          return;
        }
        setDone(r.message);
      }
    } catch (cause) {
      const failure = resolveFailure(cause, t);
      setError(failure.message);
      setErrorDetail(failure.detail);
    } finally {
      setBusy(false);
    }
  };

  const retryActions = async (actions: string[]) => {
    setConfirmStep(0);
    setRetryBusy(true);
    setError(null);
    setErrorDetail(null);
    try {
      const report = await managerApi.retryAncillary({
        actions,
        path: partialOutcome?.path ?? null,
        purgeUserData: actions.includes("purge_user_data"),
      });
      const mergedOutcome = partialOutcome
        ? mergeAncillaryRetryOutcome(partialOutcome, actions, report.outcome)
        : report.outcome;
      if (outcomeNeedsAttention(mergedOutcome)) {
        setDone(t("uninstall.partial.retryPending"));
        setPartialOutcome(mergedOutcome);
      } else {
        setDone(t(bundledWindows ? "uninstall.bundle.retryContinue" : "uninstall.partial.retryDone"));
        setNeedsFinalCleanup(bundledWindows);
        setPartialOutcome(null);
      }
      setDoneDetail(report.message);
    } catch (cause) {
      const failure = resolveFailure(cause, t);
      setError(failure.message);
      setErrorDetail(failure.detail);
    } finally {
      setRetryBusy(false);
    }
  };

  const finishUninstallHandoff = async () => {
    if (!cleanupPending || !cleanupHandoffId?.trim() || handoffBusy) {
      if (cleanupPending && !cleanupHandoffId?.trim()) {
        setError(t("uninstall.bundle.handoffMissing"));
      }
      return;
    }
    setHandoffBusy(true);
    setError(null);
    setErrorDetail(null);
    try {
      // The helper is waiting for this exact manager process. Keep the result
      // visible until the user acknowledges it, then use the guarded native
      // exit path so final cleanup can proceed.
      await managerApi.completeUninstallHandoff(cleanupHandoffId);
    } catch (cause) {
      const failure = resolveFailure(cause, t);
      setError(failure.message);
      setErrorDetail(failure.detail);
      setHandoffBusy(false);
    }
  };

  // First confirm → run when keeping data; otherwise escalate to a second,
  // explicit data-erasure confirm before doing anything destructive.
  const onFirstConfirm = () => {
    if (bundledWindows ? !purgeCodexData && !purgeCcswitchData : keepData) {
      void run();
    } else {
      setConfirmStep(2);
    }
  };

  const dataPaths = codexHome;
  const selectedSharedData = purgeCodexData || purgeCcswitchData;
  const bundleSelection = (
    <div className="uninstall-selection">
      <p><strong>{t(purgeCodexData ? "uninstall.bundle.codexErase" : "uninstall.bundle.codexKeep")}</strong>
        <span className="rsub mono path" style={DATA_PATH_STYLE}>{CODEX_WINDOWS_DATA_PATHS}</span>
        {!purgeCodexData ? <span className="rsub">{t("uninstall.bundle.codexKeepNote")}</span> : null}
      </p>
      <p><strong>{t(purgeCcswitchData ? "uninstall.bundle.ccswitchErase" : "uninstall.bundle.ccswitchKeep")}</strong>
        <span className="rsub mono path" style={DATA_PATH_STYLE}>{CCSWITCH_DATA_PATHS}</span>
      </p>
      <p>{t("uninstall.bundle.mediaKeep")}</p>
    </div>
  );
  const errorBanner = error ? (
    <>
      <StatusBanner tone="err">{error}</StatusBanner>
      {errorDetail ? (
        <details className="manual-existing-meta">
          <summary>{t("home.error.details")}</summary>
          <pre className="errdetails">{errorDetail}</pre>
        </details>
      ) : null}
    </>
  ) : null;

  const copyPath = async () => {
    try {
      await navigator.clipboard.writeText(dataPaths);
      setPathCopied(true);
    } catch {
      // Best-effort affordance; the destructive flow itself is unaffected.
    }
  };

  // The normal manager refuses to remove an external/unknown Codex.  The USB
  // bundle can still clean its own CCSwitch copy when Codex is absent or
  // external, so allow the combined action after the probe has settled.
  const canUninstall = bundledWindows
    ? probe !== "loading" && probe !== "error"
    : probe === "managed";
  return (
    <div className={`pop${bundledWindows ? " uninstall-pop" : ""}`}>
      <NavBar
        title={t(bundledWindows ? "uninstall.bundle.heading" : "uninstall.heading")}
        onBack={onBack}
        disableBack={busy || retryBusy || cleanupPending || handoffBusy}
      />
      <div className="scroll view" inert={confirmStep > 0 ? true : undefined}>
        {done ? (
          <>
            <section className="hero" style={{ marginTop: 16 }}>
              <Ring
                icon={cleanupPending ? "loader" : partialOutcome || needsFinalCleanup ? "alert" : "check"}
                variant={cleanupPending || partialOutcome || needsFinalCleanup ? "amber" : "success"}
                className="pop"
              />
              <div className="headline">
                {t(bundledWindows ? "uninstall.bundle.heading" : "uninstall.heading")}
              </div>
              <div className="desc">{done}</div>
            </section>
            {doneDetail ? (
              <details className="manual-existing-meta">
                <summary>{t("home.error.details")}</summary>
                <pre className="errdetails">{doneDetail}</pre>
              </details>
            ) : null}
            {cleanupPending ? (
              <>
                <StatusBanner tone="info">{t("uninstall.bundle.pendingNote")}</StatusBanner>
                <button
                  className="btn primary big uninstall-finish-button"
                  onClick={() => void finishUninstallHandoff()}
                  disabled={handoffBusy}
                >
                  {handoffBusy ? <Icon name="loader" className="spinicon" /> : <Icon name="check" />}
                  {handoffBusy
                    ? t("uninstall.bundle.finishing")
                    : t("uninstall.bundle.finishExit")}
                </button>
              </>
            ) : null}
            {partialOutcome ? (
              <>
                <StatusBanner tone="warn">{t("uninstall.partial.title")}</StatusBanner>
                <div className="actions">
                  {partialOutcome.recoveryActions.includes("cleanup_metadata") ? (
                    <button
                      className="btn big"
                      disabled={retryBusy}
                      onClick={() => void retryActions(["cleanup_metadata"])}
                    >
                      {t("uninstall.partial.retryCleanup")}
                    </button>
                  ) : null}
                  {partialOutcome.recoveryActions.includes("clear_provenance") ? (
                    <button
                      className="btn big"
                      disabled={retryBusy}
                      onClick={() => void retryActions(["clear_provenance"])}
                    >
                      {t("uninstall.partial.retryProvenance")}
                    </button>
                  ) : null}
                  {partialOutcome.recoveryActions.includes("purge_user_data") ? (
                    <button
                      className="btn big"
                      disabled={retryBusy}
                      onClick={() => setConfirmStep(3)}
                    >
                      {t("uninstall.partial.retryPurge")}
                    </button>
                  ) : null}
                  {partialOutcome.recoveryActions.includes("record_provenance") ? (
                    <button
                      className="btn big"
                      disabled={retryBusy}
                      onClick={() => void retryActions(["record_provenance"])}
                    >
                      {t("uninstall.partial.retryRecord")}
                    </button>
                  ) : null}
                </div>
              </>
            ) : null}
            {errorBanner}
            {needsFinalCleanup ? (
              <button className="btn big" disabled={busy || retryBusy}
                onClick={() => setConfirmStep(1)}>
                {t("uninstall.bundle.continueFinalCleanup")}
              </button>
            ) : null}
            <button className="btn ghost big" onClick={onBack} disabled={busy || retryBusy || cleanupPending || handoffBusy}>
              {t("nav.back")}
            </button>
          </>
        ) : (
          <>
            <section className="hero" style={{ marginTop: 8 }}>
              <Ring icon="trash" variant="danger" />
              <div className="headline" style={{ fontSize: 18 }}>
                {t(bundledWindows ? "uninstall.bundle.heading" : "uninstall.heading")}
              </div>
              <div className="desc">
                {t(bundledWindows ? "uninstall.bundle.shortWarn" : "uninstall.warn")}
              </div>
              {bundledWindows ? (
                <Tooltip content={t("uninstall.bundle.warn")}>
                  <span className="tooltip-icon uninstall-hero-help" aria-label={t("themes.details")}>
                    <Icon name="info" />
                  </span>
                </Tooltip>
              ) : null}
            </section>

            {probe === "loading" ? (
              <StatusBanner tone="info" icon="loader">
                {t("uninstall.status.loading")}
              </StatusBanner>
            ) : null}

            {probe === "error" ? (
              <StatusBanner
                tone="err"
                action={
                  <button className="linkbtn" onClick={() => void refreshProbe()}>
                    {t("settings.retry")}
                  </button>
                }
              >
                {t("uninstall.status.error")}
              </StatusBanner>
            ) : null}

            {probe === "none" ? (
              <StatusBanner tone="info">{t("uninstall.status.none")}</StatusBanner>
            ) : null}

            {probe === "external" ? (
              <StatusBanner tone="info">
                {t(bundledWindows ? "uninstall.bundle.external" : "uninstall.needAdopt")}
              </StatusBanner>
            ) : null}

            <div className="list">
              {bundledWindows ? (
                <div className="row uninstall-compact-row">
                  <span className="rtext">
                    <span className="rtitle">{t("uninstall.bundle.cleanupScope")}</span>
                    <span className="rsub">{t("uninstall.bundle.cleanupShort")}</span>
                  </span>
                  <Tooltip content={t("uninstall.bundle.cleanupNote")}>
                    <span className="tooltip-icon" aria-label={t("themes.details")}>
                      <Icon name="info" />
                    </span>
                  </Tooltip>
                </div>
              ) : null}
              {!bundledWindows ? <div className="row">
                <span className="rtext">
                  <span className="rtitle" id={keepDataTitleId}>{t("uninstall.keepData")}</span>
                  <span className="rsub">
                    {t("uninstall.keepDataNote", { path: codexHome })}
                  </span>
                </span>
                <Toggle
                  ariaLabelledBy={keepDataTitleId}
                  checked={keepData}
                  onChange={setKeepData}
                  disabled={busy || !canUninstall}
                />
              </div> : null}
            </div>

            {bundledWindows ? (
              <div className="list">
                <div className="row uninstall-compact-row data-path-row">
                  <span className="rtext">
                    <span className="rtitle">{t("uninstall.bundle.programPath")}</span>
                    <span className="rsub mono path">{CCSWITCH_INSTALL_DISPLAY}</span>
                  </span>
                  <Tooltip content={t("uninstall.bundle.programAffects")}>
                    <span className="tooltip-icon" aria-label={t("themes.details")}>
                      <Icon name="info" />
                    </span>
                  </Tooltip>
                </div>
              </div>
            ) : null}

            {bundledWindows ? (
              <details className="manual-existing-meta uninstall-advanced">
                <summary>{t("uninstall.bundle.advanced")}</summary>
                <p className="uninstall-detail-copy">{t("uninstall.bundle.sharedRisk")}</p>
                <div className="list">
                  <div className={`row data-path-row${purgeCodexData ? " danger" : ""}`}>
                    <span className="rtext">
                      <span className="rtitle" id={purgeCodexTitleId}>{t("uninstall.bundle.purgeCodex")}</span>
                    </span>
                    <Tooltip content={`${t("uninstall.bundle.codexRisk")}\n\n${CODEX_WINDOWS_DATA_PATHS}`}>
                      <span className="tooltip-icon" aria-label={t("themes.details")}>
                        <Icon name="info" />
                      </span>
                    </Tooltip>
                    <Toggle ariaLabelledBy={purgeCodexTitleId} checked={purgeCodexData}
                      onChange={setPurgeCodexData} disabled={busy || !canUninstall} />
                  </div>
                  <div className={`row data-path-row${purgeCcswitchData ? " danger" : ""}`}>
                    <span className="rtext">
                      <span className="rtitle" id={purgeCcswitchTitleId}>{t("uninstall.bundle.purgeCcswitch")}</span>
                    </span>
                    <Tooltip content={`${t("uninstall.bundle.ccswitchRisk")}\n\n${CCSWITCH_DATA_PATHS}`}>
                      <span className="tooltip-icon" aria-label={t("themes.details")}>
                        <Icon name="info" />
                      </span>
                    </Tooltip>
                    <Toggle ariaLabelledBy={purgeCcswitchTitleId} checked={purgeCcswitchData}
                      onChange={setPurgeCcswitchData} disabled={busy || !canUninstall} />
                  </div>
                </div>
              </details>
            ) : null}

            {probe === "managed" && !bundledWindows ? (
              <div className="list">
                <div className={`row data-path-row${keepData ? "" : " danger"}`}>
                  <span className="rtext">
                    <span className="rtitle">
                      {t("uninstall.dataPath")}
                    </span>
                    <span className="rsub mono path">{codexHome}</span>
                    <span className="rsub">
                      {t("uninstall.dataAffects")}
                    </span>
                  </span>
                  <div className="install-root-actions">
                    <button className="mini-action" onClick={copyPath}>
                      <Icon name="copy" />
                      {pathCopied ? t("uninstall.pathCopied") : t("uninstall.copyPath")}
                    </button>
                    {hasTauriRuntime() ? (
                      <button
                        className="mini-action"
                        onClick={() => void managerApi.openCodexHome().catch(() => undefined)}
                      >
                        <Icon name="folder" />
                        {t("uninstall.openDir")}
                      </button>
                    ) : null}
                  </div>
                </div>
              </div>
            ) : null}

            {errorBanner}

            <div className="actions">
              <button
                className="btn danger big"
                onClick={() => setConfirmStep(1)}
                disabled={busy || !canUninstall}
              >
                <Icon name="trash" />
                {busy
                  ? t("uninstall.working")
                  : t(bundledWindows ? "uninstall.bundle.confirm" : "uninstall.confirm")}
              </button>
              <button className="btn ghost" onClick={onBack} disabled={busy}>
                {t("uninstall.cancel")}
              </button>
            </div>
          </>
        )}
      </div>

      <Sheet
        open={confirmStep === 1}
        onDismiss={() => setConfirmStep(0)}
        labelledBy={confirm1TitleId}
        describedBy={confirm1BodyId}
        initialFocus="dismiss"
      >
        <Ring icon="trash" variant="danger" />
        <h3 id={confirm1TitleId}>
          {t(bundledWindows ? "uninstall.bundle.confirm1.title" : "uninstall.confirm1.title")}
        </h3>
        <p id={confirm1BodyId}>
          {bundledWindows
            ? t("uninstall.bundle.confirm1.scope")
            : keepData
              ? t("uninstall.confirm1.bodyKeep", { path: codexHome })
              : t("uninstall.confirm1.bodyPurge")}
        </p>
        {bundledWindows ? bundleSelection : null}
        <div className="row2 sheet-actions">
          <button className="btn ghost" onClick={() => setConfirmStep(0)}>
            {t("uninstall.cancel")}
          </button>
          <button className="btn danger" onClick={onFirstConfirm} disabled={confirmStep !== 1 || busy || retryBusy}>
            {t("uninstall.continue")}
          </button>
        </div>
      </Sheet>

      <Sheet
        open={confirmStep === 2}
        onDismiss={() => setConfirmStep(0)}
        labelledBy={confirm2TitleId}
        describedBy={confirm2BodyId}
        initialFocus="dismiss"
      >
        <Ring icon="trash" variant="danger" />
        <h3 id={confirm2TitleId}>
          {t(bundledWindows ? "uninstall.bundle.confirm2.title" : "uninstall.confirm2.title")}
        </h3>
        <p id={confirm2BodyId}>
          {bundledWindows
            ? t(selectedSharedData ? "uninstall.bundle.confirm2.selected" : "uninstall.bundle.mediaKeep")
            : t("uninstall.confirm2.body", { path: codexHome })}
        </p>
        {bundledWindows ? bundleSelection : null}
        <div className="row2 sheet-actions">
          <button className="btn ghost" onClick={() => setConfirmStep(0)}>
            {t("uninstall.cancel")}
          </button>
          <button className="btn danger" onClick={() => void run()} disabled={confirmStep !== 2 || busy || retryBusy}>
            {t(bundledWindows ? "uninstall.bundle.purgeConfirm" : "uninstall.purgeConfirm")}
          </button>
        </div>
      </Sheet>

      {/* Destructive: retry purge_user_data only — same consequence copy as full purge. */}
      <Sheet
        open={confirmStep === 3}
        onDismiss={() => setConfirmStep(0)}
        labelledBy={purgeRetryTitleId}
        describedBy={purgeRetryBodyId}
        initialFocus="dismiss"
      >
        <Ring icon="trash" variant="danger" />
        <h3 id={purgeRetryTitleId}>{t(win ? "uninstall.bundle.retryPurgeTitle" : "uninstall.confirm2.title")}</h3>
        <p id={purgeRetryBodyId}>{t(win ? "uninstall.bundle.codexRisk" : "uninstall.confirm2.body", { path: codexHome })}</p>
        {win ? (
          <>
            <p className="mono path" style={DATA_PATH_STYLE}>{CODEX_WINDOWS_DATA_PATHS}</p>
            <p>{t("uninstall.bundle.retryPurgeScope")}</p>
          </>
        ) : null}
        <div className="row2 sheet-actions">
          <button className="btn ghost" onClick={() => setConfirmStep(0)}>
            {t("uninstall.cancel")}
          </button>
          <button
            className="btn danger"
            onClick={() => void retryActions(["purge_user_data"])}
            disabled={confirmStep !== 3 || busy || retryBusy}
          >
            {t("uninstall.partial.retryPurge")}
          </button>
        </div>
      </Sheet>
    </div>
  );
}
