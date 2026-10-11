package revocation

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"embed"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"io/fs"
	"log/slog"
	"mime"
	"net"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	adminCookieName     = "codex_usb_admin"
	adminSessionTTL     = 8 * time.Hour
	adminPasswordRounds = 600_000
	adminMaxSessions    = 256
	adminDefaultPage    = 50
	adminMaxPage        = 200
	adminMaxPassword    = 128
)

var adminUsernamePattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$`)

//go:embed adminweb/*
var adminWebFiles embed.FS

type adminSession struct {
	username          string
	csrf              string
	expires           time.Time
	setupPending      bool
	pendingTOTPSecret string
}

type adminConsole struct {
	store           *Store
	username        string
	origin          string
	cookieSecure    bool
	authStatePath   string
	authState       adminAuthState
	logger          *slog.Logger
	loginLimit      *tokenBucket
	changeLimit     *tokenBucket
	authMu          sync.Mutex
	authUnavailable bool
	mu              sync.Mutex
	sessions        map[[32]byte]adminSession
}

type adminLoginRequest struct {
	Username string `json:"username"`
	Password string `json:"password"`
}

type adminChangePasswordRequest struct {
	CurrentPassword string `json:"currentPassword"`
	NewPassword     string `json:"newPassword"`
	TOTPCode        string `json:"totpCode"`
}

type adminInitialSetupRequest struct {
	NewPassword string `json:"newPassword"`
	TOTPCode    string `json:"totpCode"`
}

type adminRegistrationRequest struct {
	Registration RegistrationEnvelope `json:"registration"`
}

type adminRevokeRequest struct {
	Reason                string `json:"reason"`
	ConfirmInstallationID string `json:"confirmInstallationId"`
}

type adminLicenseSummary struct {
	Product                string     `json:"product"`
	InstallationID         string     `json:"installationId"`
	USBAssetID             string     `json:"usbAssetId,omitempty"`
	LicenseKeyID           string     `json:"licenseKeyId"`
	LicenseSignatureSHA256 string     `json:"licenseSignatureSha256"`
	FingerprintSHA256      string     `json:"fingerprintSha256"`
	Status                 string     `json:"status"`
	StateVersion           int64      `json:"stateVersion"`
	CreatedAt              time.Time  `json:"createdAt"`
	UpdatedAt              time.Time  `json:"updatedAt"`
	RevokedAt              *time.Time `json:"revokedAt,omitempty"`
}

func validAdminUsername(value string) bool {
	return adminUsernamePattern.MatchString(value)
}

func validateAdminOrigin(raw string, cookieSecure bool) error {
	parsed, err := url.Parse(raw)
	if err != nil || parsed == nil || parsed.Host == "" || parsed.User != nil || parsed.Opaque != "" || parsed.Path != "" || parsed.RawQuery != "" || parsed.Fragment != "" {
		return errors.New("origin must contain only scheme and host")
	}
	if parsed.String() != raw {
		return errors.New("origin must be canonical and must not end with a slash")
	}
	if parsed.Scheme == "https" {
		if !cookieSecure {
			return errors.New("https admin origins require secure cookies")
		}
		return nil
	}
	if cookieSecure {
		return errors.New("secure admin cookies require an https origin")
	}
	if parsed.Scheme != "http" || net.ParseIP(parsed.Hostname()) == nil || !net.ParseIP(parsed.Hostname()).IsLoopback() {
		return errors.New("http is allowed only for a loopback origin when secure cookies are disabled")
	}
	return nil
}

func newAdminConsole(store *Store, cfg HTTPConfig, logger *slog.Logger) (*adminConsole, error) {
	if cfg.AdminUsername == "" || cfg.AdminPassword == "" || !validAdminUsername(cfg.AdminUsername) || len(cfg.AdminPassword) < 12 || len(cfg.AdminPassword) > 1024 {
		return nil, errors.New("admin credentials are incomplete or invalid")
	}
	if cfg.AdminAuthStatePath == "" {
		return nil, errors.New("admin auth state path is required")
	}
	if err := validateAdminOrigin(cfg.AdminOrigin, cfg.AdminCookieSecure); err != nil {
		return nil, err
	}
	if logger == nil {
		logger = slog.Default()
	}
	state, err := loadOrInitializeAdminAuth(cfg.AdminAuthStatePath, cfg.AdminPassword, cfg.AdminTOTPSecret)
	if err != nil {
		return nil, err
	}
	return &adminConsole{
		store: store, username: cfg.AdminUsername, origin: cfg.AdminOrigin,
		cookieSecure: cfg.AdminCookieSecure, authStatePath: cfg.AdminAuthStatePath,
		authState: state,
		logger:    logger, loginLimit: newTokenBucket(10, 5), changeLimit: newTokenBucket(3, 3), sessions: make(map[[32]byte]adminSession),
	}, nil
}

func (a *adminConsole) Register(mux *http.ServeMux) {
	static, err := fs.Sub(adminWebFiles, "adminweb")
	if err != nil {
		panic("embedded admin web assets are invalid")
	}
	mux.HandleFunc("GET /admin", func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, "/admin/", http.StatusPermanentRedirect)
	})
	mux.HandleFunc("GET /admin/", a.serveIndex)
	mux.Handle("GET /admin/static/", http.StripPrefix("/admin/static/", http.FileServer(http.FS(static))))
	mux.HandleFunc("POST /admin/api/login", a.login)
	mux.HandleFunc("GET /admin/api/initial-setup", a.initialSetupInfo)
	mux.HandleFunc("POST /admin/api/initial-setup", a.completeInitialSetup)
	mux.HandleFunc("POST /admin/api/change-password", a.changePassword)
	mux.HandleFunc("GET /admin/api/session", a.sessionInfo)
	mux.HandleFunc("POST /admin/api/logout", a.logout)
	mux.HandleFunc("GET /admin/api/licenses", a.listLicenses)
	mux.HandleFunc("POST /admin/api/licenses", a.importLicense)
	mux.HandleFunc("POST /admin/api/licenses/{installationID}/revoke", a.revokeLicense)
}

func (a *adminConsole) serveIndex(w http.ResponseWriter, r *http.Request) {
	a.setSecurityHeaders(w)
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	data, err := adminWebFiles.ReadFile("adminweb/index.html")
	if err != nil {
		writeAdminError(w, http.StatusInternalServerError, "页面暂不可用")
		return
	}
	_, _ = w.Write(data)
}

func (a *adminConsole) setSecurityHeaders(w http.ResponseWriter) {
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("X-Frame-Options", "DENY")
	w.Header().Set("Referrer-Policy", "no-referrer")
	w.Header().Set("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'")
}

func (a *adminConsole) login(w http.ResponseWriter, r *http.Request) {
	a.setSecurityHeaders(w)
	if !a.originAllowed(r) {
		writeAdminError(w, http.StatusForbidden, "请求来源不允许")
		return
	}
	if !a.loginLimit.allow(time.Now()) {
		writeAdminError(w, http.StatusTooManyRequests, "登录尝试过多，请稍后再试")
		return
	}
	request, err := readAdminJSON[adminLoginRequest](w, r, 8<<10)
	if err != nil || len(request.Password) > 1024 {
		writeAdminError(w, http.StatusBadRequest, "登录信息无效")
		return
	}
	userMatch := subtle.ConstantTimeCompare([]byte(request.Username), []byte(a.username))
	if userMatch != 1 {
		writeAdminError(w, http.StatusUnauthorized, "账号或密码错误")
		return
	}
	sessionToken := make([]byte, 32)
	csrfToken := make([]byte, 32)
	if _, err := rand.Read(sessionToken); err != nil {
		writeAdminError(w, http.StatusInternalServerError, "登录暂不可用")
		return
	}
	if _, err := rand.Read(csrfToken); err != nil {
		writeAdminError(w, http.StatusInternalServerError, "登录暂不可用")
		return
	}
	token := base64.RawURLEncoding.EncodeToString(sessionToken)
	csrf := base64.RawURLEncoding.EncodeToString(csrfToken)
	passwordMatches, sessionAdded, setupRequired := a.addSessionForPassword(request.Password, sessionToken, adminSession{username: a.username, csrf: csrf, expires: time.Now().Add(adminSessionTTL)})
	if !passwordMatches {
		writeAdminError(w, http.StatusUnauthorized, "账号或密码错误")
		return
	}
	if !sessionAdded {
		writeAdminError(w, http.StatusServiceUnavailable, "当前登录人数过多，请稍后再试")
		return
	}
	http.SetCookie(w, &http.Cookie{
		Name: adminCookieName, Value: token, Path: "/admin/", HttpOnly: true,
		Secure: a.cookieSecure, SameSite: http.SameSiteStrictMode,
		Expires: time.Now().Add(adminSessionTTL), MaxAge: int(adminSessionTTL.Seconds()),
	})
	writeAdminJSON(w, http.StatusOK, map[string]any{"username": a.username, "csrfToken": csrf, "setupRequired": setupRequired})
}

func (a *adminConsole) initialSetupInfo(w http.ResponseWriter, r *http.Request) {
	a.setSecurityHeaders(w)
	session, digest, ok := a.readSession(r)
	if !ok {
		writeAdminError(w, http.StatusUnauthorized, "登录已失效")
		return
	}
	if !session.setupPending {
		writeAdminError(w, http.StatusConflict, "首次安全设置已完成")
		return
	}
	if !a.loginLimit.allow(time.Now()) {
		writeAdminError(w, http.StatusTooManyRequests, "请求过于频繁，请稍后重试")
		return
	}
	secret, err := a.pendingInitialSetupSecret(digest)
	if err != nil {
		writeAdminError(w, http.StatusServiceUnavailable, "首次安全设置暂不可用")
		return
	}
	writeAdminJSON(w, http.StatusOK, map[string]any{"totpSecret": secret, "username": session.username})
}

func (a *adminConsole) completeInitialSetup(w http.ResponseWriter, r *http.Request) {
	a.setSecurityHeaders(w)
	session, digest, ok := a.readSession(r)
	if !ok || !session.setupPending || !a.validMutation(r, session) {
		writeAdminError(w, http.StatusForbidden, "登录状态或请求校验无效")
		return
	}
	if !a.changeLimit.allow(time.Now()) {
		writeAdminError(w, http.StatusTooManyRequests, "操作过于频繁，请稍后重试")
		return
	}
	request, err := readAdminJSON[adminInitialSetupRequest](w, r, 8<<10)
	if err != nil || len(request.NewPassword) > adminMaxPassword {
		writeAdminError(w, http.StatusBadRequest, "首次安全设置无效")
		return
	}
	if err := a.completeInitialAdminSetup(digest, request.NewPassword, request.TOTPCode); err != nil {
		if errors.Is(err, errAdminAuthRejected) {
			writeAdminError(w, http.StatusUnauthorized, "新密码或动态验证码无效，请检查后重试")
			return
		}
		writeAdminError(w, http.StatusServiceUnavailable, "认证状态暂不可用，请联系运维重启服务")
		return
	}
	a.logger.Info("admin first-login security setup completed", "actor", session.username)
	clearAdminSessionCookie(w, a.cookieSecure)
	writeAdminJSON(w, http.StatusOK, map[string]bool{"ok": true})
}

func (a *adminConsole) changePassword(w http.ResponseWriter, r *http.Request) {
	a.setSecurityHeaders(w)
	session, _, ok := a.readSession(r)
	if !ok || !a.validMutation(r, session) {
		writeAdminError(w, http.StatusForbidden, "登录状态或请求校验无效")
		return
	}
	if session.setupPending {
		writeAdminError(w, http.StatusForbidden, "请先完成首次安全设置")
		return
	}
	if !a.changeLimit.allow(time.Now()) {
		writeAdminError(w, http.StatusTooManyRequests, "操作过于频繁，请稍后再试")
		return
	}
	request, err := readAdminJSON[adminChangePasswordRequest](w, r, 8<<10)
	if err != nil || len(request.CurrentPassword) > 1024 || len(request.NewPassword) > adminMaxPassword {
		writeAdminError(w, http.StatusBadRequest, "密码修改信息无效")
		return
	}
	if err := a.changeAdminPassword(request.CurrentPassword, request.NewPassword, request.TOTPCode); err != nil {
		if errors.Is(err, errAdminAuthRejected) {
			writeAdminError(w, http.StatusUnauthorized, "当前密码或动态验证码错误，或新密码不符合要求")
			return
		}
		writeAdminError(w, http.StatusServiceUnavailable, "认证状态暂不可用，请联系运维重启服务")
		return
	}
	a.logger.Info("admin password changed", "actor", session.username)
	clearAdminSessionCookie(w, a.cookieSecure)
	writeAdminJSON(w, http.StatusOK, map[string]bool{"ok": true})
}

func (a *adminConsole) sessionInfo(w http.ResponseWriter, r *http.Request) {
	a.setSecurityHeaders(w)
	session, _, ok := a.readSession(r)
	if !ok {
		writeAdminError(w, http.StatusUnauthorized, "登录已失效")
		return
	}
	writeAdminJSON(w, http.StatusOK, map[string]any{"username": session.username, "csrfToken": session.csrf, "setupRequired": session.setupPending})
}

func (a *adminConsole) logout(w http.ResponseWriter, r *http.Request) {
	a.setSecurityHeaders(w)
	session, tokenHash, ok := a.readSession(r)
	if !ok || !a.validMutation(r, session) {
		writeAdminError(w, http.StatusForbidden, "登录状态或请求校验无效")
		return
	}
	a.mu.Lock()
	delete(a.sessions, tokenHash)
	a.mu.Unlock()
	clearAdminSessionCookie(w, a.cookieSecure)
	writeAdminJSON(w, http.StatusOK, map[string]bool{"ok": true})
}

func (a *adminConsole) listLicenses(w http.ResponseWriter, r *http.Request) {
	a.setSecurityHeaders(w)
	if session, _, ok := a.readSession(r); !ok {
		writeAdminError(w, http.StatusUnauthorized, "请先登录")
		return
	} else if session.setupPending {
		writeAdminError(w, http.StatusForbidden, "请先完成首次安全设置")
		return
	}
	offset, limit, ok := parseAdminPage(r)
	if !ok {
		writeAdminError(w, http.StatusBadRequest, "分页参数无效")
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 5*time.Second)
	defer cancel()
	records, total, err := a.store.List(ctx, offset, limit)
	if err != nil {
		writeAdminError(w, http.StatusServiceUnavailable, "授权数据暂不可用")
		return
	}
	items := make([]adminLicenseSummary, 0, len(records))
	for _, record := range records {
		items = append(items, summarizeLicense(record))
	}
	writeAdminJSON(w, http.StatusOK, map[string]any{"items": items, "total": total, "offset": offset, "limit": limit})
}

func (a *adminConsole) importLicense(w http.ResponseWriter, r *http.Request) {
	a.setSecurityHeaders(w)
	session, _, ok := a.readSession(r)
	if !ok || !a.validMutation(r, session) {
		writeAdminError(w, http.StatusForbidden, "登录状态或请求校验无效")
		return
	}
	if session.setupPending {
		writeAdminError(w, http.StatusForbidden, "请先完成首次安全设置")
		return
	}
	request, err := readAdminJSON[adminRegistrationRequest](w, r, MaxRegistrationBytes+(4<<10))
	if err != nil || request.Registration.PayloadB64 == "" || request.Registration.SignatureB64 == "" {
		writeAdminError(w, http.StatusBadRequest, "登记文件格式无效")
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 5*time.Second)
	defer cancel()
	record, err := a.store.Import(ctx, request.Registration, session.username, time.Now().UTC())
	if err != nil {
		status := http.StatusServiceUnavailable
		message := "登记暂不可用"
		switch {
		case errors.Is(err, errInvalidRegistration):
			status, message = http.StatusBadRequest, "登记文件签名或内容无效"
		case errors.Is(err, errAlreadyExists):
			status, message = http.StatusConflict, "此安装编号已经登记"
		}
		writeAdminError(w, status, message)
		return
	}
	writeAdminJSON(w, http.StatusCreated, map[string]any{"license": summarizeLicense(record)})
}

func (a *adminConsole) revokeLicense(w http.ResponseWriter, r *http.Request) {
	a.setSecurityHeaders(w)
	session, _, ok := a.readSession(r)
	if !ok || !a.validMutation(r, session) {
		writeAdminError(w, http.StatusForbidden, "登录状态或请求校验无效")
		return
	}
	if session.setupPending {
		writeAdminError(w, http.StatusForbidden, "请先完成首次安全设置")
		return
	}
	request, err := readAdminJSON[adminRevokeRequest](w, r, 8<<10)
	installationID := r.PathValue("installationID")
	if err != nil || strings.TrimSpace(request.Reason) != request.Reason || request.Reason == "" || len(request.Reason) > 200 || request.ConfirmInstallationID != installationID {
		writeAdminError(w, http.StatusBadRequest, "确认信息或作废原因无效")
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 5*time.Second)
	defer cancel()
	record, _, err := a.store.ChangeState(ctx, installationID, "revoke", session.username, request.Reason, time.Now().UTC())
	if err != nil {
		status := http.StatusServiceUnavailable
		message := "作废暂不可用"
		if errors.Is(err, errNotFound) {
			status, message = http.StatusNotFound, "未找到此安装编号"
		}
		writeAdminError(w, status, message)
		return
	}
	writeAdminJSON(w, http.StatusOK, map[string]any{"license": summarizeLicense(record)})
}

func (a *adminConsole) originAllowed(r *http.Request) bool {
	return r.Header.Get("Origin") == a.origin
}

func (a *adminConsole) validMutation(r *http.Request, session adminSession) bool {
	return a.originAllowed(r) && r.Header.Get("X-CSRF-Token") != "" && subtle.ConstantTimeCompare([]byte(r.Header.Get("X-CSRF-Token")), []byte(session.csrf)) == 1
}

func (a *adminConsole) addSession(token []byte, session adminSession) bool {
	digest := sha256.Sum256(token)
	a.mu.Lock()
	defer a.mu.Unlock()
	now := time.Now()
	for key, existing := range a.sessions {
		if !now.Before(existing.expires) {
			delete(a.sessions, key)
		}
	}
	if len(a.sessions) >= adminMaxSessions {
		return false
	}
	a.sessions[digest] = session
	return true
}

// addSessionForPassword serializes password validation and session creation with
// password rotation. A password change either follows this insertion and clears
// the new session, or precedes it and causes the old password to be rejected.
func (a *adminConsole) addSessionForPassword(password string, token []byte, session adminSession) (bool, bool, bool) {
	a.authMu.Lock()
	defer a.authMu.Unlock()
	if !adminPasswordMatches(a.authState, password) {
		return false, false, false
	}
	if a.authUnavailable {
		return false, false, false
	}
	session.setupPending = !a.authState.FirstLoginComplete
	return true, a.addSession(token, session), session.setupPending
}

func (a *adminConsole) pendingInitialSetupSecret(digest [32]byte) (string, error) {
	a.authMu.Lock()
	defer a.authMu.Unlock()
	if a.authUnavailable || a.authState.FirstLoginComplete {
		return "", errors.New("initial setup is already complete")
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	session, ok := a.sessions[digest]
	if !ok || !session.setupPending {
		return "", errors.New("initial setup session is unavailable")
	}
	if session.pendingTOTPSecret == "" {
		if a.authState.TOTPSecret != "" {
			session.pendingTOTPSecret = a.authState.TOTPSecret
		} else {
			secret, err := generateAdminTOTPSecret()
			if err != nil {
				return "", err
			}
			session.pendingTOTPSecret = secret
		}
		a.sessions[digest] = session
	}
	return session.pendingTOTPSecret, nil
}

func clearAdminSessionCookie(w http.ResponseWriter, secure bool) {
	http.SetCookie(w, &http.Cookie{Name: adminCookieName, Value: "", Path: "/admin/", HttpOnly: true, Secure: secure, SameSite: http.SameSiteStrictMode, MaxAge: -1, Expires: time.Unix(1, 0)})
}

func (a *adminConsole) readSession(r *http.Request) (adminSession, [32]byte, bool) {
	var empty [32]byte
	cookie, err := r.Cookie(adminCookieName)
	if err != nil || len(cookie.Value) > 128 {
		return adminSession{}, empty, false
	}
	token, err := base64.RawURLEncoding.DecodeString(cookie.Value)
	if err != nil || len(token) != 32 {
		return adminSession{}, empty, false
	}
	digest := sha256.Sum256(token)
	a.mu.Lock()
	defer a.mu.Unlock()
	session, ok := a.sessions[digest]
	if !ok || !time.Now().Before(session.expires) {
		delete(a.sessions, digest)
		return adminSession{}, digest, false
	}
	session.expires = time.Now().Add(adminSessionTTL)
	a.sessions[digest] = session
	return session, digest, true
}

func readAdminJSON[T any](w http.ResponseWriter, r *http.Request, maxBytes int64) (T, error) {
	var zero T
	mediaType, _, err := mime.ParseMediaType(r.Header.Get("Content-Type"))
	if err != nil || mediaType != "application/json" {
		return zero, errors.New("invalid content type")
	}
	raw, err := io.ReadAll(http.MaxBytesReader(w, r.Body, maxBytes))
	if err != nil {
		return zero, err
	}
	var result T
	if err := decodeStrictJSON(raw, &result); err != nil {
		return zero, err
	}
	return result, nil
}

func parseAdminPage(r *http.Request) (int, int, bool) {
	values := r.URL.Query()
	if len(values) > 2 {
		return 0, 0, false
	}
	for key, entries := range values {
		if (key != "offset" && key != "limit") || len(entries) != 1 {
			return 0, 0, false
		}
	}
	offset, limit := 0, adminDefaultPage
	if raw := values.Get("offset"); raw != "" {
		parsed, err := strconv.Atoi(raw)
		if err != nil || parsed < 0 || parsed > 1_000_000 {
			return 0, 0, false
		}
		offset = parsed
	}
	if raw := values.Get("limit"); raw != "" {
		parsed, err := strconv.Atoi(raw)
		if err != nil || parsed < 1 || parsed > adminMaxPage {
			return 0, 0, false
		}
		limit = parsed
	}
	return offset, limit, true
}

func summarizeLicense(record LicenseRecord) adminLicenseSummary {
	return adminLicenseSummary{
		Product: record.Product, InstallationID: record.InstallationID, USBAssetID: record.USBAssetID, LicenseKeyID: record.KeyID,
		LicenseSignatureSHA256: maskedDigest(record.LicenseSignatureSHA256),
		FingerprintSHA256:      maskedDigest(record.FingerprintDigest), Status: record.Status,
		StateVersion: record.StateVersion, CreatedAt: record.CreatedAt.UTC(), UpdatedAt: record.UpdatedAt.UTC(),
		RevokedAt: record.RevokedAt,
	}
}

func maskedDigest(digest string) string {
	if len(digest) <= 20 {
		return digest
	}
	return digest[:10] + "…" + digest[len(digest)-8:]
}

func writeAdminJSON(w http.ResponseWriter, status int, value any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(value)
}

func writeAdminError(w http.ResponseWriter, status int, message string) {
	writeAdminJSON(w, status, map[string]string{"error": message})
}
