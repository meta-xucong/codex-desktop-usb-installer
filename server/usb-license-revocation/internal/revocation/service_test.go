package revocation

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"database/sql"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

const (
	fixturePayloadB64   = "eyJzY2hlbWFWZXJzaW9uIjoxLCJkb21haW4iOiJjb2RleC11c2ItcmV2b2NhdGlvbi1yZWdpc3RyYXRpb24tdjEiLCJwcm9kdWN0IjoiY29kZXgtdXNiLW9uZWNsaWNrIiwiaW5zdGFsbGF0aW9uSWQiOiJ1c2ItaW5zdGFsbC0wMDEiLCJsaWNlbnNlS2V5SWQiOiJsaWNlbnNlLXRlc3QtMjAyNiIsImxpY2Vuc2VTaWduYXR1cmVTaGEyNTYiOiJhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhIiwiZmluZ2VycHJpbnRTaGEyNTYiOiI2NDU4YTkwMzVmNjkwY2RmYzlhNjQyM2Y0NGY0ZmVmMTg2MGY2YWFmOWI5MGM1YzEzNGNhNWMwZmI3ZWNhNTFmIiwiaXNzdWVkQXQiOiIxNzcwMDAwMDAwMDAwIn0="
	fixtureSignatureB64 = "F4TgInoYksaZ57gZfQ9G/FzZXB0XXsikSeqcw2k6mw65F0Wt1OTJRWVU0g3Uv6LsAQiv1SkYN/kpx33etEf9Aw=="
	fixtureFingerprint  = "6458a9035f690cdfc9a6423f44f4fef1860f6aaf9b90c5c134ca5c0fb7eca51f"
	fixtureLicenseHash  = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	fixtureInstallation = "usb-install-001"
	fixtureLicenseKeyID = "license-test-2026"
	fixtureStatusKeyID  = "revocation-test-2026"
)

func TestFingerprintCanonicalizationVector(t *testing.T) {
	got, err := FingerprintDigest(3, 0x12345678, "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF", `CodexUsb\Bundle`)
	if err != nil {
		t.Fatal(err)
	}
	if got != fixtureFingerprint {
		t.Fatalf("digest=%s, want %s", got, fixtureFingerprint)
	}
	other, err := FingerprintDigest(3, 0x12345678, strings.ToLower("0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF"), "codexusb/bundle")
	if err != nil || other != got {
		t.Fatalf("normalized path/hash differed: %s %v", other, err)
	}
}

func TestFingerprintRejectsUnsafeOrMalformedInputs(t *testing.T) {
	anchor := strings.Repeat("01", 32)
	for _, path := range []string{"", "/bundle", `\\server\share`, `C:\bundle`, "bundle//child", "bundle/.", "bundle/..", "bundle/child:", "bundle\x00child", string([]byte{0xff})} {
		t.Run(hex.EncodeToString([]byte(path)), func(t *testing.T) {
			if _, err := FingerprintDigest(3, 1, anchor, path); !errors.Is(err, ErrInvalidFingerprint) {
				t.Fatalf("expected ErrInvalidFingerprint for %q, got %v", path, err)
			}
		})
	}
	if _, err := FingerprintDigest(3, 1, "xyz", "bundle"); !errors.Is(err, ErrInvalidFingerprint) {
		t.Fatalf("malformed anchor accepted: %v", err)
	}
}

func TestRegistrationProofFixedVectorAndRawByteSignature(t *testing.T) {
	pub, _ := licenseFixtureKey()
	ring := map[string]ed25519.PublicKey{fixtureLicenseKeyID: pub}
	env := fixtureRegistration()
	payload, err := VerifyRegistration(env, ring)
	if err != nil {
		t.Fatal(err)
	}
	if payload.InstallationID != fixtureInstallation || payload.FingerprintSHA256 != fixtureFingerprint {
		t.Fatalf("wrong fixture payload: %+v", payload)
	}
	decoded, err := decodeStdBase64(env.PayloadB64)
	if err != nil {
		t.Fatal(err)
	}
	sig, _ := decodeStdBase64(env.SignatureB64)
	if !ed25519.Verify(pub, decoded, sig) {
		t.Fatal("fixed raw byte signature did not verify")
	}
	if env.SignatureB64 != fixtureSignatureB64 {
		t.Fatal("fixture signature changed")
	}
	if payload.IssuedAt != "1770000000000" {
		t.Fatalf("issuedAt=%q, want exact unsigned-millisecond fixture", payload.IssuedAt)
	}

	altered := append([]byte(nil), decoded...)
	altered[len(altered)-3] ^= 1
	if _, err := VerifyRegistration(RegistrationEnvelope{PayloadB64: base64.StdEncoding.EncodeToString(altered), SignatureB64: env.SignatureB64}, ring); err == nil {
		t.Fatal("one-byte payload change was accepted")
	}
	wrongSig := append([]byte(nil), sig...)
	wrongSig[0] ^= 0x80
	if _, err := VerifyRegistration(RegistrationEnvelope{PayloadB64: env.PayloadB64, SignatureB64: base64.StdEncoding.EncodeToString(wrongSig)}, ring); err == nil {
		t.Fatal("modified signature was accepted")
	}
}

