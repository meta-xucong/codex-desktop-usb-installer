export type OperatingSystem = "windows" | "macos" | "linux" | "unknown";
export type Architecture = "x64" | "arm64" | "unknown";
export type OperationKind =
  "install" | "update" | "uninstall" | "set-install-root" | "adopt" | "launch";
export type OperationToken = string;
/** Lifecycle phase of a backend operation lease (mirrors Rust `OperationPhase`). */
export type OperationPhase =
  | "idle"
  | "preparing"
  | "downloading"
  | "verifying"
  | "applying"
  | "committing"
  | "finishing";

/**
 * Active same-process operation, as reported by `get_operation_snapshot`.
 * Used to reattach the UI after a renderer reload while work continues.
 */
export interface OperationSnapshot {
  id: string;
  kind: OperationKind;
  phase: OperationPhase;
  progress: DownloadProgress | null;
  paused: boolean;
  historical?: {
    selection: HistoricalInstallSelection;
    blockUpdates: boolean;
    expectation: HistoricalResumeExpectation;
    installRoot?: string | null;
  } | null;
  resume?: OperationResumeContext | null;
  cancellable: boolean;
  interruptible: boolean;
}

/** Exact latest-channel target captured when the user started the transfer.
 * Resume reuses this snapshot even if a later check observes a newer release. */
export type OperationResumeExpectation =
  | {
      platform: "macos";
      currentBuild: number | null;
      targetBuild: number;
      installPath: string | null;
      currentVersion: string | null;
      targetVersion: string;
    }
  | {
      platform: "windows";
      currentVersion: string | null;
      targetVersion: string;
      packageMoniker: string;
      route: WinInstallRoute;
    };

export interface OperationResumeContext {
  kind: "install" | "perform";
  installRoot?: string | null;
  expectation: OperationResumeExpectation;
}

export type OperationCompletionState =
  "succeeded" | "failed-before-commit" | "rolled-back" | "outcome-unknown";

/** Terminal backend evidence retained after the active lease disappears. */
export interface OperationCompletion {
  id: string;
  kind: OperationKind;
  phase: OperationPhase;
  state: OperationCompletionState;
}

/**
 * Serialized error returned by failing Tauri commands. Mirrors the backend
 * `CommandError` struct (src-tauri/src/errors.rs), which serializes
 * `#[serde(rename_all = "camelCase")]` from `AppError`.
 */
export interface CommandError {
  /**
   * Stable machine code, e.g. "unsupported_platform" | "engine_error" |
   * "stale_expectation" | "post_commit_cleanup" | "internal_error". `stale_expectation` means reality
   * no longer matches the snapshot the user confirmed — re-check, re-confirm.
   */
  code: string;
  /** Human-facing message (the `Display` of the underlying `AppError`). */
  message: string;
  /** Optional backend diagnostic detail, shown only behind the disclosure UI. */
  detail?: string;
}

export interface InstalledCodex {
  path: string;
  build: number;
  /** Human-facing version (CFBundleShortVersionString, e.g. 26.602.40724). */
  shortVersion: string;
  arch: string;
  /** Bundle file mtime as a Unix timestamp (seconds) — when this build landed
   *  on disk (install or in-place update). Reliable fallback for a date. */
  installedAt?: number | null;
}

export interface UpdateStrategy {
  kind: "delta" | "full";
  fromBuild?: number;
}

export interface UpdatePlan {
  upToDate: boolean;
  currentBuild: number;
  latestBuild: number;
  latestShortVersion: string;
  strategy: UpdateStrategy;
  downloadUrl: string;
  downloadSize: number;
  edSignature: string | null;
  fullSize: number;
  savingsPct: number;
}

export interface DownloadProgress {
  downloaded: number;
  total: number;
  /** Host the bytes come from, e.g. codexapp.agentsmirror.com. */
  source: string;
  /** Backend lifecycle phase; present on phase transition packets. */
  phase?: OperationPhase;
  /**
   * Backend operation id that produced this event. Present on live desktop
   * progress so a reloaded UI can reject late events from a previous op.
   */
  operationId?: string;
}

export type HistoricalReleasePlatform = "macos" | "windows";
export type HistoricalReleaseArchitecture = "arm64" | "x64";
export type HistoricalPackageFormat = "dmg" | "zip" | "msix";

export interface HistoricalReleaseAsset {
  name: string;
  size: number;
  architecture: HistoricalReleaseArchitecture;
  format: HistoricalPackageFormat;
  /** Four-part Appx identity version; Windows only. */
  packageVersion: string | null;
}

export interface HistoricalRelease {
  tag: string;
  /** Human-facing Codex app version. */
  version: string;
  publishedAt: string | null;
  assets: HistoricalReleaseAsset[];
}

