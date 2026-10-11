package revocation

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/cookiejar"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestAdminConsoleFreshRegistrationListingDoubleConfirmedRevoke(t *testing.T) {
	store, keys, signer := newTestStore(t, false)
	defer store.Close()

	web := httptest.NewUnstartedServer(nil)
	origin := "http://" + web.Listener.Addr().String()
	statePath := filepath.Join(t.TempDir(), "admin-auth.json")
	markAdminSetupComplete(t, statePath, "local-test-password-43", "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP")
	httpServer, err := NewHTTPServer(store, keys, HTTPConfig{
		ListenAddress: "127.0.0.1:0", AdminUsername: "operator",
		AdminPassword: "local-test-password-43", AdminOrigin: origin,
		AdminAuthStatePath: statePath,
		AdminTOTPSecret:    "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
		AdminCookieSecure:  false,
	}, slog.New(slog.NewTextHandler(io.Discard, nil)))
	if err != nil {
		t.Fatal(err)
	}
	web.Config.Handler = httpServer.Handler
	web.Start()
	defer web.Close()

	jar, err := cookiejar.New(nil)
	if err != nil {
		t.Fatal(err)
	}
	client := &http.Client{Jar: jar, Timeout: 3 * time.Second}
	baseURL := web.URL

	page, err := client.Get(baseURL + "/admin/")
	if err != nil {
		t.Fatal(err)
	}
	pageBody, _ := io.ReadAll(page.Body)
	page.Body.Close()
	if page.StatusCode != http.StatusOK || !strings.Contains(string(pageBody), "U 盘授权管理") || page.Header.Get("Content-Security-Policy") == "" {
		t.Fatalf("admin page status=%d, body contains page=%v, csp=%v", page.StatusCode, strings.Contains(string(pageBody), "U 盘授权管理"), page.Header.Get("Content-Security-Policy") != "")
	}

	if response := doAdminJSON(t, client, http.MethodGet, baseURL+"/admin/api/licenses", origin, "", nil); response.StatusCode != http.StatusUnauthorized {
		response.Body.Close()
		t.Fatalf("anonymous list status=%d, want 401", response.StatusCode)
	} else {
		response.Body.Close()
	}

	wrongLogin := doAdminJSON(t, client, http.MethodPost, baseURL+"/admin/api/login", origin, "", adminLoginRequest{Username: "operator", Password: "wrong-password"})
	wrongLogin.Body.Close()
	if wrongLogin.StatusCode != http.StatusUnauthorized {
		t.Fatalf("wrong password status=%d, want 401", wrongLogin.StatusCode)
	}

	login := doAdminJSON(t, client, http.MethodPost, baseURL+"/admin/api/login", origin, "", adminLoginRequest{Username: "operator", Password: "local-test-password-43"})
	if login.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(login.Body)
		login.Body.Close()
		t.Fatalf("login status=%d body=%s", login.StatusCode, body)
	}
	var loginResult struct {
		Username  string `json:"username"`
		CSRFToken string `json:"csrfToken"`
	}
	if err := json.NewDecoder(login.Body).Decode(&loginResult); err != nil {
		login.Body.Close()
		t.Fatal(err)
	}
	login.Body.Close()
	if loginResult.Username != "operator" || loginResult.CSRFToken == "" {
		t.Fatalf("unexpected login result: %+v", loginResult)
	}
	setCookie := strings.Join(login.Header.Values("Set-Cookie"), ";")
	if !strings.Contains(setCookie, "HttpOnly") || !strings.Contains(setCookie, "SameSite=Strict") {
		t.Fatalf("session cookie missing required attributes: %s", setCookie)
	}

	registration, request := freshAdminRegistration(t)
	missingCSRF := doAdminJSON(t, client, http.MethodPost, baseURL+"/admin/api/licenses", origin, "", adminRegistrationRequest{Registration: registration})
	missingCSRF.Body.Close()
	if missingCSRF.StatusCode != http.StatusForbidden {
		t.Fatalf("missing CSRF status=%d, want 403", missingCSRF.StatusCode)
	}
	badOrigin := doAdminJSON(t, client, http.MethodPost, baseURL+"/admin/api/licenses", "http://attacker.invalid", loginResult.CSRFToken, adminRegistrationRequest{Registration: registration})
	badOrigin.Body.Close()
	if badOrigin.StatusCode != http.StatusForbidden {
		t.Fatalf("wrong origin status=%d, want 403", badOrigin.StatusCode)
	}

	importResponse := doAdminJSON(t, client, http.MethodPost, baseURL+"/admin/api/licenses", origin, loginResult.CSRFToken, adminRegistrationRequest{Registration: registration})
	if importResponse.StatusCode != http.StatusCreated {
		body, _ := io.ReadAll(importResponse.Body)
		importResponse.Body.Close()
		t.Fatalf("fresh registration import status=%d body=%s", importResponse.StatusCode, body)
	}
	importResponse.Body.Close()
	created, err := store.Inspect(context.Background(), request.InstallationID)
	if err != nil || created.Status != "active" || created.StateVersion != 1 || created.KeyID != fixtureLicenseKeyID {
		t.Fatalf("fresh registration did not create active v1 license: record=%+v err=%v", created, err)
	}
	list := doAdminJSON(t, client, http.MethodGet, baseURL+"/admin/api/licenses?offset=0&limit=50", origin, "", nil)
	if list.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(list.Body)
		list.Body.Close()
		t.Fatalf("list status=%d body=%s", list.StatusCode, body)
	}
	var listed struct {
		Items []adminLicenseSummary `json:"items"`
		Total int                   `json:"total"`
	}
	if err := json.NewDecoder(list.Body).Decode(&listed); err != nil {
		list.Body.Close()
		t.Fatal(err)
	}
	list.Body.Close()
	if listed.Total != 1 || len(listed.Items) != 1 || listed.Items[0].InstallationID != request.InstallationID || listed.Items[0].LicenseKeyID != fixtureLicenseKeyID || listed.Items[0].Status != "active" || listed.Items[0].StateVersion != 1 {
		t.Fatalf("new USB registration missing from console list: %+v", listed)
	}
	encodedSummary, _ := json.Marshal(listed.Items[0])
	if bytes.Contains(encodedSummary, []byte("revokeReason")) || strings.Contains(string(encodedSummary), request.LicenseSignatureSHA256) || strings.Contains(string(encodedSummary), request.ObservedFingerprint.AnchorSHA256) {
		t.Fatalf("console returned an unmasked/internal field: %s", encodedSummary)
	}

	duplicate := doAdminJSON(t, client, http.MethodPost, baseURL+"/admin/api/licenses", origin, loginResult.CSRFToken, adminRegistrationRequest{Registration: registration})
	duplicate.Body.Close()
	if duplicate.StatusCode != http.StatusConflict {
		t.Fatalf("duplicate registration status=%d, want 409", duplicate.StatusCode)
	}

	checkStatus := func(expected string, requestID string, version int64) {
		t.Helper()
		check := request
		check.RequestID = requestID
		body, _ := json.Marshal(check)
		response, err := client.Post(baseURL+"/v1/licenses/check", "application/json", bytes.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
		if response.StatusCode != http.StatusOK {
			got, _ := io.ReadAll(response.Body)
			t.Fatalf("license check status=%d body=%s", response.StatusCode, got)
		}
		var envelope SignedEnvelope
		if err := json.NewDecoder(response.Body).Decode(&envelope); err != nil {
			t.Fatal(err)
		}
		payloadBytes, err := decodeStdBase64(envelope.PayloadB64)
		if err != nil {
			t.Fatal(err)
		}
		signatureBytes, err := decodeStdBase64(envelope.SignatureB64)
		if err != nil || !ed25519.Verify(ed25519.PrivateKey(signer.Key).Public().(ed25519.PublicKey), payloadBytes, signatureBytes) {
			t.Fatalf("status response signature invalid: %v", err)
		}
		var payload ResponsePayload
		if err := json.Unmarshal(payloadBytes, &payload); err != nil {
			t.Fatal(err)
		}
		if payload.Status != expected || payload.RequestID != requestID || payload.InstallationID != request.InstallationID || payload.StateVersion != version {
			t.Fatalf("unexpected signed response: %+v", payload)
		}
	}
	checkStatus("active", strings.Repeat("1", 32), 1)

	badConfirm := doAdminJSON(t, client, http.MethodPost, baseURL+"/admin/api/licenses/"+request.InstallationID+"/revoke", origin, loginResult.CSRFToken, adminRevokeRequest{Reason: "local test", ConfirmInstallationID: "another-usb"})
	badConfirm.Body.Close()
	if badConfirm.StatusCode != http.StatusBadRequest {
		t.Fatalf("unconfirmed revoke status=%d, want 400", badConfirm.StatusCode)
	}
	stillActive, err := store.Inspect(context.Background(), request.InstallationID)
	if err != nil || stillActive.Status != "active" || stillActive.StateVersion != 1 {
		t.Fatalf("bad confirmation changed record: %+v err=%v", stillActive, err)
	}

	revoke := doAdminJSON(t, client, http.MethodPost, baseURL+"/admin/api/licenses/"+request.InstallationID+"/revoke", origin, loginResult.CSRFToken, adminRevokeRequest{Reason: "local end-to-end test", ConfirmInstallationID: request.InstallationID})
	if revoke.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(revoke.Body)
		revoke.Body.Close()
		t.Fatalf("revoke status=%d body=%s", revoke.StatusCode, body)
	}
	revoke.Body.Close()
	revoked, err := store.Inspect(context.Background(), request.InstallationID)
	if err != nil || revoked.Status != "revoked" || revoked.StateVersion != 2 {
		t.Fatalf("manual revoke did not create revoked v2: record=%+v err=%v", revoked, err)
	}
	events, err := validateJournals(store.config.AuditPath, store.config.AuditBackupPath)
	if err != nil || len(events) != 2 || events[1].Actor != "operator" || events[1].Action != "revoke" {
		t.Fatalf("admin actor/revoke missing from audit events: events=%+v err=%v", events, err)
	}
	checkStatus("revoked", strings.Repeat("2", 32), 2)

	logout := doAdminJSON(t, client, http.MethodPost, baseURL+"/admin/api/logout", origin, loginResult.CSRFToken, map[string]string{})
	logout.Body.Close()
	if logout.StatusCode != http.StatusOK {
		t.Fatalf("logout status=%d", logout.StatusCode)
	}
	postLogout := doAdminJSON(t, client, http.MethodGet, baseURL+"/admin/api/licenses", origin, "", nil)
	postLogout.Body.Close()
	if postLogout.StatusCode != http.StatusUnauthorized {
		t.Fatalf("session remained valid after logout: status=%d", postLogout.StatusCode)
	}
}