func TestRegistrationIssuedAtUnsignedDecimalMilliseconds(t *testing.T) {
	pub, priv := licenseFixtureKey()
	ring := map[string]ed25519.PublicKey{fixtureLicenseKeyID: pub}
	base, err := decodeStdBase64(fixturePayloadB64)
	if err != nil {
		t.Fatal(err)
	}
	for _, valid := range []string{"0", "0001", "1770000000000", "18446744073709551615"} {
		t.Run("accept_"+valid, func(t *testing.T) {
			payload := strings.Replace(string(base), `"issuedAt":"1770000000000"`, `"issuedAt":"`+valid+`"`, 1)
			env := signRegistrationPayload(t, payload, priv)
			got, err := VerifyRegistration(env, ring)
			if err != nil || got.IssuedAt != valid {
				t.Fatalf("issuedAt %q: got %q, error %v", valid, got.IssuedAt, err)
			}
		})
	}
	for _, invalid := range []string{"", " 1770000000000", "1770000000000 ", "+1770000000000", "-1", "1.0", "2026-10-09T00:00:00Z", "18446744073709551616", "١"} {
		t.Run("reject_"+invalid, func(t *testing.T) {
			payload := strings.Replace(string(base), `"issuedAt":"1770000000000"`, `"issuedAt":"`+invalid+`"`, 1)
			if _, err := VerifyRegistration(signRegistrationPayload(t, payload, priv), ring); err == nil {
				t.Fatalf("invalid issuedAt %q accepted", invalid)
			}
		})
	}
}

func TestLocalRegistrationFixtureMatchesSignedRawPayload(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("..", "..", "fixtures", "local", "registration.json"))
	if err != nil {
		t.Fatal(err)
	}
	var envelope RegistrationEnvelope
	if err := json.Unmarshal(data, &envelope); err != nil {
		t.Fatal(err)
	}
	if envelope != fixtureRegistration() {
		t.Fatal("local registration fixture differs from the fixed raw-byte vector")
	}
	pub, _ := licenseFixtureKey()
	ring := map[string]ed25519.PublicKey{fixtureLicenseKeyID: pub}
	if _, err := VerifyRegistration(envelope, ring); err != nil {
		t.Fatalf("server rejected local signed registration fixture: %v", err)
	}
	checkData, err := os.ReadFile(filepath.Join("..", "..", "fixtures", "local", "check-request.json"))
	if err != nil {
		t.Fatal(err)
	}
	var request CheckRequest
	if err := json.Unmarshal(checkData, &request); err != nil {
		t.Fatal(err)
	}
	if request.Registration != envelope {
		t.Fatal("local check request has a different registration envelope")
	}
	if _, err := VerifyCheckRequest(request, ring); err != nil {
		t.Fatalf("server rejected local signed check request: %v", err)
	}
}

func TestRegistrationRejectsUnknownKeyIDDuplicateAndExtraFields(t *testing.T) {
	pub, priv := licenseFixtureKey()
	ring := map[string]ed25519.PublicKey{fixtureLicenseKeyID: pub}
	payload, _ := decodeStdBase64(fixturePayloadB64)
	unknownKey := strings.Replace(string(payload), fixtureLicenseKeyID, "license-unknown", 1)
	if _, err := VerifyRegistration(signRegistrationPayload(t, unknownKey, priv), ring); err == nil {
		t.Fatal("unknown licenseKeyId accepted")
	}
	duplicate := `{"schemaVersion":1,"schemaVersion":1,"domain":"codex-usb-revocation-registration-v1","product":"codex-usb-oneclick","installationId":"usb-install-001","licenseKeyId":"license-test-2026","licenseSignatureSha256":"` + fixtureLicenseHash + `","fingerprintSha256":"` + fixtureFingerprint + `","issuedAt":"1770000000000"}`
	if _, err := VerifyRegistration(signRegistrationPayload(t, duplicate, priv), ring); err == nil {
		t.Fatal("duplicate JSON key accepted")
	}
	extra := strings.TrimSuffix(string(payload), "}") + `,"accountBinding":{"email":"private@example.invalid","deviceApiKey":"secret"}}`
	if _, err := VerifyRegistration(signRegistrationPayload(t, extra, priv), ring); err == nil {
		t.Fatal("accountBinding/unknown field accepted")
	}
}

