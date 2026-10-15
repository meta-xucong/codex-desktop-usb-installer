package revocation

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/cookiejar"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestModelCatalogManagerHelperProcess(t *testing.T) {
	if os.Getenv("GO_WANT_MODEL_CATALOG_HELPER") != "1" {
		return
	}
	if marker := os.Getenv("MODEL_CATALOG_HELPER_MARKER"); marker != "" {
		file, err := os.OpenFile(marker, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
		if err == nil {
			_, _ = file.WriteString("run\n")
			_ = file.Close()
		}
	}
	if delay := os.Getenv("MODEL_CATALOG_HELPER_DELAY_MS"); delay != "" {
		millis, _ := time.ParseDuration(delay + "ms")
		time.Sleep(millis)
	}
	if output := os.Getenv("MODEL_CATALOG_HELPER_OUTPUT"); output != "" {
		_, _ = os.Stdout.WriteString(output + "\n")
	}
	if os.Getenv("MODEL_CATALOG_HELPER_OVERFLOW") == "1" {
		_, _ = os.Stdout.WriteString(strings.Repeat("x", modelCatalogOutputLimit+10))
	}
	if code := os.Getenv("MODEL_CATALOG_HELPER_EXIT"); code == "1" {
		os.Exit(1)
	}
	os.Exit(0)
}

func TestParseModelCatalogTerminalContract(t *testing.T) {
	valid := `{"schema_version":1,"status":"succeeded","reason_code":"completed","planned_count":2,"applied_count":1,"noop_count":1,"failed_count":0,"not_attempted_count":0,"summary_basename":"screen-summary-20261015T123456Z-0123456789abcdef0123456789abcdef.md"}` + "\n"
	parsed, err := parseModelCatalogTerminal([]byte(valid))
	if err != nil || parsed.Status != "succeeded" || parsed.PlannedCount != 2 {
		t.Fatalf("valid terminal rejected: %+v err=%v", parsed, err)
	}
	invalid := []string{
		strings.TrimSuffix(valid, "\n"),
		valid + valid,
		strings.Replace(valid, `"status":"succeeded"`, `"status":"succeeded","status":"failed"`, 1),
		strings.Replace(valid, `"schema_version":1`, `"schema_version":2`, 1),
		strings.Replace(valid, `"planned_count":2`, `"planned_count":9`, 1),
		strings.Replace(valid, `"summary_basename":"screen-summary-`, `"summary_basename":"../screen-summary-`, 1),
		strings.Replace(valid, `"status":"succeeded"`, `"status":"interrupted"`, 1),
	}
	for index, raw := range invalid {
		if _, err := parseModelCatalogTerminal([]byte(raw)); err == nil {
			t.Errorf("invalid terminal case %d accepted", index)
		}
	}
}

