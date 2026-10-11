package revocation

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	_ "embed"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	_ "modernc.org/sqlite"
)

var (
	errStoreUnknown  = errors.New("license state unknown")
	errAlreadyExists = errors.New("license already exists")
	errNotFound      = errors.New("license not found")
)

type Config struct {
	DatabasePath      string
	AuditPath         string
	AuditBackupPath   string
	RestoreMarkerPath string
	LicenseKeysPath   string
	SigningKeyPath    string
	SigningKeyID      string
	MaxRatePerMinute  int
	RateBurst         int
}

type Store struct {
	db            *sql.DB
	config        Config
	licenseKeys   map[string]ed25519.PublicKey
	signingKey    ed25519.PrivateKey
	unknownMu     sync.RWMutex
	unknown       bool
	unknownReason string
}

func OpenStore(ctx context.Context, cfg Config, keys map[string]ed25519.PublicKey, signer StatusSigningKey) (*Store, error) {
	if cfg.DatabasePath == "" || isSQLiteURIOrMemory(cfg.DatabasePath) || cfg.AuditPath == "" || cfg.AuditBackupPath == "" || cfg.RestoreMarkerPath == "" ||
		(len(signer.Key) != 0 && (len(signer.Key) != ed25519.PrivateKeySize || !keyIDPattern.MatchString(signer.KeyID))) {
		return nil, fmt.Errorf("invalid service configuration")
	}
	if err := ensurePathIsDistinct(storeStatePaths(cfg)...); err != nil {
		return nil, err
	}
	for _, dir := range []string{filepath.Dir(cfg.DatabasePath), filepath.Dir(cfg.AuditPath), filepath.Dir(cfg.AuditBackupPath)} {
		if err := os.MkdirAll(dir, 0750); err != nil {
			return nil, err
		}
	}
	db, err := sql.Open("sqlite", cfg.DatabasePath)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	db.SetConnMaxLifetime(0)
	s := &Store{db: db, config: cfg, licenseKeys: keys, signingKey: append(ed25519.PrivateKey(nil), signer.Key...)}
	if err := s.initialize(ctx); err != nil {
		db.Close()
		return nil, err
	}
	return s, nil
}

func isSQLiteURIOrMemory(path string) bool {
	path = strings.ToLower(strings.TrimSpace(path))
	return path == ":memory:" || strings.HasPrefix(path, "file:")
}

func (s *Store) Close() error { return s.db.Close() }

func storeStatePaths(cfg Config) []string {
	return []string{
		cfg.DatabasePath,
		cfg.DatabasePath + "-wal",
		cfg.DatabasePath + "-shm",
		cfg.DatabasePath + "-journal",
		cfg.AuditPath,
		cfg.AuditPath + ".lock",
		cfg.AuditBackupPath,
		cfg.RestoreMarkerPath,
	}
}