func TestActiveRevokeReactivateMonotonicVersionsAndIdempotence(t *testing.T) {
	store, _, _ := newTestStore(t, false)
	defer store.Close()
	ctx := context.Background()
	created := time.Date(2026, 10, 9, 0, 0, 0, 0, time.UTC)
	record, err := store.Import(ctx, fixtureRegistration(), "operator-a", created)
	if err != nil {
		t.Fatal(err)
	}
	assertState(t, record, "active", 1)
	_, licensePriv := licenseFixtureKey()
	changedPayload, _ := decodeStdBase64(fixturePayloadB64)
	conflicting := strings.Replace(string(changedPayload), fixtureLicenseHash, strings.Repeat("b", 64), 1)
	conflictProof := signRegistrationPayload(t, conflicting, licensePriv)
	if _, err := store.Import(ctx, conflictProof, "operator-a", created.Add(30*time.Second)); !errors.Is(err, errAlreadyExists) {
		t.Fatalf("conflicting duplicate import result=%v", err)
	}
	record, changed, err := store.ChangeState(ctx, fixtureInstallation, "revoke", "operator-a", "case 123", created.Add(time.Minute))
	if err != nil || !changed {
		t.Fatalf("revoke failed: changed=%v err=%v", changed, err)
	}
	assertState(t, record, "revoked", 2)
	if record.RevokedAt == nil || record.RevokeReason != "case 123" {
		t.Fatalf("missing revoke metadata: %+v", record)
	}
	record, changed, err = store.ChangeState(ctx, fixtureInstallation, "revoke", "operator-a", "duplicate", created.Add(2*time.Minute))
	if err != nil || changed {
		t.Fatalf("repeated revoke not idempotent: changed=%v err=%v", changed, err)
	}
	assertState(t, record, "revoked", 2)
	record, changed, err = store.ChangeState(ctx, fixtureInstallation, "reactivate", "operator-a", "appeal approved", created.Add(3*time.Minute))
	if err != nil || !changed {
		t.Fatalf("reactivate failed: changed=%v err=%v", changed, err)
	}
	assertState(t, record, "active", 3)
	if record.RevokedAt == nil || record.RevokeReason != "case 123" {
		t.Fatal("reactivation lost latest revoke metadata")
	}
	if _, _, err := store.ChangeState(ctx, fixtureInstallation, "reactivate", "operator-a", "again", created.Add(4*time.Minute)); err != nil {
		t.Fatalf("repeated reactivate failed: %v", err)
	}
	events, err := validateJournals(store.config.AuditPath, store.config.AuditBackupPath)
	if err != nil || len(events) != 3 {
		t.Fatalf("journal events=%d err=%v", len(events), err)
	}
}