func TestModelCatalogTerminalStatusAndReasonMapping(t *testing.T) {
	cases := []struct {
		name   string
		result modelCatalogTerminal
		valid  bool
	}{
		{"no plan", modelCatalogTerminal{SchemaVersion: 1, Status: "no_plan", ReasonCode: "no_applicable_plan"}, true},
		{"partial scan", modelCatalogTerminal{SchemaVersion: 1, Status: "partial", ReasonCode: "account_scan_partial"}, true},
		{"partial scan with counts is invalid", modelCatalogTerminal{SchemaVersion: 1, Status: "partial", ReasonCode: "account_scan_partial", PlannedCount: 1, NotAttemptedCount: 1}, false},
		{"partial apply", modelCatalogTerminal{SchemaVersion: 1, Status: "partial", ReasonCode: "apply_partial", PlannedCount: 1, AppliedCount: 1}, true},
		{"partial refresh started after a write", modelCatalogTerminal{SchemaVersion: 1, Status: "partial", ReasonCode: "native_refresh_running", PlannedCount: 2, AppliedCount: 1, NotAttemptedCount: 1}, true},
		{"partial refresh status unavailable after a write", modelCatalogTerminal{SchemaVersion: 1, Status: "partial", ReasonCode: "native_refresh_status_unavailable", PlannedCount: 2, AppliedCount: 1, NotAttemptedCount: 1}, true},
		{"partial refresh status unknown after a write", modelCatalogTerminal{SchemaVersion: 1, Status: "partial", ReasonCode: "native_refresh_status_unknown", PlannedCount: 2, AppliedCount: 1, NotAttemptedCount: 1}, true},
		{"blocked refresh", modelCatalogTerminal{SchemaVersion: 1, Status: "blocked", ReasonCode: "native_refresh_running", PlannedCount: 1, NotAttemptedCount: 1}, true},
		{"blocked after a write is invalid", modelCatalogTerminal{SchemaVersion: 1, Status: "blocked", ReasonCode: "native_refresh_running", PlannedCount: 2, AppliedCount: 1, NotAttemptedCount: 1}, false},
		{"blocked with an unverified write is invalid", modelCatalogTerminal{SchemaVersion: 1, Status: "blocked", ReasonCode: "native_refresh_running", PlannedCount: 1, FailedCount: 1}, false},
		{"failed discovery", modelCatalogTerminal{SchemaVersion: 1, Status: "failed", ReasonCode: "discovery_failed"}, true},
		{"failed after an accepted write is invalid", modelCatalogTerminal{SchemaVersion: 1, Status: "failed", ReasonCode: "apply_failed", PlannedCount: 1, AppliedCount: 1}, false},
		{"partial without accepted write is invalid", modelCatalogTerminal{SchemaVersion: 1, Status: "partial", ReasonCode: "apply_partial", PlannedCount: 1, FailedCount: 1}, false},
		{"succeeded empty plan is wrong", modelCatalogTerminal{SchemaVersion: 1, Status: "succeeded", ReasonCode: "completed"}, false},
		{"status reason mismatch", modelCatalogTerminal{SchemaVersion: 1, Status: "blocked", ReasonCode: "apply_failed"}, false},
		{"count conservation", modelCatalogTerminal{SchemaVersion: 1, Status: "blocked", ReasonCode: "native_refresh_running", PlannedCount: 2, NotAttemptedCount: 1}, false},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			if got := validModelCatalogTerminal(test.result); got != test.valid {
				t.Fatalf("validModelCatalogTerminal()=%v want %v for %+v", got, test.valid, test.result)
			}
		})
	}
}

func TestModelCatalogRunnerSuccessAndSummary(t *testing.T) {
	config := newFakeModelCatalogRunnerConfig(t, `{"schema_version":1,"status":"succeeded","reason_code":"completed","planned_count":2,"applied_count":1,"noop_count":1,"failed_count":0,"not_attempted_count":0,"summary_basename":"screen-summary-20261015T123456Z-0123456789abcdef0123456789abcdef.md"}`, "0")
	if err := os.MkdirAll(config.reportDir, 0700); err != nil {
		t.Fatal(err)
	}
	summary := "# safe summary\n<script>alert(1)</script>"
	if err := os.WriteFile(filepath.Join(config.reportDir, "screen-summary-20261015T123456Z-0123456789abcdef0123456789abcdef.md"), []byte(summary), 0600); err != nil {
		t.Fatal(err)
	}
	runner, err := newModelCatalogRunner(config)
	if err != nil {
		t.Fatal(err)
	}
	if state, started, err := runner.start(); err != nil || !started || state.Status != "running" || state.PlannedCount != nil {
		t.Fatalf("start result=%+v started=%v err=%v", state, started, err)
	}
	state := waitModelCatalogState(t, runner, "succeeded")
	if state.PlannedCount == nil || *state.PlannedCount != 2 || state.AppliedCount == nil || *state.AppliedCount != 1 || state.NoopCount == nil || *state.NoopCount != 1 {
		t.Fatalf("unexpected terminal counts: %+v", state)
	}
	response := runner.response()
	if response.Summary == nil || *response.Summary != summary || response.SummaryUnavailable {
		t.Fatalf("summary not safely returned: %+v", response)
	}
}