func (s *Store) initialize(ctx context.Context) error {
	lock, err := acquireFileLock(ctx, s.config.AuditPath+".lock", 5*time.Second)
	if err != nil {
		return err
	}
	defer lock()

	if _, err := os.Stat(s.config.RestoreMarkerPath); err == nil {
		s.setUnknown("restore marker present")
		return nil
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	if err := s.db.PingContext(ctx); err != nil {
		s.setUnknown("database unavailable")
		return nil
	}
	if _, err := s.db.ExecContext(ctx, "PRAGMA journal_mode=WAL"); err != nil {
		s.setUnknown("database unavailable")
		return nil
	}
	if _, err := s.db.ExecContext(ctx, "PRAGMA synchronous=FULL"); err != nil {
		s.setUnknown("database unavailable")
		return nil
	}
	if _, err := s.db.ExecContext(ctx, "PRAGMA busy_timeout=2500"); err != nil {
		s.setUnknown("database unavailable")
		return nil
	}
	if _, err := s.db.ExecContext(ctx, "PRAGMA foreign_keys=ON"); err != nil {
		s.setUnknown("database unavailable")
		return nil
	}
	for _, statement := range strings.Split(schemaSQL, ";") {
		if strings.TrimSpace(statement) == "" {
			continue
		}
		if _, err := s.db.ExecContext(ctx, statement); err != nil {
			s.setUnknown("database schema unavailable")
			return nil
		}
	}
	if err := s.ensureUSBAssetSchema(ctx); err != nil {
		s.setUnknown("database schema unavailable")
		return nil
	}
	events, err := validateJournals(s.config.AuditPath, s.config.AuditBackupPath)
	if err != nil {
		s.setUnknown("audit history unavailable")
		return nil
	}
	if err := s.reconcileFromJournal(ctx, events); err != nil {
		s.setUnknown("database and audit history disagree")
		return nil
	}
	s.setKnown()
	return nil
}

func (s *Store) ensureUSBAssetSchema(ctx context.Context) error {
	rows, err := s.db.QueryContext(ctx, `PRAGMA table_info(licenses)`)
	if err != nil {
		return err
	}
	hasUSBAssetID := false
	for rows.Next() {
		var cid, notNull, primaryKey int
		var name, columnType string
		var defaultValue sql.NullString
		if err := rows.Scan(&cid, &name, &columnType, &notNull, &defaultValue, &primaryKey); err != nil {
			rows.Close()
			return err
		}
		if name == "usb_asset_id" {
			hasUSBAssetID = true
		}
	}
	if err := rows.Err(); err != nil {
		rows.Close()
		return err
	}
	if err := rows.Close(); err != nil {
		return err
	}
	if !hasUSBAssetID {
		if _, err := s.db.ExecContext(ctx, `ALTER TABLE licenses ADD COLUMN usb_asset_id TEXT NOT NULL DEFAULT ''`); err != nil {
			return err
		}
	}
	var auditSchema string
	if err := s.db.QueryRowContext(ctx, `SELECT sql FROM sqlite_master WHERE type='table' AND name='audit_events'`).Scan(&auditSchema); err != nil {
		return err
	}
	if strings.Contains(auditSchema, "'asset_bind'") {
		return nil
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	rollback := func(cause error) error {
		_ = tx.Rollback()
		return cause
	}
	statements := []string{
		`CREATE TABLE audit_events_v2 (
			sequence INTEGER NOT NULL PRIMARY KEY,
			event_id TEXT NOT NULL UNIQUE,
			installation_id TEXT NOT NULL,
			actor TEXT NOT NULL,
			action TEXT NOT NULL CHECK(action IN ('import','asset_bind','revoke','reactivate')),
			old_status TEXT,
			new_status TEXT NOT NULL,
			old_version INTEGER,
			new_version INTEGER NOT NULL,
			reason TEXT NOT NULL DEFAULT '',
			occurred_at TEXT NOT NULL,
			previous_hash TEXT NOT NULL,
			event_hash TEXT NOT NULL UNIQUE
		)`,
		`INSERT INTO audit_events_v2(sequence,event_id,installation_id,actor,action,old_status,new_status,old_version,new_version,reason,occurred_at,previous_hash,event_hash)
			SELECT sequence,event_id,installation_id,actor,action,old_status,new_status,old_version,new_version,reason,occurred_at,previous_hash,event_hash FROM audit_events`,
		`DROP TABLE audit_events`,
		`ALTER TABLE audit_events_v2 RENAME TO audit_events`,
	}
	for _, statement := range statements {
		if _, err := tx.ExecContext(ctx, statement); err != nil {
			return rollback(err)
		}
	}
	return tx.Commit()
}

//go:embed migrations/001_init.sql
var schemaSQL string

func (s *Store) reconcileFromJournal(ctx context.Context, events []AuditEvent) error {
	expected, err := recordsFromEvents(events)
	if err != nil {
		return err
	}
	actual, err := s.loadAllRecords(ctx)
	if err != nil {
		return err
	}
	actualAudit, err := s.loadAuditEvents(ctx)
	if err != nil {
		return err
	}
	if compareRecordMaps(actual, expected) && compareAuditEvents(actualAudit, events) {
		return nil
	}
	// A database restored from an older snapshot is safe to catch up only when
	// it exactly matches an audited prefix. Divergent or ahead state stays unknown.
	for prefix := 0; prefix <= len(events); prefix++ {
		prefixState, err := recordsFromEvents(events[:prefix])
		if err != nil {
			return err
		}
		if compareRecordMaps(actual, prefixState) && compareAuditEvents(actualAudit, events[:prefix]) {
			if err := s.replayEvents(ctx, events[prefix:]); err != nil {
				return err
			}
			checkRecords, err := s.loadAllRecords(ctx)
			if err != nil {
				return err
			}
			checkAudit, err := s.loadAuditEvents(ctx)
			if err != nil {
				return err
			}
			if !compareRecordMaps(checkRecords, expected) || !compareAuditEvents(checkAudit, events) {
				return errStoreUnknown
			}
			return nil
		}
	}
	if len(events) == 0 && len(actual) == 0 {
		return nil
	}
	return errStoreUnknown
}

func (s *Store) replayEvents(ctx context.Context, events []AuditEvent) error {
	for _, ev := range events {
		tx, err := s.db.BeginTx(ctx, nil)
		if err != nil {
			return err
		}
		if err := writeRecord(ctx, tx, ev.Record); err != nil {
			tx.Rollback()
			return err
		}
		if err := writeAuditRow(ctx, tx, ev, eventOldState(ev)); err != nil {
			tx.Rollback()
			return err
		}
		if err := tx.Commit(); err != nil {
			return err
		}
	}
	return nil
}

func eventOldState(ev AuditEvent) *LicenseRecord {
	if ev.Action == "import" {
		return nil
	}
	old := cloneRecord(ev.Record)
	if ev.Action == "asset_bind" {
		old.USBAssetID = ""
		return &old
	}
	old.StateVersion--
	if ev.Action == "revoke" {
		old.Status = "active"
		old.RevokedAt = nil
		old.RevokeReason = ""
	} else {
		old.Status = "revoked"
	}
	return &old
}

func (s *Store) loadAllRecords(ctx context.Context) (map[string]LicenseRecord, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT product, installation_id, key_id, license_signature_sha256, fingerprint_digest, usb_asset_id, status, state_version, created_at, updated_at, revoked_at, revoke_reason FROM licenses`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := map[string]LicenseRecord{}
	for rows.Next() {
		r, err := scanRecord(rows)
		if err != nil {
			return nil, err
		}
		result[r.InstallationID] = r
	}
	return result, rows.Err()
}

func (s *Store) loadAuditEvents(ctx context.Context) ([]auditRow, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT sequence,event_id,installation_id,actor,action,new_status,new_version,reason,occurred_at,previous_hash,event_hash FROM audit_events ORDER BY sequence`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var result []auditRow
	for rows.Next() {
		var r auditRow
		var occurred string
		if err := rows.Scan(&r.Sequence, &r.EventID, &r.InstallationID, &r.Actor, &r.Action, &r.Status, &r.Version, &r.Reason, &occurred, &r.PreviousHash, &r.Hash); err != nil {
			return nil, err
		}
		r.OccurredAt, err = time.Parse(time.RFC3339Nano, occurred)
		if err != nil {
			return nil, err
		}
		result = append(result, r)
	}
	return result, rows.Err()
}

type auditRow struct {
	Sequence       uint64
	EventID        string
	InstallationID string
	Actor          string
	Action         string
	Status         string
	Version        int64
	Reason         string
	OccurredAt     time.Time
	PreviousHash   string
	Hash           string
}

func compareAuditEvents(actual []auditRow, expected []AuditEvent) bool {
	if len(actual) != len(expected) {
		return false
	}
	for i, got := range actual {
		want := expected[i]
		if got.Sequence != want.Sequence || got.EventID != want.EventID || got.InstallationID != want.Record.InstallationID ||
			got.Actor != want.Actor || got.Action != want.Action || got.Status != want.Record.Status || got.Version != want.Record.StateVersion ||
			got.Reason != want.Reason || !got.OccurredAt.Equal(want.OccurredAt) || got.PreviousHash != want.PreviousHash || got.Hash != want.Hash {
			return false
		}
	}
	return true
}

type rowScanner interface{ Scan(...any) error }

func scanRecord(scanner rowScanner) (LicenseRecord, error) {
	var r LicenseRecord
	var created, updated string
	var revoked sql.NullString
	if err := scanner.Scan(&r.Product, &r.InstallationID, &r.KeyID, &r.LicenseSignatureSHA256, &r.FingerprintDigest, &r.USBAssetID, &r.Status, &r.StateVersion, &created, &updated, &revoked, &r.RevokeReason); err != nil {
		return r, err
	}
	var err error
	r.CreatedAt, err = time.Parse(time.RFC3339Nano, created)
	if err != nil {
		return r, err
	}
	r.UpdatedAt, err = time.Parse(time.RFC3339Nano, updated)
	if err != nil {
		return r, err
	}
	if revoked.Valid {
		t, err := time.Parse(time.RFC3339Nano, revoked.String)
		if err != nil {
			return r, err
		}
		r.RevokedAt = &t
	}
	return r, nil
}

func writeRecord(ctx context.Context, tx *sql.Tx, r LicenseRecord) error {
	var revoked any
	if r.RevokedAt != nil {
		revoked = r.RevokedAt.UTC().Format(time.RFC3339Nano)
	}
	_, err := tx.ExecContext(ctx, `INSERT INTO licenses(product, installation_id, key_id, license_signature_sha256, fingerprint_digest, usb_asset_id, status, state_version, created_at, updated_at, revoked_at, revoke_reason)
VALUES(?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(product, installation_id) DO UPDATE SET key_id=excluded.key_id, license_signature_sha256=excluded.license_signature_sha256, fingerprint_digest=excluded.fingerprint_digest, usb_asset_id=excluded.usb_asset_id, status=excluded.status, state_version=excluded.state_version, created_at=excluded.created_at, updated_at=excluded.updated_at, revoked_at=excluded.revoked_at, revoke_reason=excluded.revoke_reason`,
		r.Product, r.InstallationID, r.KeyID, r.LicenseSignatureSHA256, r.FingerprintDigest, r.USBAssetID, r.Status, r.StateVersion,
		r.CreatedAt.UTC().Format(time.RFC3339Nano), r.UpdatedAt.UTC().Format(time.RFC3339Nano), revoked, r.RevokeReason)
	return err
}

func writeAuditRow(ctx context.Context, tx *sql.Tx, ev AuditEvent, old *LicenseRecord) error {
	var oldStatus, oldVersion any
	if old != nil {
		oldStatus, oldVersion = old.Status, old.StateVersion
	}
	_, err := tx.ExecContext(ctx, `INSERT INTO audit_events(sequence,event_id,installation_id,actor,action,old_status,new_status,old_version,new_version,reason,occurred_at,previous_hash,event_hash) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)`,
		ev.Sequence, ev.EventID, ev.Record.InstallationID, ev.Actor, ev.Action, oldStatus, ev.Record.Status, oldVersion, ev.Record.StateVersion, ev.Reason,
		ev.OccurredAt.UTC().Format(time.RFC3339Nano), ev.PreviousHash, ev.Hash)
	if err != nil {
		return err
	}
	return nil
}

func (s *Store) LookupForCheck(ctx context.Context, req CheckRequest, keyring map[string]ed25519.PublicKey, now time.Time) (SignedEnvelope, LicenseRecord, error) {
	var empty SignedEnvelope
	if len(s.signingKey) != ed25519.PrivateKeySize || s.config.SigningKeyID == "" {
		return empty, LicenseRecord{}, errStoreUnknown
	}
	reg, err := VerifyCheckRequest(req, keyring)
	if err != nil {
		return empty, LicenseRecord{}, err
	}
	lock, err := acquireFileLock(ctx, s.config.AuditPath+".lock", 2*time.Second)
	if err != nil {
		return empty, LicenseRecord{}, errStoreUnknown
	}
	defer lock()
	if s.isUnknown() {
		return empty, LicenseRecord{}, errStoreUnknown
	}
	if err := s.ensureConsistent(ctx); err != nil {
		s.setUnknown("database and audit history disagree")
		return empty, LicenseRecord{}, errStoreUnknown
	}
	var record LicenseRecord
	row := s.db.QueryRowContext(ctx, `SELECT product, installation_id, key_id, license_signature_sha256, fingerprint_digest, usb_asset_id, status, state_version, created_at, updated_at, revoked_at, revoke_reason FROM licenses WHERE product=? AND installation_id=?`, ProductID, reg.InstallationID)
	var revoked sql.NullString
	var created, updated string
	if err := row.Scan(&record.Product, &record.InstallationID, &record.KeyID, &record.LicenseSignatureSHA256, &record.FingerprintDigest, &record.USBAssetID, &record.Status, &record.StateVersion, &created, &updated, &revoked, &record.RevokeReason); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return empty, LicenseRecord{}, errNotFound
		}
		s.setUnknown("database query failed")
		return empty, LicenseRecord{}, errStoreUnknown
	}
	if record.LicenseSignatureSHA256 != reg.LicenseSignatureSHA256 || record.FingerprintDigest != reg.FingerprintSHA256 || record.KeyID != reg.LicenseKeyID || record.StateVersion < 1 || (record.Status != "active" && record.Status != "revoked") {
		s.setUnknown("license record integrity failed")
		return empty, LicenseRecord{}, errStoreUnknown
	}
	if record.CreatedAt, err = time.Parse(time.RFC3339Nano, created); err != nil {
		s.setUnknown("database record invalid")
		return empty, LicenseRecord{}, errStoreUnknown
	}
	if record.UpdatedAt, err = time.Parse(time.RFC3339Nano, updated); err != nil {
		s.setUnknown("database record invalid")
		return empty, LicenseRecord{}, errStoreUnknown
	}
	if revoked.Valid {
		t, e := time.Parse(time.RFC3339Nano, revoked.String)
		if e != nil {
			s.setUnknown("database record invalid")
			return empty, LicenseRecord{}, errStoreUnknown
		}
		record.RevokedAt = &t
	}
	payload := ResponsePayload{
		SchemaVersion: ResponseSchema, Product: ProductID, InstallationID: record.InstallationID,
		LicenseSignatureSHA256: record.LicenseSignatureSHA256, RequestID: req.RequestID,
		Status: record.Status, StateVersion: record.StateVersion, IssuedAt: now.UTC(), SigningKeyID: s.config.SigningKeyID,
	}
	raw, err := json.Marshal(payload)
	if err != nil {
		s.setUnknown("response encoding failed")
		return empty, LicenseRecord{}, errStoreUnknown
	}
	sig := ed25519.Sign(s.signingKey, raw)
	return SignedEnvelope{PayloadB64: base64.StdEncoding.EncodeToString(raw), SignatureB64: base64.StdEncoding.EncodeToString(sig)}, record, nil
}