export interface HistoricalReleaseCatalog {
  repository: "Wangnov/codex-app-mirror" | string;
  platform: HistoricalReleasePlatform;
  architecture: HistoricalReleaseArchitecture;
  releases: HistoricalRelease[];
}

export interface LocalReleasePackage {
  path: string;
  fileName: string;
  size: number;
  releaseTag: string;
  version: string;
  assetName: string;
  architecture: HistoricalReleaseArchitecture;
  format: HistoricalPackageFormat;
  packageVersion: string | null;
}

/** USB bundle evidence returned by the offline installer commands. */
export interface OfflinePayloadReport {
  ready: boolean;
  manifestPath: string;
  payloadPath: string;
  fileName: string;
  size: number;
  sha256: string;
  hashVerified: boolean;
  packageName: string;
  packageVersion: string;
  codexDesktopVersion: string;
  architecture: "x64" | "arm64" | string;
  minWindowsBuild: string;
  appEntry: string | null;
  appVersionVerified: boolean;
  releaseTag: string;
  authenticode: {
    trusted: boolean;
    publisherIsOpenai: boolean;
    status: string;
    statusMessage: string;
    subject: string;
    issuer: string;
    thumbprint: string;
  } | null;
  warning: string | null;
}

export interface OfflineStageReport {
  sourcePath: string;
  stagedPath: string;
  size: number;
  sha256: string;
  payload: OfflinePayloadReport;
}

/** Result of validating and launching the bundled Codex Switch configurator. */
export interface CcswitchLaunchReport {
  available: boolean;
  installed: boolean;
  launched: boolean;
  sourcePath: string | null;
  installedPath: string | null;
  copiedFiles: number;
  /** `usb` launches directly from the removable bundle; `local` copies to C:. */
  launchMode?: "usb" | "local" | "none" | string;
  /** CCSwitch keeps its settings in the current user's profile. */
  dataScope?: string;
  /** True only after a sizeable product window was observed and presented. */
  usableWindowConfirmed?: boolean;
  /** USB Windows builds use child-scoped WebView2 software rendering. */
  graphicsMode?: "software-rendering" | "unknown" | string;
  /** True when the signed USB key was passed to CCSwitch's startup bootstrap. */
  accountInjected?: boolean;
    /** `verified` means CCSwitch read back the saved key and active provider. */
    /** Other states include `pending`, `unconfirmed`, `missing-binding`, `failed`, or `unavailable`. */
    accountInjectionStatus?: string;
  message: string;
}

export interface AccountStatus {
  bound: boolean;
  accountName: string | null;
  loggedIn: boolean;
  balance: unknown;
  usage: unknown;
  message: string;
}

export interface GatewayModelProbeRequest {
  requestId: string;
  selectedModelIds: string[];
  costConsent: boolean;
  connectivityModelId?: string | null;
  reasoningModelId?: string | null;
  reasoningEfforts?: string[];
  reasoningCostConsent?: boolean;
}
export interface GatewayProbeModel {
  id: string;
  ownedBy: string | null;
  selectable: boolean;
}
export interface GatewayReasoningEffortResult {
  effort: string;
  status: "accepted_by_route" | "rejected_by_route" | "unknown";
  errorCode: string | null;
}
export interface GatewayProbeCheck {
  name: string;
  passed: boolean;
}
export interface GatewayProbeModelResult {
  modelId: string;
  passed: boolean;
  generatedRequests: number;
  checks: GatewayProbeCheck[];
  errorCode: string | null;
}
export interface GatewayModelProbeReport {
  status: "connected" | "passed" | "failed" | "cancelled" | "skipped" | string;
  endpoint: string;
  credentialFingerprint: string;
  connectionFingerprint: string;
  checkedAt: number;
  models: GatewayProbeModel[];
  results: GatewayProbeModelResult[];
  generatedRequests: number;
  errorCode: string | null;
  connectivityModelId?: string;
  connectivityProofId?: string;
  connectivityProofExpiresAt?: number;
  reasoningModelId?: string;
  reasoningResults?: GatewayReasoningEffortResult[];
  reasoningProofId?: string;
}

export interface MediaModelCatalog {
  models: string[];
  configuredSeedanceModel?: string | null;
  configuredAgentId?: string | null;
}

export interface LocalMediaModelSettings {
  licensedCredentialAvailable: boolean;
  mediaGenerationAgentId: string;
  image2Model: string;
  grokImageModel: string;
  grokVideoModel: string;
  seedanceModel: string;
  minimaxVideoModel: string;
}

export interface NativeModelCatalogActionRequest {
  operation: "state" | "add" | "reset";
  requestId?: string;
  modelId?: string;
  connectivityProofId?: string;
}

