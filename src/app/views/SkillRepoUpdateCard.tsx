import { useId, useRef, useState } from "react";

import {
  managerApi,
  sanitizeSkillRepoAttempts,
  skillRepoAttempts,
} from "../../services/managerApi";
import type {
  SkillRepoApplyReport,
  SkillRepoItem,
  SkillRepoRollbackReport,
  SkillRepoSourceAttempt,
  SkillRepoSourceLabel,
  SkillRepoSourceKind,
  SkillRepoUpdateInfo,
} from "../../shared/types";
import { resolveFailure, type FailureSurface } from "../errorCopy";
import { Icon } from "../icons";
import { useI18n, type TFn } from "../i18n";
import { StatusBanner } from "../components";
import { Sheet } from "../Sheet";

type BusyAction = "check" | "apply" | "rollback" | null;
type RetryAction = "check" | "rollback" | null;

// The Rust command is bounded at 45 seconds plus a two-second join grace.
// Keep a small renderer-side margin so a current backend error wins the race;
// the watchdog remains a last line of defence for an old/broken runtime that
// never settles the Tauri invoke promise. It does not cancel the native
// request.
export const SKILL_REPO_CHECK_UI_TIMEOUT_MS = 50_000;

function checkSkillRepoWithTimeout(): Promise<SkillRepoUpdateInfo> {
  let timer: number | undefined;
  const backendRequest = managerApi.skillRepoCheckUpdate();
  const watchdog = new Promise<SkillRepoUpdateInfo>((_, reject) => {
    timer = window.setTimeout(() => {
      reject({
        code: "SKILL_REPO_TIMEOUT",
        message: "Skill Hub check watchdog expired",
        detail: `stage=ui-check-watchdog; timeout_ms=${SKILL_REPO_CHECK_UI_TIMEOUT_MS}`,
      });
    }, SKILL_REPO_CHECK_UI_TIMEOUT_MS);
  });
  return Promise.race([backendRequest, watchdog]).finally(() => {
    if (timer !== undefined) window.clearTimeout(timer);
  });
}

function sourceLabelForId(sourceId: unknown): SkillRepoSourceLabel | null {
  switch (sourceId) {
    case "cn-mirror-1":
      return "domestic-1";
    case "cn-mirror-2":
      return "domestic-2";
    case "cn-mirror-3":
      return "domestic-3";
    case "ph-relay":
      return "relay";
    case "github":
      return "github";
    default:
      return null;
  }
}

function sourceLabelFromValue(value: unknown): SkillRepoSourceLabel | null {
  switch (value) {
    case "domestic-1":
    case "domestic-2":
    case "domestic-3":
    case "relay":
    case "github":
      return value;
    default:
      return null;
  }
}

function safeSourceKind(value: unknown): SkillRepoSourceKind | null {
  switch (value) {
    case "domestic":
    case "relay":
    case "github":
      return value;
    default:
      return null;
  }
}

function sourceLabelCopy(sourceLabel: SkillRepoSourceLabel, t: TFn): string {
  switch (sourceLabel) {
    case "domestic-1":
      return t("skillRepo.sourceDomestic1");
    case "domestic-2":
      return t("skillRepo.sourceDomestic2");
    case "domestic-3":
      return t("skillRepo.sourceDomestic3");
    case "relay":
      return t("skillRepo.sourceRelay");
    case "github":
      return t("skillRepo.sourceGithub");
  }
}

function attemptSourceCopy(attempt: SkillRepoSourceAttempt, t: TFn): string {
  const sourceLabel = sourceLabelForId(attempt.sourceId);
  if (sourceLabel) return sourceLabelCopy(sourceLabel, t);
  return t("skillRepo.sourceUnknown");
}

function attemptPhaseCopy(attempt: SkillRepoSourceAttempt, t: TFn): string {
  switch (attempt.phase) {
    case "metadata":
      return t("skillRepo.phase.metadata");
    case "archive":
      return t("skillRepo.phase.archive");
    case "verify":
      return t("skillRepo.phase.verify");
  }
}

