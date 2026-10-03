import { useCallback, useEffect, useRef, useState } from "react";

import { errorCode, managerApi } from "../../services/managerApi";
import type { GatewayModelProbeReport } from "../../shared/types";
import { useI18n } from "../i18n";

function newRequestId() {
  return globalThis.crypto?.randomUUID?.()
    ?? `gateway-probe-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

function checkedAtLabel(epochSeconds: number, locale: string) {
  if (!epochSeconds) return "—";
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "medium" })
    .format(new Date(epochSeconds * 1000));
}

export function GatewayModelProbePanel() {
  const { t, lang } = useI18n();
  const [report, setReport] = useState<GatewayModelProbeReport | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [costConsent, setCostConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const activeRequestId = useRef<string | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const requestId = activeRequestId.current;
      activeRequestId.current = null;
      if (requestId) void managerApi.cancelGatewayModelProbe(requestId).catch(() => false);
    };
  }, []);

  const run = useCallback(async (selectedModelIds: string[]) => {
    if (busy || (selectedModelIds.length > 0 && !costConsent)) return;
    const requestId = newRequestId();
    activeRequestId.current = requestId;
    setBusy(true);
    setError(null);
    setMessage(null);
    if (selectedModelIds.length > 0) setCostConsent(false);
    try {
      const next = await managerApi.gatewayModelProbe({
        requestId,
        selectedModelIds,
        costConsent: selectedModelIds.length > 0 && costConsent,
      });
      if (!mounted.current || activeRequestId.current !== requestId) return;
      setReport(next);
      setSelected((current) => current.filter((id) => next.models.some((model) => model.id === id && model.selectable)));
      setMessage(next.status === "connected"
        ? t("offline.gatewayProbeConnected")
        : next.status === "passed"
          ? t("offline.gatewayProbePassed")
          : next.status === "cancelled"
            ? t("offline.gatewayProbeCancelled")
            : next.status === "failed"
              ? t("offline.gatewayProbeFailed")
              : t("offline.gatewayProbeUnavailable"));
    } catch (cause) {
      if (!mounted.current || activeRequestId.current !== requestId) return;
      setError(errorCode(cause) || t("offline.gatewayProbeUnavailable"));
    } finally {
      if (activeRequestId.current === requestId) activeRequestId.current = null;
      if (mounted.current) setBusy(false);
    }
  }, [busy, costConsent, t]);

  const cancel = useCallback(async () => {
    const requestId = activeRequestId.current;
    if (!requestId) return;
    setMessage(t("offline.gatewayProbeCancelling"));
    const confirmed = await managerApi.cancelGatewayModelProbe(requestId).catch(() => false);
    if (mounted.current) {
      setMessage(t(confirmed ? "offline.gatewayProbeCancelRequested" : "offline.gatewayProbeCancelUnconfirmed"));
    }
  }, [t]);

  const toggleModel = (id: string) => {
    setSelected((current) => current.includes(id)
      ? current.filter((candidate) => candidate !== id)
      : current.length < 3 ? [...current, id] : current);
    setCostConsent(false);
  };

  const checkLabels: Record<string, string> = {
    text: t("offline.probeText"),
    singleTool: t("offline.probeSingleTool"),
    toolResult: t("offline.probeToolResult"),
    multipleTools: t("offline.probeMultipleTools"),
    toolError: t("offline.probeToolError"),
    stream: t("offline.probeStream"),
    multiTurn: t("offline.probeMultiTurn"),
  };

  return (
    <section className="gateway-model-probe" aria-label={t("offline.gatewayProbeTitle")} data-testid="gateway-model-probe">
      <p>{t("offline.gatewayProbeBoundary")}</p>
      <div className="actions">
        <button type="button" className="btn ghost" onClick={() => void run([])} disabled={busy}>
          {busy ? t("offline.gatewayProbeRunning") : t("offline.gatewayProbeListOnly")}
        </button>
        {busy ? (
          <button type="button" className="btn ghost" onClick={() => void cancel()}>
            {t("offline.gatewayProbeCancel")}
          </button>
        ) : null}
      </div>

      {report ? (
        <div className="gateway-probe-report" data-testid="gateway-probe-report">
          <p>{t("offline.gatewayProbeCredential", { fingerprint: report.credentialFingerprint || "—" })}</p>
          <p>{t("offline.gatewayProbeEndpoint", { endpoint: report.endpoint || "—" })}</p>
          <p>{t("offline.gatewayProbeCheckedAt", { time: checkedAtLabel(report.checkedAt, lang) })}</p>
          <p>{t("offline.gatewayProbeGeneratedCount", { count: report.generatedRequests })}</p>
          {report.errorCode ? <code>{report.errorCode}</code> : null}
          {report.models.length > 0 ? (
            <fieldset disabled={busy}>
              <legend>{t("offline.gatewayProbeChooseModels")}</legend>
              {report.models.map((model, index) => (
                <label key={`${model.id}:${model.ownedBy ?? ""}:${index}`}>
                  <input
                    type="checkbox"
                    checked={selected.includes(model.id)}
                    disabled={!model.selectable || (!selected.includes(model.id) && selected.length >= 3)}
                    onChange={() => toggleModel(model.id)}
                  />
                  {model.ownedBy ? `${model.ownedBy} / ` : ""}{model.id}
                  {!model.selectable ? ` — ${t("offline.gatewayProbeDuplicateId")}` : ""}
                </label>
              ))}
            </fieldset>
          ) : null}
          {selected.length > 0 ? (
            <div className="gateway-probe-cost-consent">
              <p>{t("offline.gatewayProbeCostWarning", { requests: selected.length * 12 })}</p>
              <label>
                <input
                  type="checkbox"
                  checked={costConsent}
                  disabled={busy}
                  onChange={(event) => setCostConsent(event.target.checked)}
                />
                {t("offline.gatewayProbeCostConsent")}
              </label>
              <button
                type="button"
                className="btn ghost"
                disabled={busy || !costConsent}
                onClick={() => void run(selected)}
              >
                {busy ? t("offline.gatewayProbeRunning") : t("offline.gatewayProbeRunSelected", { count: selected.length })}
              </button>
            </div>
          ) : null}
          {report.results.length > 0 ? (
            <ul>
              {report.results.map((result) => (
                <li key={result.modelId}>
                  <strong>{result.modelId}</strong>{" — "}
                  {result.passed ? t("offline.gatewayProbeModelPassed") : t("offline.gatewayProbeModelFailed")}
                  {" · "}{t("offline.gatewayProbeModelRequests", { count: result.generatedRequests })}
                  {result.errorCode ? <code> {result.errorCode}</code> : null}
                  <ul>{result.checks.map((check) => (
                    <li key={check.name}>
                      {checkLabels[check.name] ?? check.name}: {check.passed ? t("offline.probeVerified") : t("offline.probeUnknown")}
                    </li>
                  ))}</ul>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
      {message ? <p role="status">{message}</p> : null}
      {error ? <p role="alert">{t("offline.gatewayProbeError", { code: error })}</p> : null}
    </section>
  );
}