export interface NativeModelCatalogActionReport {
  status: string;
  modelIds: string[];
  restartRequired: boolean;
  message: string;
  errorCode?: string | null;
  commitState?: "not_started" | "started" | "uncertain" | string;
  proofState?: "valid" | "pending_validation" | "consumed" | "invalid" | "expired" | "none" | string;
  retryable?: boolean;
  connectionFingerprint?: string | null;
  credentialFingerprint?: string | null;
}
/** Options for the offline capability-pack installer. Secrets are never
 * serialized to the USB bundle; the native command uses them only for the
 * current install and writes configured values to the current user's private
 * environment after a successful run. */
export interface CapabilityPackInstallOptions {
  enableFeishu: boolean;
  persistConnections: boolean;
  feishuAppId: string;
  feishuAppSecret: string;
  feishuUserAccessToken: string;
  feishuDomain: string;
  feishuAuthMode: string;
  enabledApiServices: string[];
  enabledRemoteMcps: string[];
  enableMediaGeneration: boolean;
  mediaGatewayCatalogStatus: "not-checked" | "verified" | "failed" | string;
  mediaGatewayModelCount: number;
  mediaGatewayCatalogVerifiedAt: string | null;
  dashscopeApiKey: string;
  dashscopeBaseUrl: string;
  dashscopeModel: string;
  mediaGenerationAgentId: string;
  image2Model: string;
  grokImageModel: string;
  grokVideoModel: string;
  seedanceModel: string;
  minimaxVideoModel: string;
}

export interface CapabilityPackReport {
  available: boolean;
  installed: boolean;
  success: boolean;
  coreReady: boolean;
  overallStatus: "ready" | "partial" | "failed" | string;
  sourcePath: string | null;
  packVersion: string | null;
  skills: number;
  agents: number;
  runtimeReady: boolean;
  runtimeStatus: "ready" | "partial" | "failed" | "skipped" | "unknown" | string;
  configStatus: "merged" | "partial" | "failed" | "conflict" | "unknown" | string;
  diagnosticsPath: string | null;
  feishuEnabled: boolean;
  enabledApiServices: string[];
  mediaRuntimeStatus: string;
  mediaProtocolStatus: string;
  mediaGatewayCatalogStatus: string;
  mediaGatewayModelCount: number;
  mediaGatewayCatalogVerifiedAt: string | null;
  mediaGenerationCapability: string;
  codexRestartRequired: boolean;
  codexRunning: boolean;
  message: string;
}

export type SkillRepoSourceKind = "domestic" | "relay" | "github";
export type SkillRepoSourceLabel =
  | "domestic-1"
  | "domestic-2"
  | "domestic-3"
  | "relay"
  | "github";
export type SkillRepoSourceAttemptPhase = "metadata" | "archive" | "verify";
export type SkillRepoSourceAttemptResult =
  | "ok"
  | "timeout"
  | "unavailable"
  | "http"
  | "invalid"
  | "hash-mismatch"
  | "cancelled";

export interface SkillRepoSourceAttempt {
  sourceId: string;
  sourceKind: SkillRepoSourceKind;
  phase: SkillRepoSourceAttemptPhase;
  result: SkillRepoSourceAttemptResult;
  httpStatus: number | null;
  elapsedMs: number;
}

export type SkillRepoErrorCode =
  | "SKILL_REPO_UNAVAILABLE"
  | "SKILL_REPO_TIMEOUT"
  | "SKILL_REPO_SOURCE_UNAVAILABLE"
  | "SKILL_REPO_ALL_SOURCES_FAILED"
  | "SKILL_REPO_SOURCE_INVALID"
  | "SKILL_REPO_STALE"
  | "SKILL_REPO_HASH_MISMATCH"
  | "SKILL_REPO_ARCHIVE_INVALID"
  | "SKILL_REPO_BASE_CONFLICT"
  | "SKILL_REPO_USER_CONFLICT"
  | "SKILL_REPO_HOT_MODIFIED"
  | "SKILL_REPO_LEDGER_INVALID"
  | "SKILL_REPO_BASE_RESOURCE_INVALID"
  | "SKILL_REPO_LOCAL_IO"
  | "SKILL_REPO_LOCKED"
  | "SKILL_REPO_ROLLBACK_FAILED"
  | "SKILL_REPO_UNSUPPORTED";

export type SkillRepoItemStatus = "applied" | "conflict" | "skipped" | "failed";
export type SkillRepoApplyStatus = "success" | "partial" | "failed" | "blocked";
export type SkillRepoRollbackStatus = "success" | "failed" | "blocked";

export interface SkillRepoExpected {
  schemaVersion: number;
  repo: string;
  branch: string;
  commit: string;
  sourceKind: SkillRepoSourceKind;
  sourceId: string;
  archiveSize: number;
  archiveSha256: string;
}

export interface SkillRepoItem {
  id: string;
  status: SkillRepoItemStatus;
  code: SkillRepoErrorCode | null;
  fileCount: number;
}