function attemptResultCopy(attempt: SkillRepoSourceAttempt, t: TFn): string {
  switch (attempt.result) {
    case "ok":
      return t("skillRepo.attemptResult.ok");
    case "timeout":
      return t("skillRepo.attemptResult.timeout");
    case "unavailable":
      return t("skillRepo.attemptResult.unavailable");
    case "http":
      return t("skillRepo.attemptResult.http");
    case "invalid":
      return t("skillRepo.attemptResult.invalid");
    case "hash-mismatch":
      return t("skillRepo.attemptResult.hashMismatch");
    case "cancelled":
      return t("skillRepo.attemptResult.cancelled");
  }
}

function safeDiagnosticItem(item: SkillRepoItem) {
  const id = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(item.id)
    ? item.id
    : "[redacted]";
  const code = item.code && /^SKILL_REPO_[A-Z_]+$/.test(item.code)
    ? item.code
    : null;
  const status = ["applied", "conflict", "skipped", "failed"].includes(item.status)
    ? item.status
    : "failed";
  const fileCount = Number.isInteger(item.fileCount) && item.fileCount >= 0
    ? item.fileCount
    : 0;
  return { id, status, code, fileCount };
}

function SourceAttempts({
  attempts,
  selectedSourceLabel,
  expanded,
  t,
}: {
  attempts: SkillRepoSourceAttempt[];
  selectedSourceLabel: SkillRepoSourceLabel | null;
  expanded: boolean;
  t: TFn;
}) {
  if (!attempts.length && !selectedSourceLabel) return null;
  return (
    <div className="skill-repo-source-summary" data-testid="skill-repo-source-summary">
      {selectedSourceLabel ? (
        <div className="skill-repo-source-selected">
          {t("skillRepo.sourceSelected", { source: sourceLabelCopy(selectedSourceLabel, t) })}
        </div>
      ) : null}
      {attempts.length ? (
        <details className="skill-repo-source-attempts" open={expanded}>
          <summary>{t("skillRepo.sourceAttempts", { count: attempts.length })}</summary>
          <div className="skill-repo-source-attempt-list">
            {attempts.map((attempt, index) => (
              <div
                className="skill-repo-source-attempt"
                data-result={attempt.result}
                data-testid="skill-repo-source-attempt"
                key={`${attempt.sourceId}:${attempt.phase}:${index}`}
              >
                <div className="skill-repo-source-attempt-heading">
                  <span>{attemptSourceCopy(attempt, t)}</span>
                  <span>{attemptPhaseCopy(attempt, t)}</span>
                </div>
                <div className="skill-repo-source-attempt-meta">
                  <span>{attemptResultCopy(attempt, t)}</span>
                  <span>{t("skillRepo.elapsed", { seconds: (attempt.elapsedMs / 1000).toFixed(1) })}</span>
                  {attempt.httpStatus !== null ? (
                    <span>{t("skillRepo.httpStatus", { status: attempt.httpStatus })}</span>
                  ) : null}
                </div>
              </div>
            ))}
          </div>
        </details>
      ) : null}
    </div>
  );
}

function shortCommit(value: string | null): string {
  return value && /^[0-9a-f]{40}$/i.test(value) ? value.slice(0, 8) : "—";
}

function applyTone(status: SkillRepoApplyReport["status"]): "err" | "ok" | "warn" {
  if (status === "success") return "ok";
  if (status === "failed" || status === "blocked") return "err";
  return "warn";
}

function rollbackTone(
  status: SkillRepoRollbackReport["status"],
): "err" | "ok" | "warn" {
  if (status === "success") return "ok";
  if (status === "failed") return "err";
  return "warn";
}