func TestAdminPasswordChangeRequiresTOTPAndPersists(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()

	statePath := filepath.Join(t.TempDir(), "admin-auth.json")
	secret := "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"
	markAdminSetupComplete(t, statePath, "local-test-password-43", secret)
	web := httptest.NewUnstartedServer(nil)
	origin := "http://" + web.Listener.Addr().String()
	cfg := HTTPConfig{
		ListenAddress: "127.0.0.1:0", AdminUsername: "operator",
		AdminPassword: "local-test-password-43", AdminAuthStatePath: statePath,
		AdminTOTPSecret: secret, AdminOrigin: origin, AdminCookieSecure: false,
	}
	httpServer, err := NewHTTPServer(store, keys, cfg, slog.New(slog.NewTextHandler(io.Discard, nil)))
	if err != nil {
		t.Fatal(err)
	}
	web.Config.Handler = httpServer.Handler
	web.Start()
	defer web.Close()

	jar, err := cookiejar.New(nil)
	if err != nil {
		t.Fatal(err)
	}
	client := &http.Client{Jar: jar, Timeout: 3 * time.Second}
	login := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/login", origin, "", adminLoginRequest{Username: "operator", Password: "local-test-password-43"})
	if login.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(login.Body)
		login.Body.Close()
		t.Fatalf("login status=%d body=%s", login.StatusCode, body)
	}
	var loginResult struct {
		CSRFToken string `json:"csrfToken"`
	}
	if err := json.NewDecoder(login.Body).Decode(&loginResult); err != nil {
		login.Body.Close()
		t.Fatal(err)
	}
	login.Body.Close()

	badOTP := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/change-password", origin, loginResult.CSRFToken, adminChangePasswordRequest{
		CurrentPassword: "local-test-password-43", NewPassword: "new-local-password-87", TOTPCode: "not-a-code",
	})
	badOTP.Body.Close()
	if badOTP.StatusCode != http.StatusUnauthorized {
		t.Fatalf("password change without a valid TOTP status=%d, want 401", badOTP.StatusCode)
	}

	decodedSecret, err := decodeAdminTOTPSecret(secret)
	if err != nil {
		t.Fatal(err)
	}
	step := time.Now().Unix() / adminTOTPPeriod
	validOTP := adminTOTPCode(decodedSecret, step)
	changed := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/change-password", origin, loginResult.CSRFToken, adminChangePasswordRequest{
		CurrentPassword: "local-test-password-43", NewPassword: "new-local-password-87", TOTPCode: validOTP,
	})
	if changed.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(changed.Body)
		changed.Body.Close()
		t.Fatalf("password change status=%d body=%s", changed.StatusCode, body)
	}
	changed.Body.Close()

	oldSession := doAdminJSON(t, client, http.MethodGet, web.URL+"/admin/api/session", origin, "", nil)
	oldSession.Body.Close()
	if oldSession.StatusCode != http.StatusUnauthorized {
		t.Fatalf("password change left the old session active: status=%d", oldSession.StatusCode)
	}

	newServer, err := NewHTTPServer(store, keys, cfg, slog.New(slog.NewTextHandler(io.Discard, nil)))
	if err != nil {
		t.Fatal(err)
	}
	newWeb := httptest.NewServer(newServer.Handler)
	defer newWeb.Close()
	newJar, err := cookiejar.New(nil)
	if err != nil {
		t.Fatal(err)
	}
	newClient := &http.Client{Jar: newJar, Timeout: 3 * time.Second}
	newOrigin := origin
	newLogin := doAdminJSON(t, newClient, http.MethodPost, newWeb.URL+"/admin/api/login", newOrigin, "", adminLoginRequest{Username: "operator", Password: "new-local-password-87"})
	newLogin.Body.Close()
	if newLogin.StatusCode != http.StatusOK {
		t.Fatalf("new password did not survive service restart: login status=%d", newLogin.StatusCode)
	}
}

