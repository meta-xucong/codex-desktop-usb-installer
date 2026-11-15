package revocation

import (
	"bytes"
	"context"
	"encoding/json"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestConflictingCheckDoesNotDisableOtherLicenses(t *testing.T) {
	for _, field := range []string{"license_signature", "fingerprint", "key_id"} {
		t.Run(field, func(t *testing.T) {
			store, keys, _ := newTestStore(t, false)
			defer store.Close()
			ctx := context.Background()
			now := time.Now().UTC()
			if _, _, err := store.Register(ctx, fixtureRegistration(), now); err != nil {
				t.Fatal(err)
			}
			pub, private := licenseFixtureKey()
			raw, _ := decodeStdBase64(fixturePayloadB64)
			other := fixtureCheckRequest()
			other.InstallationID = "usb-install-other"
			other.Registration = signRegistrationPayload(t, strings.Replace(string(raw), fixtureInstallation, other.InstallationID, 1), private)
			if _, _, err := store.Register(ctx, other.Registration, now); err != nil {
				t.Fatal(err)
			}
			if _, _, err := store.ChangeState(ctx, other.InstallationID, "revoke", "operator-a", "test revoke", now.Add(time.Second)); err != nil {
				t.Fatal(err)
			}
			before, err := store.loadAllRecords(ctx)
			if err != nil {
				t.Fatal(err)
			}
			beforeAudit := countRows(t, store.db, "audit_events")
			bad := fixtureCheckRequest()
			payload := string(raw)
			switch field {
			case "license_signature":
				bad.LicenseSignatureSHA256 = strings.Repeat("b", 64)
				payload = strings.Replace(payload, fixtureLicenseHash, bad.LicenseSignatureSHA256, 1)
			case "fingerprint":
				bad.ObservedFingerprint.VolumeSerial++
				fp := bad.ObservedFingerprint
				digest, err := FingerprintDigest(fp.AnchorSchema, fp.VolumeSerial, fp.AnchorSHA256, fp.BundleRelativePath)
				if err != nil {
					t.Fatal(err)
				}
				payload = strings.Replace(payload, fixtureFingerprint, digest, 1)
			case "key_id":
				keys["license-other-2026"] = pub
				payload = strings.Replace(payload, fixtureLicenseKeyID, "license-other-2026", 1)
			}
			bad.Registration = signRegistrationPayload(t, payload, private)
			if _, err := VerifyCheckRequest(bad, keys); err != nil {
				t.Fatalf("fixture must have a valid signed proof: %v", err)
			}
			var logs bytes.Buffer
			server, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "127.0.0.1:8787", RateBurst: 100}, slog.New(slog.NewJSONHandler(&logs, nil)))
			if err != nil {
				t.Fatal(err)
			}
			body, _ := json.Marshal(bad)
			req := httptest.NewRequest(http.MethodPost, "/v1/licenses/check", bytes.NewReader(body))
			req.Header.Set("Content-Type", "application/json")
			response := httptest.NewRecorder()
			server.Handler.ServeHTTP(response, req)
			if response.Code != http.StatusConflict || strings.Contains(response.Body.String(), "payloadB64") {
				t.Errorf("conflict response=%d %s", response.Code, response.Body.String())
			}
			if !store.IsHealthy() {
				t.Fatal("one conflicting proof disabled the entire store")
			}
			for _, good := range []CheckRequest{fixtureCheckRequest(), other} {
				envelope, record, err := store.LookupForCheck(ctx, good, keys, now.Add(2*time.Second))
				if err != nil || envelope.SignatureB64 == "" {
					t.Fatalf("other checks blocked: %v", err)
				}
				want := before[good.InstallationID]
				assertState(t, record, want.Status, want.StateVersion)
			}
			after, err := store.loadAllRecords(ctx)
			if err != nil || !compareRecordMaps(before, after) || countRows(t, store.db, "audit_events") != beforeAudit {
				t.Fatal("conflicting check changed existing authorizations/audit")
			}
			newProof := signRegistrationPayload(t, strings.Replace(string(raw), fixtureInstallation, "usb-install-new", 1), private)
			if _, created, err := store.Register(ctx, newProof, now); err != nil || !created {
				t.Fatalf("new registration blocked: %v", err)
			}
			w := httptest.NewRecorder()
			server.Handler.ServeHTTP(w, httptest.NewRequest(http.MethodGet, "/healthz", nil))
			if w.Code != http.StatusOK {
				t.Fatalf("healthy store health=%d", w.Code)
			}
			if !strings.Contains(logs.String(), `"reason":"registration_mismatch"`) {
				t.Fatal("missing safe conflict diagnostic")
			}
			for _, secret := range []string{bad.Registration.PayloadB64, bad.Registration.SignatureB64, fixtureLicenseHash, fixtureFingerprint} {
				if strings.Contains(logs.String(), secret) {
					t.Fatal("diagnostic leaked proof/digest")
				}
			}
		})
	}
}