export interface SkillRepoUpdateInfo {
  schemaVersion: number;
  currentCommit: string | null;
  latestCommit: string;
  sourceKind: SkillRepoSourceKind;
  sourceLabel: SkillRepoSourceLabel;
  attempts: SkillRepoSourceAttempt[];
  archiveSize: number;
  archiveSha256: string;
  expected: SkillRepoExpected;
  updateAvailable: boolean;
  installableCount: number;
  conflictCount: number;
  skippedCount: number;
  /** Product Design is a separate offline plugin, not part of Skill Hub's repo-skill count. */
  productDesignSkillCount?: number | null;
  /** Reserved for a real non-fatal runtime notice; normal Skill Hub updates are null. */
  warning: string | null;
}

export interface SkillRepoApplyReport {
  schemaVersion: number;
  status: SkillRepoApplyStatus;
  identity: SkillRepoExpected;
  applied: SkillRepoItem[];
  conflicts: SkillRepoItem[];
  skipped: SkillRepoItem[];
  failed: SkillRepoItem[];
  lastGoodGeneration: string | null;
  reloadedRequired: boolean;
  logPath: string | null;
  productDesignSkillCount?: number;
}

export interface SkillRepoRollbackReport {
  schemaVersion: number;
  status: SkillRepoRollbackStatus;
  restoredGeneration: string | null;
  restoredSkills: string[];
  reloadedRequired: boolean;
  logPath: string | null;
}

/** Result of removing the per-user CCSwitch copy installed by the manager. */
export interface CcswitchUninstallReport {
  success: boolean;
  installedBefore: boolean;
  removedInstall: boolean;
  removedAutostart: boolean;
  purgedUserData: boolean;
  installDir: string;
  dataDirs: string[];
  message: string;
  notes: string[];
}

export interface HistoricalInstallSelection {
  releaseTag: string;
  version: string;
  assetName: string;
  architecture: HistoricalReleaseArchitecture;
  format: HistoricalPackageFormat;
  packageVersion: string | null;
  localPath: string | null;
  localFileName: string | null;
}

/** Frozen install snapshot from the original confirmation. Resume must reuse it
 * even if a renderer reload has since refreshed the visible installation. */
export type HistoricalResumeExpectation =
  | {
      platform: "macos";
      currentPath: string | null;
      currentBuild: number | null;
    }
  | {
      platform: "windows";
      currentPath: string | null;
      currentVersion: string | null;
      currentSource: string | null;
    };

export interface MacUpdateReport {
  appcastUrl: string;
  installed: InstalledCodex | null;
  simulatedBuild: number | null;
  plan: UpdatePlan | null;
  /** Sparkle <pubDate> of the appcast item matching the INSTALLED build (the
   *  true release date of the running version), when the feed publishes it. */
  installedPubDate?: string | null;
  /** Sparkle <pubDate> of the latest appcast item, used for update metadata. */
  latestPubDate?: string | null;
}

export interface MacPerformReport {
  upToDate: boolean;
  fromBuild: number;
  toBuild: number;
  strategy: string;
  installedPath: string;
  verified: boolean;
  relaunched: boolean;
  /** Codex was running but the relaunch failed — the user must start it
   *  manually. Distinct from `!relaunched`, which also covers the clean case
   *  where Codex simply wasn't running (no action needed). */
  relaunchFailed: boolean;
  rolledBack: boolean;
  /** Non-fatal warning to surface on an otherwise successful update (e.g. a
   *  provenance save failure, or where the previous backup was kept). null when
   *  the update was fully clean. */
  warning: string | null;
  message: string;
}

export type InstallClass = "managed" | "external" | "none";

/** UI-facing install probe state — distinguishes a failed status query from
 *  "external" so we never mislabel probe errors as unmanaged installs. */
export type InstallProbeState =
  "loading" | "managed" | "external" | "none" | "error";

/** Ancillary step result inside a structured operation outcome. */
export interface StepOutcome {
  /** "ok" | "failed" | "skipped" | "not_applicable" */
  state: string;
  detail: string | null;
}

/**
 * Structured result of install/uninstall (and similar) ops.
 * Primary work can succeed while provenance/cleanup fail — the UI must show
 * disk truth and only retry the failed ancillary steps.
 */
export interface OperationOutcome {
  primaryOk: boolean;
  /** "present" | "absent" | "unknown" */
  appState: string;
  installClass: InstallClass | string | null;
  /** Install path context for targeted recovery (clear a specific record, etc.). */
  path?: string | null;
  provenance: StepOutcome;
  cleanup: StepOutcome;
  warnings: string[];
  /** Stable keys: record_provenance | clear_provenance | cleanup_metadata | purge_user_data */
  recoveryActions: string[];
}

export interface AncillaryRetryRequest {
  actions: string[];
  path?: string | null;
  purgeUserData?: boolean;
}