func TestAdminFirstLoginChangesPasswordAndEnrollsTOTP(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()

	statePath := filepath.Join(t.TempDir(), "admin-auth.json")
	web := httptest.NewUnstartedServer(nil)
	origin := "http://" + web.Listener.Addr().String()
	cfg := HTTPConfig{
		ListenAddress: "127.0.0.1:0", AdminUsername: "operator",
		AdminPassword: "bootstrap-password-43", AdminAuthStatePath: statePath,
		AdminOrigin: origin, AdminCookieSecure: false,
	}
	httpServer, err := NewHTTPServer(store, keys, cfg, slog.New(slog.NewTextHandler(io.Discard, nil)))
	if err != nil {
		t.Fatal(err)
	}
	web.Config.Handler = httpServer.Handler
	web.Start()
	defer web.Close()

	jar, err := cookiejar.New(nil)
	if err != nil {
		t.Fatal(err)
	}
	client := &http.Client{Jar: jar, Timeout: 3 * time.Second}
	login := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/login", origin, "", adminLoginRequest{
		Username: "operator", Password: "bootstrap-password-43",
	})
	var loginResult struct {
		CSRFToken     string `json:"csrfToken"`
		SetupRequired bool   `json:"setupRequired"`
	}
	if login.StatusCode != http.StatusOK || json.NewDecoder(login.Body).Decode(&loginResult) != nil {
		body, _ := io.ReadAll(login.Body)
		login.Body.Close()
		t.Fatalf("bootstrap login status=%d body=%s", login.StatusCode, body)
	}
	login.Body.Close()
	if !loginResult.SetupRequired || loginResult.CSRFToken == "" {
		t.Fatalf("first login did not require secure setup: %+v", loginResult)
	}
	blocked := doAdminJSON(t, client, http.MethodGet, web.URL+"/admin/api/licenses", origin, "", nil)
	blocked.Body.Close()
	if blocked.StatusCode != http.StatusForbidden {
		t.Fatalf("license listing before setup status=%d, want 403", blocked.StatusCode)
	}

	setup := doAdminJSON(t, client, http.MethodGet, web.URL+"/admin/api/initial-setup", origin, "", nil)
	var setupResult struct {
		TOTPSecret string `json:"totpSecret"`
	}
	if setup.StatusCode != http.StatusOK || json.NewDecoder(setup.Body).Decode(&setupResult) != nil {
		body, _ := io.ReadAll(setup.Body)
		setup.Body.Close()
		t.Fatalf("initial setup key status=%d body=%s", setup.StatusCode, body)
	}
	setup.Body.Close()
	secret, err := decodeAdminTOTPSecret(setupResult.TOTPSecret)
	if err != nil || len(secret) != 20 {
		t.Fatalf("initial setup returned an invalid TOTP secret: bytes=%d err=%v", len(secret), err)
	}

	badCode := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/initial-setup", origin, loginResult.CSRFToken, adminInitialSetupRequest{
		NewPassword: "rotated-password-87", TOTPCode: "not-a-code",
	})
	badCode.Body.Close()
	if badCode.StatusCode != http.StatusUnauthorized {
		t.Fatalf("invalid setup code status=%d, want 401", badCode.StatusCode)
	}

	code := adminTOTPCode(secret, time.Now().Unix()/adminTOTPPeriod)
	completed := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/initial-setup", origin, loginResult.CSRFToken, adminInitialSetupRequest{
		NewPassword: "rotated-password-87", TOTPCode: code,
	})
	completed.Body.Close()
	if completed.StatusCode != http.StatusOK {
		t.Fatalf("first login setup status=%d, want 200", completed.StatusCode)
	}
	oldSession := doAdminJSON(t, client, http.MethodGet, web.URL+"/admin/api/session", origin, "", nil)
	oldSession.Body.Close()
	if oldSession.StatusCode != http.StatusUnauthorized {
		t.Fatalf("first setup left bootstrap session active: status=%d", oldSession.StatusCode)
	}

	newServer, err := NewHTTPServer(store, keys, cfg, slog.New(slog.NewTextHandler(io.Discard, nil)))
	if err != nil {
		t.Fatal(err)
	}
	newWeb := httptest.NewServer(newServer.Handler)
	defer newWeb.Close()
	newJar, err := cookiejar.New(nil)
	if err != nil {
		t.Fatal(err)
	}
	newClient := &http.Client{Jar: newJar, Timeout: 3 * time.Second}
	oldLogin := doAdminJSON(t, newClient, http.MethodPost, newWeb.URL+"/admin/api/login", origin, "", adminLoginRequest{
		Username: "operator", Password: "bootstrap-password-43",
	})
	oldLogin.Body.Close()
	if oldLogin.StatusCode != http.StatusUnauthorized {
		t.Fatalf("bootstrap password remained valid after setup: status=%d", oldLogin.StatusCode)
	}
	newLogin := doAdminJSON(t, newClient, http.MethodPost, newWeb.URL+"/admin/api/login", origin, "", adminLoginRequest{
		Username: "operator", Password: "rotated-password-87",
	})
	var newLoginResult struct {
		SetupRequired bool `json:"setupRequired"`
	}
	if newLogin.StatusCode != http.StatusOK || json.NewDecoder(newLogin.Body).Decode(&newLoginResult) != nil {
		body, _ := io.ReadAll(newLogin.Body)
		newLogin.Body.Close()
		t.Fatalf("new password login status=%d body=%s", newLogin.StatusCode, body)
	}
	newLogin.Body.Close()
	if newLoginResult.SetupRequired {
		t.Fatal("setup remained pending after successful password and TOTP enrollment")
	}
}

