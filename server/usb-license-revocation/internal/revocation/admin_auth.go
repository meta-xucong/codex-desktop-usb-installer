package revocation

import (
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha1"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base32"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"crypto/pbkdf2"
)

const adminTOTPPeriod = int64(30)

var (
	errAdminAuthRejected = errors.New("admin authentication rejected")
	errAdminAuthStorage  = errors.New("admin authentication state unavailable")
)

type adminAuthState struct {
	SchemaVersion      int    `json:"schemaVersion"`
	PasswordSalt       string `json:"passwordSalt"`
	PasswordHash       string `json:"passwordHash"`
	TOTPSecret         string `json:"totpSecret,omitempty"`
	FirstLoginComplete bool   `json:"firstLoginComplete"`
	LastTOTPStep       int64  `json:"lastTotpStep"`
}

func normalizeAdminTOTPSecret(encoded string) (string, []byte, error) {
	cleaned := strings.NewReplacer(" ", "", "-", "").Replace(strings.ToUpper(strings.TrimSpace(encoded)))
	secret, err := base32.StdEncoding.WithPadding(base32.NoPadding).DecodeString(cleaned)
	if err != nil || len(secret) < 20 {
		return "", nil, errors.New("admin TOTP secret must be unpadded Base32 encoding of at least 20 bytes")
	}
	return base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(secret), secret, nil
}

func decodeAdminTOTPSecret(encoded string) ([]byte, error) {
	_, secret, err := normalizeAdminTOTPSecret(encoded)
	return secret, err
}

func generateAdminTOTPSecret() (string, error) {
	secret := make([]byte, 20)
	if _, err := rand.Read(secret); err != nil {
		return "", err
	}
	return base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(secret), nil
}

func loadOrInitializeAdminAuth(path, initialPassword, initialTOTPSecret string) (adminAuthState, error) {
	if path == "" {
		return adminAuthState{}, errors.New("admin auth state path is required")
	}
	initialTOTP := ""
	if initialTOTPSecret != "" {
		var err error
		initialTOTP, _, err = normalizeAdminTOTPSecret(initialTOTPSecret)
		if err != nil {
			return adminAuthState{}, errors.New("admin TOTP secret is invalid")
		}
	}
	info, err := os.Lstat(path)
	if err == nil {
		if !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 {
			return adminAuthState{}, errors.New("admin auth state must be a regular file")
		}
		if runtime.GOOS != "windows" && info.Mode().Perm()&0077 != 0 {
			return adminAuthState{}, errors.New("admin auth state permissions must be restricted to owner")
		}
		raw, readErr := os.ReadFile(path)
		if readErr != nil {
			return adminAuthState{}, errors.New("admin auth state cannot be read")
		}
		var state adminAuthState
		if decodeStrictJSON(raw, &state) != nil || state.SchemaVersion < 1 || state.SchemaVersion > 3 || state.LastTOTPStep < 0 {
			return adminAuthState{}, errors.New("admin auth state is invalid")
		}
		salt, saltErr := base64.StdEncoding.DecodeString(state.PasswordSalt)
		hash, hashErr := base64.StdEncoding.DecodeString(state.PasswordHash)
		if saltErr != nil || len(salt) != 16 || hashErr != nil || len(hash) != 32 {
			return adminAuthState{}, errors.New("admin auth state is invalid")
		}
		migrated := state.SchemaVersion < 3
		if state.SchemaVersion == 1 {
			state.TOTPSecret = initialTOTP
		}
		if migrated {
			state.SchemaVersion = 3
			state.FirstLoginComplete = false
		}
		if state.TOTPSecret != "" {
			if _, err := decodeAdminTOTPSecret(state.TOTPSecret); err != nil {
				return adminAuthState{}, errors.New("admin auth state is invalid")
			}
		} else if initialTOTP != "" {
			state.TOTPSecret = initialTOTP
			migrated = true
		}
		if state.FirstLoginComplete && state.TOTPSecret == "" {
			return adminAuthState{}, errors.New("admin auth state is invalid")
		}
		if migrated {
			if err := writeAdminAuthState(path, state); err != nil {
				return adminAuthState{}, errors.New("admin auth state cannot be migrated")
			}
		}
		return state, nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return adminAuthState{}, errors.New("admin auth state cannot be checked")
	}
	if len(initialPassword) < 12 || len(initialPassword) > 1024 {
		return adminAuthState{}, errors.New("initial admin password is invalid")
	}
	salt := make([]byte, 16)
	if _, err := rand.Read(salt); err != nil {
		return adminAuthState{}, errors.New("admin auth initialization failed")
	}
	hash, err := pbkdf2.Key(sha256.New, initialPassword, salt, adminPasswordRounds, 32)
	if err != nil {
		return adminAuthState{}, errors.New("admin auth initialization failed")
	}
	state := adminAuthState{
		SchemaVersion:      3,
		PasswordSalt:       base64.StdEncoding.EncodeToString(salt),
		PasswordHash:       base64.StdEncoding.EncodeToString(hash),
		TOTPSecret:         initialTOTP,
		FirstLoginComplete: false,
	}
	if err := writeAdminAuthState(path, state); err != nil {
		return adminAuthState{}, errors.New("admin auth state cannot be initialized")
	}
	return state, nil
}

func writeAdminAuthState(path string, state adminAuthState) error {
	raw, err := json.Marshal(state)
	if err != nil {
		return err
	}
	return writeDurableFile(filepath.Clean(path), append(raw, '\n'), 0600)
}

