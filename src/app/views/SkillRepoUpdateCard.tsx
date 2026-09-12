import { useId, useRef, useState } from "react";

import { managerApi } from "../../services/managerApi";
import type {
  SkillRepoApplyReport,
  SkillRepoRollbackReport,
  SkillRepoUpdateInfo,
} from "../../shared/types";
import { resolveFailure, type FailureSurface } from "../errorCopy";
import { Icon } from "../icons";
import { useI18n } from "../i18n";
import { StatusBanner } from "../components";
import { Sheet } from "../Sheet";

type BusyAction = "check" | "apply" | "rollback" | null;
const SKILL_REPO_UI_TIMEOUT_MS = 50_000;

function withTimeout<T>(operation: Promise<T>): Promise<T> {
  let timeoutId: number | null = null;
  const deadline = new Promise<T>((_, reject) => {
    timeoutId = window.setTimeout(() => {
      reject({
        code: "SKILL_REPO_TIMEOUT",
        message: "Skill Hub request timed out",
      });
    }, SKILL_REPO_UI_TIMEOUT_MS);
  });
  return Promise.race([operation, deadline]).finally(() => {
    if (timeoutId !== null) window.clearTimeout(timeoutId);
  });
}

function shortCommit(value: string | null): string {
  return value ? value.slice(0, 8) : "—";
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
    try {
      setInfo(await withTimeout(managerApi.skillRepoCheckUpdate()));
    } catch (error) {
      setFailure(resolveFailure(error, t, { sanitizeDetail: true }));
    } finally {
      finish();
    }
  };

  const apply = async () => {
    if (busyRef.current !== null || !info) return;
    if (!window.confirm(t("skillRepo.confirm"))) return;
    const expected = info.expected;
    if (!begin("apply")) return;
    setFailure(null);
    setInfo(null);
    setReport(null);
    setRollbackReport(null);
    try {
      setReport(await withTimeout(managerApi.skillRepoApplyUpdate(expected)));
    } catch (error) {
      setFailure(resolveFailure(error, t, { sanitizeDetail: true }));
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
    try {
      setRollbackReport(await withTimeout(managerApi.skillRepoRollback()));
    } catch (error) {
      setFailure(resolveFailure(error, t, { sanitizeDetail: true }));
    } finally {
      finish();
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
    const payload = {
      feature: "skill-hub",
      failure,
      check: info
        ? {
            currentCommit: info.currentCommit,
            latestCommit: info.latestCommit,
            sourceKind: info.sourceKind,
            updateAvailable: info.updateAvailable,
            installableCount: info.installableCount,
            conflictCount: info.conflictCount,
            skippedCount: info.skippedCount,
          }
        : null,
      apply: report
        ? {
            status: report.status,
            applied: report.applied,
            conflicts: report.conflicts,
            skipped: report.skipped,
            failed: report.failed,
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
              <details className="skill-repo-failure-details" data-testid="skill-repo-failure-details">
                <summary>{t("skillRepo.details")}</summary>
                <div className="skill-repo-failure-detail">
                  <div>{failure.detail ?? t("skillRepo.noDetails")}</div>
                  <div className="skill-repo-failure-code">{failure.code}</div>
                </div>
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