func TestCheckSignsExactNonceAndRequiredPayloadFields(t *testing.T) {
	store, keys, signer := newTestStore(t, false)
	defer store.Close()
	_, err := store.Import(context.Background(), fixtureRegistration(), "operator-a", time.Date(2026, 10, 8, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	req := fixtureCheckRequest()
	now := time.Date(2026, 10, 9, 0, 0, 0, 0, time.UTC)
	envelope, rec, err := store.LookupForCheck(context.Background(), req, keys, now)
	if err != nil {
		t.Fatal(err)
	}
	if rec.Status != "active" || rec.StateVersion != 1 {
		t.Fatalf("unexpected record: %+v", rec)
	}
	raw, err := decodeStdBase64(envelope.PayloadB64)
	if err != nil {
		t.Fatal(err)
	}
	want := `{"schemaVersion":1,"product":"codex-usb-oneclick","installationId":"usb-install-001","licenseSignatureSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","requestId":"00112233445566778899aabbccddeeff","status":"active","stateVersion":1,"issuedAt":"2026-10-09T00:00:00Z","signingKeyId":"revocation-test-2026"}`
	if string(raw) != want {
		t.Fatalf("signed payload bytes\n got: %s\nwant: %s", raw, want)
	}
	sig, err := decodeStdBase64(envelope.SignatureB64)
	if err != nil || !ed25519.Verify(ed25519.PrivateKey(signer.Key).Public().(ed25519.PublicKey), raw, sig) {
		t.Fatalf("response signature failed: %v", err)
	}
	if envelope.SignatureB64 != "xpUQ2sKA/5I6GCh76LTmoUYnMnVia7tfQEtTbkQsmhTuiK+HFRW6hYUB6PdzS5d9IY63rPa3RioJxvkwBn71CQ==" {
		t.Fatalf("response test vector changed: %s", envelope.SignatureB64)
	}
	var decoded ResponsePayload
	if err := json.Unmarshal(raw, &decoded); err != nil {
		t.Fatal(err)
	}
	if decoded.RequestID != req.RequestID || decoded.InstallationID != req.InstallationID || decoded.LicenseSignatureSHA256 != req.LicenseSignatureSHA256 || decoded.SigningKeyID != fixtureStatusKeyID {
		t.Fatalf("response identity mismatch: %+v", decoded)
	}
	bad := req
	bad.RequestID = strings.ToUpper(bad.RequestID)
	if _, _, err := store.LookupForCheck(context.Background(), bad, keys, now); !errors.Is(err, errInvalidRegistration) {
		t.Fatalf("invalid request nonce accepted: %v", err)
	}
	bad = req
	bad.LicenseSignatureSHA256 = strings.Repeat("b", 64)
	if _, _, err := store.LookupForCheck(context.Background(), bad, keys, now); !errors.Is(err, errInvalidRegistration) {
		t.Fatalf("license signature digest mismatch accepted: %v", err)
	}
	bad = req
	bad.ObservedFingerprint.VolumeSerial++
	if _, _, err := store.LookupForCheck(context.Background(), bad, keys, now); !errors.Is(err, errInvalidRegistration) {
		t.Fatalf("observed fingerprint mismatch accepted: %v", err)
	}
	bad = req
	bad.Registration.SignatureB64 = fixtureSignatureB64[:len(fixtureSignatureB64)-4] + "AAAA"
	if _, _, err := store.LookupForCheck(context.Background(), bad, keys, now); !errors.Is(err, errInvalidRegistration) {
		t.Fatalf("invalid registration signature accepted: %v", err)
	}
}

func TestUnknownRecordAndRestoreMarkerNeverSign(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()
	if _, _, err := store.LookupForCheck(context.Background(), fixtureCheckRequest(), keys, time.Now()); !errors.Is(err, errNotFound) {
		t.Fatalf("unknown record result=%v", err)
	}
	server, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "127.0.0.1:8787"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := json.Marshal(fixtureCheckRequest())
	request := httptest.NewRequest(http.MethodPost, "/v1/licenses/check", bytes.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	server.Handler.ServeHTTP(response, request)
	if response.Code != http.StatusServiceUnavailable || strings.Contains(response.Body.String(), "payloadB64") {
		t.Fatalf("unknown public record response=%d %s", response.Code, response.Body.String())
	}
	_ = os.WriteFile(store.config.RestoreMarkerPath, []byte("restore in progress\n"), 0600)
	if _, _, err := store.LookupForCheck(context.Background(), fixtureCheckRequest(), keys, time.Now()); !errors.Is(err, errStoreUnknown) {
		t.Fatalf("restore marker allowed signed state: %v", err)
	}
}

func TestMalformedAndOversizedRequestsAreRejected(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()
	server, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "127.0.0.1:8787", RateBurst: 100}, nil)
	if err != nil {
		t.Fatal(err)
	}
	malformed := httptest.NewRequest(http.MethodPost, "/v1/licenses/check", strings.NewReader(`{"schemaVersion":1,"schemaVersion":1}`))
	malformed.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	server.Handler.ServeHTTP(response, malformed)
	if response.Code != http.StatusBadRequest {
		t.Fatalf("duplicate keys status=%d", response.Code)
	}
	large := httptest.NewRequest(http.MethodPost, "/v1/licenses/check", strings.NewReader(strings.Repeat("x", MaxRequestBytes+1)))
	large.Header.Set("Content-Type", "application/json")
	response = httptest.NewRecorder()
	server.Handler.ServeHTTP(response, large)
	if response.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("oversized status=%d", response.Code)
	}
	wrongType := httptest.NewRequest(http.MethodPost, "/v1/licenses/check", strings.NewReader("{}"))
	wrongType.Header.Set("Content-Type", "text/plain")
	response = httptest.NewRecorder()
	server.Handler.ServeHTTP(response, wrongType)
	if response.Code != http.StatusUnsupportedMediaType {
		t.Fatalf("content type status=%d", response.Code)
	}
	if _, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "0.0.0.0:8787"}, nil); err == nil {
		t.Fatal("non-loopback listener was accepted")
	}
}