func adminPasswordMatches(state adminAuthState, candidate string) bool {
	salt, err := base64.StdEncoding.DecodeString(state.PasswordSalt)
	if err != nil || len(salt) != 16 {
		return false
	}
	want, err := base64.StdEncoding.DecodeString(state.PasswordHash)
	if err != nil || len(want) != 32 {
		return false
	}
	got, err := pbkdf2.Key(sha256.New, candidate, salt, adminPasswordRounds, len(want))
	return err == nil && subtle.ConstantTimeCompare(got, want) == 1
}

func matchingTOTPStep(secret []byte, code string, now time.Time, lastStep int64) (int64, bool) {
	if len(code) != 6 || len(secret) < 20 {
		return 0, false
	}
	for _, char := range code {
		if char < '0' || char > '9' {
			return 0, false
		}
	}
	current := now.Unix() / adminTOTPPeriod
	var match int64 = -1
	for step := current - 1; step <= current+1; step++ {
		if step <= lastStep || step < 0 {
			continue
		}
		candidate := adminTOTPCode(secret, step)
		if subtle.ConstantTimeCompare([]byte(candidate), []byte(code)) == 1 && step > match {
			match = step
		}
	}
	return match, match >= 0
}

func adminTOTPCode(secret []byte, step int64) string {
	var counter [8]byte
	binary.BigEndian.PutUint64(counter[:], uint64(step))
	mac := hmac.New(sha1.New, secret)
	_, _ = mac.Write(counter[:])
	digest := mac.Sum(nil)
	offset := digest[len(digest)-1] & 0x0f
	value := binary.BigEndian.Uint32(digest[offset:offset+4]) & 0x7fffffff
	return fmt.Sprintf("%06d", value%1_000_000)
}

func (a *adminConsole) changeAdminPassword(currentPassword, newPassword, code string) error {
	if len(newPassword) < 12 || len(newPassword) > 128 || hasControl(newPassword) || newPassword == currentPassword {
		return errAdminAuthRejected
	}
	a.authMu.Lock()
	defer a.authMu.Unlock()
	if a.authUnavailable || !a.authState.FirstLoginComplete {
		return errAdminAuthStorage
	}
	if !adminPasswordMatches(a.authState, currentPassword) {
		return errAdminAuthRejected
	}
	secret, err := decodeAdminTOTPSecret(a.authState.TOTPSecret)
	if err != nil {
		return errAdminAuthRejected
	}
	step, ok := matchingTOTPStep(secret, code, time.Now(), a.authState.LastTOTPStep)
	if !ok {
		return errAdminAuthRejected
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	salt := make([]byte, 16)
	if _, err := rand.Read(salt); err != nil {
		return errAdminAuthStorage
	}
	hash, err := pbkdf2.Key(sha256.New, newPassword, salt, adminPasswordRounds, 32)
	if err != nil {
		return errAdminAuthStorage
	}
	next := adminAuthState{
		SchemaVersion:      3,
		PasswordSalt:       base64.StdEncoding.EncodeToString(salt),
		PasswordHash:       base64.StdEncoding.EncodeToString(hash),
		TOTPSecret:         a.authState.TOTPSecret,
		FirstLoginComplete: true,
		LastTOTPStep:       step,
	}
	if err := writeAdminAuthState(a.authStatePath, next); err != nil {
		a.authUnavailable = true
		clear(a.sessions)
		return errAdminAuthStorage
	}
	a.authState = next
	clear(a.sessions)
	return nil
}

func (a *adminConsole) completeInitialAdminSetup(sessionDigest [32]byte, newPassword, code string) error {
	if len(newPassword) < 12 || len(newPassword) > adminMaxPassword || hasControl(newPassword) {
		return errAdminAuthRejected
	}
	a.authMu.Lock()
	defer a.authMu.Unlock()
	if a.authUnavailable || a.authState.FirstLoginComplete || adminPasswordMatches(a.authState, newPassword) {
		return errAdminAuthRejected
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	session, ok := a.sessions[sessionDigest]
	if !ok || !session.setupPending || session.pendingTOTPSecret == "" {
		return errAdminAuthRejected
	}
	secret, err := decodeAdminTOTPSecret(session.pendingTOTPSecret)
	if err != nil {
		return errAdminAuthStorage
	}
	step, ok := matchingTOTPStep(secret, code, time.Now(), a.authState.LastTOTPStep)
	if !ok {
		return errAdminAuthRejected
	}
	salt := make([]byte, 16)
	if _, err := rand.Read(salt); err != nil {
		return errAdminAuthStorage
	}
	hash, err := pbkdf2.Key(sha256.New, newPassword, salt, adminPasswordRounds, 32)
	if err != nil {
		return errAdminAuthStorage
	}
	next := adminAuthState{
		SchemaVersion:      3,
		PasswordSalt:       base64.StdEncoding.EncodeToString(salt),
		PasswordHash:       base64.StdEncoding.EncodeToString(hash),
		TOTPSecret:         session.pendingTOTPSecret,
		FirstLoginComplete: true,
		LastTOTPStep:       step,
	}
	if err := writeAdminAuthState(a.authStatePath, next); err != nil {
		a.authUnavailable = true
		clear(a.sessions)
		return errAdminAuthStorage
	}
	a.authState = next
	clear(a.sessions)
	return nil
}
