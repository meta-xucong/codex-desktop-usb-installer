import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from "react";

import { errorMessage, managerApi } from "../../services/managerApi";
import type {
  GatewayModelProbeReport,
  GatewayProbeModel,
  GatewayReasoningEffortResult,
  NativeModelCatalogActionReport,
} from "../../shared/types";
import { Icon } from "../icons";
import { useI18n } from "../i18n";
import { Sheet } from "../Sheet";

function newRequestId(prefix: string) {
  return globalThis.crypto?.randomUUID?.()
    ?? `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

type TestTicket = {
  requestId: string;
  proofId: string;
  expiresAtMs: number;
  connectionFingerprint: string;
  credentialFingerprint: string;
};
type ModelRow = GatewayProbeModel & { activeOnly?: boolean };
type Availability = "passed" | "failed";
const CODEX_EFFORTS = ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const;
function reasoningEffortsForModel(modelId: string): string[] {
  return CODEX_EFFORTS.filter((effort) =>
    !(modelId.toLowerCase() === "claude-opus-5-5" || modelId.toLowerCase().endsWith("/claude-opus-5-5")) || effort !== "none",
  );
}
const PROBE_CACHE_KEY = "codex-manager.upstream-model-probe.v1";
type ProbeCache = {
  schemaVersion: 1;
  updatedAtMs: number;
  report: GatewayModelProbeReport;
  availabilityById: Record<string, Availability>;
  failedTestIds: string[];
  failureCodeById?: Record<string, string>;
};

const SAFE_PROBE_ERROR_CODE = /^[a-z0-9-]{1,64}$/;
type ProbeFailureReasonKey = "auth" | "http" | "rejected" | "noCompatibleSettings" | "network" | "timeout" | "response" | "large" | "binding" | "cancelled" | "budget" | "incomplete" | "local" | "unknown";

function safeProbeErrorCode(value: unknown): string | null {
  return typeof value === "string" && SAFE_PROBE_ERROR_CODE.test(value) ? value : null;
}

function commandProbeErrorCode(cause: unknown): string | null {
  if (cause && typeof cause === "object" && "code" in cause) {
    const code = safeProbeErrorCode((cause as { code?: unknown }).code);
    if (code) return code;
  }
  const message = cause instanceof Error
    ? cause.message
    : typeof cause === "string"
      ? cause
      : cause && typeof cause === "object" && "message" in cause
        ? String((cause as { message?: unknown }).message ?? "")
        : "";
  return message.match(/\b(upstream-[a-z-]+|probe-[a-z-]+|effort-[a-z-]+|binding-changed|engine_error|internal_error)\b/i)?.[1]?.toLowerCase() ?? null;
}

function probeFailureReasonKey(code: string | null, localFailure = false): ProbeFailureReasonKey {
  if (localFailure) return "local";
  switch (code) {
    case "upstream-auth": return "auth";
    case "upstream-http": return "http";
    case "effort-rejected": return "rejected";
    case "no-effort-accepted": return "noCompatibleSettings";
    case "upstream-unavailable": return "network";
    case "probe-timeout": return "timeout";
    case "upstream-response":
    case "invalid-response":
    case "stream-invalid":
    case "tool-correlation-invalid": return "response";
    case "response-too-large": return "large";
    case "binding-changed": return "binding";
    case "probe-cancelled": return "cancelled";
    case "probe-budget-exhausted": return "budget";
    case "effort-probe-incomplete": return "incomplete";
    case "engine_error":
    case "internal_error": return "local";
    default: return "unknown";
  }
}

function readProbeCache(): ProbeCache | null {
  try {
    const raw = localStorage.getItem(PROBE_CACHE_KEY);
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<ProbeCache>;
    const report = value.report;
    if (
      value.schemaVersion !== 1
      || !Number.isFinite(value.updatedAtMs)
      || !report
      || typeof report.status !== "string"
      || typeof report.endpoint !== "string"
      || typeof report.connectionFingerprint !== "string"
      || typeof report.credentialFingerprint !== "string"
      || !Number.isFinite(report.checkedAt)
      || !Array.isArray(report.models)
      || !Array.isArray(report.results)
      || !value.availabilityById
      || typeof value.availabilityById !== "object"
      || !Array.isArray(value.failedTestIds)
    ) return null;
    return value as ProbeCache;
  } catch {
    return null;
  }
}

function writeProbeCache(
  report: GatewayModelProbeReport,
  availabilityById: Record<string, Availability>,
  failedTestIds: string[],
  failureCodeById: Record<string, string> = {},
): number | null {
  const updatedAtMs = Date.now();
  // Persist only the displayable discovery/check results. Short-lived proof IDs
  // are deliberately excluded and must be reacquired before adding a model.
  const safeReport: GatewayModelProbeReport = {
    status: report.status,
    endpoint: report.endpoint,
    credentialFingerprint: report.credentialFingerprint,
    connectionFingerprint: report.connectionFingerprint,
    checkedAt: report.checkedAt,
    models: report.models.map(({ id, ownedBy, selectable }) => ({ id, ownedBy, selectable })),
    results: report.results.map(({ modelId, passed, generatedRequests, checks, errorCode }) => ({
      modelId,
      passed,
      generatedRequests,
      checks: checks.map(({ name, passed: checkPassed }) => ({ name, passed: checkPassed })),
      errorCode,
    })),
    generatedRequests: report.generatedRequests,
    errorCode: report.errorCode,
    reasoningModelId: report.reasoningModelId,
    reasoningResults: report.reasoningResults,
  };
  const cache: ProbeCache = {
    schemaVersion: 1,
    updatedAtMs,
    report: safeReport,
    availabilityById,
    failedTestIds,
    failureCodeById: Object.fromEntries(
      Object.entries(failureCodeById).filter(([, code]) => safeProbeErrorCode(code) !== null),
    ),
  };
  try {
    localStorage.setItem(PROBE_CACHE_KEY, JSON.stringify(cache));
    return updatedAtMs;
  } catch {
    // Keep the live probe usable when browser storage is unavailable or full,
    // but don't claim that the result was persisted.
    return null;
  }
}

export function UpstreamModelManagerSheet({
  open,
  onDismiss,
}: {
  open: boolean;
  onDismiss: () => void;
}) {
  const { t } = useI18n();
  const titleId = useId();
  const descriptionId = useId();
  const [report, setReport] = useState<GatewayModelProbeReport | null>(() => readProbeCache()?.report ?? null);
  const [activeIds, setActiveIds] = useState<string[]>([]);
  const [catalogStatus, setCatalogStatus] = useState<string | null>(null);
  const [failedTestIds, setFailedTestIds] = useState<string[]>(() => readProbeCache()?.failedTestIds ?? []);
  const [failureCodeById, setFailureCodeById] = useState<Record<string, string>>(() => readProbeCache()?.failureCodeById ?? {});
  const [availabilityById, setAvailabilityById] = useState<Record<string, Availability>>(() => readProbeCache()?.availabilityById ?? {});
  const [cacheUpdatedAtMs, setCacheUpdatedAtMs] = useState<number | null>(() => readProbeCache()?.updatedAtMs ?? null);
  const [bulkProgress, setBulkProgress] = useState<{ current: number; total: number } | null>(null);
  const [loading, setLoading] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [reasoningTestingId, setReasoningTestingId] = useState<string | null>(null);
  const [reasoningSelectionModel, setReasoningSelectionModel] = useState<string | null>(null);
  const [addingId, setAddingId] = useState<string | null>(null);
  const [resetting, setResetting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [ticketRevision, setTicketRevision] = useState(0);
  const session = useRef(0);
  const wasOpen = useRef(false);
  const requestIds = useRef(new Set<string>());
  const tickets = useRef(new Map<string, TestTicket>());
  const addRequestId = useRef<string | null>(null);

  const invalidateTicket = useCallback((modelId: string) => {
    const ticket = tickets.current.get(modelId);
    if (!ticket) return;
    tickets.current.delete(modelId);
    requestIds.current.delete(ticket.requestId);
    setTicketRevision((revision) => revision + 1);
    void managerApi.cancelGatewayModelProbe(ticket.requestId).catch(() => false);
  }, []);

  const cancelOutstanding = useCallback(async () => {
    const activeAddRequestId = addRequestId.current;
    const ids = new Set([...requestIds.current].filter((id) => id !== activeAddRequestId));
    for (const ticket of tickets.current.values()) ids.add(ticket.requestId);
    requestIds.current.clear();
    const hadTickets = tickets.current.size > 0;
    tickets.current.clear();
    if (hadTickets) setTicketRevision((revision) => revision + 1);
    await Promise.all([...ids].map((id) => managerApi.cancelGatewayModelProbe(id).catch(() => false)));
  }, []);

  const refreshCatalogState = useCallback(async (expectedSession: number) => {
    try {
      const state = await managerApi.manageNativeModelCatalog({ operation: "state" });
      if (session.current !== expectedSession) return null;
      setActiveIds(state.modelIds ?? []);
      setCatalogStatus(state.status);
      if (state.status === "orphaned") {
        setError(t("offline.nativeModelManagerOrphaned"));
        setMessage(null);
      }
      return state;
    } catch {
      if (session.current !== expectedSession) return null;
      setCatalogStatus("unknown");
      for (const modelId of tickets.current.keys()) invalidateTicket(modelId);
      setError(t("offline.nativeModelManagerConnectionChanged"));
      setMessage(null);
      return null;
    }
  }, [invalidateTicket, t]);

  const saveAvailability = useCallback((modelId: string, availability: Availability, errorCode?: string | null) => {
    const nextAvailability = { ...availabilityById, [modelId]: availability };
    const nextFailedIds = availability === "failed"
      ? (failedTestIds.includes(modelId) ? failedTestIds : [...failedTestIds, modelId])
      : failedTestIds.filter((id) => id !== modelId);
    const nextFailureCodes = { ...failureCodeById };
    if (availability === "failed" && errorCode) nextFailureCodes[modelId] = errorCode;
    else delete nextFailureCodes[modelId];
    setAvailabilityById(nextAvailability);
    setFailedTestIds(nextFailedIds);
    setFailureCodeById(nextFailureCodes);
    if (report) setCacheUpdatedAtMs(writeProbeCache(report, nextAvailability, nextFailedIds, nextFailureCodes));
  }, [availabilityById, failedTestIds, failureCodeById, report]);

  const clearAvailability = useCallback((modelId: string) => {
    const nextAvailability = { ...availabilityById };
    delete nextAvailability[modelId];
    const nextFailedIds = failedTestIds.filter((id) => id !== modelId);
    const nextFailureCodes = { ...failureCodeById };
    delete nextFailureCodes[modelId];
    setAvailabilityById(nextAvailability);
    setFailedTestIds(nextFailedIds);
    setFailureCodeById(nextFailureCodes);
    if (report) setCacheUpdatedAtMs(writeProbeCache(report, nextAvailability, nextFailedIds, nextFailureCodes));
  }, [availabilityById, failedTestIds, failureCodeById, report]);

  const discover = useCallback(async () => {
    if (loading || testingId || reasoningTestingId || addingId || resetting || addRequestId.current) return;
    const currentSession = ++session.current;
    setLoading(true);
    setTestingId(null);
    setAddingId(null);
    setError(null);
    setMessage(null);
    setBulkProgress(null);
    let requestId: string | null = null;
    try {
      await cancelOutstanding();
      if (session.current !== currentSession) return;

      requestId = newRequestId("upstream-discovery");
      requestIds.current.add(requestId);
      const [state, next] = await Promise.all([
        managerApi.manageNativeModelCatalog({ operation: "state" }),
        managerApi.gatewayModelProbe({
          requestId,
          selectedModelIds: [],
          costConsent: false,
        }),
      ]);
      if (session.current !== currentSession) return;
      setActiveIds(state.modelIds ?? []);
      setCatalogStatus(state.status);
      if (state.status === "orphaned") setError(t("offline.nativeModelManagerOrphaned"));
      if (next.status === "connected") {
        setReport(next);
        setFailedTestIds([]);
        setAvailabilityById({});
        setFailureCodeById({});
        setCacheUpdatedAtMs(writeProbeCache(next, {}, []));
      } else {
        if (!report) setReport(next);
        setError(state.status === "orphaned" ? t("offline.nativeModelManagerOrphaned") : t("offline.nativeModelManagerError"));
      }
    } catch {
      if (session.current !== currentSession) return;
      setError(t("offline.nativeModelManagerError"));
    } finally {
      if (requestId) requestIds.current.delete(requestId);
      if (session.current === currentSession) setLoading(false);
    }
  }, [addingId, cancelOutstanding, loading, reasoningTestingId, report, resetting, t, testingId]);

  useLayoutEffect(() => {
    if (!open && wasOpen.current) {
      wasOpen.current = false;
      session.current += 1;
      setLoading(false);
      setTestingId(null);
      setBulkProgress(null);
      if (!addRequestId.current) setAddingId(null);
      setResetting(false);
      void cancelOutstanding();
    }
  }, [cancelOutstanding, open]);

  useEffect(() => {
    if (!open || wasOpen.current) return;
    wasOpen.current = true;
    const currentSession = ++session.current;
    const cache = readProbeCache();
    if (cache) {
      setReport(cache.report);
      setAvailabilityById(cache.availabilityById);
      setFailedTestIds(cache.failedTestIds);
      setFailureCodeById(cache.failureCodeById ?? {});
      setCacheUpdatedAtMs(cache.updatedAtMs);
      setError(null);
      setMessage(null);
      void refreshCatalogState(currentSession);
    } else {
      void discover();
    }
  }, [discover, open, refreshCatalogState]);

  useEffect(() => {
    if (!open || tickets.current.size === 0) return;
    const currentSession = session.current;
    let disposed = false;
    const validateTicketContext = async () => {
      const now = Date.now();
      let invalidated = false;
      for (const [modelId, ticket] of tickets.current) {
        if (ticket.expiresAtMs <= now) {
          invalidateTicket(modelId);
          invalidated = true;
        }
      }
      if (tickets.current.size === 0 || disposed) {
        if (invalidated && session.current === currentSession) {
          setError(t("offline.nativeModelManagerTestExpired"));
        }
        return;
      }
      try {
        const state = await managerApi.manageNativeModelCatalog({ operation: "state" });
        if (disposed || session.current !== currentSession) return;
        setCatalogStatus(state.status);
        for (const [modelId, ticket] of tickets.current) {
          if (
            state.status !== "ok"
            || state.connectionFingerprint !== ticket.connectionFingerprint
            || state.credentialFingerprint !== ticket.credentialFingerprint
          ) {
            invalidateTicket(modelId);
            invalidated = true;
          }
        }
        if (invalidated) {
          setError(t("offline.nativeModelManagerConnectionChanged"));
          setMessage(null);
        }
      } catch {
        if (disposed || session.current !== currentSession) return;
        for (const modelId of tickets.current.keys()) invalidateTicket(modelId);
        setError(t("offline.nativeModelManagerConnectionChanged"));
        setMessage(null);
      }
    };
    const onFocus = () => void validateTicketContext();
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") void validateTicketContext();
    };
    const interval = window.setInterval(() => void validateTicketContext(), 10_000);
    const earliestExpiry = Math.min(...[...tickets.current.values()].map((ticket) => ticket.expiresAtMs));
    const expiryTimer = window.setTimeout(() => void validateTicketContext(), Math.max(0, earliestExpiry - Date.now() + 20));
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisibilityChange);
    void validateTicketContext();
    return () => {
      disposed = true;
      window.clearInterval(interval);
      window.clearTimeout(expiryTimer);
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [invalidateTicket, open, t, ticketRevision]);

  const testModel = useCallback(async (model: ModelRow) => {
    if (loading || testingId || reasoningTestingId || addingId || resetting || addRequestId.current || catalogStatus !== "ok" || !model.selectable || model.activeOnly) return;
    const previous = tickets.current.get(model.id);
    if (previous) {
      invalidateTicket(model.id);
    }
    const currentSession = session.current;
    const requestId = newRequestId("upstream-connectivity");
    requestIds.current.add(requestId);
    setTestingId(model.id);
    setFailedTestIds((ids) => ids.filter((id) => id !== model.id));
    setFailureCodeById((codes) => { const next = { ...codes }; delete next[model.id]; return next; });
    setAvailabilityById((current) => { const next = { ...current }; delete next[model.id]; return next; });
    setError(null);
    setMessage(null);
    try {
      const next = await managerApi.gatewayModelProbe({
        requestId,
        selectedModelIds: [],
        costConsent: false,
        connectivityModelId: model.id,
      });
      if (session.current !== currentSession) return;
      const result = next.results.find((item) => item.modelId === model.id);
      const connectionMatchesDiscovery = report
        && next.connectionFingerprint === report.connectionFingerprint
        && next.credentialFingerprint === report.credentialFingerprint
        && next.endpoint === report.endpoint;
      if (next.status === "passed" && result?.passed && connectionMatchesDiscovery) {
        saveAvailability(model.id, "passed");
        setMessage(t("offline.nativeModelTestPassed", { model: model.id }));
      } else {
        const errorCode = next.status === "passed" && !connectionMatchesDiscovery
          ? "binding-changed"
          : safeProbeErrorCode(result?.errorCode ?? next.errorCode);
        saveAvailability(model.id, "failed", errorCode);
        requestIds.current.delete(requestId);
        if (next.connectivityProofId) {
          await managerApi.cancelGatewayModelProbe(requestId).catch(() => false);
        }
        if (next.status === "passed" && !connectionMatchesDiscovery) {
          setError(t("offline.nativeModelManagerConnectionChanged"));
          return;
        }
        const reason = t(`offline.nativeModelProbeFailureReason.${probeFailureReasonKey(errorCode)}` as const);
        setError(t("offline.nativeModelProbeFailure", {
          reason,
          code: errorCode ?? t("offline.nativeModelProbeFailureNoCode"),
        }));
      }
    } catch (cause) {
      if (session.current === currentSession) {
        const errorCode = commandProbeErrorCode(cause);
        saveAvailability(model.id, "failed", errorCode);
        const reason = t(`offline.nativeModelProbeFailureReason.${probeFailureReasonKey(errorCode, true)}` as const);
        setError(t("offline.nativeModelProbeFailure", {
          reason,
          code: errorCode ?? t("offline.nativeModelProbeFailureNoCode"),
        }));
      }
      requestIds.current.delete(requestId);
    } finally {
      if (session.current === currentSession) setTestingId(null);
    }
  }, [addingId, catalogStatus, invalidateTicket, loading, reasoningTestingId, report, resetting, saveAvailability, t, testingId]);

  const testReasoningEfforts = useCallback(async (model: ModelRow, efforts: string[]) => {
    if (loading || testingId || reasoningTestingId || addingId || resetting || addRequestId.current || catalogStatus !== "ok" || !model.selectable || model.activeOnly) return;
    if (efforts.length === 0) return;
    if (!window.confirm(t("offline.nativeModelReasoningConfirm", { count: efforts.length, model: model.id }))) return;
    invalidateTicket(model.id);
    const currentSession = session.current;
    const requestId = newRequestId("upstream-reasoning");
    requestIds.current.add(requestId);
    setReasoningTestingId(model.id);
    setError(null);
    setMessage(null);
    try {
      const next = await managerApi.gatewayModelProbe({
        requestId,
        selectedModelIds: [],
        costConsent: false,
        reasoningModelId: model.id,
        reasoningEfforts: efforts,
        reasoningCostConsent: true,
      });
      if (session.current !== currentSession) return;
      const proofLifetimeMs = ((next.connectivityProofExpiresAt ?? 0) - next.checkedAt) * 1000;
      const connectionMatchesDiscovery = report
        && next.connectionFingerprint === report.connectionFingerprint
        && next.credentialFingerprint === report.credentialFingerprint
        && next.endpoint === report.endpoint;
      if (connectionMatchesDiscovery) {
        setReport(next);
        setCacheUpdatedAtMs(writeProbeCache(next, availabilityById, failedTestIds, failureCodeById));
      }
      if (next.status === "passed" && next.reasoningProofId && next.connectivityProofId && proofLifetimeMs > 0 && connectionMatchesDiscovery) {
        tickets.current.set(model.id, {
          requestId,
          proofId: next.reasoningProofId,
          expiresAtMs: Date.now() + proofLifetimeMs,
          connectionFingerprint: next.connectionFingerprint,
          credentialFingerprint: next.credentialFingerprint,
        });
        setTicketRevision((revision) => revision + 1);
        setMessage(null);
      } else {
        if (next.reasoningProofId) await managerApi.cancelGatewayModelProbe(requestId).catch(() => false);
        if (next.status === "passed" && !connectionMatchesDiscovery) setError(t("offline.nativeModelManagerConnectionChanged"));
        else {
          const errorCode = safeProbeErrorCode(
            next.errorCode === "no-effort-accepted"
              ? next.errorCode
              : next.reasoningResults?.find((row) => row.errorCode)?.errorCode ?? next.errorCode,
          );
          const reason = t(`offline.nativeModelProbeFailureReason.${probeFailureReasonKey(errorCode)}` as const);
          setError(t("offline.nativeModelProbeFailure", {
            reason,
            code: errorCode ?? t("offline.nativeModelProbeFailureNoCode"),
          }));
        }
      }
    } catch (cause) {
      if (session.current === currentSession) {
        const errorCode = commandProbeErrorCode(cause);
        const reason = t(`offline.nativeModelProbeFailureReason.${probeFailureReasonKey(errorCode, true)}` as const);
        setError(t("offline.nativeModelProbeFailure", {
          reason,
          code: errorCode ?? t("offline.nativeModelProbeFailureNoCode"),
        }));
      }
    } finally {
      requestIds.current.delete(requestId);
      if (session.current === currentSession) setReasoningTestingId(null);
    }
  }, [addingId, availabilityById, catalogStatus, failedTestIds, failureCodeById, invalidateTicket, loading, reasoningTestingId, report, resetting, t, testingId]);

  const testAllModels = useCallback(async () => {
    if (loading || testingId || reasoningTestingId || addingId || resetting || addRequestId.current || catalogStatus !== "ok" || !report) return;
    const models = report.models.filter((model) => model.selectable);
    if (models.length === 0) return;
    if (!window.confirm(t("offline.nativeModelManagerTestAllConfirm", { count: models.length }))) return;

    const currentSession = session.current;
    let passed = 0;
    let failed = 0;
    let stoppedForConnectionChange = false;
    let nextAvailability = { ...availabilityById };
    let nextFailedIds = [...failedTestIds];
    const nextFailureCodes = { ...failureCodeById };
    setError(null);
    setMessage(null);
    setBulkProgress({ current: 0, total: models.length });
    for (let index = 0; index < models.length; index += 1) {
      if (session.current !== currentSession) return;
      const model = models[index];
      const requestId = newRequestId("upstream-bulk-connectivity");
      requestIds.current.add(requestId);
      setTestingId(model.id);
      setBulkProgress({ current: index + 1, total: models.length });
      let available = false;
      let connectionChanged = false;
      let failureCode: string | null = null;
      try {
        const next = await managerApi.gatewayModelProbe({
          requestId,
          selectedModelIds: [],
          costConsent: false,
          connectivityModelId: model.id,
        });
        if (session.current !== currentSession) return;
        const result = next.results.find((item) => item.modelId === model.id);
        const connectionMatchesDiscovery = next.connectionFingerprint === report.connectionFingerprint
          && next.credentialFingerprint === report.credentialFingerprint
          && next.endpoint === report.endpoint;
        available = next.status === "passed" && result?.passed === true && connectionMatchesDiscovery;
        connectionChanged = !connectionMatchesDiscovery;
        if (!available && !connectionChanged) failureCode = safeProbeErrorCode(result?.errorCode ?? next.errorCode);
        if (connectionChanged) setError(t("offline.nativeModelManagerConnectionChanged"));
      } catch (cause) {
        if (session.current !== currentSession) return;
        failureCode = commandProbeErrorCode(cause);
      } finally {
        requestIds.current.delete(requestId);
        await managerApi.cancelGatewayModelProbe(requestId).catch(() => false);
      }
      if (session.current !== currentSession) return;
      nextAvailability = { ...nextAvailability, [model.id]: available ? "passed" : "failed" };
      nextFailedIds = available
        ? nextFailedIds.filter((id) => id !== model.id)
        : nextFailedIds.includes(model.id) ? nextFailedIds : [...nextFailedIds, model.id];
      if (available || connectionChanged || !failureCode) delete nextFailureCodes[model.id];
      else nextFailureCodes[model.id] = failureCode;
      setAvailabilityById(nextAvailability);
      setFailedTestIds(nextFailedIds);
      setFailureCodeById(nextFailureCodes);
      setCacheUpdatedAtMs(writeProbeCache(report, nextAvailability, nextFailedIds, nextFailureCodes));
      if (available) passed += 1;
      else failed += 1;
      if (connectionChanged) {
        stoppedForConnectionChange = true;
        break;
      }
    }
    if (session.current === currentSession) {
      setTestingId(null);
      setBulkProgress(null);
      if (!stoppedForConnectionChange) {
        setMessage(t("offline.nativeModelManagerTestAllDone", { passed, failed }));
      }
    }
  }, [addingId, availabilityById, catalogStatus, failedTestIds, failureCodeById, loading, reasoningTestingId, report, resetting, t, testingId]);

  const addModel = useCallback(async (model: ModelRow) => {
    const ticket = tickets.current.get(model.id);
    if (!ticket || addingId || testingId || resetting || loading || addRequestId.current || model.activeOnly) return;
    if (
      ticket.expiresAtMs <= Date.now()
      || ticket.connectionFingerprint !== report?.connectionFingerprint
      || ticket.credentialFingerprint !== report?.credentialFingerprint
      || catalogStatus !== "ok"
    ) {
      invalidateTicket(model.id);
      setError(t("offline.nativeModelManagerTestExpired"));
      return;
    }
    const requestId = newRequestId("upstream-add");
    addRequestId.current = requestId;
    requestIds.current.add(requestId);
    setAddingId(model.id);
    setError(null);
    setMessage(null);
    try {
      const result = await managerApi.manageNativeModelCatalog({
        operation: "add",
        requestId,
        modelId: model.id,
        connectivityProofId: ticket.proofId,
      });
      const removedTicket = tickets.current.delete(model.id);
      if (removedTicket) setTicketRevision((revision) => revision + 1);
      requestIds.current.delete(ticket.requestId);
      setActiveIds(result.modelIds ?? []);
      setMessage(null);
      await managerApi.cancelGatewayModelProbe(ticket.requestId).catch(() => false);
      const confirmedStatuses = new Set(["applied_restart_pending", "already_added"]);
      const addWasReportedSuccessful = confirmedStatuses.has(result.status)
        && result.modelIds?.includes(model.id) === true;
      const refreshedState = await refreshCatalogState(session.current);
      if (result.status === "cleanup_pending") {
        clearAvailability(model.id);
        setError(t("offline.nativeModelManagerAddCleanupPending"));
      } else if (
        result.status === "outcome_unconfirmed"
        || !addWasReportedSuccessful
        || !refreshedState?.modelIds?.includes(model.id)
      ) {
        clearAvailability(model.id);
        setError(t("offline.nativeModelManagerAddUnconfirmed"));
      } else {
        setError(null);
        setMessage(t("offline.nativeModelManagerAdded"));
        setReasoningSelectionModel(null);
      }
    } catch (cause) {
      setError(errorMessage(cause) || t("offline.nativeModelManagerError"));
      // A successful connectivity probe is not evidence that the Add commit
      // succeeded. Clear and persist that stale badge when the write fails so
      // the row cannot fall back to a misleading "available" state.
      clearAvailability(model.id);
      const removedTicket = tickets.current.delete(model.id);
      if (removedTicket) setTicketRevision((revision) => revision + 1);
      requestIds.current.delete(ticket.requestId);
      await managerApi.cancelGatewayModelProbe(ticket.requestId).catch(() => false);
      await refreshCatalogState(session.current);
    } finally {
      requestIds.current.delete(requestId);
      if (addRequestId.current === requestId) addRequestId.current = null;
      setAddingId(null);
    }
  }, [addingId, catalogStatus, clearAvailability, invalidateTicket, loading, refreshCatalogState, report, resetting, t, testingId]);

  const resetCatalog = useCallback(async () => {
    if (resetting || loading || testingId || reasoningTestingId || addingId || addRequestId.current) return;
    const currentSession = ++session.current;
    setResetting(true);
    setLoading(false);
    setTestingId(null);
    setAddingId(null);
    setError(null);
    setMessage(null);
    await cancelOutstanding();
    try {
      const result: NativeModelCatalogActionReport = await managerApi.manageNativeModelCatalog({ operation: "reset" });
      if (session.current !== currentSession) return;
      setActiveIds(result.modelIds ?? []);
      const resultMessage = result.status === "cleanup_pending"
        ? t("offline.nativeModelManagerResetCleanupPending")
        : result.status === "outcome_unconfirmed"
          ? t("offline.nativeModelManagerResetUnconfirmed")
          : result.status === "already_native"
            ? t("offline.nativeModelManagerResetAlready")
            : t("offline.nativeModelManagerResetDone");
      if (result.status === "cleanup_pending" || result.status === "outcome_unconfirmed") {
        setMessage(null);
        setError(resultMessage);
      } else {
        setMessage(resultMessage);
      }
      await refreshCatalogState(currentSession);
    } catch {
      if (session.current === currentSession) setError(t("offline.nativeModelManagerError"));
      await refreshCatalogState(currentSession);
    } finally {
      if (session.current === currentSession) setResetting(false);
    }
  }, [addingId, cancelOutstanding, loading, reasoningTestingId, refreshCatalogState, resetting, t, testingId]);

  const rows: ModelRow[] = report?.models.map((model) => ({ ...model })) ?? [];
  const activeOnlyRows: ModelRow[] = report?.status === "connected"
    ? activeIds
      .filter((id) => !rows.some((row) => row.id === id))
      .map((id) => ({ id, ownedBy: null, selectable: false, activeOnly: true }))
    : [];
  const catalogActionReason = catalogStatus === "orphaned"
    ? t("offline.nativeModelManagerOrphaned")
    : catalogStatus !== "ok"
      ? t("offline.nativeModelManagerCatalogUnavailable")
      : null;
  const catalogActionReasonId = `${titleId}-catalog-action-reason`;
  const managerTooltip = `${t("offline.nativeModelManagerSteps")}\n\n${t("offline.nativeModelManagerCostWarning")}`;
  const operationBusy = Boolean(loading || testingId || reasoningTestingId || addingId || resetting || addRequestId.current || bulkProgress);

  return (
    <Sheet
      open={open}
      onDismiss={onDismiss}
      labelledBy={titleId}
      describedBy={descriptionId}
      initialFocus="first"
      centeredInExpanded
    >
      <div className="upstream-model-manager" data-testid="upstream-model-manager">
        <div className="upstream-model-manager-topbar">
          <button type="button" className="btn ghost upstream-model-manager-back" onClick={onDismiss}>
            <Icon name="back" />
            {t("offline.nativeModelManagerBack")}
          </button>
          <button type="button" className="icon-btn" onClick={onDismiss} aria-label={t("nav.close")}>
            <Icon name="close" />
          </button>
        </div>
        <div className="upstream-model-manager-heading">
          <div>
            <h3 id={titleId}>{t("offline.nativeModelManagerTitle")}</h3>
            <p id={descriptionId} title={managerTooltip}>{t("offline.nativeModelManagerDescription")}</p>
          </div>
        </div>

        <div className="upstream-model-manager-toolbar">
          <button
            type="button"
            className="btn primary upstream-model-manager-compact-action"
            onClick={() => void discover()}
            disabled={operationBusy}
            title={t("offline.nativeModelManagerSteps")}
          >
            {loading ? <Icon name="loader" className="spinicon" /> : <Icon name="refresh" />}
            {loading ? t("offline.nativeModelManagerDiscovering") : t("offline.nativeModelManagerRefresh")}
          </button>
          <button
            type="button"
            className="btn ghost upstream-model-manager-compact-action"
            onClick={() => void testAllModels()}
            disabled={operationBusy || !report || report.models.every((model) => !model.selectable) || catalogStatus !== "ok"}
            title={t("offline.nativeModelManagerCostWarning")}
          >
            {bulkProgress
              ? t("offline.nativeModelManagerTestAllProgress", bulkProgress)
              : t("offline.nativeModelManagerTestAll")}
          </button>
        </div>

        {report && cacheUpdatedAtMs !== null ? (
          <small
            className="upstream-model-manager-cache"
            data-testid="upstream-model-cache-notice"
            title={t("offline.nativeModelManagerCacheNotice", {
              time: new Date(cacheUpdatedAtMs).toLocaleString(),
            })}
          >
            {t("offline.nativeModelManagerCacheNotice", {
              time: new Date(cacheUpdatedAtMs).toLocaleString(),
            })}
          </small>
        ) : null}

        {catalogActionReason ? (
          <p
            id={catalogActionReasonId}
            role={catalogStatus === "orphaned" ? "alert" : "status"}
            className="upstream-model-manager-error"
          >
            {catalogActionReason}
          </p>
        ) : null}
        {rows.length > 0 ? (
          <ul className="upstream-model-manager-list" data-testid="upstream-model-candidate-list">
            {rows.map((model, index) => {
              const isActive = activeIds.includes(model.id);
              const ticket = tickets.current.get(model.id);
              const duplicate = !model.selectable && !model.activeOnly;
              const availability = availabilityById[model.id];
              const failed = availability === "failed" || failedTestIds.includes(model.id);
              const failureCode = safeProbeErrorCode(failureCodeById[model.id]);
              const failureReason = t(`offline.nativeModelProbeFailureReason.${probeFailureReasonKey(failureCode, failed && !failureCode)}` as const);
              const rowActionReason = duplicate
                ? t("offline.gatewayProbeDuplicateId")
                : operationBusy
                  ? t("offline.nativeModelManagerBusy")
                  : null;
              const actionStatus = model.activeOnly
                ? t("offline.nativeModelManagerMissingUpstream")
                : addingId === model.id
                  ? t("offline.nativeModelManagerAdding")
                  : testingId === model.id
                    ? t("offline.nativeModelManagerTesting")
                    : reasoningTestingId === model.id
                      ? t("offline.nativeModelReasoningTesting")
                    : isActive
                      ? t("offline.nativeModelManagerActive")
                      : availability === "passed"
                        ? t("offline.nativeModelManagerAvailabilityPassed")
                        : failed
                          ? (failureCode
                            ? t(`offline.nativeModelProbeFailureShort.${probeFailureReasonKey(failureCode)}` as const)
                            : t("offline.nativeModelManagerTestFailedStatus"))
                          : duplicate
                            ? t("offline.gatewayProbeDuplicateId")
                            : ticket
                              ? t("offline.nativeModelManagerTestPassed")
                              : t("offline.nativeModelManagerNotTested");
              const reasonId = `${titleId}-action-reason-${index}`;
              const actionDisabled = Boolean(catalogActionReason || rowActionReason);
              const actionReasonId = catalogActionReason
                ? catalogActionReasonId
                : rowActionReason
                  ? reasonId
                  : undefined;
              return (
                <li key={`${model.id}:${model.ownedBy ?? ""}:${index}`} data-testid={`upstream-model-${model.id}`}>
                  <div className="upstream-model-manager-model">
                    <strong
                      className="upstream-model-manager-model-id"
                      title={model.ownedBy ? `${model.id} · ${model.ownedBy}` : model.id}
                      dir="ltr"
                    >
                      {model.id}
                    </strong>
                    <small
                      className="upstream-model-manager-row-status"
                      title={failed ? `${actionStatus} · ${failureReason}${failureCode ? ` (${failureCode})` : ""}` : actionStatus}
                    >
                      {actionStatus}
                    </small>
                  </div>
                  <div className="upstream-model-manager-row-actions">
                    {!isActive && !model.activeOnly ? (
                      <>
                        {ticket ? (
                          <button
                            type="button"
                            className="btn primary upstream-model-manager-compact-action"
                            onClick={() => {
                              setReasoningSelectionModel(model.id);
                              setError(null);
                              setMessage(null);
                            }}
                            disabled={actionDisabled}
                            aria-describedby={actionReasonId}
                            title={t("offline.nativeModelManagerAdd")}
                          >
                            {t("offline.nativeModelManagerAdd")}
                          </button>
                        ) : (
                          <button
                            type="button"
                            className="btn ghost upstream-model-manager-compact-action"
                            onClick={() => setReasoningSelectionModel((current) => current === model.id ? null : model.id)}
                            disabled={actionDisabled}
                            aria-describedby={actionReasonId}
                            title={t("offline.nativeModelPrepareAddHelp")}
                            aria-label={t("offline.nativeModelManagerPrepareAdd")}
                          >
                            {t("offline.nativeModelManagerPrepareAdd")}
                          </button>
                        )}
                        <button
                            type="button"
                            className="btn ghost upstream-model-manager-compact-action"
                            onClick={() => void testModel(model)}
                            disabled={actionDisabled}
                            aria-describedby={actionReasonId}
                            title={t("offline.nativeModelManagerTest")}
                            aria-label={t("offline.nativeModelManagerTest")}
                          >
                            {testingId === model.id ? t("offline.nativeModelManagerTesting") : t("offline.nativeModelManagerTestShort")}
                        </button>
                        {rowActionReason ? (
                          <small id={reasonId} className="sr-only">
                            {rowActionReason}
                          </small>
                        ) : null}
                      </>
                    ) : (
                      <span className="upstream-model-manager-row-status-spacer" aria-hidden="true" />
                    )}
                  </div>
                  {!isActive && !model.activeOnly && reasoningSelectionModel === model.id ? (
                    <div className="upstream-model-manager-add-panel">
                      <div className="upstream-model-manager-add-heading">
                        <div>
                          <strong>{ticket ? t("offline.nativeModelManagerConfirmTitle") : t("offline.nativeModelManagerPrepareTitle")}</strong>
                          <span className="upstream-model-manager-add-model" dir="ltr">{model.id}</span>
                        </div>
                        <button
                          type="button"
                          className="icon-btn"
                          aria-label={t("confirm.cancel")}
                          onClick={() => setReasoningSelectionModel(null)}
                          disabled={Boolean(addingId)}
                        >
                          <Icon name="close" />
                        </button>
                      </div>
                      {ticket ? (
                        <>
                          <p>{t("offline.nativeModelManagerConfirmDescription")}</p>
                          {error ? <p role="alert" className="upstream-model-manager-error">{error}</p> : null}
                          {report?.reasoningModelId === model.id ? (
                            <p className="upstream-model-manager-add-ready" role="status">
                              {t("offline.nativeModelReasoningDone", {
                                accepted: report.reasoningResults?.filter((row) => row.status === "accepted_by_route").length ?? 0,
                              })}
                            </p>
                          ) : null}
                          <div className="upstream-model-manager-add-footer">
                            <button
                              type="button"
                              className="btn ghost upstream-model-manager-compact-action"
                              onClick={() => setReasoningSelectionModel(null)}
                              disabled={Boolean(addingId)}
                            >
                              {t("confirm.cancel")}
                            </button>
                            <button
                              type="button"
                              className="btn primary upstream-model-manager-compact-action"
                              onClick={() => void addModel(model)}
                              disabled={operationBusy || actionDisabled}
                            >
                              {addingId === model.id ? t("offline.nativeModelManagerAdding") : t("offline.nativeModelManagerConfirmAdd")}
                            </button>
                          </div>
                        </>
                      ) : (
                        <>
                          <p>{t("offline.nativeModelPrepareAddHelp")}</p>
                          {error ? <p role="alert" className="upstream-model-manager-error">{error}</p> : null}
                          <div className="upstream-model-manager-add-footer">
                            <button
                              type="button"
                              className="btn ghost upstream-model-manager-compact-action"
                              onClick={() => setReasoningSelectionModel(null)}
                              disabled={Boolean(reasoningTestingId)}
                            >
                              {t("confirm.cancel")}
                            </button>
                            <button
                              type="button"
                              className="btn primary upstream-model-manager-compact-action"
                              disabled={operationBusy}
                              onClick={() => void testReasoningEfforts(model, reasoningEffortsForModel(model.id))}
                            >
                              {reasoningTestingId === model.id
                                ? t("offline.nativeModelReasoningTesting")
                                : t("offline.nativeModelReasoningStart")}
                            </button>
                          </div>
                        </>
                      )}
                      {report?.reasoningModelId === model.id && report.reasoningResults?.length ? (
                        <details className="upstream-model-manager-reasoning-details">
                          <summary>{t("offline.nativeModelReasoningResults")}</summary>
                          <ul className="upstream-model-manager-reasoning-results" aria-label={t("offline.nativeModelReasoningResults")}>
                            {report.reasoningResults.map((result: GatewayReasoningEffortResult) => (
                              <li key={result.effort}>
                                <span>{result.effort}</span>
                                <span className={`reasoning-result-${result.status}`}>
                                  <span title={result.errorCode === "effort-probe-incomplete" ? t("offline.nativeModelReasoningIncomplete") : result.errorCode ?? undefined}>
                                    {t(`offline.nativeModelReasoningStatus.${result.status}` as const)}
                                  </span>
                                </span>
                              </li>
                            ))}
                          </ul>
                        </details>
                      ) : null}
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        ) : loading ? (
          <p>{t("offline.nativeModelManagerDiscovering")}</p>
        ) : report ? (
          <p>{t("offline.nativeModelManagerEmpty")}</p>
        ) : null}

        {activeOnlyRows.length > 0 ? (
          <details className="upstream-model-manager-local-only">
            <summary>{t("offline.nativeModelManagerLocalOnlyHeading")}</summary>
            <p>{t("offline.nativeModelManagerLocalOnlyNotice")}</p>
            <ul className="upstream-model-manager-list">
              {activeOnlyRows.map((model) => (
                <li key={model.id} data-testid={"upstream-model-configured-only-" + model.id}>
                  <strong className="upstream-model-manager-model-id" dir="ltr">{model.id}</strong>
                </li>
              ))}
            </ul>
          </details>
        ) : null}

        {error && !catalogActionReason && !reasoningSelectionModel ? <p role="alert" className="upstream-model-manager-error">{error}</p> : null}
        {message ? <p role="status">{message}</p> : null}

        <div className="upstream-model-manager-reset">
          <button
            type="button"
            className="btn ghost upstream-model-manager-compact-action"
            onClick={() => void resetCatalog()}
            disabled={operationBusy}
            title={t("offline.nativeModelManagerResetHelp")}
          >
            {resetting ? <Icon name="loader" className="spinicon" /> : <Icon name="refresh" />}
            {resetting ? t("offline.nativeModelManagerResetting") : t("offline.nativeModelManagerReset")}
          </button>
        </div>
      </div>
    </Sheet>
  );
}
