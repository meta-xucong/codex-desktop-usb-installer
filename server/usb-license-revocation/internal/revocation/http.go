package revocation

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"mime"
	"net"
	"net/http"
	"strings"
	"sync"
	"time"
)

type HTTPConfig struct {
	ListenAddress      string
	MaxRatePerMinute   int
	RateBurst          int
	AdminUsername      string
	AdminPassword      string
	AdminAuthStatePath string
	AdminTOTPSecret    string
	AdminOrigin        string
	AdminCookieSecure  bool
	modelCatalog       *modelCatalogRunnerConfig
}

type tokenBucket struct {
	mu     sync.Mutex
	tokens float64
	last   time.Time
	rate   float64
	burst  float64
}

func newTokenBucket(perMinute, burst int) *tokenBucket {
	if perMinute <= 0 {
		perMinute = 60
	}
	if burst <= 0 {
		burst = 10
	}
	return &tokenBucket{tokens: float64(burst), last: time.Now(), rate: float64(perMinute) / 60, burst: float64(burst)}
}

func (b *tokenBucket) allow(now time.Time) bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	elapsed := now.Sub(b.last).Seconds()
	if elapsed > 0 {
		b.tokens += elapsed * b.rate
		if b.tokens > b.burst {
			b.tokens = b.burst
		}
		b.last = now
	}
	if b.tokens < 1 {
		return false
	}
	b.tokens--
	return true
}

func NewHTTPServer(store *Store, keyring map[string]ed25519.PublicKey, cfg HTTPConfig, logger *slog.Logger) (*http.Server, error) {
	if !isLoopbackAddress(cfg.ListenAddress) {
		return nil, errors.New("service listener must be a loopback IP")
	}
	if logger == nil {
		logger = slog.New(slog.NewTextHandler(io.Discard, nil))
	}
	bucket := newTokenBucket(cfg.MaxRatePerMinute, cfg.RateBurst)
	registrationBucket := newTokenBucket(20, 4)
	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Header().Set("Cache-Control", "no-store")
		w.WriteHeader(http.StatusOK)
		_, _ = io.WriteString(w, `{"status":"ok"}`)
	})
	mux.HandleFunc("POST /v1/licenses/check", func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		if !bucket.allow(start) {
			writeError(w, http.StatusTooManyRequests)
			logger.Info("license check", "outcome", "rate_limited", "elapsed_ms", time.Since(start).Milliseconds())
			return
		}
		mediaType, _, err := mime.ParseMediaType(r.Header.Get("Content-Type"))
		if err != nil || mediaType != "application/json" {
			writeError(w, http.StatusUnsupportedMediaType)
			logger.Info("license check", "outcome", "invalid_content_type", "elapsed_ms", time.Since(start).Milliseconds())
			return
		}
		body := http.MaxBytesReader(w, r.Body, MaxRequestBytes)
		defer body.Close()
		raw, err := io.ReadAll(body)
		if err != nil {
			status, outcome := http.StatusBadRequest, "malformed_request"
			var tooLarge *http.MaxBytesError
			if errors.As(err, &tooLarge) {
				status, outcome = http.StatusRequestEntityTooLarge, "request_too_large"
			}
			writeError(w, status)
			logger.Info("license check", "outcome", outcome, "elapsed_ms", time.Since(start).Milliseconds())
			return
		}
		var req CheckRequest
		if err := decodeStrictJSON(raw, &req); err != nil {
			writeError(w, http.StatusBadRequest)
			logger.Info("license check", "outcome", "malformed_request", "elapsed_ms", time.Since(start).Milliseconds())
			return
		}
		ctx, cancel := context.WithTimeout(r.Context(), 3*time.Second)
		defer cancel()
		envelope, record, err := store.LookupForCheck(ctx, req, keyring, time.Now().UTC())
		if err != nil {
			status, outcome := http.StatusServiceUnavailable, "state_unknown"
			if errors.Is(err, errInvalidRegistration) {
				status, outcome = http.StatusBadRequest, "proof_rejected"
			}
			writeError(w, status)
			logger.Info("license check", "outcome", outcome, "installation_id", safeLogID(req.InstallationID), "request_id", safeLogID(req.RequestID), "elapsed_ms", time.Since(start).Milliseconds())
			return
		}
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		_ = json.NewEncoder(w).Encode(envelope)
		logger.Info("license check", "outcome", "signed_"+record.Status, "installation_id", record.InstallationID, "state_version", record.StateVersion, "request_id", req.RequestID, "elapsed_ms", time.Since(start).Milliseconds())
	})
	mux.HandleFunc("POST /v1/licenses/register", func(w http.ResponseWriter, r *http.Request) {
		handleRegistrationUpload(w, r, store, registrationBucket, logger)
	})
	if cfg.AdminUsername != "" {
		if err := ensurePathIsDistinct(append([]string{cfg.AdminAuthStatePath}, storeStatePaths(store.config)...)...); err != nil {
			return nil, errors.New("admin auth state path must be distinct from license database and recovery files")
		}
		admin, err := newAdminConsole(store, cfg, logger)
		if err != nil {
			return nil, err
		}
		if cfg.modelCatalog != nil {
			runner, runnerErr := newModelCatalogRunner(*cfg.modelCatalog)
			if runnerErr != nil {
				logger.Error("model catalog admin feature unavailable")
			} else {
				admin.modelCatalog = runner
			}
		}
		admin.Register(mux)
	}
	return &http.Server{
		Addr: cfg.ListenAddress, Handler: mux, ReadHeaderTimeout: 3 * time.Second,
		ReadTimeout: 5 * time.Second, WriteTimeout: 5 * time.Second, IdleTimeout: 30 * time.Second,
		MaxHeaderBytes: 16 << 10,
	}, nil
}