export interface AncillaryRetryReport {
  outcome: OperationOutcome;
  message: string;
}

export interface MacInstallStatus {
  installed: InstalledCodex | null;
  status: InstallClass;
  /** Present when several Codex-lineage installs coexist (e.g. an old
   *  Codex.app plus a hand-dragged post-rebrand ChatGPT.app); the user should
   *  adopt one explicitly. Absent when unambiguous. */
  ambiguousPaths?: string[];
  /** Present after install mutators that can partial-succeed. */
  outcome?: OperationOutcome | null;
}

export type UpdateSourceKind = "auto" | "mirror" | "official" | "custom";
export type CodexPayloadSourceKind = "auto" | "mirror" | "relay" | "github";
export type WindowsInstallMode = "msix" | "portable";
export type ProxyMode = "system" | "direct" | "custom";
export type CodexUpdatePlatform = "macos" | "windows";

export interface SkippedCodexUpdate {
  platform: CodexUpdatePlatform;
  /** Stable per-release target: macOS build or Windows package moniker. */
  target: string;
  /** Human-facing version shown in Settings. */
  version: string;
  /** Unix timestamp in milliseconds. */
  skippedAt: number;
}

/** A user-defined grouping of local skins. */
export interface SkinGroup {
  id: string;
  name: string;
  skinIds: string[];
}

export interface AppSettings {
  source: UpdateSourceKind;
  customUrl: string;
  /** Legacy compatibility alias for automatic checks. */
  autoCheck: boolean;
  /** Check once when the manager home screen starts. */
  checkOnStartup: boolean;
  /** Keep checking while the manager is open. */
  periodicCheck: boolean;
  /** Periodic check cadence, in seconds. Defaults to 15 minutes. */
  periodicCheckIntervalSeconds: number;
  askBefore: boolean;
  /** Always true at the backend; surfaced read-only in the UI. */
  signedOnly: boolean;
  /** Ask for confirmation before closing (quitting) the window. */
  confirmClose: boolean;
  /** Windows payload install preference. MSIX still falls back safely if blocked. */
  windowsInstallMode: WindowsInstallMode;
  /** Remembered portable install root for Windows. */
  installRoot: string;
  /** Network proxy behavior for update checks and downloads. */
  proxyMode: ProxyMode;
  /** Proxy URL used when proxyMode is custom. */
  customProxyUrl: string;
  /** Disable Codex App's own embedded update checks and silent downloads. */
  disableCodexSelfUpdates: boolean;
  /** One exact Codex app update the user chose not to be reminded about. */
  skippedCodexUpdate: SkippedCodexUpdate | null;
  /** Persistent Codex UI theme selection (theme id); null = stock. */
  codexTheme: string | null;
  /** Extra local directory scanned for theme packages (dev workflow). */
  codexThemeDir: string | null;
  /** Managed skin store; null = platform default. */
  codexThemeStoreDir: string | null;
  /** User-defined local skin groups, in display order. */
  skinGroups: SkinGroup[];
  /** Source strategy for the Codex payload channel. */
  codexPayloadSourceKind: CodexPayloadSourceKind;
  /** Base URL used by the Codex payload Relay source. */
  codexPayloadBaseUrl: string;
  /**
   * GitHub repository ("owner/repo") hosting the codex payload releases used
   * by the codex update channel. Empty string = channel's baked-in default.
   */
  codexPayloadRepo: string;
}

export interface ConfigHealth {
  settingsStatus: string;
  provenanceStatus: string;
  unknownSource: string | null;
  detail: string | null;
  settingsBackupAvailable: boolean;
  provenanceBackupAvailable: boolean;
}

export type ConfigWhich = "settings" | "provenance";

export interface Diagnostics {
  appVersion: string;
  os: string;
  arch: string;
  locale: string | null;
  updateSource: string;
  customSourceHost: string | null;
  windowsInstallMode: string | null;
  installStatus: string;
  configHealth: ConfigHealth;
  logsDir: string | null;
  recentErrors: string[];
  logTail: string;
  generatedAtUnix: number;
}

export const DEFAULT_SETTINGS: AppSettings = {
  source: "auto",
  customUrl: "",
  autoCheck: true,
  checkOnStartup: true,
  periodicCheck: true,
  periodicCheckIntervalSeconds: 15 * 60,
  askBefore: true,
  signedOnly: true,
  confirmClose: true,
  windowsInstallMode: "msix",
  installRoot: "%LOCALAPPDATA%\\Programs\\Codex",
  proxyMode: "system",
  customProxyUrl: "",
  disableCodexSelfUpdates: false,
  skippedCodexUpdate: null,
  codexTheme: null,
  codexThemeDir: null,
  codexThemeStoreDir: null,
  skinGroups: [],
  codexPayloadSourceKind: "auto",
  codexPayloadBaseUrl: "https://codexapp.agentsmirror.com/",
  codexPayloadRepo: "",
};