func TestHealthReportsUnknownAndLogsReason(t *testing.T) {
	for _, cause := range []string{"restore_marker", "database_divergence"} {
		t.Run(cause, func(t *testing.T) {
			store, keys, _ := newTestStore(t, cause == "restore_marker")
			defer store.Close()
			wantReason := "restore marker present"
			if cause == "database_divergence" {
				if _, _, err := store.Register(context.Background(), fixtureRegistration(), time.Now()); err != nil {
					t.Fatal(err)
				}
				if _, err := store.db.Exec(`UPDATE licenses SET state_version=9`); err != nil {
					t.Fatal(err)
				}
				if _, _, err := store.LookupForCheck(context.Background(), fixtureCheckRequest(), keys, time.Now()); err == nil {
					t.Fatal("divergent state was signed")
				}
				wantReason = "database and audit history disagree"
			}
			var logs bytes.Buffer
			server, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "127.0.0.1:8787"}, slog.New(slog.NewJSONHandler(&logs, nil)))
			if err != nil {
				t.Fatal(err)
			}
			w := httptest.NewRecorder()
			server.Handler.ServeHTTP(w, httptest.NewRequest(http.MethodGet, "/healthz", nil))
			if w.Code != http.StatusServiceUnavailable || w.Body.String() != `{"status":"unavailable"}` {
				t.Fatalf("unknown health=%d %s", w.Code, w.Body.String())
			}
			if !strings.Contains(logs.String(), wantReason) {
				t.Fatal("unknown reason not logged")
			}
			if strings.Contains(w.Body.String(), wantReason) {
				t.Fatal("public health leaked internal reason")
			}
			if _, _, err := store.Register(context.Background(), fixtureRegistration(), time.Now()); err == nil {
				t.Fatal("unknown store accepted registration")
			}
		})
	}
}

func TestUnregisteredCheckDiagnosticDoesNotMarkStoreUnknown(t *testing.T) {
	store, keys, _ := newTestStore(t, false)
	defer store.Close()
	var logs bytes.Buffer
	server, err := NewHTTPServer(store, keys, HTTPConfig{ListenAddress: "127.0.0.1:8787"}, slog.New(slog.NewJSONHandler(&logs, nil)))
	if err != nil {
		t.Fatal(err)
	}
	body, _ := json.Marshal(fixtureCheckRequest())
	req := httptest.NewRequest(http.MethodPost, "/v1/licenses/check", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	server.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusServiceUnavailable || strings.Contains(w.Body.String(), "payloadB64") {
		t.Fatalf("unregistered response=%d %s", w.Code, w.Body.String())
	}
	if !store.IsHealthy() || !strings.Contains(logs.String(), `"reason":"license_not_registered"`) {
		t.Fatal("missing license was confused with broken storage")
	}
	store.setUnknown("database query failed")
	logs.Reset()
	registration, _ := json.Marshal(RegistrationUploadRequest{Registration: fixtureRegistration()})
	req = httptest.NewRequest(http.MethodPost, "/v1/licenses/register", bytes.NewReader(registration))
	req.Header.Set("Content-Type", "application/json")
	w = httptest.NewRecorder()
	server.Handler.ServeHTTP(w, req)
	if w.Code != http.StatusServiceUnavailable || !strings.Contains(logs.String(), `"reason":"database query failed"`) {
		t.Fatal("registration failure did not retain safe store reason")
	}
	if countRows(t, store.db, "licenses") != 0 {
		t.Fatal("unavailable registration changed state")
	}
}
