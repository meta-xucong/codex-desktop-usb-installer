import { useEffect, useId, useRef, useState } from "react";

import { errorMessage, managerApi } from "../../services/managerApi";
import type {
  CapabilityPackInstallOptions,
  CapabilityPackReport,
  CcswitchLaunchReport,
} from "../../shared/types";
import { Icon } from "../icons";
import { useI18n } from "../i18n";
import { Sheet } from "../Sheet";
import { Tooltip } from "../components";

type ApiServiceId = "dashscope-web-search";
type RemoteMcpId = "cloudflare" | "figma" | "linear" | "notion";
type ServiceId = "feishu" | "media-generation" | ApiServiceId | RemoteMcpId;
export type OfflineComponentsView = "connections" | "maintenance";
type FormValues = Omit<CapabilityPackInstallOptions, "enabledApiServices" | "enabledRemoteMcps" | "enableMediaGeneration">;

const EMPTY_FORM: FormValues = {
  enableFeishu: false,
  persistConnections: true,
  feishuAppId: "",
  feishuAppSecret: "",
  feishuUserAccessToken: "",
  feishuDomain: "https://open.feishu.cn",
  feishuAuthMode: "tenant",
  dashscopeApiKey: "",
  dashscopeBaseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
  dashscopeModel: "",
  mediaGenerationApiBaseUrl: "",
  mediaGenerationApiProfile: "auto",
  mediaGenerationApiKey: "",
  mediaGenerationAgentId: "",
  image2Model: "",
  grokImageModel: "",
  grokVideoModel: "",
  seedanceModel: "",
  minimaxVideoModel: "",
};

const API_SERVICE_KEYS: ApiServiceId[] = ["dashscope-web-search"];
const REMOTE_MCP_KEYS: RemoteMcpId[] = ["cloudflare", "figma", "linear", "notion"];
const REMOTE_MCP_LABELS = {
  cloudflare: "offline.components.remoteMcp.cloudflare",
  figma: "offline.components.remoteMcp.figma",
  linear: "offline.components.remoteMcp.linear",
  notion: "offline.components.remoteMcp.notion",
} as const;

const CONFIG_ATTENTION_STATUSES = new Set(["partial", "failed", "conflict"]);

function configNeedsAttention(report: CapabilityPackReport): boolean {
  return (
    report.coreReady &&
    report.runtimeReady &&
    !report.success &&
    CONFIG_ATTENTION_STATUSES.has(report.configStatus)
  );
}

function mediaUsesLegacyProfile(profile: string, baseUrl: string): boolean {
  if (profile === "legacy") return true;
  if (profile === "wokey") return false;
  try {
    return new URL(baseUrl.trim()).hostname.toLowerCase() !== "api.wokey.ai";
  } catch {
    return true;
  }
}

