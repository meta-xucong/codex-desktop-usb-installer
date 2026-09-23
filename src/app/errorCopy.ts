import { errorCode, errorDetail, errorMessage } from "../services/managerApi";
import type { TFn, TKey } from "./i18n";

/**
 * Stable, localized failure surface for every user-visible error path.
 * Raw engine/OS detail is never the primary copy — it lives in `detail`
 * for a "Show details" disclosure only.
 */
export interface FailureSurface {
  /** Stable machine code from the backend, or "unknown". */
  code: string;
  /** Localized short message for banners / hero body. */
  message: string;
  /** Raw diagnostic for disclosure only; null when nothing useful remains. */
  detail: string | null;
  /** Whether the user can reasonably retry or change a setting to recover. */
  recoverable: boolean;
}

export interface ResolveFailureOptions {
  /**
   * Network/source diagnostics can contain URLs or machine paths.  Callers
   * that expose a user-copyable remote-operation diagnostic must opt in to
   * redaction; legacy local cleanup surfaces intentionally preserve their
   * exact backend detail in the disclosure for repair work.
   */
  sanitizeDetail?: boolean;
}

const CODE_COPY: Record<string, TKey> = {
  network: "error.network",
  timeout: "error.timeout",
  disk_space: "error.disk_space",
  disk_write: "error.disk_write",
  permission: "error.permission",
  usb_authorization: "error.usbAuthorization",
  signature: "error.signature",
  bundle_integrity: "error.bundle_integrity",
  artifact: "error.artifact",
  incompatible: "error.incompatible",
  install: "error.install",
  cancelled: "progress.cancelled",
  operation_busy: "error.busy",
  stale_expectation: "home.stale.rechecked",
  // The payload is already committed; only transaction residue needs a
  // retry. Keep the diagnostic in the disclosure while offering recovery.
  post_commit_cleanup: "error.generic",
  unsupported_platform: "error.unsupported",
  internal_error: "error.generic",
  engine_error: "error.generic",
  contract_mismatch: "error.generic",
  SKILL_REPO_UNAVAILABLE: "error.network",
  SKILL_REPO_TIMEOUT: "error.timeout",
  SKILL_REPO_SOURCE_UNAVAILABLE: "error.network",
  SKILL_REPO_ALL_SOURCES_FAILED: "skillRepo.allSourcesFailed",
  SKILL_REPO_SOURCE_INVALID: "skillRepo.sourceInvalid",
  SKILL_REPO_STALE: "error.artifact",
  SKILL_REPO_HASH_MISMATCH: "error.artifact",
  SKILL_REPO_ARCHIVE_INVALID: "error.artifact",
  SKILL_REPO_BASE_CONFLICT: "error.artifact",
  SKILL_REPO_USER_CONFLICT: "error.artifact",
  SKILL_REPO_HOT_MODIFIED: "error.artifact",
  SKILL_REPO_LEDGER_INVALID: "error.artifact",
  SKILL_REPO_BASE_RESOURCE_INVALID: "skillRepo.baseResourceInvalid",
  SKILL_REPO_LOCAL_IO: "error.disk_write",
  SKILL_REPO_LOCKED: "error.busy",
  SKILL_REPO_ROLLBACK_FAILED: "error.artifact",
  SKILL_REPO_UNSUPPORTED: "error.unsupported",
};

/** Codes where a retry / settings change / closing Codex is a reasonable next step. */
const RECOVERABLE = new Set([
  "network",
  "timeout",
  "disk_space",
  "disk_write",
  "permission",
  "usb_authorization",
  "artifact",
  "bundle_integrity",
  "install",
  "cancelled",
  "operation_busy",
  "stale_expectation",
  "post_commit_cleanup",
  "engine_error",
  "internal_error",
  "contract_mismatch",
  "SKILL_REPO_UNAVAILABLE",
  "SKILL_REPO_TIMEOUT",
  "SKILL_REPO_SOURCE_UNAVAILABLE",
  "SKILL_REPO_ALL_SOURCES_FAILED",
  "SKILL_REPO_SOURCE_INVALID",
  "SKILL_REPO_STALE",
  "SKILL_REPO_HASH_MISMATCH",
  "SKILL_REPO_ARCHIVE_INVALID",
  "SKILL_REPO_BASE_CONFLICT",
  "SKILL_REPO_USER_CONFLICT",
  "SKILL_REPO_HOT_MODIFIED",
  "SKILL_REPO_LEDGER_INVALID",
  "SKILL_REPO_BASE_RESOURCE_INVALID",
  "SKILL_REPO_LOCAL_IO",
  "SKILL_REPO_LOCKED",
  "SKILL_REPO_ROLLBACK_FAILED",
  "SKILL_REPO_UNSUPPORTED",
  "unknown",
]);

/**
 * Keep the disclosure useful without turning backend transport errors into a
 * credential, local-profile, or source-repository leak. The backend log keeps
 * the stable source/result category; the UI only needs a safe explanation.
 */
export function sanitizeDiagnosticDetail(value: string): string | null {
  let detail = value.trim();
  if (!detail) return null;
  detail = detail.replace(/https?:\/\/[^\s"'<>]+/gi, "[remote source]");
  detail = detail.replace(/(?:[A-Za-z]:\\|\\\\)[^\s"']+/g, "[local path]");
  detail = detail.replace(/\/(?:Users|home|tmp|var|private|AppData)[^\s"']*/gi, "[local path]");
  detail = detail.replace(
    /((?:authorization|bearer|api[_-]?key|access[_-]?token|app[_-]?secret|password|secret)\s*[:=]\s*)[^\s,;]+/gi,
    "$1[redacted]",
  );
  detail = detail.replace(/[\r\n]+/g, " ").replace(/\s{2,}/g, " ").trim();
  if (detail.length > 1200) detail = `${detail.slice(0, 1197)}...`;
  return detail || null;
}

export function resolveFailure(cause: unknown, t: TFn, options: ResolveFailureOptions = {}): FailureSurface {
  const code = errorCode(cause) ?? "unknown";
  const raw = (errorDetail(cause) ?? errorMessage(cause)).trim();
  const key = CODE_COPY[code] ?? "error.generic";
  const message = t(key);
  // Never surface raw engine / PowerShell / OS text as primary copy. Keep it
  // only when it actually adds diagnostic value beyond the localized line.
  const detail = raw && raw !== message && raw !== code
    ? options.sanitizeDetail
      ? sanitizeDiagnosticDetail(raw)
      : raw
    : null;
  return {
    code,
    message,
    detail,
    recoverable: RECOVERABLE.has(code),
  };
}

/** Localized primary message only — safe for banners and toasts. */
export function userErrorMessage(cause: unknown, t: TFn): string {
  return resolveFailure(cause, t).message;
}

/** Build a FailureSurface from already-localized copy (no raw detail). */
export function messageFailure(
  message: string,
  code = "unknown",
  recoverable = true,
): FailureSurface {
  return { code, message, detail: null, recoverable };
}

/**
 * Add action-specific recovery copy without throwing away the backend / bridge
 * diagnostic. The contextual code is intentionally stable so callers can
 * alter controls for a known state without parsing localized prose.
 */
export function contextualFailure(
  cause: unknown,
  t: TFn,
  message: string,
  code: string,
  recoverable = true,
): FailureSurface {
  const failure = resolveFailure(cause, t);
  return { ...failure, code, message, recoverable };
}

/** True when the failure is a connectivity class (DNS / TLS / timeout). */
export function isConnectivityFailure(cause: unknown): boolean {
  const code = errorCode(cause);
  return code === "network" || code === "timeout";
}