func TestPreconfiguredTOTPDoesNotSkipFirstLoginSetup(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "admin-auth.json")
	secret := "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"
	admin, err := newAdminConsole(nil, HTTPConfig{
		AdminUsername: "operator", AdminPassword: "bootstrap-password-43",
		AdminAuthStatePath: statePath, AdminTOTPSecret: secret,
		AdminOrigin: "http://127.0.0.1:8787", AdminCookieSecure: false,
	}, nil)
	if err != nil {
		t.Fatal(err)
	}
	token := make([]byte, 32)
	matched, added, setupRequired := admin.addSessionForPassword("bootstrap-password-43", token, adminSession{
		username: "operator", csrf: "csrf", expires: time.Now().Add(adminSessionTTL),
	})
	if !matched || !added || !setupRequired {
		t.Fatalf("preconfigured TOTP bypassed first-login setup: matched=%v added=%v setupRequired=%v", matched, added, setupRequired)
	}
	digest := sha256.Sum256(token)
	pending, err := admin.pendingInitialSetupSecret(digest)
	if err != nil || pending != secret {
		t.Fatalf("first-login setup did not retain the preconfigured TOTP seed: equal=%v err=%v", pending == secret, err)
	}
}

func TestAdminAuthStateV1MigrationRequiresFirstLoginSetup(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "admin-auth.json")
	state, err := loadOrInitializeAdminAuth(statePath, "bootstrap-password-43", "")
	if err != nil {
		t.Fatal(err)
	}
	state.SchemaVersion = 1
	state.FirstLoginComplete = false
	state.TOTPSecret = ""
	if err := writeAdminAuthState(statePath, state); err != nil {
		t.Fatal(err)
	}

	migrated, err := loadOrInitializeAdminAuth(statePath, "bootstrap-password-43", "")
	if err != nil {
		t.Fatal(err)
	}
	if migrated.SchemaVersion != 3 || migrated.FirstLoginComplete || migrated.TOTPSecret != "" {
		t.Fatalf("legacy auth state migration did not require first-login setup: %+v", migrated)
	}
}