func TestModelCatalogRunnerRejectsUnknownResultAndOutputOverflow(t *testing.T) {
	t.Run("invalid result", func(t *testing.T) {
		config := newFakeModelCatalogRunnerConfig(t, `{"schema_version":1,"status":"succeeded"}`, "1")
		runner, err := newModelCatalogRunner(config)
		if err != nil {
			t.Fatal(err)
		}
		if _, _, err := runner.start(); err != nil {
			t.Fatal(err)
		}
		state := waitModelCatalogState(t, runner, "interrupted")
		if state.ReasonCode != "manager_result_invalid" || state.PlannedCount != nil || state.AppliedCount != nil {
			t.Fatalf("invalid result must have unknown counts: %+v", state)
		}
	})
	t.Run("overflow", func(t *testing.T) {
		config := newFakeModelCatalogRunnerConfig(t, "", "0")
		config.env = append(config.env, "MODEL_CATALOG_HELPER_OVERFLOW=1")
		config.outputLimit = 1024
		runner, err := newModelCatalogRunner(config)
		if err != nil {
			t.Fatal(err)
		}
		if _, _, err := runner.start(); err != nil {
			t.Fatal(err)
		}
		state := waitModelCatalogState(t, runner, "interrupted")
		if state.ReasonCode != "manager_result_invalid" || state.PlannedCount != nil {
			t.Fatalf("overflow must have unknown result: %+v", state)
		}
	})
}

func TestModelCatalogRunnerTimeoutAndRestartRecovery(t *testing.T) {
	t.Run("timeout", func(t *testing.T) {
		config := newFakeModelCatalogRunnerConfig(t, "", "0")
		config.timeout = 80 * time.Millisecond
		config.env = append(config.env, "MODEL_CATALOG_HELPER_DELAY_MS=500")
		runner, err := newModelCatalogRunner(config)
		if err != nil {
			t.Fatal(err)
		}
		if _, _, err := runner.start(); err != nil {
			t.Fatal(err)
		}
		state := waitModelCatalogState(t, runner, "interrupted")
		if state.ReasonCode != "process_timeout" || state.PlannedCount != nil {
			t.Fatalf("timeout must have unknown counts: %+v", state)
		}
	})
	t.Run("restart recovery", func(t *testing.T) {
		config := newFakeModelCatalogRunnerConfig(t, "", "0")
		if err := os.MkdirAll(filepath.Dir(config.statePath), 0700); err != nil {
			t.Fatal(err)
		}
		running := modelCatalogRunState{Status: "running", StartedAt: timePtr(time.Now().UTC())}
		setUnknownModelCatalogCounts(&running)
		data, _ := json.Marshal(running)
		if err := os.WriteFile(config.statePath, data, 0600); err != nil {
			t.Fatal(err)
		}
		runner, err := newModelCatalogRunner(config)
		if err != nil {
			t.Fatal(err)
		}
		state := runner.snapshot()
		if state.Status != "interrupted" || state.ReasonCode != "service_interrupted" || state.PlannedCount != nil {
			t.Fatalf("running state did not recover as interrupted: %+v", state)
		}
	})
}

func TestModelCatalogRunnerStartFailureHasKnownZeroCountsAndHidesErrors(t *testing.T) {
	config := newFakeModelCatalogRunnerConfig(t, "", "0")
	config.managerPath = filepath.Join(t.TempDir(), "missing-manager")
	runner, err := newModelCatalogRunner(config)
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := runner.start(); err != nil {
		t.Fatal(err)
	}
	state := waitModelCatalogState(t, runner, "failed")
	if state.ReasonCode != "configuration_invalid" || state.PlannedCount == nil || *state.PlannedCount != 0 || state.FailedCount == nil || *state.FailedCount != 0 {
		t.Fatalf("confirmed pre-start failure should have known zero counts: %+v", state)
	}
	encoded, _ := json.Marshal(state)
	if strings.Contains(string(encoded), "missing-manager") {
		t.Fatalf("state leaked a path: %s", encoded)
	}
}