export interface MacUninstallReport {
  removed: boolean;
  keptCodexHome: boolean;
  message: string;
  outcome: OperationOutcome;
}

export interface InstalledWindowsCodex {
  path: string;
  /** Human-facing Codex app version; falls back to the MSIX package version when unreadable. */
  version: string;
  arch: string | null;
  source: "msix" | "portable" | string;
  packageFamilyName: string | null;
  /** Registered AppX volume root, e.g. `F:\\` for the USB edition. */
  packageVolume?: string | null;
  /** Install-dir / executable mtime as a Unix timestamp (seconds). */
  installedAt?: number | null;
}

export interface WindowsRelease {
  /** Human-facing Codex app version, e.g. 26.623.42026. */
  version: string;
  /** Four-part Windows MSIX package version, e.g. 26.623.5546.0. */
  packageVersion: string;
  /** Release/publish timestamp for this Windows package, when the manifest provides it. */
  releasedAt?: string | null;
  packageMoniker: string;
  architecture: string | null;
  contentLength: number | null;
  etag: string | null;
  storeProductId: string | null;
  packageIdentity: string | null;
}

export type CapabilityState = "available" | "unavailable" | "unknown";

export interface CapabilityCheck {
  state: CapabilityState;
  detail: string;
}

export type SideloadRecommendation = "msix-preferred" | "portable-fallback";

export interface WinCapabilityReport {
  addAppxPackage: CapabilityCheck;
  appxService: CapabilityCheck;
  sideloadPolicy: CapabilityCheck;
  appInstaller: CapabilityCheck;
  /** Can the WinRT PackageManager actually activate? Catches the "registered but
   * broken" machines where MSIX deploy dies with 0x80040154 (没有注册类). */
  msixDeployment: CapabilityCheck;
  meteredNetwork: CapabilityCheck;
  recommendation: SideloadRecommendation;
  notes: string[];
}

export type WinInstallRoute = "msix-sideload" | "portable-fallback";

export interface WindowsUpdatePlan {
  upToDate: boolean;
  currentVersion: string | null;
  latestVersion: string;
  packageMoniker: string;
  packageUrl: string;
  downloadSize: number | null;
  sha256: string;
  route: WinInstallRoute;
  portableFallbackReady: boolean;
  warnings: string[];
}

export interface WinUpdateReport {
  manifestUrl: string;
  checksumsUrl: string;
  packageUrl: string;
  release: WindowsRelease;
  installed: InstalledWindowsCodex | null;
  capabilities: WinCapabilityReport;
  plan: WindowsUpdatePlan;
}

export interface AuthenticodeReport {
  trusted: boolean;
  publisherIsOpenai: boolean;
  status: string;
  statusMessage: string;
  subject: string;
  issuer: string;
  thumbprint: string;
}

export interface MsixIdentity {
  name: string;
  publisher: string;
  version: string;
  processorArchitecture: string;
}

export interface WinStageReport {
  upToDate: boolean;
  route: WinInstallRoute;
  latestVersion: string;
  packageMoniker: string;
  downloadSize: number;
  stagedPath: string | null;
  sha256: string;
  hashVerified: boolean;
  authenticode: AuthenticodeReport | null;
  identity: MsixIdentity | null;
  identityVerified: boolean;
  installReady: boolean;
  portableFallbackReady: boolean;
  notes: string[];
}

export interface MsixSideloadReport {
  success: boolean;
  message: string;
  installed: InstalledWindowsCodex | null;
  fallbackRecommended: boolean;
  rawError: string | null;
  /** HRESULT returned by Windows AppX deployment, when present. */
  hresult?: string | null;
  /** AppX deployment ActivityId, useful for Get-AppxLog/Event Viewer. */
  activityId?: string | null;
  /** Bounded deployment-server log captured for this ActivityId. */
  deploymentLog?: string | null;
}

export interface MsixHealthReport {
  healthy: boolean;
  /**
   * Whether the health probe actually ran. When false, `healthy` is a
   * conservative "keep the MSIX" default (the probe could not run), not an
   * observed clean bill of health. Use this to tell "verified healthy" apart
   * from "kept because unverifiable".
   */
  verified: boolean;
  packageRegistered: boolean;
  /** Raw Get-AppxPackage Status string (e.g. "Ok", "Modified"). */
  status: string;
  statusOk: boolean;
  aumidResolved: boolean;
  /** Declared framework dependencies missing on this machine. */
  missingDependencies: string[];
  /**
   * Real shell activation left a process under the package install location
   * for the liveness window. Registration alone is not enough.
   */
  activationOk?: boolean;
  /**
   * Machine-stable failure class for notes / routing. Empty when healthy.
   * Values: not-registered | status-bad | aumid-unresolved |
   * missing-dependencies | activation-failed | immediate-exit | timeout |
   * probe-failed | policy.
   */
  failureKind?: string;
  /** Human-facing reason when unhealthy; empty when healthy. */
  reason: string;
}