func TestAdminAuthWriteFailureFailsClosed(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "admin-auth.json")
	secret := "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"
	markAdminSetupComplete(t, statePath, "local-test-password-43", secret)
	admin, err := newAdminConsole(nil, HTTPConfig{
		AdminUsername: "operator", AdminPassword: "local-test-password-43",
		AdminAuthStatePath: statePath, AdminTOTPSecret: secret,
		AdminOrigin: "http://127.0.0.1:8787", AdminCookieSecure: false,
	}, nil)
	if err != nil {
		t.Fatal(err)
	}
	token := make([]byte, 32)
	matched, added, setupRequired := admin.addSessionForPassword("local-test-password-43", token, adminSession{
		username: "operator", csrf: "csrf", expires: time.Now().Add(adminSessionTTL),
	})
	if !matched || !added || setupRequired {
		t.Fatalf("could not create established admin session: matched=%v added=%v setupRequired=%v", matched, added, setupRequired)
	}
	if err := os.Rename(statePath, statePath+".saved"); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(statePath, 0700); err != nil {
		t.Fatal(err)
	}
	decodedSecret, err := decodeAdminTOTPSecret(secret)
	if err != nil {
		t.Fatal(err)
	}
	code := adminTOTPCode(decodedSecret, time.Now().Unix()/adminTOTPPeriod)
	if err := admin.changeAdminPassword("local-test-password-43", "new-password-for-failure-77", code); !errors.Is(err, errAdminAuthStorage) {
		t.Fatalf("failed auth-state write returned %v, want storage failure", err)
	}
	if !admin.authUnavailable || len(admin.sessions) != 0 {
		t.Fatalf("auth-state write failure did not fail closed: unavailable=%v sessions=%d", admin.authUnavailable, len(admin.sessions))
	}
	if matched, added, _ := admin.addSessionForPassword("local-test-password-43", token, adminSession{
		username: "operator", csrf: "csrf", expires: time.Now().Add(adminSessionTTL),
	}); matched || added {
		t.Fatal("admin login remained available after auth-state write failure")
	}
}