func TestModelCatalogCommandEnvironmentIsAllowlisted(t *testing.T) {
	config := newFakeModelCatalogRunnerConfig(t, "", "0")
	config.env = nil
	runner, err := newModelCatalogRunner(config)
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("USB_REVOCATION_ADMIN_PASSWORD", "synthetic-admin-password")
	credentials := filepath.Join(t.TempDir(), "systemd-credentials")
	if err := os.MkdirAll(credentials, 0700); err != nil {
		t.Fatal(err)
	}
	t.Setenv("CREDENTIALS_DIRECTORY", credentials)
	env := strings.Join(runner.commandEnvironment(), "\n")
	if strings.Contains(env, "synthetic-admin-password") || strings.Contains(env, "USB_REVOCATION_ADMIN_PASSWORD") {
		t.Fatalf("admin service environment was passed to manager: %s", env)
	}
	if !strings.Contains(env, "CREDENTIALS_DIRECTORY="+credentials) || !strings.Contains(env, "XDG_CACHE_HOME=/var/lib/codex-usb-license-revocation/model-catalog/cache") {
		t.Fatalf("required manager environment missing: %s", env)
	}
}

func TestModelCatalogRunnerSingleFlight(t *testing.T) {
	config := newFakeModelCatalogRunnerConfig(t, `{"schema_version":1,"status":"no_plan","reason_code":"no_applicable_plan","planned_count":0,"applied_count":0,"noop_count":0,"failed_count":0,"not_attempted_count":0,"summary_basename":""}`, "0")
	config.env = append(config.env, "MODEL_CATALOG_HELPER_DELAY_MS=250")
	marker := filepath.Join(t.TempDir(), "calls.txt")
	config.env = append(config.env, "MODEL_CATALOG_HELPER_MARKER="+marker)
	runner, err := newModelCatalogRunner(config)
	if err != nil {
		t.Fatal(err)
	}
	if _, started, err := runner.start(); err != nil || !started {
		t.Fatalf("first start=%v err=%v", started, err)
	}
	if state, started, err := runner.start(); err != nil || started || state.Status != "running" {
		t.Fatalf("duplicate start=%+v started=%v err=%v", state, started, err)
	}
	waitModelCatalogState(t, runner, "no_plan")
	data, err := os.ReadFile(marker)
	if err != nil || strings.Count(string(data), "run\n") != 1 {
		t.Fatalf("manager calls=%q err=%v", data, err)
	}
}

func TestModelCatalogSummaryRejectsTraversalAndSymlink(t *testing.T) {
	dir := t.TempDir()
	if _, err := readModelCatalogSummary(dir, "../secret.md"); err == nil {
		t.Fatal("accepted traversal")
	}
	name := "screen-summary-20261015T123456Z-0123456789abcdef0123456789abcdef.md"
	outside := filepath.Join(t.TempDir(), "outside.md")
	if err := os.WriteFile(outside, []byte("secret"), 0600); err != nil {
		t.Fatal(err)
	}
	oversized := filepath.Join(dir, "screen-summary-20261015T123457Z-0123456789abcdef0123456789abcdef.md")
	if err := os.WriteFile(oversized, []byte(strings.Repeat("x", modelCatalogSummaryLimit+1)), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := readModelCatalogSummary(dir, filepath.Base(oversized)); err == nil {
		t.Fatal("accepted oversized summary")
	}
	if err := os.Symlink(outside, filepath.Join(dir, name)); err != nil {
		t.Skipf("symlink creation unavailable on this Windows configuration: %v", err)
	}
	if _, err := readModelCatalogSummary(dir, name); err == nil {
		t.Fatal("accepted symlink summary")
	}
}

func TestModelCatalogAdminAssetsRenderSummaryAsText(t *testing.T) {
	page, err := adminWebFiles.ReadFile("adminweb/index.html")
	if err != nil {
		t.Fatal(err)
	}
	script, err := adminWebFiles.ReadFile("adminweb/app.js")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(page), `id="model-catalog-summary"`) || !strings.Contains(string(page), `id="model-catalog-run-button"`) {
		t.Fatal("model catalog panel assets are missing")
	}
	if !strings.Contains(string(script), `summary.textContent = typeof data.summary`) || strings.Contains(string(script), ".innerHTML") {
		t.Fatal("model catalog summary must be rendered with textContent only")
	}
	if !strings.Contains(string(script), `Number.isInteger(data.failed_count) && data.failed_count > 0`) || !strings.Contains(string(script), "映射写入结果未核实") || !strings.Contains(string(script), "核对摘要和备份") {
		t.Fatal("unverified mapping writes must prompt the admin to check the summary and backup before retrying")
	}
	for _, unavailable := range []string{
		"function renderModelCatalogUnavailable(message)",
		`$("model-catalog-planned").textContent = unknown`,
		`summary.textContent = ""`,
		"state.modelCatalogTimer = setTimeout",
	} {
		if !strings.Contains(string(script), unavailable) {
			t.Errorf("unavailable task state must clear stale results and safely retry status reads; missing %q", unavailable)
		}
	}
}

