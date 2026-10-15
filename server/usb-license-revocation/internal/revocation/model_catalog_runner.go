package revocation

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

const (
	modelCatalogRunTimeout   = 45 * time.Minute
	modelCatalogOutputLimit  = 1 << 20
	modelCatalogSummaryLimit = 2 << 20
	modelCatalogStateLimit   = 64 << 10
)

var modelCatalogSummaryName = regexp.MustCompile(`^screen-summary-[0-9]{8}T[0-9]{6}Z-[a-f0-9]{32}\.md$`)

type modelCatalogRunnerConfig struct {
	managerPath string
	configPath  string
	statePath   string
	reportDir   string
	workingDir  string
	timeout     time.Duration
	outputLimit int
	args        []string // Tests only; production uses the fixed full-screen command.
	env         []string // Tests only; production passes a small environment allowlist.
}

func defaultModelCatalogRunnerConfig() *modelCatalogRunnerConfig {
	return &modelCatalogRunnerConfig{
		managerPath: "/opt/codex-usb-license-revocation/model-catalog-manager",
		configPath:  "/etc/codex-usb-license-revocation/model-catalog/config.json",
		statePath:   "/var/lib/codex-usb-license-revocation/model-catalog/run-state.json",
		reportDir:   "/var/lib/codex-usb-license-revocation/model-catalog/reports",
		workingDir:  "/opt/codex-usb-license-revocation",
		timeout:     modelCatalogRunTimeout,
		outputLimit: modelCatalogOutputLimit,
	}
}

type modelCatalogTerminal struct {
	SchemaVersion     int    `json:"schema_version"`
	Status            string `json:"status"`
	ReasonCode        string `json:"reason_code"`
	PlannedCount      int    `json:"planned_count"`
	AppliedCount      int    `json:"applied_count"`
	NoopCount         int    `json:"noop_count"`
	FailedCount       int    `json:"failed_count"`
	NotAttemptedCount int    `json:"not_attempted_count"`
	SummaryBasename   string `json:"summary_basename"`
}

type modelCatalogRunState struct {
	Status            string     `json:"status"`
	ReasonCode        string     `json:"reason_code,omitempty"`
	StartedAt         *time.Time `json:"started_at,omitempty"`
	FinishedAt        *time.Time `json:"finished_at,omitempty"`
	PlannedCount      *int       `json:"planned_count"`
	AppliedCount      *int       `json:"applied_count"`
	NoopCount         *int       `json:"noop_count"`
	FailedCount       *int       `json:"failed_count"`
	NotAttemptedCount *int       `json:"not_attempted_count"`
	SummaryBasename   string     `json:"summary_basename,omitempty"`
}

type modelCatalogRunResponse struct {
	modelCatalogRunState
	Summary            *string `json:"summary"`
	SummaryUnavailable bool    `json:"summary_unavailable"`
}

type modelCatalogRunner struct {
	mu    sync.Mutex
	cfg   modelCatalogRunnerConfig
	state modelCatalogRunState
}

func newModelCatalogRunner(cfg modelCatalogRunnerConfig) (*modelCatalogRunner, error) {
	if cfg.managerPath == "" || cfg.configPath == "" || cfg.statePath == "" || cfg.reportDir == "" || cfg.workingDir == "" {
		return nil, errors.New("model catalog runner paths are incomplete")
	}
	for _, path := range []string{cfg.managerPath, cfg.configPath, cfg.statePath, cfg.reportDir, cfg.workingDir} {
		if !filepath.IsAbs(path) || strings.ContainsRune(path, '\x00') {
			return nil, errors.New("model catalog runner path is invalid")
		}
	}
	if cfg.timeout <= 0 {
		cfg.timeout = modelCatalogRunTimeout
	}
	if cfg.outputLimit <= 0 || cfg.outputLimit > modelCatalogOutputLimit {
		cfg.outputLimit = modelCatalogOutputLimit
	}
	if err := ensurePrivateDirectory(filepath.Dir(cfg.statePath)); err != nil {
		return nil, err
	}
	if err := ensurePrivateDirectory(cfg.reportDir); err != nil {
		return nil, err
	}
	runner := &modelCatalogRunner{cfg: cfg, state: emptyModelCatalogState("idle")}
	if err := runner.loadState(); err != nil {
		return nil, err
	}
	if runner.state.Status == "running" {
		runner.state.Status = "interrupted"
		runner.state.ReasonCode = "service_interrupted"
		runner.state.FinishedAt = timePtr(time.Now().UTC())
		setUnknownModelCatalogCounts(&runner.state)
		if err := runner.persistState(); err != nil {
			return nil, err
		}
	}
	return runner, nil
}

