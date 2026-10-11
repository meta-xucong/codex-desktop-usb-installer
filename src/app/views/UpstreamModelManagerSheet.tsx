import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from "react";

import { managerApi } from "../../services/managerApi";
import type {
  GatewayModelProbeReport,
  GatewayProbeModel,
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
  const [report, setReport] = useState<GatewayModelProbeReport | null>(null);
  const [activeIds, setActiveIds] = useState<string[]>([]);
  const [catalogStatus, setCatalogStatus] = useState<string | null>(null);
  const [failedTestIds, setFailedTestIds] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);
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

  const discover = useCallback(async () => {
    if (loading || testingId || addingId || resetting || addRequestId.current) return;
    const currentSession = ++session.current;
    setLoading(true);
    setTestingId(null);
    setAddingId(null);
    setError(null);
    setMessage(null);
    setReport(null);
    setFailedTestIds([]);
    tickets.current.clear();
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
      setReport(next);
      if (state.status === "orphaned") setError(t("offline.nativeModelManagerOrphaned"));
      else if (next.status !== "connected") setError(t("offline.nativeModelManagerError"));
    } catch {
      if (session.current !== currentSession) return;
      setError(t("offline.nativeModelManagerError"));
    } finally {
      if (requestId) requestIds.current.delete(requestId);
      if (session.current === currentSession) setLoading(false);
    }
  }, [addingId, cancelOutstanding, loading, resetting, t, testingId]);

  useLayoutEffect(() => {
    if (!open && wasOpen.current) {
      wasOpen.current = false;
      session.current += 1;
      setLoading(false);
      setTestingId(null);
      if (!addRequestId.current) setAddingId(null);
      setResetting(false);
      void cancelOutstanding();
    }
  }, [cancelOutstanding, open]);

  useEffect(() => {
    if (!open || wasOpen.current) return;
    wasOpen.current = true;
    void discover();
  }, [discover, open]);

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
    if (loading || testingId || addingId || resetting || addRequestId.current || catalogStatus !== "ok" || !model.selectable || model.activeOnly) return;
    const previous = tickets.current.get(model.id);
    if (previous) {
      invalidateTicket(model.id);
    }
    const currentSession = session.current;
    const requestId = newRequestId("upstream-connectivity");
    requestIds.current.add(requestId);
    setTestingId(model.id);
    setFailedTestIds((ids) => ids.filter((id) => id !== model.id));
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
      const proofLifetimeMs = ((next.connectivityProofExpiresAt ?? 0) - next.checkedAt) * 1000;
      const hasFreshProof = Number.isSafeInteger(next.connectivityProofExpiresAt)
        && Number.isSafeInteger(next.checkedAt)
        && proofLifetimeMs > 0;
      const connectionMatchesDiscovery = report
        && next.connectionFingerprint === report.connectionFingerprint
        && next.credentialFingerprint === report.credentialFingerprint
        && next.endpoint === report.endpoint;
      if (next.status === "passed" && next.connectivityProofId && hasFreshProof && result?.passed && connectionMatchesDiscovery) {
        tickets.current.set(model.id, {
          requestId,
          proofId: next.connectivityProofId,
          expiresAtMs: Date.now() + proofLifetimeMs,
          connectionFingerprint: next.connectionFingerprint,
          credentialFingerprint: next.credentialFingerprint,
        });
        setTicketRevision((revision) => revision + 1);
        setMessage(t("offline.nativeModelTestPassed", { model: model.id }));
      } else {
        setFailedTestIds((ids) => ids.includes(model.id) ? ids : [...ids, model.id]);
        requestIds.current.delete(requestId);
        if (next.connectivityProofId) {
          await managerApi.cancelGatewayModelProbe(requestId).catch(() => false);
        }
        if (next.status === "passed" && !connectionMatchesDiscovery) {
          setError(t("offline.nativeModelManagerConnectionChanged"));
          return;
        }
        if (next.status === "passed" && !hasFreshProof) {
          setError(t("offline.nativeModelManagerTestExpired"));
          return;
        }
        setError(t("offline.nativeModelManagerTestFailed"));
      }
    } catch {
      if (session.current === currentSession) {
        setFailedTestIds((ids) => ids.includes(model.id) ? ids : [...ids, model.id]);
        setError(t("offline.nativeModelManagerTestFailed"));
      }
      requestIds.current.delete(requestId);
    } finally {
      if (session.current === currentSession) setTestingId(null);
    }
  }, [addingId, catalogStatus, invalidateTicket, loading, report, resetting, t, testingId]);

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
      const resultMessage = result.status === "cleanup_pending"
        ? t("offline.nativeModelManagerAddCleanupPending")
        : result.status === "outcome_unconfirmed"
          ? t("offline.nativeModelManagerAddUnconfirmed")
          : t("offline.nativeModelManagerAdded");
      if (result.status === "cleanup_pending" || result.status === "outcome_unconfirmed") {
        setMessage(null);
        setError(resultMessage);
      } else {
        setMessage(resultMessage);
      }
      await managerApi.cancelGatewayModelProbe(ticket.requestId).catch(() => false);
      await refreshCatalogState(session.current);
    } catch {
      setError(t("offline.nativeModelManagerError"));
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
  }, [addingId, catalogStatus, invalidateTicket, loading, refreshCatalogState, report, resetting, t, testingId]);

  const resetCatalog = useCallback(async () => {
    if (resetting || loading || testingId || addingId || addRequestId.current) return;
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
  }, [addingId, cancelOutstanding, loading, refreshCatalogState, resetting, t, testingId]);

  const rows: ModelRow[] = report?.models.map((model) => ({ ...model })) ?? [];
  for (const id of activeIds) {
    if (!rows.some((row) => row.id === id)) {
      rows.push({ id, ownedBy: null, selectable: false, activeOnly: true });
    }
  }
  const catalogActionReason = catalogStatus === "orphaned"
    ? t("offline.nativeModelManagerOrphaned")
    : catalogStatus !== "ok"
      ? t("offline.nativeModelManagerCatalogUnavailable")
      : null;
  const catalogActionReasonId = `${titleId}-catalog-action-reason`;
  const operationBusy = Boolean(loading || testingId || addingId || resetting || addRequestId.current);

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
            <p id={descriptionId}>{t("offline.nativeModelManagerDescription")}</p>
          </div>
        </div>

        <p className="upstream-model-manager-steps">{t("offline.nativeModelManagerSteps")}</p>
        <p className="upstream-model-manager-warning">{t("offline.nativeModelManagerCostWarning")}</p>
        <div className="upstream-model-manager-toolbar">
          <button type="button" className="btn primary" onClick={() => void discover()} disabled={operationBusy}>
            {loading ? <Icon name="loader" className="spinicon" /> : <Icon name="refresh" />}
            {loading ? t("offline.nativeModelManagerDiscovering") : t("offline.nativeModelManagerRefresh")}
          </button>
        </div>

        {report ? (
          <div className="upstream-model-manager-summary" role="status">
            {t("offline.nativeModelManagerSummary", { discovered: report.models.length, active: activeIds.length })}
          </div>
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
          <ul className="upstream-model-manager-list">
            {rows.map((model, index) => {
              const isActive = activeIds.includes(model.id);
              const ticket = tickets.current.get(model.id);
              const duplicate = !model.selectable && !model.activeOnly;
              const failed = failedTestIds.includes(model.id);
              const rowActionReason = duplicate
                ? t("offline.gatewayProbeDuplicateId")
                : operationBusy
                  ? t("offline.nativeModelManagerBusy")
                  : null;
              const actionStatus = model.activeOnly
                ? t("offline.nativeModelManagerMissingUpstream")
                : isActive
                  ? t("offline.nativeModelManagerActive")
                  : addingId === model.id
                    ? t("offline.nativeModelManagerAdding")
                    : testingId === model.id
                      ? t("offline.nativeModelManagerTesting")
                      : duplicate
                        ? t("offline.gatewayProbeDuplicateId")
                        : ticket
                          ? t("offline.nativeModelManagerTestPassed")
                          : failed
                            ? t("offline.nativeModelManagerTestFailedStatus")
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
                    <strong className="upstream-model-manager-model-id" title={model.id} dir="ltr">{model.id}</strong>
                    {model.ownedBy ? <span>{model.ownedBy}</span> : null}
                    <small className="upstream-model-manager-row-status">{actionStatus}</small>
                  </div>
                  <div className="upstream-model-manager-row-actions">
                    {!isActive && !model.activeOnly ? (
                      <>
                        {ticket ? (
                          <button
                            type="button"
                            className="btn primary"
                            onClick={() => void addModel(model)}
                            disabled={actionDisabled}
                            aria-describedby={actionReasonId}
                          >
                            {addingId === model.id ? t("offline.nativeModelManagerAdding") : t("offline.nativeModelManagerAdd")}
                          </button>
                        ) : (
                          <button
                            type="button"
                            className="btn primary"
                            onClick={() => void testModel(model)}
                            disabled={actionDisabled}
                            aria-describedby={actionReasonId}
                          >
                            {testingId === model.id ? t("offline.nativeModelManagerTesting") : t("offline.nativeModelManagerTest")}
                          </button>
                        )}
                        {rowActionReason ? (
                          <small id={reasonId} className="upstream-model-manager-action-reason">
                            {rowActionReason}
                          </small>
                        ) : null}
                      </>
                    ) : (
                      <span className="upstream-model-manager-row-status-spacer" aria-hidden="true" />
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        ) : loading ? (
          <p>{t("offline.nativeModelManagerDiscovering")}</p>
        ) : report ? (
          <p>{t("offline.nativeModelManagerEmpty")}</p>
        ) : null}

        {error && !catalogActionReason ? <p role="alert" className="upstream-model-manager-error">{error}</p> : null}
        {message ? <p role="status">{message}</p> : null}

        <div className="upstream-model-manager-reset">
          <p>{t("offline.nativeModelManagerResetHelp")}</p>
          <button type="button" className="btn ghost" onClick={() => void resetCatalog()} disabled={operationBusy}>
            {resetting ? <Icon name="loader" className="spinicon" /> : <Icon name="refresh" />}
            {resetting ? t("offline.nativeModelManagerResetting") : t("offline.nativeModelManagerReset")}
          </button>
        </div>
      </div>
    </Sheet>
  );
}