func (s *Store) Import(ctx context.Context, envelope RegistrationEnvelope, actor string, now time.Time) (LicenseRecord, error) {
	reg, err := VerifyRegistration(envelope, s.licenseKeys)
	if err != nil {
		return LicenseRecord{}, errInvalidRegistration
	}
	record, _, err := s.mutate(ctx, reg, "import", actor, "", now.UTC())
	return record, err
}

// Register activates a USB license from its signed registration proof. Retries
// are idempotent, a matching v2 proof can bind the missing public USB asset ID
// once, and registration never changes a revoked record back to active.
func (s *Store) Register(ctx context.Context, envelope RegistrationEnvelope, now time.Time) (LicenseRecord, bool, error) {
	reg, err := VerifyRegistration(envelope, s.licenseKeys)
	if err != nil {
		return LicenseRecord{}, false, errInvalidRegistration
	}
	return s.mutate(ctx, reg, "register", "usb-signer", "", now.UTC())
}

func (s *Store) ChangeState(ctx context.Context, installationID, action, actor, reason string, now time.Time) (LicenseRecord, bool, error) {
	if !validID(installationID) || (action != "revoke" && action != "reactivate") || !validID(actor) || len(reason) == 0 || len(reason) > 200 || strings.TrimSpace(reason) != reason || hasControl(reason) {
		return LicenseRecord{}, false, fmt.Errorf("invalid admin operation")
	}
	lock, err := acquireFileLock(ctx, s.config.AuditPath+".lock", 5*time.Second)
	if err != nil {
		return LicenseRecord{}, false, errStoreUnknown
	}
	defer lock()
	if s.isUnknown() {
		return LicenseRecord{}, false, errStoreUnknown
	}
	if err := s.ensureConsistent(ctx); err != nil {
		s.setUnknown("database and audit history disagree")
		return LicenseRecord{}, false, errStoreUnknown
	}
	current, err := s.lookup(ctx, installationID)
	if err != nil {
		return LicenseRecord{}, false, err
	}
	to := "revoked"
	if action == "reactivate" {
		to = "active"
	}
	if current.Status == to {
		return current, false, nil
	}
	if action == "reactivate" && current.Status != "revoked" || action == "revoke" && current.Status != "active" {
		return LicenseRecord{}, false, errStoreUnknown
	}
	current.StateVersion++
	if !now.After(current.UpdatedAt) {
		now = current.UpdatedAt.Add(time.Nanosecond)
	}
	current.Status = to
	current.UpdatedAt = now
	if to == "revoked" {
		t := now
		current.RevokedAt = &t
		current.RevokeReason = reason
	}
	if err := s.commitEvent(ctx, current, action, actor, reason); err != nil {
		s.setUnknown("audit or database commit failed")
		return LicenseRecord{}, false, errStoreUnknown
	}
	return current, true, nil
}