/**
 * Outcome of `win_perform_update`. Enumerates the exact action strings the
 * backend sets in src-tauri/src/app/win_update.rs:
 *   - "none"                                   — already up to date.
 *   - "msix-sideload"                          — MSIX sideload succeeded.
 *   - "portable-fallback"                      — user chose portable mode.
 *   - "portable-fallback-after-msix-failure"   — sideload failed, fell back.
 *   - "portable-fallback-after-msix-unhealthy" — sideload registered but the
 *                                                package failed its health check.
 *   - "portable-fallback-missing-framework"    — staged MSIX declared framework
 *                                                dependencies absent locally.
 */
export type WinPerformAction =
  | "none"
  | "msix-sideload"
  | "portable-fallback"
  | "portable-fallback-after-msix-failure"
  | "portable-fallback-after-msix-unhealthy"
  | "portable-fallback-missing-framework";

export interface WinPerformReport {
  success: boolean;
  action: WinPerformAction;
  message: string;
  stage: WinStageReport;
  sideload: MsixSideloadReport | null;
  portable: PortableInstallReport | null;
  msixHealth: MsixHealthReport | null;
  installed: InstalledWindowsCodex | null;
  fallbackAvailable: boolean;
  fallbackAttempted: boolean;
  notes: string[];
  outcome: OperationOutcome;
}

export interface PortableInstallReport {
  success: boolean;
  installRoot: string;
  executablePath: string | null;
  version: string;
  backupPath: string | null;
  shortcutCreated: boolean;
  uninstallEntryCreated: boolean;
  relaunched: boolean;
  message: string;
  notes: string[];
}

export interface MsixRemoveReport {
  success: boolean;
  message: string;
  rawError: string | null;
  notes: string[];
}

export interface PortableUninstallReport {
  success: boolean;
  partial: boolean;
  installRoot: string;
  removedFiles: boolean;
  removedShortcut: boolean;
  removedUninstallEntry: boolean;
  purgedUserData: boolean;
  message: string;
  notes: string[];
}

/**
 * Outcome of `win_uninstall`. Enumerates the exact action strings the backend
 * sets in src-tauri/src/app/win_update.rs:
 *   - "none"                 — nothing installed to remove.
 *   - "external-not-managed" — detected install isn't manager-managed; refused.
 *   - "remove-msix"          — removed the sideloaded MSIX package.
 *   - "remove-portable"      — removed the portable install.
 */
export type WinUninstallAction =
  "none" | "external-not-managed" | "remove-msix" | "remove-portable";

export interface WinUninstallReport {
  success: boolean;
  action: WinUninstallAction;
  message: string;
  installedBefore: InstalledWindowsCodex | null;
  msix: MsixRemoveReport | null;
  portable: PortableUninstallReport | null;
  purgedUserData: boolean;
  notes: string[];
  outcome: OperationOutcome;
}

/** Combined Windows cleanup used by the USB build. */
export interface WinBundleUninstallReport {
  success: boolean;
  codex: WinUninstallReport | null;
  codexError: string | null;
  ccswitch: CcswitchUninstallReport | null;
  ccswitchError: string | null;
  message: string;
  notes: string[];
  purgedUserData: boolean;
  purgedCcswitchData: boolean;
  /** Phase 1 finished; no final filesystem success is claimed before exit. */
  cleanupPending?: boolean;
  /** USB applications were removed, but shared recovery data was retained. */
  cleanupDeferred?: boolean;
  cleanupReportPath?: string | null;
  cleanupHandoffId?: string | null;
  outcome: OperationOutcome;
}

export interface UninstallHandoffStatus {
  id: string;
  reportPath: string;
  message: string;
}

export function emptyStepOutcome(state = "not_applicable"): StepOutcome {
  return { state, detail: null };
}

export function emptyOperationOutcome(
  overrides: Partial<OperationOutcome> = {},
): OperationOutcome {
  return {
    primaryOk: true,
    appState: "unknown",
    installClass: null,
    path: null,
    provenance: emptyStepOutcome("ok"),
    cleanup: emptyStepOutcome("ok"),
    warnings: [],
    recoveryActions: [],
    ...overrides,
  };
}

export function outcomeIsPartial(
  outcome: OperationOutcome | null | undefined,
): boolean {
  if (!outcome?.primaryOk) return false;
  return (
    outcome.provenance.state === "failed" || outcome.cleanup.state === "failed"
  );
}

export interface WinInstallStatus {
  installed: InstalledWindowsCodex | null;
  status: InstallClass;
  /** Older managers omit this field; current USB builds always report it. */
  recoveryPending?: boolean;
}