func TestPublicRegistrationEndpointActivatesIdempotentlyAndNeverUnrevokes(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()
	server, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "127.0.0.1:8787"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	post := func(envelope RegistrationEnvelope) *httptest.ResponseRecorder {
		t.Helper()
		body, err := json.Marshal(RegistrationUploadRequest{Registration: envelope})
		if err != nil {
			t.Fatal(err)
		}
		request := httptest.NewRequest(http.MethodPost, "/v1/licenses/register", bytes.NewReader(body))
		request.Header.Set("Content-Type", "application/json")
		response := httptest.NewRecorder()
		server.Handler.ServeHTTP(response, request)
		return response
	}

	created := post(fixtureRegistration())
	if created.Code != http.StatusCreated {
		t.Fatalf("first registration status=%d body=%s", created.Code, created.Body.String())
	}
	var result struct {
		Status         string `json:"status"`
		InstallationID string `json:"installationId"`
		StateVersion   int64  `json:"stateVersion"`
		Created        bool   `json:"created"`
	}
	if err := json.Unmarshal(created.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if result.Status != "active" || result.InstallationID != fixtureInstallation || result.StateVersion != 1 || !result.Created {
		t.Fatalf("unexpected activation response: %+v", result)
	}

	retried := post(fixtureRegistration())
	if retried.Code != http.StatusOK {
		t.Fatalf("idempotent retry status=%d body=%s", retried.Code, retried.Body.String())
	}
	if err := json.Unmarshal(retried.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if result.Status != "active" || result.StateVersion != 1 || result.Created {
		t.Fatalf("retry changed the record: %+v", result)
	}

	if _, changed, err := store.ChangeState(context.Background(), fixtureInstallation, "revoke", "operator-a", "test revoke", time.Now().UTC()); err != nil || !changed {
		t.Fatalf("revoke before retry failed: changed=%v err=%v", changed, err)
	}
	revokedRetry := post(fixtureRegistration())
	if revokedRetry.Code != http.StatusConflict || !strings.Contains(revokedRetry.Body.String(), `"license_revoked"`) {
		t.Fatalf("revoked retry status=%d body=%s", revokedRetry.Code, revokedRetry.Body.String())
	}
	record, err := store.Inspect(context.Background(), fixtureInstallation)
	if err != nil {
		t.Fatal(err)
	}
	assertState(t, record, "revoked", 2)
}

func TestPublicRegistrationRejectsInvalidAndConflictingProofs(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()
	server, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "127.0.0.1:8787", RateBurst: 10}, nil)
	if err != nil {
		t.Fatal(err)
	}
	post := func(envelope RegistrationEnvelope) *httptest.ResponseRecorder {
		t.Helper()
		body, _ := json.Marshal(RegistrationUploadRequest{Registration: envelope})
		request := httptest.NewRequest(http.MethodPost, "/v1/licenses/register", bytes.NewReader(body))
		request.Header.Set("Content-Type", "application/json")
		response := httptest.NewRecorder()
		server.Handler.ServeHTTP(response, request)
		return response
	}
	if response := post(RegistrationEnvelope{PayloadB64: fixturePayloadB64, SignatureB64: strings.Repeat("A", 88)}); response.Code != http.StatusBadRequest {
		t.Fatalf("invalid proof status=%d body=%s", response.Code, response.Body.String())
	}
	if response := post(fixtureRegistration()); response.Code != http.StatusCreated {
		t.Fatalf("initial registration status=%d body=%s", response.Code, response.Body.String())
	}
	_, private := licenseFixtureKey()
	payload, _ := decodeStdBase64(fixturePayloadB64)
	conflictingPayload := strings.Replace(string(payload), fixtureLicenseHash, strings.Repeat("b", 64), 1)
	if response := post(signRegistrationPayload(t, conflictingPayload, private)); response.Code != http.StatusConflict || !strings.Contains(response.Body.String(), `"registration_conflict"`) {
		t.Fatalf("conflicting proof status=%d body=%s", response.Code, response.Body.String())
	}
}