func emptyModelCatalogState(status string) modelCatalogRunState {
	return modelCatalogRunState{Status: status}
}

func setUnknownModelCatalogCounts(state *modelCatalogRunState) {
	state.PlannedCount = nil
	state.AppliedCount = nil
	state.NoopCount = nil
	state.FailedCount = nil
	state.NotAttemptedCount = nil
}

func setKnownZeroModelCatalogCounts(state *modelCatalogRunState) {
	zero := 0
	state.PlannedCount = intPtr(zero)
	state.AppliedCount = intPtr(zero)
	state.NoopCount = intPtr(zero)
	state.FailedCount = intPtr(zero)
	state.NotAttemptedCount = intPtr(zero)
}

func intPtr(value int) *int              { return &value }
func timePtr(value time.Time) *time.Time { return &value }

func ensurePrivateDirectory(path string) error {
	if err := os.MkdirAll(path, 0700); err != nil {
		return errors.New("model catalog state directory unavailable")
	}
	info, err := os.Lstat(path)
	if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return errors.New("model catalog state directory is invalid")
	}
	if err := os.Chmod(path, 0700); err != nil {
		return errors.New("model catalog state directory permissions unavailable")
	}
	return nil
}

func (r *modelCatalogRunner) loadState() error {
	info, err := os.Lstat(r.cfg.statePath)
	if errors.Is(err, os.ErrNotExist) {
		return r.persistState()
	}
	if err != nil || !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 || info.Size() > modelCatalogStateLimit || (runtime.GOOS != "windows" && info.Mode().Perm()&0077 != 0) {
		return errors.New("model catalog state file is invalid")
	}
	file, err := os.Open(r.cfg.statePath)
	if err != nil {
		return errors.New("model catalog state file unavailable")
	}
	defer file.Close()
	opened, err := file.Stat()
	if err != nil || !os.SameFile(info, opened) || !opened.Mode().IsRegular() || opened.Size() > modelCatalogStateLimit {
		return errors.New("model catalog state file changed while opening")
	}
	data, err := io.ReadAll(io.LimitReader(file, modelCatalogStateLimit+1))
	if err != nil || len(data) > modelCatalogStateLimit {
		return errors.New("model catalog state file could not be read")
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	var state modelCatalogRunState
	if err := decoder.Decode(&state); err != nil || ensureJSONEOF(decoder) != nil || !validModelCatalogState(state) {
		return errors.New("model catalog state file is malformed")
	}
	r.state = state
	return nil
}

func validModelCatalogState(state modelCatalogRunState) bool {
	switch state.Status {
	case "idle", "running", "succeeded", "no_plan", "partial", "blocked", "failed", "interrupted":
	default:
		return false
	}
	for _, count := range []*int{state.PlannedCount, state.AppliedCount, state.NoopCount, state.FailedCount, state.NotAttemptedCount} {
		if count != nil && *count < 0 {
			return false
		}
	}
	return state.SummaryBasename == "" || modelCatalogSummaryName.MatchString(state.SummaryBasename)
}

func ensureJSONEOF(decoder *json.Decoder) error {
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		return errors.New("extra JSON data")
	}
	return nil
}

func (r *modelCatalogRunner) persistState() error {
	if err := ensurePrivateDirectory(filepath.Dir(r.cfg.statePath)); err != nil {
		return err
	}
	data, err := json.Marshal(r.state)
	if err != nil || len(data) > modelCatalogStateLimit {
		return errors.New("model catalog state could not be encoded")
	}
	file, err := os.CreateTemp(filepath.Dir(r.cfg.statePath), ".model-catalog-state-*.tmp")
	if err != nil {
		return errors.New("model catalog state could not be created")
	}
	tempPath := file.Name()
	defer os.Remove(tempPath)
	if err := file.Chmod(0600); err != nil {
		_ = file.Close()
		return errors.New("model catalog state permissions unavailable")
	}
	if _, err := file.Write(data); err != nil {
		_ = file.Close()
		return errors.New("model catalog state write failed")
	}
	if err := file.Sync(); err != nil {
		_ = file.Close()
		return errors.New("model catalog state flush failed")
	}
	if err := file.Close(); err != nil {
		return errors.New("model catalog state close failed")
	}
	if err := os.Rename(tempPath, r.cfg.statePath); err != nil {
		return errors.New("model catalog state publish failed")
	}
	return nil
}