func (s *Store) mutate(ctx context.Context, reg RegistrationPayload, action, actor, reason string, now time.Time) (LicenseRecord, bool, error) {
	assetID := ""
	if reg.USBAssetID != nil {
		assetID = *reg.USBAssetID
	}
	if !validID(actor) {
		return LicenseRecord{}, false, fmt.Errorf("invalid actor")
	}
	lock, err := acquireFileLock(ctx, s.config.AuditPath+".lock", 5*time.Second)
	if err != nil {
		return LicenseRecord{}, false, errStoreUnknown
	}
	defer lock()
	if s.isUnknown() {
		return LicenseRecord{}, false, errStoreUnknown
	}
	if err := s.ensureConsistent(ctx); err != nil {
		s.setUnknown("database and audit history disagree")
		return LicenseRecord{}, false, errStoreUnknown
	}
	if action == "import" || action == "register" {
		current, err := s.lookup(ctx, reg.InstallationID)
		if err == nil {
			if current.KeyID == reg.LicenseKeyID && current.LicenseSignatureSHA256 == reg.LicenseSignatureSHA256 && current.FingerprintDigest == reg.FingerprintSHA256 {
				if action == "import" && current.Status == "active" && assetID != "" && assetID == current.USBAssetID {
					return current, false, nil
				}
				if assetID != "" && current.USBAssetID == "" && current.Status == "active" && usbAssetIDPattern.MatchString(assetID) {
					current.USBAssetID = assetID
					if !now.After(current.UpdatedAt) {
						now = current.UpdatedAt.Add(time.Nanosecond)
					}
					current.UpdatedAt = now
					if err := s.commitEvent(ctx, current, "asset_bind", actor, ""); err != nil {
						s.setUnknown("audit or database commit failed")
						return LicenseRecord{}, false, errStoreUnknown
					}
					return current, false, nil
				}
				if action == "register" && (assetID == "" || assetID == current.USBAssetID) {
					return current, false, nil
				}
			}
			return LicenseRecord{}, false, errAlreadyExists
		}
		if !errors.Is(err, errNotFound) {
			return LicenseRecord{}, false, err
		}
	}
	r := LicenseRecord{
		Product: ProductID, InstallationID: reg.InstallationID, KeyID: reg.LicenseKeyID,
		LicenseSignatureSHA256: reg.LicenseSignatureSHA256, FingerprintDigest: reg.FingerprintSHA256, USBAssetID: assetID,
		Status: "active", StateVersion: 1, CreatedAt: now, UpdatedAt: now,
	}
	auditAction := action
	if action == "register" {
		// Reuse the existing durable audit action and SQL schema. The actor
		// still distinguishes signer registration from an admin file import.
		auditAction = "import"
	}
	if err := s.commitEvent(ctx, r, auditAction, actor, reason); err != nil {
		s.setUnknown("audit or database commit failed")
		return LicenseRecord{}, false, errStoreUnknown
	}
	return r, true, nil
}

