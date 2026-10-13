import { useCallback, useEffect, useId, useRef, useState } from "react";

import { errorMessage, managerApi } from "../../services/managerApi";
import type {
  CapabilityPackInstallOptions,
  CapabilityPackReport,
  CcswitchLaunchReport,
} from "../../shared/types";
import { selectMediaModels } from "../mediaModelSelection";
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
  mediaGenerationAgentId: "",
  image2Model: "",
  grokImageModel: "",
  grokVideoModel: "",
  seedanceModel: "",
  minimaxVideoModel: "",
  mediaGatewayCatalogStatus: "not-checked",
  mediaGatewayModelCount: 0,
  mediaGatewayCatalogVerifiedAt: null,
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
    "media-generation": true,
    "dashscope-web-search": false,
    cloudflare: false,
    figma: false,
    linear: false,
    notion: false,
  });
  const [busy, setBusy] = useState<"pack" | "ccswitch" | null>(null);
  const [statusLoading, setStatusLoading] = useState(false);
  const [mediaCatalogLoading, setMediaCatalogLoading] = useState(false);
  const [mediaCatalogError, setMediaCatalogError] = useState<string | null>(null);
  const [maintenanceSettingsLoading, setMaintenanceSettingsLoading] = useState(false);
  const [maintenanceSettingsError, setMaintenanceSettingsError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [packReport, setPackReport] = useState<CapabilityPackReport | null>(null);
  const [ccswitchReport, setCcswitchReport] = useState<CcswitchLaunchReport | null>(null);
  const statusRequestRef = useRef(0);
  const mediaCatalogRequestRef = useRef(0);
  const maintenanceSettingsRequestRef = useRef(0);
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

  const refreshMediaModels = useCallback(async () => {
    const requestId = mediaCatalogRequestRef.current + 1;
    mediaCatalogRequestRef.current = requestId;
    setMediaCatalogLoading(true);
    setMediaCatalogError(null);
    try {
      const catalog = await managerApi.mediaModelCatalog();
      if (mediaCatalogRequestRef.current !== requestId) return;
      // Seedance requires an account-specific route ID. Do not auto-select an
      // un-routed model from /v1/models; only restore a route explicitly
      // configured by the user and still present upstream.
      const selected = selectMediaModels(catalog.models, { includeSeedance: false });
      const configuredSeedanceModel = catalog.configuredSeedanceModel?.trim() ?? "";
      const configuredAgentId = catalog.configuredAgentId?.trim() ?? "";
      if (
        configuredSeedanceModel &&
        configuredAgentId &&
        catalog.models.includes(configuredSeedanceModel)
      ) {
        selected.seedanceModel = configuredSeedanceModel;
        selected.mediaGenerationAgentId = configuredAgentId;
      }
      setForm((current) => ({
        ...current,
        ...selected,
        mediaGatewayCatalogStatus: "verified",
        mediaGatewayModelCount: catalog.models.length,
        mediaGatewayCatalogVerifiedAt: new Date().toISOString(),
        mediaGenerationAgentId: selected.mediaGenerationAgentId,
      }));
      const hasSupportedRoute = Boolean(
        selected.image2Model ||
        selected.grokImageModel ||
        selected.grokVideoModel ||
        (selected.seedanceModel && selected.mediaGenerationAgentId),
      );
      if (!hasSupportedRoute) {
        setEnabled((current) => ({ ...current, "media-generation": false }));
      }
    } catch (cause) {
      if (mediaCatalogRequestRef.current !== requestId) return;
      setForm((current) => ({
        ...current,
        image2Model: "",
        grokImageModel: "",
        grokVideoModel: "",
        seedanceModel: "",
        minimaxVideoModel: "",
        mediaGenerationAgentId: "",
        mediaGatewayCatalogStatus: "failed",
        mediaGatewayModelCount: 0,
        mediaGatewayCatalogVerifiedAt: null,
      }));
      setMediaCatalogError(errorMessage(cause));
    } finally {
      if (mediaCatalogRequestRef.current === requestId) setMediaCatalogLoading(false);
    }
  }, []);

  useEffect(() => {
    const requestId = maintenanceSettingsRequestRef.current + 1;
    maintenanceSettingsRequestRef.current = requestId;
    if (!open || view !== "maintenance") {
      setMaintenanceSettingsLoading(false);
      setMaintenanceSettingsError(null);
      return;
    }

    setMaintenanceSettingsLoading(true);
    setMaintenanceSettingsError(null);
    void managerApi
      .mediaModelSettings()
      .then((settings) => {
        if (maintenanceSettingsRequestRef.current !== requestId) return;
        setForm((current) => ({
          ...current,
          mediaGenerationAgentId: settings.mediaGenerationAgentId,
          image2Model: settings.image2Model,
          grokImageModel: settings.grokImageModel,
          grokVideoModel: settings.grokVideoModel,
          seedanceModel: settings.seedanceModel,
          minimaxVideoModel: settings.minimaxVideoModel,
        }));
      })
      .catch((cause) => {
        if (maintenanceSettingsRequestRef.current === requestId) {
          setMaintenanceSettingsError(errorMessage(cause));
        }
      })
      .finally(() => {
        if (maintenanceSettingsRequestRef.current === requestId) setMaintenanceSettingsLoading(false);
      });
    return () => {
      if (maintenanceSettingsRequestRef.current === requestId) maintenanceSettingsRequestRef.current += 1;
    };
  }, [open, view]);

  const mediaGenerationEnabled = enabled["media-generation"];

  useEffect(() => {
    if (!open || view !== "connections" || !mediaGenerationEnabled) {
      mediaCatalogRequestRef.current += 1;
      setMediaCatalogLoading(false);
      return;
    }
    void refreshMediaModels();
    return () => {
      mediaCatalogRequestRef.current += 1;
    };
  }, [open, view, mediaGenerationEnabled, refreshMediaModels]);

  const update = <K extends keyof FormValues>(key: K, value: FormValues[K]) => {
    setForm((current) => ({ ...current, [key]: value }));
    setError(null);
  };

  const toggleService = (service: ServiceId) => {
    setEnabled((current) => ({ ...current, [service]: !current[service] }));
    setError(null);
  };

  const mediaNeedsRouteId = Boolean(form.seedanceModel.trim());

  useEffect(() => {
    if (mediaNeedsRouteId && mediaAdvancedRef.current) {
      mediaAdvancedRef.current.open = true;
    }
  }, [mediaNeedsRouteId]);

  const validate = (): string | null => {
    if (view === "maintenance" && maintenanceSettingsError) {
      return t("offline.components.maintenanceSettingsUnavailable");
    }
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
      if (mediaCatalogError) {
        return t("offline.components.mediaModelsRetryBeforeSave");
      }
      if (mediaNeedsRouteId && !form.mediaGenerationAgentId.trim()) {
        return t("offline.components.validationMediaRoute");
      }
      if (
        !form.image2Model.trim() &&
        !form.grokImageModel.trim() &&
        !form.grokVideoModel.trim() &&
        !(form.seedanceModel.trim() && form.mediaGenerationAgentId.trim())
      ) {
        return t("offline.components.validationMedia");
      }
    }
    return null;
  };

  const installPack = async () => {
    if (busy !== null || statusLoading || mediaCatalogLoading) return;
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
        // Maintenance repairs the pack, not user connections. Keep the saved
        // media route fields in the request for config rendering, while
        // explicitly preventing credential persistence/clearing.
        persistConnections: view === "connections" && form.persistConnections,
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
    readOnly = false,
  ) => (
    <label className="component-field" key={String(key)}>
      <span>{label}</span>
      <input
        className="input"
        type={secret ? "password" : "text"}
        autoComplete={secret ? "new-password" : "off"}
        value={String(form[key])}
        placeholder={placeholder}
        readOnly={readOnly}
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
                  <div className="component-field-group">
                    <strong>{t("offline.components.mediaModelsTitle")}</strong>
                    <small>{t("offline.components.mediaModelsHint")}</small>
                    <button className="btn ghost" type="button" onClick={() => void refreshMediaModels()} disabled={busy !== null || mediaCatalogLoading}>
                      {mediaCatalogLoading ? t("offline.components.mediaModelsLoading") : t("offline.components.refreshMediaModels")}
                    </button>
                    {mediaCatalogError ? <small role="status">{t("offline.components.mediaModelsError", { error: mediaCatalogError })}</small> : null}
                    {field("image2Model", t("offline.components.image2Model"), false, t("offline.components.modelNotFound"), true)}
                    {field("grokImageModel", t("offline.components.grokImageModel"), false, t("offline.components.modelNotFound"), true)}
                    {field("grokVideoModel", t("offline.components.grokVideoModel"), false, t("offline.components.modelNotFound"), true)}
                    {field("seedanceModel", t("offline.components.seedanceModel"), false, t("offline.components.modelNotFound"), true)}
                    <div className="component-provider-status" role="status">
                      <strong>{t("offline.components.minimaxUnavailableTitle")}</strong>
                      <span>{t("offline.components.minimaxUnavailableBody")}</span>
                    </div>
                  </div>

                  {mediaNeedsRouteId ? (
                    <details className="component-advanced" ref={mediaAdvancedRef} open>
                      <summary>{t("offline.components.mediaAdvanced")}</summary>
                      <div className="component-advanced-fields">
                        {field("mediaGenerationAgentId", t("offline.components.routeId"))}
                        <small>{t("offline.components.mediaRouteHint")}</small>
                      </div>
                    </details>
                  ) : null}
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
            {maintenanceSettingsLoading ? (
              <small role="status">{t("offline.components.maintenanceSettingsLoading")}</small>
            ) : null}
            {maintenanceSettingsError ? (
              <small role="alert">{t("offline.components.maintenanceSettingsUnavailable")}</small>
            ) : null}
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
            {packReport.codexRestartRequired ? (
              <small role="status">
                {t("offline.components.mediaReadiness", {
                  protocol: t(packReport.mediaProtocolStatus === "passed" ? "offline.components.statusPassed" : "offline.components.statusNotChecked"),
                  catalog: t(packReport.mediaGatewayCatalogStatus === "verified" ? "offline.components.statusPassed" : packReport.mediaGatewayCatalogStatus === "failed" ? "offline.components.statusFailed" : "offline.components.statusNotChecked"),
                  restart: packReport.codexRunning
                    ? t("offline.components.restartRunning")
                    : t("offline.components.restartRequired"),
                })}
              </small>
            ) : null}
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
            disabled={
              busy !== null ||
              statusLoading ||
              maintenanceSettingsLoading ||
              (view === "maintenance" && Boolean(maintenanceSettingsError)) ||
              (view === "connections" && enabled["media-generation"] && mediaCatalogLoading)
            }
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