func handleRegistrationUpload(w http.ResponseWriter, r *http.Request, store *Store, bucket *tokenBucket, logger *slog.Logger) {
	start := time.Now()
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	if !bucket.allow(start) {
		writeError(w, http.StatusTooManyRequests)
		logger.Info("license registration", "outcome", "rate_limited", "elapsed_ms", time.Since(start).Milliseconds())
		return
	}
	mediaType, _, err := mime.ParseMediaType(r.Header.Get("Content-Type"))
	if err != nil || mediaType != "application/json" {
		writeError(w, http.StatusUnsupportedMediaType)
		logger.Info("license registration", "outcome", "invalid_content_type", "elapsed_ms", time.Since(start).Milliseconds())
		return
	}
	body := http.MaxBytesReader(w, r.Body, MaxRequestBytes)
	defer body.Close()
	raw, err := io.ReadAll(body)
	if err != nil {
		status := http.StatusBadRequest
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			status = http.StatusRequestEntityTooLarge
		}
		writeError(w, status)
		logger.Info("license registration", "outcome", "invalid_body", "elapsed_ms", time.Since(start).Milliseconds())
		return
	}
	var request RegistrationUploadRequest
	if err := decodeStrictJSON(raw, &request); err != nil || request.Registration.PayloadB64 == "" || request.Registration.SignatureB64 == "" {
		writeError(w, http.StatusBadRequest)
		logger.Info("license registration", "outcome", "malformed_request", "elapsed_ms", time.Since(start).Milliseconds())
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 5*time.Second)
	defer cancel()
	record, created, err := store.Register(ctx, request.Registration, time.Now().UTC())
	if err != nil {
		status, outcome, code := http.StatusServiceUnavailable, "state_unknown", "registration_unavailable"
		switch {
		case errors.Is(err, errInvalidRegistration):
			status, outcome, code = http.StatusBadRequest, "proof_rejected", "invalid_registration"
		case errors.Is(err, errAlreadyExists):
			status, outcome, code = http.StatusConflict, "registration_conflict", "registration_conflict"
		}
		writeRegistrationError(w, status, code)
		logger.Info("license registration", "outcome", outcome, "elapsed_ms", time.Since(start).Milliseconds())
		return
	}
	if record.Status != "active" {
		writeRegistrationError(w, http.StatusConflict, "license_revoked")
		logger.Info("license registration", "outcome", "license_revoked", "installation_id", safeLogID(record.InstallationID), "state_version", record.StateVersion, "elapsed_ms", time.Since(start).Milliseconds())
		return
	}
	w.Header().Set("Content-Type", "application/json")
	status := http.StatusOK
	if created {
		status = http.StatusCreated
	}
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(map[string]any{
		"status": record.Status, "installationId": record.InstallationID,
		"stateVersion": record.StateVersion, "created": created,
	})
	outcome := "idempotent_active"
	if created {
		outcome = "activated"
	}
	logger.Info("license registration", "outcome", outcome, "installation_id", safeLogID(record.InstallationID), "state_version", record.StateVersion, "elapsed_ms", time.Since(start).Milliseconds())
}

func writeRegistrationError(w http.ResponseWriter, status int, code string) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(map[string]string{"error": code})
}

func isLoopbackAddress(address string) bool {
	host, _, err := net.SplitHostPort(address)
	if err != nil {
		return false
	}
	ip := net.ParseIP(strings.Trim(host, "[]"))
	return ip != nil && ip.IsLoopback()
}

func safeLogID(value string) string {
	if validID(value) {
		return value
	}
	return "invalid"
}

func writeError(w http.ResponseWriter, status int) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	_, _ = io.WriteString(w, `{"error":"request not accepted"}`)
}