/** Local removable-media lease/watchdog status for the USB build. */
export interface UsbManagedProcess {
  pid: number;
  role: string;
  executable: string | null;
  startedAtMs: number;
}

export interface UsbGuardStatus {
  schemaVersion: number;
  status:
    | "authorized"
    | "grace"
    | "missing"
    | "mismatch"
    | "unsupported"
    | "disabled"
    | "stopped"
    | string;
  watchdog: "running" | "not-running" | string;
  removable: boolean;
  driveRoot: string | null;
  bundlePath: string | null;
  bundleRelativePath: string | null;
  signed: boolean;
  /** Current license classification; older managers may omit these fields. */
  signatureState?:
    | "not-applicable"
    | "missing"
    | "unsigned-development"
    | "unsigned"
    | "valid"
    | "invalid"
    | "fingerprint-mismatch"
    | "unknown-key"
    | "legacy-unsupported"
    | string;
  /** Backend's current signed Codex decision; never trusted as authorization. */
  codexAllowed?: boolean;
  codexBlockReason?: string | null;
  /** Signed remote license decision from the manager's USB startup gate. */
  revocationState?: string;
  revocationStateVersion?: number | null;
  /** Only true for the narrowly authorized active-after-revoked reinstall state. */
  reinstallAllowed?: boolean;
  lastCheckedAtMs: number;
  missingSinceMs: number | null;
  countdownSeconds: number | null;
  managedProcesses: UsbManagedProcess[];
  message: string;
}

export interface UsbInstallPreflightReport {
  internetAvailable: boolean;
  apiKeyReady: boolean;
}

/** Delivery metadata (optional schemaVersion-2 fields; packer enforces). */
export interface CodexThemeMeta {
  version: string | null;
  author: string | null;
  /** Codex version the theme was verified against at build time. */
  codexVerified: string | null;
  appearance: "dark" | "light" | "dual" | null;
  tags: string[];
  license: string | null;
  /** Package-relative preview images; first is the cover. */
  previews: string[];
}

/** A locally installed Codex UI theme package (codex-theme-engine). */
export interface CodexThemeSummary {
  id: string;
  name: string;
  description: string;
  dir: string;
  hasNativeTheme: boolean;
  /** Color tokens from theme.json — swatch/fallback card art. */
  colors: Record<string, string>;
  /** Absolute path of the cover preview when the package ships one. */
  preview: string | null;
  meta: CodexThemeMeta;
  /** Which root produced this entry. The list may carry the same id twice
   *  (dev checkout shadowing the store copy); dev entries come first. */
  origin: "dev" | "store";
}

export interface CodexThemeDaemonStatus {
  running: boolean;
  port: number;
  themeId: string | null;
  stamp: string | null;
  connectedTargets: number;
  lastError: string | null;
}

export interface CodexThemeStatusReport {
  /** Whether this platform can theme Codex (macOS and Windows). */
  supported: boolean;
  /** The persisted selection — what manager-launches will apply. */
  activeTheme: string | null;
  daemon: CodexThemeDaemonStatus | null;
  /** A CDP endpoint answers on the theme port right now. */
  cdpReady: boolean;
  codexRunning: boolean;
  /** A pristine config.toml appearance backup exists (full restore possible). */
  nativeBackupPresent: boolean;
  /** Where managed skins currently live (downloads/imports land here). */
  storeDir: string | null;
  /** A pre-try-on native settings stash exists (the try-on is undoable). */
  tryOnStash: boolean;
  /** An unresolved native transaction blocks native theme operations. */
  recoveryRequired: boolean;
}

/** Result of relocating the skin store. */
export interface StoreMigrationReport {
  from: string;
  to: string;
  moved: string[];
  /** Ids left untouched because the destination already had them. */
  skipped: string[];
}

/** One entry of the online skin catalog (skins.agentsmirror.com). */
export interface CatalogSkin {
  id: string;
  name: string;
  description: string;
  version: string;
  author: string;
  appearance: "dark" | "light" | "dual" | null;
  license: string | null;
  tags: string[];
  codexVerified: string | null;
  bytes: number;
  sha256: string;
  /** Catalog-relative paths, resolved backend-side against the pinned origin. */
  pack: string;
  preview: string;
  /** Theme category for store grouping (anime/stars/tech/guofeng/games);
   *  absent → grouped under "other". */
  category?: string | null;
}

/** Main-window form factor: `compact` is the fixed 400×640 dashboard,
 *  `expanded` the resizable desktop-sized workbench. */
export type WindowMode = "compact" | "expanded";

/** What the backend actually applied (post-clamp), echoed so the frontend
 *  persists reality instead of its request. Sizes are logical px. */
export interface WindowModeReport {
  mode: WindowMode;
  width: number;
  height: number;
}