func (r *modelCatalogRunner) snapshot() modelCatalogRunState {
	r.mu.Lock()
	defer r.mu.Unlock()
	copy := r.state
	copy.StartedAt = cloneTime(r.state.StartedAt)
	copy.FinishedAt = cloneTime(r.state.FinishedAt)
	copy.PlannedCount = cloneInt(r.state.PlannedCount)
	copy.AppliedCount = cloneInt(r.state.AppliedCount)
	copy.NoopCount = cloneInt(r.state.NoopCount)
	copy.FailedCount = cloneInt(r.state.FailedCount)
	copy.NotAttemptedCount = cloneInt(r.state.NotAttemptedCount)
	return copy
}

func cloneTime(value *time.Time) *time.Time {
	if value == nil {
		return nil
	}
	copy := *value
	return &copy
}

func cloneInt(value *int) *int {
	if value == nil {
		return nil
	}
	copy := *value
	return &copy
}

func (r *modelCatalogRunner) start() (modelCatalogRunState, bool, error) {
	r.mu.Lock()
	if r.state.Status == "running" {
		state := r.state
		r.mu.Unlock()
		return state, false, nil
	}
	now := time.Now().UTC()
	r.state = modelCatalogRunState{Status: "running", StartedAt: timePtr(now)}
	setUnknownModelCatalogCounts(&r.state)
	if err := r.persistState(); err != nil {
		r.state = modelCatalogRunState{Status: "failed", ReasonCode: "configuration_invalid", FinishedAt: timePtr(now)}
		setKnownZeroModelCatalogCounts(&r.state)
		r.mu.Unlock()
		return modelCatalogRunState{}, false, errors.New("model catalog state could not be saved")
	}
	state := r.state
	r.mu.Unlock()
	go r.execute()
	return state, true, nil
}

func (r *modelCatalogRunner) execute() {
	ctx, cancel := context.WithTimeout(context.Background(), r.cfg.timeout)
	defer cancel()
	stdout := &limitedProcessBuffer{}
	stderr := &limitedProcessBuffer{}
	budget := &processOutputBudget{limit: r.cfg.outputLimit}
	stdout.budget = budget
	stderr.budget = budget
	stdout.cancel = cancel
	stderr.cancel = cancel
	command := exec.CommandContext(ctx, r.cfg.managerPath, r.commandArgs()...)
	command.Dir = r.cfg.workingDir
	command.Env = r.commandEnvironment()
	command.Stdout = stdout
	command.Stderr = stderr
	if err := command.Start(); err != nil {
		r.finishFailure("configuration_invalid", err.Error())
		return
	}
	waitErr := command.Wait()
	if stdout.overflow.Load() || stderr.overflow.Load() {
		r.finishInterrupted("manager_result_invalid")
		return
	}
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		r.finishInterrupted("process_timeout")
		return
	}
	terminal, parseErr := parseModelCatalogTerminal(stdout.Bytes())
	if parseErr != nil || !validModelCatalogExit(terminal, waitErr) {
		r.finishInterrupted("manager_result_invalid")
		return
	}
	r.finishTerminal(terminal)
}

func (r *modelCatalogRunner) commandArgs() []string {
	if len(r.cfg.args) > 0 {
		return append([]string(nil), r.cfg.args...)
	}
	return []string{"run", "--config", r.cfg.configPath, "--full-screen"}
}

func (r *modelCatalogRunner) commandEnvironment() []string {
	if len(r.cfg.env) > 0 {
		return append([]string(nil), r.cfg.env...)
	}
	env := []string{
		"PATH=/usr/bin:/bin",
		"HOME=/var/lib/codex-usb-license-revocation",
		"XDG_CACHE_HOME=/var/lib/codex-usb-license-revocation/model-catalog/cache",
	}
	if credentialDir := os.Getenv("CREDENTIALS_DIRECTORY"); credentialDir != "" && filepath.IsAbs(credentialDir) {
		env = append(env, "CREDENTIALS_DIRECTORY="+credentialDir)
	}
	return env
}