func TestAdminInitialSetupWriteFailureFailsClosed(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "admin-auth.json")
	admin, err := newAdminConsole(nil, HTTPConfig{
		AdminUsername: "operator", AdminPassword: "bootstrap-password-43",
		AdminAuthStatePath: statePath,
		AdminOrigin:        "http://127.0.0.1:8787", AdminCookieSecure: false,
	}, nil)
	if err != nil {
		t.Fatal(err)
	}
	token := make([]byte, 32)
	matched, added, setupRequired := admin.addSessionForPassword("bootstrap-password-43", token, adminSession{
		username: "operator", csrf: "csrf", expires: time.Now().Add(adminSessionTTL),
	})
	if !matched || !added || !setupRequired {
		t.Fatalf("could not create setup-pending session: matched=%v added=%v setupRequired=%v", matched, added, setupRequired)
	}
	digest := sha256.Sum256(token)
	pendingSecret, err := admin.pendingInitialSetupSecret(digest)
	if err != nil {
		t.Fatal(err)
	}
	decodedSecret, err := decodeAdminTOTPSecret(pendingSecret)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Rename(statePath, statePath+".saved"); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(statePath, 0700); err != nil {
		t.Fatal(err)
	}
	code := adminTOTPCode(decodedSecret, time.Now().Unix()/adminTOTPPeriod)
	if err := admin.completeInitialAdminSetup(digest, "new-password-for-setup-77", code); !errors.Is(err, errAdminAuthStorage) {
		t.Fatalf("failed first-login auth-state write returned %v, want storage failure", err)
	}
	if !admin.authUnavailable || len(admin.sessions) != 0 {
		t.Fatalf("first-login auth-state write failure did not fail closed: unavailable=%v sessions=%d", admin.authUnavailable, len(admin.sessions))
	}
	if matched, added, _ := admin.addSessionForPassword("bootstrap-password-43", token, adminSession{
		username: "operator", csrf: "csrf", expires: time.Now().Add(adminSessionTTL),
	}); matched || added {
		t.Fatal("admin login remained available after first-login auth-state write failure")
	}
}

func TestAdminTOTPMatchesRFC6238AndAllowsFirstLoginSetup(t *testing.T) {
	if got := adminTOTPCode([]byte("12345678901234567890"), 1); got != "287082" {
		t.Fatalf("RFC 6238 SHA-1 code=%s, want 287082", got)
	}
	t.Setenv("USB_REVOCATION_ADMIN_USERNAME", "operator")
	t.Setenv("USB_REVOCATION_ADMIN_PASSWORD", "local-test-password-43")
	t.Setenv("USB_REVOCATION_ADMIN_AUTH_STATE", filepath.Join(t.TempDir(), "admin-auth.json"))
	t.Setenv("USB_REVOCATION_ADMIN_ORIGIN", "http://127.0.0.1:8787")
	t.Setenv("USB_REVOCATION_ADMIN_COOKIE_SECURE", "false")
	t.Setenv("USB_REVOCATION_ADMIN_TOTP_SECRET", "")
	if _, _, err := ConfigFromEnv(); err != nil {
		t.Fatalf("admin console without a preconfigured TOTP secret should permit first-login setup: %v", err)
	}
	t.Setenv("USB_REVOCATION_ADMIN_TOTP_SECRET", "not-a-valid-secret")
	if _, _, err := ConfigFromEnv(); err == nil || !strings.Contains(err.Error(), "TOTP_SECRET") {
		t.Fatalf("admin console with an invalid optional TOTP secret returned error %v", err)
	}
	t.Setenv("USB_REVOCATION_ADMIN_TOTP_SECRET", "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP")
	if _, _, err := ConfigFromEnv(); err != nil {
		t.Fatalf("valid Google Authenticator secret was rejected: %v", err)
	}
}