func TestFailedDatabaseTransactionCannotSignUntilJournalReplay(t *testing.T) {
	store, keys, signer := newTestStore(t, false)
	if _, err := store.db.Exec(`CREATE TRIGGER fail_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT, 'blocked'); END`); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Import(context.Background(), fixtureRegistration(), "operator-a", time.Now().UTC()); err == nil {
		t.Fatal("triggered database transaction succeeded")
	}
	if store.IsHealthy() {
		t.Fatal("failed transaction left store healthy")
	}
	if got := countRows(t, store.db, "licenses"); got != 0 {
		t.Fatalf("failed transaction persisted license row: %d", got)
	}
	store.Close()
	db, err := sql.Open("sqlite", store.config.DatabasePath)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`DROP TRIGGER fail_audit`); err != nil {
		t.Fatal(err)
	}
	db.Close()
	reopened, _, _ := openTestStore(t, store.config, keys, signer)
	defer reopened.Close()
	if !reopened.IsHealthy() {
		t.Fatal("complete journal did not recover failed transaction")
	}
	record, err := reopened.Inspect(context.Background(), fixtureInstallation)
	if err != nil {
		t.Fatal(err)
	}
	assertState(t, record, "active", 1)
}

func TestRestoredOldDatabaseReplaysNewerRevocationAudit(t *testing.T) {
	store, keys, signer := newTestStore(t, false)
	created := time.Date(2026, 10, 9, 0, 0, 0, 0, time.UTC)
	if _, err := store.Import(context.Background(), fixtureRegistration(), "operator-a", created); err != nil {
		t.Fatal(err)
	}
	backupDir := filepath.Join(filepath.Dir(store.config.DatabasePath), "backup-snapshot")
	if err := store.Backup(context.Background(), backupDir); err != nil {
		t.Fatal(err)
	}
	oldDatabase, err := os.ReadFile(filepath.Join(backupDir, "state.db"))
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := store.ChangeState(context.Background(), fixtureInstallation, "revoke", "operator-a", "case 123", created.Add(time.Minute)); err != nil {
		t.Fatal(err)
	}
	config := store.config
	store.Close()
	_ = os.Remove(config.DatabasePath + "-wal")
	_ = os.Remove(config.DatabasePath + "-shm")
	if err := os.WriteFile(config.DatabasePath, oldDatabase, 0600); err != nil {
		t.Fatal(err)
	}
	reopened, _, err := openTestStore(t, config, keys, signer)
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	if !reopened.IsHealthy() {
		t.Fatal("stale snapshot did not reconcile from current journals")
	}
	record, err := reopened.Inspect(context.Background(), fixtureInstallation)
	if err != nil {
		t.Fatal(err)
	}
	assertState(t, record, "revoked", 2)
	if record.RevokedAt == nil {
		t.Fatal("replayed revocation missing time")
	}
}

