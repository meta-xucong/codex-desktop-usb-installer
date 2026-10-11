package revocation

import (
	"bufio"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"
	"unicode"
)

var errJournalInvalid = errors.New("audit journal is invalid")

func readJournal(path string) ([]AuditEvent, error) {
	f, err := os.Open(path)
	if errors.Is(err, os.ErrNotExist) {
		return []AuditEvent{}, nil
	}
	if err != nil {
		return nil, err
	}
	defer f.Close()

	s := bufio.NewScanner(f)
	s.Buffer(make([]byte, 4096), 1<<20)
	events := make([]AuditEvent, 0)
	previous := ""
	var sequence uint64
	for s.Scan() {
		line := append([]byte(nil), s.Bytes()...)
		if len(line) == 0 {
			return nil, errJournalInvalid
		}
		var ev AuditEvent
		if err := decodeStrictJSON(line, &ev); err != nil {
			return nil, errJournalInvalid
		}
		sequence++
		if ev.Sequence != sequence || ev.PreviousHash != previous || ev.Hash != auditHash(ev) || !validAuditRecord(ev) {
			return nil, errJournalInvalid
		}
		previous = ev.Hash
		events = append(events, ev)
	}
	if err := s.Err(); err != nil {
		return nil, err
	}
	return events, nil
}

func appendAudit(primary, backup string, ev AuditEvent) error {
	if primary == "" || backup == "" || filepath.Clean(primary) == filepath.Clean(backup) {
		return errJournalInvalid
	}
	if err := os.MkdirAll(filepath.Dir(primary), 0750); err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(backup), 0750); err != nil {
		return err
	}
	line, err := json.Marshal(ev)
	if err != nil {
		return err
	}
	line = append(line, '\n')
	for _, path := range []string{primary, backup} {
		f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0640)
		if err != nil {
			return err
		}
		_, writeErr := f.Write(line)
		if writeErr == nil {
			writeErr = f.Sync()
		}
		closeErr := f.Close()
		if writeErr != nil {
			return writeErr
		}
		if closeErr != nil {
			return closeErr
		}
	}
	return nil
}

func auditHash(ev AuditEvent) string {
	ev.Hash = ""
	raw, _ := json.Marshal(ev)
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:])
}

func validateJournals(primary, backup string) ([]AuditEvent, error) {
	a, err := readJournal(primary)
	if err != nil {
		return nil, err
	}
	b, err := readJournal(backup)
	if err != nil {
		return nil, err
	}
	if len(a) != len(b) {
		return nil, errJournalInvalid
	}
	for i := range a {
		if a[i].Hash != b[i].Hash {
			return nil, errJournalInvalid
		}
	}
	return a, nil
}

func writeDurableFile(path string, data []byte, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(path), 0750); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".revocation-*.tmp")
	if err != nil {
		return err
	}
	name := tmp.Name()
	defer os.Remove(name)
	if err := tmp.Chmod(mode); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Rename(name, path); err != nil {
		return err
	}
	return syncDirectory(filepath.Dir(path))
}

func syncDirectory(path string) error {
	if runtime.GOOS == "windows" {
		return nil
	}
	d, err := os.Open(path)
	if err != nil {
		return err
	}
	defer d.Close()
	if err := d.Sync(); err != nil && !errors.Is(err, os.ErrInvalid) {
		return err
	}
	return nil
}

func validAuditRecord(ev AuditEvent) bool {
	if ev.Sequence == 0 || !validID(ev.EventID) || !validID(ev.Actor) || ev.OccurredAt.IsZero() {
		return false
	}
	r := ev.Record
	if r.Product != ProductID || !validID(r.InstallationID) || !keyIDPattern.MatchString(r.KeyID) ||
		!digestPattern.MatchString(r.LicenseSignatureSHA256) || !digestPattern.MatchString(r.FingerprintDigest) || r.StateVersion < 1 ||
		r.CreatedAt.IsZero() || r.UpdatedAt.IsZero() || !r.UpdatedAt.Equal(ev.OccurredAt) {
		return false
	}
	if r.USBAssetID != "" && !usbAssetIDPattern.MatchString(r.USBAssetID) {
		return false
	}
	switch ev.Action {
	case "import":
		return r.Status == "active" && r.StateVersion == 1 && r.CreatedAt.Equal(r.UpdatedAt) && r.RevokedAt == nil && r.RevokeReason == "" && ev.Reason == ""
	case "revoke":
		return r.Status == "revoked" && r.StateVersion >= 2 && r.RevokedAt != nil && r.RevokedAt.Equal(ev.OccurredAt) && r.RevokeReason == ev.Reason && ev.Reason != ""
	case "reactivate":
		return r.Status == "active" && r.StateVersion >= 3 && r.RevokedAt != nil && r.RevokeReason != "" && ev.Reason != ""
	case "asset_bind":
		return r.USBAssetID != "" && ev.Reason == ""
	default:
		return false
	}
}

func hasControl(value string) bool {
	for _, r := range value {
		if unicode.IsControl(r) {
			return true
		}
	}
	return false
}