export function OfflineComponentsSheet({
  open,
  onDismiss,
  view = "connections",
}: {
  open: boolean;
  onDismiss: () => void;
  view?: OfflineComponentsView;
}) {
  const { t } = useI18n();
  const titleId = useId();
  const bodyId = useId();
  const [form, setForm] = useState<FormValues>(EMPTY_FORM);
  const [enabled, setEnabled] = useState<Record<ServiceId, boolean>>({
    feishu: false,
    "media-generation": false,
    "dashscope-web-search": false,
    cloudflare: false,
    figma: false,
    linear: false,
    notion: false,
  });
  const [busy, setBusy] = useState<"pack" | "ccswitch" | null>(null);
  const [statusLoading, setStatusLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [packReport, setPackReport] = useState<CapabilityPackReport | null>(null);
  const [ccswitchReport, setCcswitchReport] = useState<CcswitchLaunchReport | null>(null);
  const statusRequestRef = useRef(0);
  const mediaAdvancedRef = useRef<HTMLDetailsElement>(null);

  useEffect(() => {
    if (!open) {
      statusRequestRef.current += 1;
      setStatusLoading(false);
      return;
    }
    const requestId = statusRequestRef.current + 1;
    statusRequestRef.current = requestId;
    setStatusLoading(true);
    setError(null);
    void managerApi
      .capabilityPackStatus()
      .then((report) => {
        if (statusRequestRef.current === requestId) setPackReport(report);
      })
      .catch((cause) => {
        if (statusRequestRef.current === requestId) setError(errorMessage(cause));
      })
      .finally(() => {
        if (statusRequestRef.current === requestId) setStatusLoading(false);
      });
    return () => {
      if (statusRequestRef.current === requestId) statusRequestRef.current += 1;
    };
  }, [open]);

  const update = <K extends keyof FormValues>(key: K, value: FormValues[K]) => {
    setForm((current) => ({ ...current, [key]: value }));
    setError(null);
  };

  const toggleService = (service: ServiceId) => {
    setEnabled((current) => ({ ...current, [service]: !current[service] }));
    setError(null);
  };

  const mediaNeedsRouteId =
    mediaUsesLegacyProfile(form.mediaGenerationApiProfile, form.mediaGenerationApiBaseUrl) &&
    Boolean(form.image2Model.trim() || form.seedanceModel.trim());

  useEffect(() => {
    if (mediaNeedsRouteId && mediaAdvancedRef.current) {
      mediaAdvancedRef.current.open = true;
    }
  }, [mediaNeedsRouteId]);

  const validate = (): string | null => {
    if (enabled.feishu && (!form.feishuAppId.trim() || !form.feishuAppSecret.trim())) {
      return t("offline.components.validationFeishu");
    }
    if (enabled.feishu && form.feishuAuthMode === "user" && !form.feishuUserAccessToken.trim()) {
      return t("offline.components.validationFeishuUser");
    }
    if (
      enabled["dashscope-web-search"] &&
      (!form.dashscopeApiKey.trim() || !form.dashscopeBaseUrl.trim() || !form.dashscopeModel.trim())
    ) {
      return t("offline.components.validationDashscope");
    }
    if (enabled["media-generation"]) {
      const mediaModels = [form.image2Model, form.grokImageModel, form.grokVideoModel, form.seedanceModel, form.minimaxVideoModel];
      if (!form.mediaGenerationApiBaseUrl.trim() || !form.mediaGenerationApiKey.trim() || mediaModels.every((model) => !model.trim())) {
        return t("offline.components.validationMedia");
      }
      if (mediaNeedsRouteId && !form.mediaGenerationAgentId.trim()) {
        return t("offline.components.validationMediaRoute");
      }
    }
    return null;
  };

  const installPack = async () => {
    if (busy !== null || statusLoading) return;
    const validation = validate();
    if (validation) {
      setError(validation);
      return;
    }
    // An install result is newer than any status query started by opening the
    // sheet. Invalidate that query before allowing it to update the report.
    statusRequestRef.current += 1;
    setStatusLoading(false);
    setBusy("pack");
    setError(null);
    try {
      const report = await managerApi.installCapabilityPack({
        ...form,
        enableFeishu: enabled.feishu,
        enableMediaGeneration: enabled["media-generation"],
        enabledApiServices: API_SERVICE_KEYS.filter((service) => enabled[service]),
        enabledRemoteMcps: REMOTE_MCP_KEYS.filter((server) => enabled[server]),
      });
      setPackReport(report);
      if (!report.coreReady) setError(report.message);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const retryCcswitch = async () => {
    setBusy("ccswitch");
    setError(null);
    setCcswitchReport(null);
    try {
      const report = await managerApi.installAndLaunchCcswitch();
      setCcswitchReport(report);
      if (!report.launched) setError(report.message);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const field = (
    key: keyof FormValues,
    label: string,
    secret = false,
    placeholder?: string,
  ) => (
    <label className="component-field" key={String(key)}>
      <span>{label}</span>
      <input
        className="input"
        type={secret ? "password" : "text"}
        autoComplete={secret ? "new-password" : "off"}
        value={String(form[key])}
        placeholder={placeholder}
        onChange={(event) => update(key, event.target.value)}
        disabled={busy !== null}
      />
    </label>
  );

  return (
    <Sheet
      open={open}
      onDismiss={() => {
        if (!busy) onDismiss();
      }}
      dismissable={busy === null}
      labelledBy={titleId}
      describedBy={bodyId}
      initialFocus="dismiss"
      centeredInExpanded
    >
      <div className="component-sheet">
        <div className="component-sheet-heading">
          <Icon name="sliders" />
          <h3 id={titleId}>
            {view === "connections"
              ? t("offline.components.title")
              : t("offline.components.repairEntry")}
          </h3>
          <Tooltip
            content={`${t("offline.components.body")}\n\n${t("offline.components.skipNote")}`}
            side="bottom"
          >
            <span className="tooltip-icon" aria-label={t("themes.details")}>
              <Icon name="info" />
            </span>
          </Tooltip>
        </div>
        <span id={bodyId} className="sr-only">
          {t("offline.components.body")} {t("offline.components.skipNote")}
        </span>

        {view === "connections" ? (
          <div className="component-service-list">
          <section className="component-service">
            <div className="component-service-toggle">
              <input
                id="component-feishu-enabled"
                type="checkbox"
                checked={enabled.feishu}
                onChange={() => toggleService("feishu")}
                disabled={busy !== null}
              />
              <label
                className="component-service-toggle-label"
                htmlFor="component-feishu-enabled"
                aria-label={t("offline.components.feishu")}
              >
                <span>
                  <strong>{t("offline.components.feishu")}</strong>
                </span>
              </label>
              <Tooltip content={t("offline.components.feishuDesc")}>
                <span className="tooltip-icon" aria-label={t("themes.details")}>
                  <Icon name="info" />
                </span>
              </Tooltip>
            </div>
            {enabled.feishu ? (
              <div className="component-fields">
                {field("feishuAppId", t("offline.components.feishuAppId"))}
                {field("feishuAppSecret", t("offline.components.feishuAppSecret"), true)}
                <label className="component-field">
                  <span>{t("offline.components.feishuAuthMode")}</span>
                  <select
                    className="input"
                    value={form.feishuAuthMode}
                    onChange={(event) => update("feishuAuthMode", event.target.value)}
                    disabled={busy !== null}
                  >
                    <option value="tenant">tenant</option>
                    <option value="user">user</option>
                    <option value="oauth">oauth</option>
                  </select>
                </label>
                {field("feishuUserAccessToken", t("offline.components.feishuToken"), true)}
                {field("feishuDomain", t("offline.components.feishuDomain"))}
              </div>
            ) : null}
          </section>
          <section className="component-service">
            <div className="component-service-toggle">
              <input
                id="component-media-generation-enabled"
                type="checkbox"
                checked={enabled["media-generation"]}
                onChange={() => toggleService("media-generation")}
                disabled={busy !== null}
              />
              <label className="component-service-toggle-label" htmlFor="component-media-generation-enabled" aria-label={t("offline.components.mediaGeneration")}>
                <span><strong>{t("offline.components.mediaGeneration")}</strong></span>
              </label>
              <Tooltip content={t("offline.components.mediaGenerationDesc")}>
                <span className="tooltip-icon" aria-label={t("themes.details")}><Icon name="info" /></span>
              </Tooltip>
            </div>
            {enabled["media-generation"] ? (
              <>
                <p className="component-note component-service-description">
                  {t("offline.components.mediaGenerationIntro")}
                </p>
                <div className="component-fields">
                  {field("mediaGenerationApiBaseUrl", t("offline.components.mediaBaseUrl"))}
                  {field("mediaGenerationApiKey", t("offline.components.mediaApiKey"), true)}

                  <div className="component-field-group">
                    <strong>{t("offline.components.mediaModelsTitle")}</strong>
                    <small>{t("offline.components.mediaModelsHint")}</small>
                    {field("image2Model", t("offline.components.image2Model"), false, t("offline.components.modelIdPlaceholder"))}
                    {field("grokImageModel", t("offline.components.grokImageModel"), false, t("offline.components.modelIdPlaceholder"))}
                    {field("grokVideoModel", t("offline.components.grokVideoModel"), false, t("offline.components.modelIdPlaceholder"))}
                    {field("seedanceModel", t("offline.components.seedanceModel"), false, t("offline.components.modelIdPlaceholder"))}
                    <div className="component-provider-status" role="status">
                      <strong>{t("offline.components.minimaxUnavailableTitle")}</strong>
                      <span>{t("offline.components.minimaxUnavailableBody")}</span>
                    </div>
                  </div>

                  <details className="component-advanced" ref={mediaAdvancedRef}>
                    <summary>{t("offline.components.mediaAdvanced")}</summary>
                    <div className="component-advanced-fields">
                      <label className="component-field">
                        <span>{t("offline.components.apiProfile")}</span>
                        <select
                          className="input"
                          value={form.mediaGenerationApiProfile}
                          onChange={(event) => update("mediaGenerationApiProfile", event.target.value)}
                          disabled={busy !== null}
                        >
                          <option value="auto">{t("offline.components.profileAuto")}</option>
                          <option value="legacy">{t("offline.components.profileLegacy")}</option>
                          <option value="wokey">{t("offline.components.profileWokey")}</option>
                        </select>
                      </label>
                      {mediaNeedsRouteId ? (
                        <>
                          {field("mediaGenerationAgentId", t("offline.components.routeId"))}
                          <small>{t("offline.components.mediaRouteHint")}</small>
                        </>
                      ) : null}
                    </div>
                  </details>
                </div>
              </>
            ) : null}
          </section>

          <section className="component-service">
            <div className="component-service-toggle">
              <span className="component-service-toggle-label"><strong>{t("offline.components.remoteMcpTitle")}</strong></span>
              <Tooltip content={t("offline.components.remoteMcpDesc")}>
                <span className="tooltip-icon" aria-label={t("themes.details")}><Icon name="info" /></span>
              </Tooltip>
            </div>
            {REMOTE_MCP_KEYS.map((server) => (
              <div className="component-service-toggle" key={server}>
                <input
                  id={`component-mcp-${server}-enabled`}
                  type="checkbox"
                  checked={enabled[server]}
                  onChange={() => toggleService(server)}
                  disabled={busy !== null}
                />
                <label
                  className="component-service-toggle-label"
                  htmlFor={`component-mcp-${server}-enabled`}
                  aria-label={t(REMOTE_MCP_LABELS[server])}
                >
                  <span><strong>{t(REMOTE_MCP_LABELS[server])}</strong></span>
                </label>
              </div>
            ))}
          </section>

          <section className="component-service">
            <div className="component-service-toggle">
              <input
                id="component-dashscope-enabled"
                type="checkbox"
                checked={enabled["dashscope-web-search"]}
                onChange={() => toggleService("dashscope-web-search")}
                disabled={busy !== null}
              />
              <label
                className="component-service-toggle-label"
                htmlFor="component-dashscope-enabled"
                aria-label={t("offline.components.dashscope")}
              >
                <span>
                  <strong>{t("offline.components.dashscope")}</strong>
                </span>
              </label>
              <Tooltip content={t("offline.components.dashscopeDesc")}>
                <span className="tooltip-icon" aria-label={t("themes.details")}>
                  <Icon name="info" />
                </span>
              </Tooltip>
            </div>
            {enabled["dashscope-web-search"] ? (
              <div className="component-fields">
                {field("dashscopeApiKey", t("offline.components.apiKey"), true)}
                {field("dashscopeBaseUrl", t("offline.components.baseUrl"))}
                {field("dashscopeModel", t("offline.components.model"))}
              </div>
            ) : null}
          </section>

          </div>
        ) : (
          <div className="component-maintenance-panel">
            <p className="component-note">{t("offline.components.maintenanceBody")}</p>
            <div className="component-separate-action">
              <div>
                <strong>{t("offline.components.ccswitchTitle")}</strong>
                <Tooltip content={t("offline.components.ccswitchDesc")}>
                  <span className="tooltip-icon" aria-label={t("themes.details")}>
                    <Icon name="info" />
                  </span>
                </Tooltip>
              </div>
              <button className="btn ghost" onClick={() => void retryCcswitch()} disabled={busy !== null}>
                {busy === "ccswitch" ? <Icon name="loader" className="spinicon" /> : <Icon name="gear" />}
                {busy === "ccswitch" ? t("offline.ccswitchLaunching") : t("offline.components.ccswitchRetry")}
              </button>
            </div>
          </div>
        )}

        {packReport ? (
          <div className={`component-result${packReport.coreReady ? " success" : ""}`}>
            <strong>
              {packReport.coreReady
                ? packReport.success
                  ? t("offline.components.ready")
                  : configNeedsAttention(packReport)
                    ? t("offline.components.configConflict")
                    : t("offline.components.partial")
                : t("offline.components.notReady")}
            </strong>
            <span>{packReport.message}</span>
            {packReport.skills > 0 ? (
              <small>
                {t("offline.components.counts", {
                  skills: packReport.skills,
                  agents: packReport.agents,
                })}
              </small>
            ) : null}
          </div>
        ) : null}
        {ccswitchReport ? (
          <div className={`component-result${ccswitchReport.launched ? " success" : ""}`}>
            <strong>
              {ccswitchReport.launched
                ? t("offline.components.ccswitchReady")
                : t("offline.components.ccswitchNotReady")}
            </strong>
            <span>{ccswitchReport.message}</span>
          </div>
        ) : null}
        {error ? <div className="component-error">{error}</div> : null}

        <div className="row2 sheet-actions">
          <button className="btn ghost" onClick={onDismiss} disabled={busy !== null}>
            {t("offline.components.skip")}
          </button>
          <button
            className="btn primary"
            onClick={() => void installPack()}
            disabled={busy !== null || statusLoading}
          >
            {busy === "pack" ? <Icon name="loader" className="spinicon" /> : <Icon name="download" />}
            {busy === "pack"
              ? t("offline.components.installing")
              : view === "connections"
                ? t("offline.components.apply")
                : t("offline.components.install")}
          </button>
        </div>
      </div>
    </Sheet>
  );
}