func TestMarkedRecoveryReplaysButDoesNotEnableSigning(t *testing.T) {
	store, keys, signer := newTestStore(t, false)
	created := time.Date(2026, 10, 9, 0, 0, 0, 0, time.UTC)
	if _, err := store.Import(context.Background(), fixtureRegistration(), "operator-a", created); err != nil {
		t.Fatal(err)
	}
	backupDir := filepath.Join(filepath.Dir(store.config.DatabasePath), "manual-recovery-backup")
	if err := store.Backup(context.Background(), backupDir); err != nil {
		t.Fatal(err)
	}
	oldDatabase, err := os.ReadFile(filepath.Join(backupDir, "state.db"))
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := store.ChangeState(context.Background(), fixtureInstallation, "revoke", "operator-a", "case 123", created.Add(time.Minute)); err != nil {
		t.Fatal(err)
	}
	config := store.config
	store.Close()
	_ = os.Remove(config.DatabasePath + "-wal")
	_ = os.Remove(config.DatabasePath + "-shm")
	if err := os.WriteFile(config.DatabasePath, oldDatabase, 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(config.RestoreMarkerPath, []byte("restore in progress\n"), 0600); err != nil {
		t.Fatal(err)
	}
	recovering, _, err := openTestStore(t, config, keys, signer)
	if err != nil {
		t.Fatal(err)
	}
	if recovering.IsHealthy() {
		t.Fatal("restore marker did not block signing")
	}
	if err := recovering.Recover(context.Background()); err != nil {
		t.Fatalf("recovery failed: %v", err)
	}
	if err := recovering.VerifyRecovery(context.Background()); err != nil {
		t.Fatalf("verified recovery failed: %v", err)
	}
	if _, _, err := recovering.LookupForCheck(context.Background(), fixtureCheckRequest(), keys, time.Now()); !errors.Is(err, errStoreUnknown) {
		t.Fatalf("marker allowed response before operator review: %v", err)
	}
	recovering.Close()
	if err := os.Remove(config.RestoreMarkerPath); err != nil {
		t.Fatal(err)
	}
	ready, _, err := openTestStore(t, config, keys, signer)
	if err != nil {
		t.Fatal(err)
	}
	defer ready.Close()
	if !ready.IsHealthy() {
		t.Fatal("verified recovery did not become healthy after marker removal")
	}
	record, err := ready.Inspect(context.Background(), fixtureInstallation)
	if err != nil {
		t.Fatal(err)
	}
	assertState(t, record, "revoked", 2)
}

func TestJournalMirrorDivergenceDisablesSigning(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	if _, err := store.Import(context.Background(), fixtureRegistration(), "operator-a", time.Now().UTC()); err != nil {
		t.Fatal(err)
	}
	backupPath := store.config.AuditBackupPath
	store.Close()
	if err := os.WriteFile(backupPath, nil, 0600); err != nil {
		t.Fatal(err)
	}
	reopened, _, err := openTestStore(t, store.config, keys, testStatusSigner())
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	if reopened.IsHealthy() {
		t.Fatal("divergent journals did not fail closed")
	}
	if _, _, err := reopened.LookupForCheck(context.Background(), fixtureCheckRequest(), keys, time.Now()); !errors.Is(err, errStoreUnknown) {
		t.Fatalf("divergent journals signed state: %v", err)
	}
}

func TestNoRegistrationSecretPersistenceOrLogging(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()
	if _, err := store.Import(context.Background(), fixtureRegistration(), "operator-a", time.Now().UTC()); err != nil {
		t.Fatal(err)
	}
	var logs bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logs, nil))
	server, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "127.0.0.1:8787", RateBurst: 10}, logger)
	if err != nil {
		t.Fatal(err)
	}
	body, err := json.Marshal(fixtureCheckRequest())
	if err != nil {
		t.Fatal(err)
	}
	req := httptest.NewRequest(http.MethodPost, "/v1/licenses/check", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	server.Handler.ServeHTTP(response, req)
	if response.Code != http.StatusOK {
		t.Fatalf("valid request status=%d body=%s", response.Code, response.Body.String())
	}
	if strings.Contains(logs.String(), fixturePayloadB64) || strings.Contains(logs.String(), fixtureSignatureB64) || strings.Contains(logs.String(), "payloadB64") || strings.Contains(logs.String(), fixtureFingerprint) {
		t.Fatalf("sensitive registration material in logs: %s", logs.String())
	}
	persisted := append([]byte(nil), logs.Bytes()...)
	for _, path := range []string{store.config.DatabasePath, store.config.DatabasePath + "-wal", store.config.AuditPath, store.config.AuditBackupPath} {
		if raw, err := os.ReadFile(path); err == nil {
			persisted = append(persisted, raw...)
		}
	}
	if bytes.Contains(persisted, []byte(fixtureSignatureB64)) || bytes.Contains(persisted, []byte(fixturePayloadB64)) || bytes.Contains(persisted, []byte("private@example.invalid")) || bytes.Contains(persisted, []byte("deviceApiKey")) {
		t.Fatal("registration secret material persisted")
	}
	if strings.Contains(logs.String(), fixtureRequestID) == false {
		t.Fatal("expected request ID log field absent")
	}
}

func TestBackupContainsDatabaseAuditAndManifest(t *testing.T) {
	store, _, _ := newTestStore(t, false)
	defer store.Close()
	if _, err := store.Import(context.Background(), fixtureRegistration(), "operator-a", time.Now().UTC()); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(filepath.Dir(store.config.DatabasePath), "independent-snapshot")
	if err := store.Backup(context.Background(), out); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"state.db", "audit.jsonl", "manifest.json"} {
		if _, err := os.Stat(filepath.Join(out, name)); err != nil {
			t.Fatalf("backup missing %s: %v", name, err)
		}
	}
}

func TestHealthzDoesNotExposeStateAndOtherRoutesAreNotAvailable(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()
	server, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "127.0.0.1:8787"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		method, path string
		want         int
	}{{http.MethodGet, "/healthz", 200}, {http.MethodGet, "/v1/admin", 404}, {http.MethodGet, "/v1/licenses/check", 405}} {
		w := httptest.NewRecorder()
		server.Handler.ServeHTTP(w, httptest.NewRequest(tc.method, tc.path, nil))
		if w.Code != tc.want {
			t.Errorf("%s %s: got %d want %d", tc.method, tc.path, w.Code, tc.want)
		}
		if tc.path == "/healthz" && (strings.Contains(w.Body.String(), "database") || strings.Contains(w.Body.String(), "key")) {
			t.Fatalf("health leaked internals: %s", w.Body.String())
		}
	}
}