func TestLoginSessionCreationSerializesWithPasswordChange(t *testing.T) {
	statePath := filepath.Join(t.TempDir(), "admin-auth.json")
	markAdminSetupComplete(t, statePath, "local-test-password-43", "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP")
	admin, err := newAdminConsole(nil, HTTPConfig{
		AdminUsername: "operator", AdminPassword: "local-test-password-43",
		AdminAuthStatePath: statePath,
		AdminTOTPSecret:    "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
		AdminOrigin:        "http://127.0.0.1:8787", AdminCookieSecure: false,
	}, nil)
	if err != nil {
		t.Fatal(err)
	}

	// Hold the session-map lock so a login pauses after password validation.
	// Password rotation must wait for the login to finish inserting its session;
	// the change-password handler then clears that session before returning.
	admin.mu.Lock()
	type loginResult struct{ passwordMatches, sessionAdded bool }
	loginDone := make(chan loginResult, 1)
	token := make([]byte, 32)
	go func() {
		matches, added, _ := admin.addSessionForPassword("local-test-password-43", token, adminSession{
			username: "operator", csrf: "csrf", expires: time.Now().Add(adminSessionTTL),
		})
		loginDone <- loginResult{matches, added}
	}()

	deadline := time.Now().Add(3 * time.Second)
	for admin.authMu.TryLock() {
		admin.authMu.Unlock()
		if time.Now().After(deadline) {
			admin.mu.Unlock()
			t.Fatal("login did not enter the serialized password/session section")
		}
		runtime.Gosched()
	}

	changeDone := make(chan error, 1)
	go func() {
		step := time.Now().Unix() / adminTOTPPeriod
		secret, err := decodeAdminTOTPSecret(admin.authState.TOTPSecret)
		if err != nil {
			changeDone <- err
			return
		}
		code := adminTOTPCode(secret, step)
		changeDone <- admin.changeAdminPassword("local-test-password-43", "new-local-password-87", code)
	}()
	admin.mu.Unlock()

	select {
	case result := <-loginDone:
		if !result.passwordMatches || !result.sessionAdded {
			t.Fatalf("valid login did not create its session before rotation: %+v", result)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("login did not finish")
	}
	select {
	case err := <-changeDone:
		if err != nil {
			t.Fatalf("password rotation failed: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("password rotation did not finish")
	}

	admin.mu.Lock()
	remaining := len(admin.sessions)
	admin.mu.Unlock()
	if remaining != 0 {
		t.Fatalf("session created during rotation remained active: %d", remaining)
	}

	oldPasswordMatches, oldSessionAdded, _ := admin.addSessionForPassword("local-test-password-43", token, adminSession{
		username: "operator", csrf: "csrf", expires: time.Now().Add(adminSessionTTL),
	})
	if oldPasswordMatches || oldSessionAdded {
		t.Fatal("old password created a session after rotation")
	}
}

func TestAdminAuthStatePathMustBeDistinctFromStoreFiles(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()
	for _, path := range []string{
		store.config.DatabasePath,
		store.config.DatabasePath + "-wal",
		store.config.DatabasePath + "-shm",
		store.config.DatabasePath + "-journal",
		store.config.AuditPath,
		store.config.AuditPath + ".lock",
		store.config.AuditBackupPath,
		store.config.RestoreMarkerPath,
	} {
		_, err := NewHTTPServer(store, keys, HTTPConfig{
			ListenAddress: "127.0.0.1:0", AdminUsername: "operator",
			AdminPassword: "local-test-password-43", AdminAuthStatePath: path,
			AdminOrigin: "http://127.0.0.1:8787", AdminCookieSecure: false,
		}, nil)
		if err == nil || !strings.Contains(err.Error(), "admin auth state path") {
			t.Errorf("auth state path colliding with store file %q returned %v", path, err)
		}
	}
}

func TestEnsurePathIsDistinctResolvesSymlinkedParent(t *testing.T) {
	root := t.TempDir()
	targetDir := filepath.Join(root, "target")
	if err := os.Mkdir(targetDir, 0700); err != nil {
		t.Fatal(err)
	}
	linkDir := filepath.Join(root, "link")
	if err := os.Symlink(targetDir, linkDir); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	if err := ensurePathIsDistinct(filepath.Join(targetDir, "auth.json"), filepath.Join(linkDir, "auth.json")); err == nil {
		t.Fatal("paths that alias through a symlinked parent were accepted")
	}
}

func TestAdminConsoleDisabledWithoutCredentials(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()
	server, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "127.0.0.1:0"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	request := httptest.NewRequest(http.MethodGet, "/admin/", nil)
	response := httptest.NewRecorder()
	server.Handler.ServeHTTP(response, request)
	if response.Code != http.StatusNotFound {
		t.Fatalf("admin console without credentials status=%d, want 404", response.Code)
	}
}

func TestValidateAdminOrigin(t *testing.T) {
	for _, test := range []struct {
		origin string
		secure bool
		valid  bool
	}{
		{"https://license.404token.xyz", true, true},
		{"https://license.404token.xyz", false, false},
		{"http://127.0.0.1:8787", false, true},
		{"http://license.404token.xyz", false, false},
		{"https://license.404token.xyz/admin", true, false},
		{"https://license.404token.xyz/", true, false},
		{"https://user@license.404token.xyz", true, false},
	} {
		if err := validateAdminOrigin(test.origin, test.secure); (err == nil) != test.valid {
			t.Errorf("validateAdminOrigin(%q, secure=%v) err=%v, valid=%v", test.origin, test.secure, err, test.valid)
		}
	}
}

func doAdminJSON(t *testing.T, client *http.Client, method, endpoint, origin, csrf string, payload any) *http.Response {
	t.Helper()
	var body io.Reader
	if payload != nil {
		encoded, err := json.Marshal(payload)
		if err != nil {
			t.Fatal(err)
		}
		body = bytes.NewReader(encoded)
	}
	request, err := http.NewRequest(method, endpoint, body)
	if err != nil {
		t.Fatal(err)
	}
	request.Header.Set("Origin", origin)
	request.Header.Set("Content-Type", "application/json")
	if csrf != "" {
		request.Header.Set("X-CSRF-Token", csrf)
	}
	response, err := client.Do(request)
	if err != nil {
		t.Fatal(err)
	}
	return response
}

func freshAdminRegistration(t *testing.T) (RegistrationEnvelope, CheckRequest) {
	t.Helper()
	_, privateKey := licenseFixtureKey()
	installationID := fmt.Sprintf("admin-web-%d", time.Now().UnixNano())
	anchor := strings.Repeat("b", 64)
	fingerprint, err := FingerprintDigest(3, 0x1234abcd, anchor, "codexusb/portable")
	if err != nil {
		t.Fatal(err)
	}
	payload := RegistrationPayload{
		SchemaVersion: RegistrationSchema, Domain: RegistrationDomain, Product: ProductID,
		InstallationID: installationID, LicenseKeyID: fixtureLicenseKeyID,
		LicenseSignatureSHA256: strings.Repeat("c", 64), FingerprintSHA256: fingerprint,
		IssuedAt: "1780000000000",
	}
	raw, err := json.Marshal(payload)
	if err != nil {
		t.Fatal(err)
	}
	registration := RegistrationEnvelope{
		PayloadB64:   base64.StdEncoding.EncodeToString(raw),
		SignatureB64: base64.StdEncoding.EncodeToString(ed25519.Sign(privateKey, raw)),
	}
	request := CheckRequest{
		SchemaVersion: RequestSchema, RequestID: strings.Repeat("1", 32), Product: ProductID,
		InstallationID: installationID, LicenseSignatureSHA256: payload.LicenseSignatureSHA256,
		Registration:        registration,
		ObservedFingerprint: ObservedFingerprint{VolumeSerial: 0x1234abcd, AnchorSHA256: anchor, BundleRelativePath: "codexusb/portable", AnchorSchema: 3},
	}
	return registration, request
}

func markAdminSetupComplete(t *testing.T, path, password, totpSecret string) {
	t.Helper()
	state, err := loadOrInitializeAdminAuth(path, password, totpSecret)
	if err != nil {
		t.Fatal(err)
	}
	state.FirstLoginComplete = true
	if err := writeAdminAuthState(path, state); err != nil {
		t.Fatal(err)
	}
}