func parseModelCatalogTerminal(raw []byte) (modelCatalogTerminal, error) {
	var result modelCatalogTerminal
	if len(raw) == 0 || len(raw) > modelCatalogOutputLimit || !bytes.HasSuffix(raw, []byte("\n")) {
		return result, errors.New("terminal result framing is invalid")
	}
	lines := bytes.Split(raw, []byte("\n"))
	if len(lines) != 2 || len(lines[0]) == 0 {
		return result, errors.New("terminal result must contain exactly one JSON line")
	}
	decoder := json.NewDecoder(bytes.NewReader(lines[0]))
	openToken, err := decoder.Token()
	if err != nil || openToken != json.Delim('{') {
		return result, errors.New("terminal result JSON is invalid")
	}
	fields := make(map[string]json.RawMessage, 9)
	for decoder.More() {
		token, tokenErr := decoder.Token()
		key, ok := token.(string)
		if tokenErr != nil || !ok {
			return result, errors.New("terminal result JSON field is invalid")
		}
		if _, duplicate := fields[key]; duplicate {
			return result, errors.New("terminal result contains a duplicate field")
		}
		var value json.RawMessage
		if err := decoder.Decode(&value); err != nil {
			return result, errors.New("terminal result JSON field is invalid")
		}
		fields[key] = value
	}
	closeToken, closeErr := decoder.Token()
	if closeErr != nil || closeToken != json.Delim('}') || ensureJSONEOF(decoder) != nil || len(fields) != 9 {
		return result, errors.New("terminal result JSON fields are incomplete")
	}
	decodeInt := func(key string, target *int) bool {
		raw, ok := fields[key]
		return ok && !bytes.Equal(bytes.TrimSpace(raw), []byte("null")) && json.Unmarshal(raw, target) == nil
	}
	decodeString := func(key string, target *string) bool {
		raw, ok := fields[key]
		return ok && !bytes.Equal(bytes.TrimSpace(raw), []byte("null")) && json.Unmarshal(raw, target) == nil
	}
	if !decodeInt("schema_version", &result.SchemaVersion) || !decodeString("status", &result.Status) || !decodeString("reason_code", &result.ReasonCode) || !decodeInt("planned_count", &result.PlannedCount) || !decodeInt("applied_count", &result.AppliedCount) || !decodeInt("noop_count", &result.NoopCount) || !decodeInt("failed_count", &result.FailedCount) || !decodeInt("not_attempted_count", &result.NotAttemptedCount) || !decodeString("summary_basename", &result.SummaryBasename) {
		return result, errors.New("terminal result JSON fields are malformed")
	}
	if !validModelCatalogTerminal(result) {
		return result, errors.New("terminal result fields are invalid")
	}
	return result, nil
}

func validModelCatalogTerminal(result modelCatalogTerminal) bool {
	if result.SchemaVersion != 1 || result.PlannedCount < 0 || result.AppliedCount < 0 || result.NoopCount < 0 || result.FailedCount < 0 || result.NotAttemptedCount < 0 {
		return false
	}
	if result.AppliedCount+result.NoopCount+result.FailedCount+result.NotAttemptedCount != result.PlannedCount {
		return false
	}
	switch result.Status {
	case "succeeded", "no_plan", "partial", "blocked", "failed":
	default:
		return false
	}
	switch result.ReasonCode {
	case "completed", "no_applicable_plan", "account_scan_partial", "native_refresh_running", "native_refresh_status_unavailable", "native_refresh_status_unknown", "manager_lock_requires_review", "configuration_invalid", "credential_unavailable", "discovery_failed", "approved_rules_blocked", "apply_partial", "apply_failed":
	default:
		return false
	}
	if result.SummaryBasename != "" && !modelCatalogSummaryName.MatchString(result.SummaryBasename) {
		return false
	}
	switch result.Status {
	case "succeeded":
		return result.ReasonCode == "completed" && result.PlannedCount > 0 && result.FailedCount == 0 && result.NotAttemptedCount == 0 && result.AppliedCount+result.NoopCount == result.PlannedCount
	case "no_plan":
		return result.ReasonCode == "no_applicable_plan" && result.PlannedCount == 0
	case "blocked":
		return result.AppliedCount == 0 && result.FailedCount == 0 && (result.ReasonCode == "native_refresh_running" || result.ReasonCode == "native_refresh_status_unavailable" || result.ReasonCode == "native_refresh_status_unknown" || result.ReasonCode == "manager_lock_requires_review" || result.ReasonCode == "approved_rules_blocked")
	case "partial":
		switch result.ReasonCode {
		case "account_scan_partial":
			return result.PlannedCount == 0 && result.AppliedCount == 0 && result.NoopCount == 0 && result.FailedCount == 0 && result.NotAttemptedCount == 0
		case "apply_partial", "native_refresh_running", "native_refresh_status_unavailable", "native_refresh_status_unknown":
			return result.AppliedCount > 0
		default:
			return false
		}
	case "failed":
		return result.AppliedCount == 0 && (result.ReasonCode == "configuration_invalid" || result.ReasonCode == "credential_unavailable" || result.ReasonCode == "discovery_failed" || result.ReasonCode == "apply_failed")
	default:
		return false
	}
}