func newTestStore(t *testing.T, withMarker bool) (*Store, map[string]ed25519.PublicKey, StatusSigningKey) {
	t.Helper()
	root := t.TempDir()
	cfg := Config{
		DatabasePath: filepath.Join(root, "state.db"), AuditPath: filepath.Join(root, "audit", "audit.jsonl"),
		AuditBackupPath: filepath.Join(root, "independent", "audit.jsonl"), RestoreMarkerPath: filepath.Join(root, "RESTORE-IN-PROGRESS"),
		LicenseKeysPath: filepath.Join(root, "license-keyring.json"), SigningKeyID: fixtureStatusKeyID,
	}
	if withMarker {
		if err := os.WriteFile(cfg.RestoreMarkerPath, []byte("marker\n"), 0600); err != nil {
			t.Fatal(err)
		}
	}
	keys, signer := testKeyrings()
	store, err := OpenStore(context.Background(), cfg, keys, signer)
	if err != nil {
		t.Fatal(err)
	}
	return store, keys, signer
}

func TestOpenStoreRejectsSQLiteURIAndMemoryDatabase(t *testing.T) {
	store, keys, signer := newTestStore(t, false)
	cfg := store.config
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}

	for _, databasePath := range []string{
		"file:" + filepath.ToSlash(cfg.DatabasePath) + "?mode=rwc",
		":memory:",
	} {
		cfg.DatabasePath = databasePath
		if opened, err := OpenStore(context.Background(), cfg, keys, signer); err == nil {
			opened.Close()
			t.Errorf("SQLite DSN %q was accepted; want a plain filesystem path only", databasePath)
		}
	}
}

func openTestStore(t *testing.T, cfg Config, keys map[string]ed25519.PublicKey, signer StatusSigningKey) (*Store, map[string]ed25519.PublicKey, error) {
	t.Helper()
	store, err := OpenStore(context.Background(), cfg, keys, signer)
	return store, keys, err
}

func testKeyrings() (map[string]ed25519.PublicKey, StatusSigningKey) {
	pub, _ := licenseFixtureKey()
	status := testStatusSigner()
	return map[string]ed25519.PublicKey{fixtureLicenseKeyID: pub}, status
}

func licenseFixtureKey() (ed25519.PublicKey, ed25519.PrivateKey) {
	seed := bytes.Repeat([]byte{0x11}, ed25519.SeedSize)
	priv := ed25519.NewKeyFromSeed(seed)
	return priv.Public().(ed25519.PublicKey), priv
}

func testStatusSigner() StatusSigningKey {
	priv := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{0x22}, ed25519.SeedSize))
	return StatusSigningKey{KeyID: fixtureStatusKeyID, Key: priv}
}

func fixtureRegistration() RegistrationEnvelope {
	return RegistrationEnvelope{PayloadB64: fixturePayloadB64, SignatureB64: fixtureSignatureB64}
}

func fixtureCheckRequest() CheckRequest {
	return CheckRequest{
		SchemaVersion: 1, RequestID: fixtureRequestID, Product: ProductID, InstallationID: fixtureInstallation,
		LicenseSignatureSHA256: fixtureLicenseHash, Registration: fixtureRegistration(),
		ObservedFingerprint: ObservedFingerprint{VolumeSerial: 0x12345678, AnchorSHA256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", BundleRelativePath: `CodexUsb\Bundle`, AnchorSchema: 3},
	}
}

const fixtureRequestID = "00112233445566778899aabbccddeeff"

func signRegistrationPayload(t *testing.T, payload string, priv ed25519.PrivateKey) RegistrationEnvelope {
	t.Helper()
	raw := []byte(payload)
	return RegistrationEnvelope{PayloadB64: base64.StdEncoding.EncodeToString(raw), SignatureB64: base64.StdEncoding.EncodeToString(ed25519.Sign(priv, raw))}
}

func assertState(t *testing.T, record LicenseRecord, status string, version int64) {
	t.Helper()
	if record.Status != status || record.StateVersion != version {
		t.Fatalf("got status=%s version=%d, want %s/%d", record.Status, record.StateVersion, status, version)
	}
}

func countRows(t *testing.T, db *sql.DB, table string) int {
	t.Helper()
	var n int
	if err := db.QueryRow("SELECT COUNT(*) FROM " + table).Scan(&n); err != nil {
		t.Fatal(err)
	}
	return n
}