export function SkillRepoUpdateCard() {
  const { t } = useI18n();
  const busyRef = useRef<BusyAction>(null);
  const [busy, setBusy] = useState<BusyAction>(null);
  const [open, setOpen] = useState(false);
  const [info, setInfo] = useState<SkillRepoUpdateInfo | null>(null);
  const [report, setReport] = useState<SkillRepoApplyReport | null>(null);
  const [rollbackReport, setRollbackReport] = useState<SkillRepoRollbackReport | null>(null);
  const [failure, setFailure] = useState<FailureSurface | null>(null);
  const [sourceAttempts, setSourceAttempts] = useState<SkillRepoSourceAttempt[]>([]);
  const [selectedSourceLabel, setSelectedSourceLabel] = useState<SkillRepoSourceLabel | null>(null);
  const [retryAction, setRetryAction] = useState<RetryAction>(null);
  const [diagnosticStatus, setDiagnosticStatus] = useState<"idle" | "copied" | "failed">("idle");
  const detailsId = useId();
  const titleId = useId();
  const bodyId = useId();

  const begin = (action: Exclude<BusyAction, null>): boolean => {
    if (busyRef.current !== null) return false;
    busyRef.current = action;
    setBusy(action);
    return true;
  };

  const finish = () => {
    busyRef.current = null;
    setBusy(null);
  };

  const check = async () => {
    if (!begin("check")) return;
    setFailure(null);
    setInfo(null);
    setReport(null);
    setRollbackReport(null);
    setSourceAttempts([]);
    setSelectedSourceLabel(null);
    setRetryAction(null);
    try {
      const nextInfo = await checkSkillRepoWithTimeout();
      setInfo(nextInfo);
      setSourceAttempts(sanitizeSkillRepoAttempts(nextInfo.attempts));
      setSelectedSourceLabel(
        sourceLabelFromValue(nextInfo.sourceLabel) ?? sourceLabelForId(nextInfo.expected.sourceId),
      );
    } catch (error) {
      setSourceAttempts(skillRepoAttempts(error));
      setFailure(resolveFailure(error, t, { sanitizeDetail: true }));
      setRetryAction("check");
    } finally {
      finish();
    }
  };

  const apply = async () => {
    if (busyRef.current !== null || !info) return;
    if (!window.confirm(t("skillRepo.confirm"))) return;
    const expected = info.expected;
    if (!begin("apply")) return;
    const checkedSourceLabel = selectedSourceLabel ?? sourceLabelForId(expected.sourceId);
    setFailure(null);
    setInfo(null);
    setReport(null);
    setRollbackReport(null);
    setSourceAttempts([]);
    setSelectedSourceLabel(checkedSourceLabel);
    setRetryAction(null);
    try {
      const nextReport = await managerApi.skillRepoApplyUpdate(expected);
      setReport(nextReport);
      setSelectedSourceLabel(sourceLabelForId(nextReport.identity.sourceId) ?? checkedSourceLabel);
    } catch (error) {
      setSourceAttempts(skillRepoAttempts(error));
      setFailure(resolveFailure(error, t, { sanitizeDetail: true }));
      setRetryAction("check");
    } finally {
      finish();
    }
  };

  const rollback = async () => {
    if (busyRef.current !== null || !window.confirm(t("skillRepo.rollbackConfirm"))) return;
    if (!begin("rollback")) return;
    setFailure(null);
    setInfo(null);
    setReport(null);
    setRollbackReport(null);
    setSourceAttempts([]);
    setSelectedSourceLabel(null);
    setRetryAction(null);
    try {
      setRollbackReport(await managerApi.skillRepoRollback());
    } catch (error) {
      setFailure(resolveFailure(error, t, { sanitizeDetail: true }));
      setRetryAction("rollback");
    } finally {
      finish();
    }
  };

  const retry = () => {
    if (retryAction === "rollback") {
      void rollback();
    } else if (retryAction === "check") {
      void check();
    }
  };

  const openLogs = async () => {
    try {
      await managerApi.openLogsDir();
    } catch (error) {
      setFailure(resolveFailure(error, t, { sanitizeDetail: true }));
    }
  };

  const copyDiagnostics = async () => {
    const safeFailure = failure
      ? {
          code: failure.code,
          message: failure.message,
          detail: failure.code.startsWith("SKILL_REPO_") ? null : failure.detail,
          recoverable: failure.recoverable,
        }
      : null;
    const payload = {
      feature: "skill-hub",
      failure: safeFailure,
      sourceLabel: selectedSourceLabel,
      attempts: sourceAttempts,
      check: info
        ? {
            sourceKind: safeSourceKind(info.sourceKind),
            sourceLabel: sourceLabelFromValue(info.sourceLabel),
            currentCommitShort: shortCommit(info.currentCommit),
            latestCommitShort: shortCommit(info.latestCommit),
            attempts: sanitizeSkillRepoAttempts(info.attempts),
            updateAvailable: info.updateAvailable,
            installableCount: info.installableCount,
            conflictCount: info.conflictCount,
            skippedCount: info.skippedCount,
          }
        : null,
      apply: report
        ? {
            status: report.status,
            sourceId: sourceLabelForId(report.identity.sourceId)
              ? report.identity.sourceId
              : null,
            sourceKind: safeSourceKind(report.identity.sourceKind),
            applied: report.applied.map(safeDiagnosticItem),
            conflicts: report.conflicts.map(safeDiagnosticItem),
            skipped: report.skipped.map(safeDiagnosticItem),
            failed: report.failed.map(safeDiagnosticItem),
            reloadedRequired: report.reloadedRequired,
          }
        : null,
      rollback: rollbackReport
        ? {
            status: rollbackReport.status,
            restoredSkills: rollbackReport.restoredSkills,
            reloadedRequired: rollbackReport.reloadedRequired,
          }
        : null,
      capturedAt: new Date().toISOString(),
    };
    try {
      await navigator.clipboard.writeText(JSON.stringify(payload, null, 2));
      setDiagnosticStatus("copied");
    } catch {
      setDiagnosticStatus("failed");
    }
  };

  return (
    <>
      <section className="skill-repo-entry" data-testid="skill-repo-update-card">
        <button
          type="button"
          className="skill-repo-entry-button"
          aria-expanded={open}
          aria-controls={detailsId}
          onClick={() => setOpen(true)}
        >
          <Icon name="download" />
          <span>{t("skillRepo.entry")}</span>
          <Icon name="chevron" className="skill-repo-entry-chevron" />
        </button>
      </section>

      <Sheet
        open={open}
        onDismiss={() => {
          if (busy === null) setOpen(false);
        }}
        dismissable={busy === null}
        labelledBy={titleId}
        describedBy={bodyId}
        initialFocus="dismiss"
        centeredInExpanded
      >
        <div className="skill-repo-sheet" id={detailsId}>
          <div className="skill-repo-sheet-heading">
            <Icon name="download" />
            <h3 id={titleId}>{t("skillRepo.title")}</h3>
          </div>
          <p id={bodyId} className="skill-repo-sheet-description">
            {t("skillRepo.description")}
          </p>

          <div className="skill-repo-sheet-actions">
            <button type="button" className="btn primary" onClick={() => void check()} disabled={busy !== null}>
              {busy === "check" ? t("skillRepo.checking") : t("skillRepo.check")}
            </button>
            <button type="button" className="btn ghost" onClick={() => void rollback()} disabled={busy !== null}>
              {busy === "rollback" ? t("skillRepo.rollingBack") : t("skillRepo.rollback")}
            </button>
          </div>

          {failure ? (
            <StatusBanner tone="err">
              <div>{failure.message}</div>
              {failure.recoverable && retryAction ? <div>{t("skillRepo.retryHint")}</div> : null}
              {failure.code === "SKILL_REPO_ALL_SOURCES_FAILED" ? (
                <div data-testid="skill-repo-no-local-changes">{t("skillRepo.noLocalChanges")}</div>
              ) : null}
              <details className="skill-repo-failure-details" data-testid="skill-repo-failure-details">
                <summary>{t("skillRepo.details")}</summary>
                <div className="skill-repo-failure-detail">
                  <div>
                    {failure.detail ?? t("skillRepo.noDetails")}
                  </div>
                  <div className="skill-repo-failure-code">{failure.code}</div>
                </div>
                {failure.recoverable && retryAction ? (
                  <button type="button" className="btn primary" onClick={retry} disabled={busy !== null} data-testid="skill-repo-retry">
                    {busy !== null ? t("skillRepo.retrying") : t("skillRepo.retry")}
                  </button>
                ) : null}
                <button type="button" className="btn ghost" onClick={() => void openLogs()}>
                  {t("skillRepo.openLogs")}
                </button>
                <button type="button" className="btn ghost" onClick={() => void copyDiagnostics()}>
                  {t("skillRepo.copyDiagnostics")}
                </button>
                {diagnosticStatus === "copied" ? <div className="skill-repo-diagnostic-status">{t("skillRepo.diagnosticsCopied")}</div> : null}
                {diagnosticStatus === "failed" ? <div className="skill-repo-diagnostic-status">{t("skillRepo.diagnosticsFailed")}</div> : null}
              </details>
            </StatusBanner>
          ) : null}
          <SourceAttempts
            attempts={sourceAttempts}
            selectedSourceLabel={selectedSourceLabel}
            expanded={Boolean(failure || info)}
            t={t}
          />
          {info ? (
            <div className="skill-repo-update-summary">
              <div>{t("skillRepo.version", { current: shortCommit(info.currentCommit), latest: shortCommit(info.latestCommit) })}</div>
              <div>{t("skillRepo.counts", { installable: info.installableCount, conflicts: info.conflictCount, skipped: info.skippedCount })}</div>
              <button type="button" className="btn primary" onClick={() => void apply()} disabled={busy !== null || !info.updateAvailable}>
                {busy === "apply" ? t("skillRepo.updating") : t("skillRepo.update")}
              </button>
            </div>
          ) : null}
          {report ? (
            <StatusBanner tone={applyTone(report.status)}>
              <div data-testid="skill-repo-apply-result">
                {t("skillRepo.result", { status: report.status, applied: report.applied.length, conflicts: report.conflicts.length })}
              </div>
              <div data-testid="skill-repo-apply-counts">
                {t("skillRepo.counts", {
                  installable: report.applied.length,
                  conflicts: report.conflicts.length,
                  skipped: report.skipped.length,
                })}
              </div>
              {report.failed.length || report.conflicts.length ? (
                <details className="skill-repo-failure-details" data-testid="skill-repo-apply-details">
                  <summary>{t("skillRepo.failedItems")}</summary>
                  <div className="skill-repo-failure-detail">
                    {[...report.failed, ...report.conflicts].map((item) => (
                      <div key={`${item.id}:${item.status}`}>
                        {item.id} · {item.status}{item.code ? ` · ${item.code}` : ""}
                      </div>
                    ))}
                  </div>
                </details>
              ) : null}
            </StatusBanner>
          ) : null}
          {rollbackReport ? (
            <StatusBanner tone={rollbackTone(rollbackReport.status)}>
              {rollbackReport.status === "success"
                ? t("skillRepo.rollbackResult", { count: rollbackReport.restoredSkills.length })
                : t("error.generic")}
            </StatusBanner>
          ) : null}

          <div className="row2 sheet-actions">
            <button type="button" className="btn ghost" onClick={() => setOpen(false)} disabled={busy !== null}>
              {t("skillRepo.close")}
            </button>
          </div>
        </div>
      </Sheet>
    </>
  );
}