func TestAdminLicenseListUsesTwentyItemsPerPage(t *testing.T) {
	script, err := adminWebFiles.ReadFile("adminweb/app.js")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(script), `limit: 20`) {
		t.Fatal("admin USB registration list must request at most 20 items per page")
	}
}
func TestModelCatalogServiceUnitHasScopedCredentialAndStopSemantics(t *testing.T) {
	unit, err := os.ReadFile(filepath.Join("..", "..", "deploy", "codex-usb-license-revocation.service"))
	if err != nil {
		t.Fatal(err)
	}
	for _, required := range []string{
		"Environment=XDG_CACHE_HOME=/var/lib/codex-usb-license-revocation/model-catalog/cache",
		"LoadCredential=admin-api-key:/etc/codex-usb-license-revocation/model-catalog/admin-api-key.json",
		"KillMode=control-group",
		"TimeoutStopSec=15s",
	} {
		if !strings.Contains(string(unit), required) {
			t.Errorf("systemd unit lacks %q", required)
		}
	}
}

func TestAdminModelCatalogRoutesRequireSessionAndCSRF(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()
	web := httptest.NewUnstartedServer(nil)
	origin := "http://" + web.Listener.Addr().String()
	authPath := filepath.Join(t.TempDir(), "admin-auth.json")
	markAdminSetupComplete(t, authPath, "local-test-password-43", "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP")
	managerConfig := newFakeModelCatalogRunnerConfig(t, `{"schema_version":1,"status":"no_plan","reason_code":"no_applicable_plan","planned_count":0,"applied_count":0,"noop_count":0,"failed_count":0,"not_attempted_count":0,"summary_basename":""}`, "0")
	managerConfig.env = append(managerConfig.env, "MODEL_CATALOG_HELPER_DELAY_MS=250")
	httpServer, err := NewHTTPServer(store, keys, HTTPConfig{
		ListenAddress: "127.0.0.1:0", AdminUsername: "operator", AdminPassword: "local-test-password-43",
		AdminOrigin: origin, AdminAuthStatePath: authPath, AdminTOTPSecret: "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
		AdminCookieSecure: false, modelCatalog: &managerConfig,
	}, nil)
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
	page, err := client.Get(web.URL + "/admin/")
	if err != nil {
		t.Fatal(err)
	}
	pageBody, err := io.ReadAll(page.Body)
	page.Body.Close()
	if err != nil || page.StatusCode != http.StatusOK || !strings.Contains(string(pageBody), "刷新并整理模型") || !strings.Contains(string(pageBody), "未知/不可用") {
		t.Fatalf("model catalog admin panel missing or invalid, status=%d err=%v", page.StatusCode, err)
	}
	staticResponse, err := client.Get(web.URL + "/admin/static/app.js")
	if err != nil {
		t.Fatal(err)
	}
	staticResponse.Body.Close()
	if staticResponse.StatusCode != http.StatusOK || staticResponse.Header.Get("Cache-Control") != "no-store" {
		t.Fatalf("admin script status=%d cache-control=%q", staticResponse.StatusCode, staticResponse.Header.Get("Cache-Control"))
	}
	get := doAdminJSON(t, client, http.MethodGet, web.URL+"/admin/api/model-catalog/runs/current", origin, "", nil)
	if get.StatusCode != http.StatusUnauthorized {
		get.Body.Close()
		t.Fatalf("anonymous GET status=%d", get.StatusCode)
	}
	get.Body.Close()
	unauthPost := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/model-catalog/runs", origin, "", nil)
	if unauthPost.StatusCode != http.StatusForbidden {
		unauthPost.Body.Close()
		t.Fatalf("anonymous POST status=%d", unauthPost.StatusCode)
	}
	unauthPost.Body.Close()
	login := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/login", origin, "", adminLoginRequest{Username: "operator", Password: "local-test-password-43"})
	var session struct {
		CSRFToken string `json:"csrfToken"`
	}
	if login.StatusCode != http.StatusOK || json.NewDecoder(login.Body).Decode(&session) != nil || session.CSRFToken == "" {
		login.Body.Close()
		t.Fatal("test admin login failed")
	}
	login.Body.Close()
	badOrigin := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/model-catalog/runs", "http://attacker.invalid", session.CSRFToken, nil)
	if badOrigin.StatusCode != http.StatusForbidden {
		badOrigin.Body.Close()
		t.Fatalf("bad Origin status=%d", badOrigin.StatusCode)
	}
	badOrigin.Body.Close()
	missingCSRF := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/model-catalog/runs", origin, "", nil)
	if missingCSRF.StatusCode != http.StatusForbidden {
		missingCSRF.Body.Close()
		t.Fatalf("missing CSRF status=%d", missingCSRF.StatusCode)
	}
	missingCSRF.Body.Close()
	withBody := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/model-catalog/runs", origin, session.CSRFToken, map[string]string{"target": "not-allowed"})
	if withBody.StatusCode != http.StatusBadRequest {
		withBody.Body.Close()
		t.Fatalf("parameterized POST status=%d", withBody.StatusCode)
	}
	withBody.Body.Close()
	started := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/model-catalog/runs", origin, session.CSRFToken, nil)
	if started.StatusCode != http.StatusAccepted {
		body, _ := ioReadAllClose(started)
		t.Fatalf("start status=%d body=%s", started.StatusCode, body)
	}
	var state modelCatalogRunResponse
	if err := json.NewDecoder(started.Body).Decode(&state); err != nil {
		started.Body.Close()
		t.Fatal(err)
	}
	started.Body.Close()
	if state.Status != "running" || state.PlannedCount != nil {
		t.Fatalf("start state=%+v", state)
	}
	duplicate := doAdminJSON(t, client, http.MethodPost, web.URL+"/admin/api/model-catalog/runs", origin, session.CSRFToken, nil)
	if duplicate.StatusCode != http.StatusOK {
		duplicate.Body.Close()
		t.Fatalf("duplicate status=%d", duplicate.StatusCode)
	}
	duplicate.Body.Close()
	for deadline := time.Now().Add(3 * time.Second); time.Now().Before(deadline); {
		response := doAdminJSON(t, client, http.MethodGet, web.URL+"/admin/api/model-catalog/runs/current", origin, "", nil)
		var current modelCatalogRunResponse
		_ = json.NewDecoder(response.Body).Decode(&current)
		response.Body.Close()
		if current.Status == "no_plan" {
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatal("fake manager did not finish")
}

func newFakeModelCatalogRunnerConfig(t *testing.T, output, exit string) modelCatalogRunnerConfig {
	t.Helper()
	dir := t.TempDir()
	return modelCatalogRunnerConfig{
		managerPath: os.Args[0], configPath: filepath.Join(dir, "manager-config.json"),
		statePath: filepath.Join(dir, "state", "run-state.json"), reportDir: filepath.Join(dir, "reports"), workingDir: dir,
		timeout: 2 * time.Second, outputLimit: modelCatalogOutputLimit,
		args: []string{"-test.run=^TestModelCatalogManagerHelperProcess$", "--"},
		env:  []string{"GO_WANT_MODEL_CATALOG_HELPER=1", "MODEL_CATALOG_HELPER_OUTPUT=" + output, "MODEL_CATALOG_HELPER_EXIT=" + exit},
	}
}

func waitModelCatalogState(t *testing.T, runner *modelCatalogRunner, status string) modelCatalogRunState {
	t.Helper()
	deadline := time.Now().Add(4 * time.Second)
	for time.Now().Before(deadline) {
		state := runner.snapshot()
		if state.Status == status {
			return state
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("model catalog state did not reach %q: %+v", status, runner.snapshot())
	return modelCatalogRunState{}
}

func ioReadAllClose(response *http.Response) ([]byte, error) {
	defer response.Body.Close()
	return io.ReadAll(response.Body)
}