func recordsFromEvents(events []AuditEvent) (map[string]LicenseRecord, error) {
	state := make(map[string]LicenseRecord)
	for _, ev := range events {
		key := ev.Record.InstallationID
		old, exists := state[key]
		if ev.Action == "asset_bind" {
			if !exists || old.USBAssetID != "" || ev.Record.USBAssetID == "" ||
				old.Product != ev.Record.Product || old.KeyID != ev.Record.KeyID ||
				old.LicenseSignatureSHA256 != ev.Record.LicenseSignatureSHA256 || old.FingerprintDigest != ev.Record.FingerprintDigest ||
				old.Status != ev.Record.Status || old.StateVersion != ev.Record.StateVersion || !old.CreatedAt.Equal(ev.Record.CreatedAt) ||
				!sameOptionalTime(old.RevokedAt, ev.Record.RevokedAt) || old.RevokeReason != ev.Record.RevokeReason || ev.Record.UpdatedAt.Before(old.UpdatedAt) {
				return nil, errJournalInvalid
			}
			state[key] = cloneRecord(ev.Record)
			continue
		}
		if ev.Action == "import" {
			if exists || ev.Record.StateVersion != 1 {
				return nil, errJournalInvalid
			}
			state[key] = cloneRecord(ev.Record)
			continue
		}
		if !exists || ev.Record.StateVersion != old.StateVersion+1 || old.Product != ev.Record.Product || old.KeyID != ev.Record.KeyID ||
			old.LicenseSignatureSHA256 != ev.Record.LicenseSignatureSHA256 || old.FingerprintDigest != ev.Record.FingerprintDigest || old.USBAssetID != ev.Record.USBAssetID || !old.CreatedAt.Equal(ev.Record.CreatedAt) || ev.Record.UpdatedAt.Before(old.UpdatedAt) {
			return nil, errJournalInvalid
		}
		if ev.Action == "revoke" && old.Status != "active" {
			return nil, errJournalInvalid
		}
		if ev.Action == "reactivate" && old.Status != "revoked" {
			return nil, errJournalInvalid
		}
		if ev.Action == "reactivate" && (old.RevokedAt == nil || ev.Record.RevokedAt == nil || !old.RevokedAt.Equal(*ev.Record.RevokedAt) || old.RevokeReason != ev.Record.RevokeReason) {
			return nil, errJournalInvalid
		}
		state[key] = cloneRecord(ev.Record)
	}
	return state, nil
}

func sameOptionalTime(a, b *time.Time) bool {
	if a == nil || b == nil {
		return a == nil && b == nil
	}
	return a.Equal(*b)
}

func cloneRecord(r LicenseRecord) LicenseRecord {
	if r.RevokedAt != nil {
		t := *r.RevokedAt
		r.RevokedAt = &t
	}
	return r
}

func formatHead(events []AuditEvent) JournalHead {
	if len(events) == 0 {
		return JournalHead{}
	}
	return JournalHead{Sequence: events[len(events)-1].Sequence, Hash: events[len(events)-1].Hash}
}

func compareRecordMaps(actual, expected map[string]LicenseRecord) bool {
	if len(actual) != len(expected) {
		return false
	}
	for key, want := range expected {
		got, ok := actual[key]
		if !ok || !sameRecord(got, want) {
			return false
		}
	}
	return true
}

func sameRecord(a, b LicenseRecord) bool {
	if a.Product != b.Product || a.InstallationID != b.InstallationID || a.KeyID != b.KeyID ||
		a.LicenseSignatureSHA256 != b.LicenseSignatureSHA256 || a.FingerprintDigest != b.FingerprintDigest || a.USBAssetID != b.USBAssetID ||
		a.Status != b.Status || a.StateVersion != b.StateVersion || !a.CreatedAt.Equal(b.CreatedAt) ||
		!a.UpdatedAt.Equal(b.UpdatedAt) || a.RevokeReason != b.RevokeReason {
		return false
	}
	if a.RevokedAt == nil || b.RevokedAt == nil {
		return a.RevokedAt == nil && b.RevokedAt == nil
	}
	return a.RevokedAt.Equal(*b.RevokedAt)
}

func ensurePathIsDistinct(paths ...string) error {
	seen := map[string]bool{}
	for _, path := range paths {
		key, err := canonicalPathKey(path)
		if err != nil {
			return err
		}
		if seen[key] {
			return fmt.Errorf("configured paths overlap")
		}
		seen[key] = true
	}
	return nil
}

// canonicalPathKey resolves symlinked existing path components while still
// supporting files that have not been created yet. This prevents two config
// paths from aliasing through a symlinked parent directory.
func canonicalPathKey(path string) (string, error) {
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	abs = filepath.Clean(abs)
	resolved, err := filepath.EvalSymlinks(abs)
	if err == nil {
		return strings.ToLower(filepath.Clean(resolved)), nil
	}
	if !os.IsNotExist(err) {
		return "", err
	}

	var missing []string
	current := abs
	for {
		parent := filepath.Dir(current)
		if parent == current {
			return "", err
		}
		missing = append(missing, filepath.Base(current))
		current = parent
		resolved, err = filepath.EvalSymlinks(current)
		if err == nil {
			for i := len(missing) - 1; i >= 0; i-- {
				resolved = filepath.Join(resolved, missing[i])
			}
			return strings.ToLower(filepath.Clean(resolved)), nil
		}
		if !os.IsNotExist(err) {
			return "", err
		}
	}
}

func copyFileDurable(src, dst string, mode os.FileMode) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	if err := os.MkdirAll(filepath.Dir(dst), 0750); err != nil {
		return err
	}
	out, err := os.OpenFile(dst, os.O_CREATE|os.O_EXCL|os.O_WRONLY, mode)
	if err != nil {
		return err
	}
	_, copyErr := io.Copy(out, in)
	if copyErr == nil {
		copyErr = out.Sync()
	}
	closeErr := out.Close()
	if copyErr != nil {
		return copyErr
	}
	return closeErr
}