func (s *Store) commitEvent(ctx context.Context, r LicenseRecord, action, actor, reason string) error {
	events, err := validateJournals(s.config.AuditPath, s.config.AuditBackupPath)
	if err != nil {
		return err
	}
	all, err := s.loadAllRecords(ctx)
	if err != nil {
		return err
	}
	auditRows, err := s.loadAuditEvents(ctx)
	if err != nil {
		return err
	}
	expected, err := recordsFromEvents(events)
	if err != nil || !compareRecordMaps(all, expected) || !compareAuditEvents(auditRows, events) {
		return errStoreUnknown
	}
	var old *LicenseRecord
	if action != "import" {
		prev, ok := all[r.InstallationID]
		if !ok {
			return errNotFound
		}
		cp := cloneRecord(prev)
		old = &cp
	}
	var idBytes [16]byte
	if _, err := rand.Read(idBytes[:]); err != nil {
		return err
	}
	idBytes[6] = (idBytes[6] & 0x0f) | 0x40
	idBytes[8] = (idBytes[8] & 0x3f) | 0x80
	eventID := fmt.Sprintf("%08x-%04x-%04x-%04x-%012x", idBytes[0:4], idBytes[4:6], idBytes[6:8], idBytes[8:10], idBytes[10:16])
	previous := ""
	if len(events) > 0 {
		previous = events[len(events)-1].Hash
	}
	ev := AuditEvent{Sequence: uint64(len(events) + 1), PreviousHash: previous, EventID: eventID, Actor: actor, Action: action, Record: cloneRecord(r), Reason: reason, OccurredAt: r.UpdatedAt.UTC()}
	ev.Hash = auditHash(ev)
	if !validAuditRecord(ev) {
		return errJournalInvalid
	}
	if err := appendAudit(s.config.AuditPath, s.config.AuditBackupPath, ev); err != nil {
		return err
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	if err := writeRecord(ctx, tx, r); err != nil {
		tx.Rollback()
		return err
	}
	if err := writeAuditRow(ctx, tx, ev, old); err != nil {
		tx.Rollback()
		return err
	}
	if err := tx.Commit(); err != nil {
		return err
	}
	return nil
}

func (s *Store) lookup(ctx context.Context, id string) (LicenseRecord, error) {
	row := s.db.QueryRowContext(ctx, `SELECT product, installation_id, key_id, license_signature_sha256, fingerprint_digest, usb_asset_id, status, state_version, created_at, updated_at, revoked_at, revoke_reason FROM licenses WHERE product=? AND installation_id=?`, ProductID, id)
	r, err := scanRecord(row)
	if errors.Is(err, sql.ErrNoRows) {
		return LicenseRecord{}, errNotFound
	}
	return r, err
}

func (s *Store) ensureConsistent(ctx context.Context) error {
	if _, err := os.Stat(s.config.RestoreMarkerPath); err == nil {
		return errStoreUnknown
	} else if !errors.Is(err, os.ErrNotExist) {
		return errStoreUnknown
	}
	return s.verifyDatabaseJournalConsistency(ctx)
}

func (s *Store) verifyDatabaseJournalConsistency(ctx context.Context) error {
	events, err := validateJournals(s.config.AuditPath, s.config.AuditBackupPath)
	if err != nil {
		return err
	}
	expected, err := recordsFromEvents(events)
	if err != nil {
		return err
	}
	actual, err := s.loadAllRecords(ctx)
	if err != nil {
		return err
	}
	if !compareRecordMaps(actual, expected) {
		return errStoreUnknown
	}
	auditRows, err := s.loadAuditEvents(ctx)
	if err != nil {
		return err
	}
	if !compareAuditEvents(auditRows, events) {
		return errStoreUnknown
	}
	return nil
}

func (s *Store) Backup(ctx context.Context, outDir string) error {
	lock, err := acquireFileLock(ctx, s.config.AuditPath+".lock", 5*time.Second)
	if err != nil {
		return errStoreUnknown
	}
	defer lock()
	if s.isUnknown() {
		return errStoreUnknown
	}
	if err := s.ensureConsistent(ctx); err != nil {
		return errStoreUnknown
	}
	if filepath.Clean(outDir) == filepath.Clean(filepath.Dir(s.config.DatabasePath)) || filepath.Clean(outDir) == filepath.Clean(filepath.Dir(s.config.AuditPath)) || filepath.Clean(outDir) == filepath.Clean(filepath.Dir(s.config.AuditBackupPath)) {
		return fmt.Errorf("backup destination must be separate")
	}
	if err := os.Mkdir(outDir, 0700); err != nil {
		return err
	}
	dbBackup := filepath.Join(outDir, "state.db")
	var busy, logFrames, checkpointed int
	if err := s.db.QueryRowContext(ctx, `PRAGMA wal_checkpoint(FULL)`).Scan(&busy, &logFrames, &checkpointed); err != nil {
		return err
	}
	if busy != 0 || checkpointed < logFrames {
		return errStoreUnknown
	}
	if err := copyFileDurable(s.config.DatabasePath, dbBackup, 0600); err != nil {
		return err
	}
	primary, err := os.ReadFile(s.config.AuditPath)
	if err != nil {
		return err
	}
	backup, err := os.ReadFile(s.config.AuditBackupPath)
	if err != nil {
		return err
	}
	if string(primary) != string(backup) {
		return errJournalInvalid
	}
	if err := writeDurableFile(filepath.Join(outDir, "audit.jsonl"), primary, 0600); err != nil {
		return err
	}
	meta := struct {
		DatabaseSHA256 string      `json:"databaseSha256"`
		AuditSHA256    string      `json:"auditSha256"`
		JournalHead    JournalHead `json:"journalHead"`
		CreatedAt      time.Time   `json:"createdAt"`
	}{
		DatabaseSHA256: fileSHA256(dbBackup), AuditSHA256: sha256Hex(primary), JournalHead: formatHead(mustReadJournalBytes(primary)), CreatedAt: time.Now().UTC(),
	}
	raw, _ := json.MarshalIndent(meta, "", "  ")
	if err := writeDurableFile(filepath.Join(outDir, "manifest.json"), append(raw, '\n'), 0600); err != nil {
		return err
	}
	return syncDirectory(outDir)
}

func (s *Store) Recover(ctx context.Context) error {
	if _, err := os.Stat(s.config.RestoreMarkerPath); err != nil {
		return fmt.Errorf("restore marker must be present")
	}
	lock, err := acquireFileLock(ctx, s.config.AuditPath+".lock", 5*time.Second)
	if err != nil {
		return errStoreUnknown
	}
	defer lock()
	if _, err := s.db.ExecContext(ctx, "PRAGMA journal_mode=WAL"); err != nil {
		return err
	}
	if _, err := s.db.ExecContext(ctx, "PRAGMA synchronous=FULL"); err != nil {
		return err
	}
	if _, err := s.db.ExecContext(ctx, "PRAGMA busy_timeout=2500"); err != nil {
		return err
	}
	for _, statement := range strings.Split(schemaSQL, ";") {
		if strings.TrimSpace(statement) == "" {
			continue
		}
		if _, err := s.db.ExecContext(ctx, statement); err != nil {
			return err
		}
	}
	if err := s.ensureUSBAssetSchema(ctx); err != nil {
		return err
	}
	events, err := validateJournals(s.config.AuditPath, s.config.AuditBackupPath)
	if err != nil {
		return errStoreUnknown
	}
	actual, err := s.loadAllRecords(ctx)
	if err != nil {
		return errStoreUnknown
	}
	actualAudit, err := s.loadAuditEvents(ctx)
	if err != nil {
		return errStoreUnknown
	}
	for prefix := 0; prefix <= len(events); prefix++ {
		state, err := recordsFromEvents(events[:prefix])
		if err != nil {
			return errStoreUnknown
		}
		if compareRecordMaps(actual, state) && compareAuditEvents(actualAudit, events[:prefix]) {
			if err := s.replayEvents(ctx, events[prefix:]); err != nil {
				return errStoreUnknown
			}
			return s.verifyDatabaseJournalConsistency(ctx)
		}
	}
	return errStoreUnknown
}

func (s *Store) VerifyRecovery(ctx context.Context) error {
	lock, err := acquireFileLock(ctx, s.config.AuditPath+".lock", 2*time.Second)
	if err != nil {
		return errStoreUnknown
	}
	defer lock()
	return s.verifyDatabaseJournalConsistency(ctx)
}

func mustReadJournalBytes(raw []byte) []AuditEvent {
	var events []AuditEvent
	for _, line := range strings.Split(strings.TrimSpace(string(raw)), "\n") {
		if line == "" {
			continue
		}
		var ev AuditEvent
		if json.Unmarshal([]byte(line), &ev) == nil {
			events = append(events, ev)
		}
	}
	return events
}

func fileSHA256(path string) string {
	raw, err := os.ReadFile(path)
	if err != nil {
		return ""
	}
	return sha256Hex(raw)
}
func sha256Hex(raw []byte) string { sum := sha256.Sum256(raw); return hex.EncodeToString(sum[:]) }

func (s *Store) isUnknown() bool { s.unknownMu.RLock(); defer s.unknownMu.RUnlock(); return s.unknown }
func (s *Store) IsHealthy() bool { return !s.isUnknown() }
func (s *Store) setUnknown(reason string) {
	s.unknownMu.Lock()
	s.unknown, s.unknownReason = true, reason
	s.unknownMu.Unlock()
}
func (s *Store) setKnown() {
	s.unknownMu.Lock()
	s.unknown, s.unknownReason = false, ""
	s.unknownMu.Unlock()
}

func LoadKeyring(path string) (map[string]ed25519.PublicKey, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var ring Keyring
	if err := decodeStrictJSON(raw, &ring); err != nil || len(ring.Keys) == 0 {
		return nil, fmt.Errorf("invalid license keyring")
	}
	keys := make(map[string]ed25519.PublicKey, len(ring.Keys))
	for id, encoded := range ring.Keys {
		if !keyIDPattern.MatchString(id) {
			return nil, fmt.Errorf("invalid license keyring")
		}
		b, err := decodeStdBase64(encoded)
		if err != nil || len(b) != ed25519.PublicKeySize {
			return nil, fmt.Errorf("invalid license keyring")
		}
		keys[id] = ed25519.PublicKey(b)
	}
	return keys, nil
}

func LoadSigningKey(path, keyID string) (StatusSigningKey, error) {
	if !keyIDPattern.MatchString(keyID) {
		return StatusSigningKey{}, fmt.Errorf("invalid signing key id")
	}
	info, err := os.Lstat(path)
	if err != nil {
		return StatusSigningKey{}, err
	}
	if !info.Mode().IsRegular() {
		return StatusSigningKey{}, fmt.Errorf("signing key must be a regular file")
	}
	if runtime.GOOS != "windows" && info.Mode().Perm()&0137 != 0 {
		return StatusSigningKey{}, fmt.Errorf("signing key permissions must be restricted to owner and service group")
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return StatusSigningKey{}, err
	}
	decoded, err := decodeStdBase64(strings.TrimSpace(string(raw)))
	if err != nil || len(decoded) != ed25519.PrivateKeySize {
		return StatusSigningKey{}, fmt.Errorf("invalid signing key file")
	}
	return StatusSigningKey{KeyID: keyID, Key: ed25519.PrivateKey(decoded)}, nil
}

func (s *Store) Inspect(ctx context.Context, id string) (LicenseRecord, error) {
	if !validID(id) || s.isUnknown() {
		return LicenseRecord{}, errStoreUnknown
	}
	lock, err := acquireFileLock(ctx, s.config.AuditPath+".lock", 2*time.Second)
	if err != nil {
		return LicenseRecord{}, errStoreUnknown
	}
	defer lock()
	if err := s.ensureConsistent(ctx); err != nil {
		return LicenseRecord{}, errStoreUnknown
	}
	r, err := s.lookup(ctx, id)
	if err != nil {
		return LicenseRecord{}, err
	}
	return r, nil
}

// List returns a bounded page of registered USB licenses only when the
// database and both audit journals still agree. It deliberately exposes
// records to the authenticated admin console through a separate HTTP layer;
// callers must not serialize LicenseRecord directly to an untrusted client.
func (s *Store) List(ctx context.Context, offset, limit int) ([]LicenseRecord, int, error) {
	if offset < 0 || offset > 1_000_000 || limit < 1 || limit > 200 || s.isUnknown() {
		return nil, 0, errStoreUnknown
	}
	lock, err := acquireFileLock(ctx, s.config.AuditPath+".lock", 2*time.Second)
	if err != nil {
		return nil, 0, errStoreUnknown
	}
	defer lock()
	if s.isUnknown() {
		return nil, 0, errStoreUnknown
	}
	if err := s.ensureConsistent(ctx); err != nil {
		s.setUnknown("database and audit history disagree")
		return nil, 0, errStoreUnknown
	}
	var total int
	if err := s.db.QueryRowContext(ctx, `SELECT COUNT(*) FROM licenses WHERE product=?`, ProductID).Scan(&total); err != nil {
		s.setUnknown("license list query failed")
		return nil, 0, errStoreUnknown
	}
	rows, err := s.db.QueryContext(ctx, `SELECT product, installation_id, key_id, license_signature_sha256, fingerprint_digest, usb_asset_id, status, state_version, created_at, updated_at, revoked_at, revoke_reason FROM licenses WHERE product=? ORDER BY updated_at DESC, installation_id ASC LIMIT ? OFFSET ?`, ProductID, limit, offset)
	if err != nil {
		s.setUnknown("license list query failed")
		return nil, 0, errStoreUnknown
	}
	defer rows.Close()
	records := make([]LicenseRecord, 0, min(limit, total))
	for rows.Next() {
		record, err := scanRecord(rows)
		if err != nil {
			s.setUnknown("license list record invalid")
			return nil, 0, errStoreUnknown
		}
		records = append(records, record)
	}
	if err := rows.Err(); err != nil {
		s.setUnknown("license list query failed")
		return nil, 0, errStoreUnknown
	}
	return records, total, nil
}

func GenerateSigningKey(path string) (string, ed25519.PublicKey, error) {
	pub, priv, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return "", nil, err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return "", nil, err
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		return "", nil, err
	}
	if _, err := f.WriteString(base64.StdEncoding.EncodeToString(priv) + "\n"); err != nil {
		f.Close()
		os.Remove(path)
		return "", nil, err
	}
	if err := f.Sync(); err != nil {
		f.Close()
		os.Remove(path)
		return "", nil, err
	}
	if err := f.Close(); err != nil {
		os.Remove(path)
		return "", nil, err
	}
	if err := syncDirectory(filepath.Dir(path)); err != nil {
		os.Remove(path)
		return "", nil, err
	}
	return base64.StdEncoding.EncodeToString(pub), pub, nil
}