func validModelCatalogExit(result modelCatalogTerminal, waitErr error) bool {
	if result.Status == "succeeded" || result.Status == "no_plan" {
		return waitErr == nil
	}
	var exitErr *exec.ExitError
	return errors.As(waitErr, &exitErr)
}

func (r *modelCatalogRunner) finishTerminal(result modelCatalogTerminal) {
	r.mu.Lock()
	defer r.mu.Unlock()
	now := time.Now().UTC()
	r.state.Status = result.Status
	r.state.ReasonCode = result.ReasonCode
	r.state.FinishedAt = timePtr(now)
	r.state.PlannedCount = intPtr(result.PlannedCount)
	r.state.AppliedCount = intPtr(result.AppliedCount)
	r.state.NoopCount = intPtr(result.NoopCount)
	r.state.FailedCount = intPtr(result.FailedCount)
	r.state.NotAttemptedCount = intPtr(result.NotAttemptedCount)
	r.state.SummaryBasename = result.SummaryBasename
	_ = r.persistState()
}

func (r *modelCatalogRunner) finishFailure(reason, _ string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	now := time.Now().UTC()
	r.state.Status = "failed"
	r.state.ReasonCode = reason
	r.state.FinishedAt = timePtr(now)
	setKnownZeroModelCatalogCounts(&r.state)
	r.state.SummaryBasename = ""
	_ = r.persistState()
}

func (r *modelCatalogRunner) finishInterrupted(reason string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.state.Status = "interrupted"
	r.state.ReasonCode = reason
	r.state.FinishedAt = timePtr(time.Now().UTC())
	setUnknownModelCatalogCounts(&r.state)
	r.state.SummaryBasename = ""
	_ = r.persistState()
}

func (r *modelCatalogRunner) response() modelCatalogRunResponse {
	state := r.snapshot()
	response := modelCatalogRunResponse{modelCatalogRunState: state}
	if state.SummaryBasename == "" {
		return response
	}
	summary, err := readModelCatalogSummary(r.cfg.reportDir, state.SummaryBasename)
	if err != nil {
		response.SummaryUnavailable = true
		return response
	}
	response.Summary = &summary
	return response
}

func readModelCatalogSummary(directory, basename string) (string, error) {
	if !modelCatalogSummaryName.MatchString(basename) || filepath.Base(basename) != basename {
		return "", errors.New("summary name is invalid")
	}
	path := filepath.Join(directory, basename)
	before, err := os.Lstat(path)
	if err != nil || !before.Mode().IsRegular() || before.Mode()&os.ModeSymlink != 0 || before.Size() > modelCatalogSummaryLimit || (runtime.GOOS != "windows" && before.Mode().Perm()&0077 != 0) {
		return "", errors.New("summary file is unavailable")
	}
	file, err := os.Open(path)
	if err != nil {
		return "", errors.New("summary file is unavailable")
	}
	defer file.Close()
	after, err := file.Stat()
	if err != nil || !os.SameFile(before, after) || !after.Mode().IsRegular() || after.Size() > modelCatalogSummaryLimit || (runtime.GOOS != "windows" && after.Mode().Perm()&0077 != 0) {
		return "", errors.New("summary file changed while opening")
	}
	body, err := io.ReadAll(io.LimitReader(file, modelCatalogSummaryLimit+1))
	if err != nil || len(body) > modelCatalogSummaryLimit {
		return "", errors.New("summary exceeds the size limit")
	}
	return string(body), nil
}

type limitedProcessBuffer struct {
	mu       sync.Mutex
	data     bytes.Buffer
	budget   *processOutputBudget
	cancel   context.CancelFunc
	overflow atomic.Bool
}

type processOutputBudget struct {
	mu    sync.Mutex
	total int
	limit int
}

func (b *limitedProcessBuffer) Write(data []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.budget.mu.Lock()
	defer b.budget.mu.Unlock()
	remaining := b.budget.limit - b.budget.total
	if len(data) > remaining {
		b.overflow.Store(true)
		b.cancel()
		if remaining > 0 {
			_, _ = b.data.Write(data[:remaining])
			b.budget.total += remaining
		}
		return len(data), nil
	}
	_, _ = b.data.Write(data)
	b.budget.total += len(data)
	return len(data), nil
}

func (b *limitedProcessBuffer) Bytes() []byte {
	b.mu.Lock()
	defer b.mu.Unlock()
	return append([]byte(nil), b.data.Bytes()...)
}

func (r *modelCatalogRunner) currentResponse() modelCatalogRunResponse {
	return r.response()
}
