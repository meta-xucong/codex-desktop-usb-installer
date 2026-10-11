package revocation

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"regexp"
	"strconv"
	"strings"
	"unicode/utf8"
)

var (
	errInvalidRegistration = errors.New("invalid registration proof")
	keyIDPattern           = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`)
	digestPattern          = regexp.MustCompile(`^[0-9a-f]{64}$`)
	requestIDPattern       = regexp.MustCompile(`^[0-9a-f]{32}$`)
	usbAssetIDPattern      = regexp.MustCompile(`^USB-[0-9]{1,60}$`)
)

func decodeStdBase64(value string) ([]byte, error) {
	decoded, err := base64.StdEncoding.Strict().DecodeString(value)
	if err != nil || base64.StdEncoding.EncodeToString(decoded) != value {
		return nil, errors.New("invalid base64")
	}
	return decoded, nil
}

func VerifyRegistration(env RegistrationEnvelope, keyring map[string]ed25519.PublicKey) (RegistrationPayload, error) {
	var payload RegistrationPayload
	if len(env.PayloadB64) == 0 || len(env.PayloadB64) > MaxRegistrationBytes*2 || len(env.SignatureB64) > 256 {
		return payload, errInvalidRegistration
	}
	raw, err := decodeStdBase64(env.PayloadB64)
	if err != nil || len(raw) == 0 || len(raw) > MaxRegistrationBytes {
		return payload, errInvalidRegistration
	}
	sig, err := decodeStdBase64(env.SignatureB64)
	if err != nil || len(sig) != ed25519.SignatureSize {
		return payload, errInvalidRegistration
	}
	if err = decodeStrictJSON(raw, &payload); err != nil {
		return RegistrationPayload{}, errInvalidRegistration
	}
	registrationVersionValid := false
	switch {
	case payload.SchemaVersion == RegistrationSchema && payload.Domain == RegistrationDomain && payload.USBAssetID == nil:
		registrationVersionValid = true
	case payload.SchemaVersion == RegistrationSchemaV2 && payload.Domain == RegistrationDomainV2 && payload.USBAssetID != nil && usbAssetIDPattern.MatchString(*payload.USBAssetID):
		registrationVersionValid = true
	}
	if !registrationVersionValid || payload.Product != ProductID ||
		!validID(payload.InstallationID) || !keyIDPattern.MatchString(payload.LicenseKeyID) ||
		!digestPattern.MatchString(payload.LicenseSignatureSHA256) || !digestPattern.MatchString(payload.FingerprintSHA256) {
		return RegistrationPayload{}, errInvalidRegistration
	}
	if _, ok := keyring[payload.LicenseKeyID]; !ok {
		return RegistrationPayload{}, errInvalidRegistration
	}
	if !validIssuedAtMillis(payload.IssuedAt) || !ed25519.Verify(keyring[payload.LicenseKeyID], raw, sig) {
		return RegistrationPayload{}, errInvalidRegistration
	}
	return payload, nil
}

func validIssuedAtMillis(value string) bool {
	if value == "" {
		return false
	}
	for i := 0; i < len(value); i++ {
		if value[i] < '0' || value[i] > '9' {
			return false
		}
	}
	_, err := strconv.ParseUint(value, 10, 64)
	return err == nil
}

func ParseRegistrationFile(reader io.Reader, keyring map[string]ed25519.PublicKey) (RegistrationPayload, error) {
	_, payload, err := ReadRegistrationEnvelope(reader, keyring)
	return payload, err
}

func ReadRegistrationEnvelope(reader io.Reader, keyring map[string]ed25519.PublicKey) (RegistrationEnvelope, RegistrationPayload, error) {
	limited := io.LimitReader(reader, MaxRegistrationBytes*3+1)
	raw, err := io.ReadAll(limited)
	if err != nil || len(raw) == 0 || len(raw) > MaxRegistrationBytes*3 {
		return RegistrationEnvelope{}, RegistrationPayload{}, errInvalidRegistration
	}
	var envelope RegistrationEnvelope
	if err := decodeStrictJSON(raw, &envelope); err != nil {
		return RegistrationEnvelope{}, RegistrationPayload{}, errInvalidRegistration
	}
	payload, err := VerifyRegistration(envelope, keyring)
	if err != nil {
		return RegistrationEnvelope{}, RegistrationPayload{}, err
	}
	return envelope, payload, nil
}

func VerifyCheckRequest(req CheckRequest, keyring map[string]ed25519.PublicKey) (RegistrationPayload, error) {
	if req.SchemaVersion != RequestSchema || !requestIDPattern.MatchString(req.RequestID) || req.Product != ProductID || !validID(req.InstallationID) || !digestPattern.MatchString(req.LicenseSignatureSHA256) {
		return RegistrationPayload{}, errInvalidRegistration
	}
	registration, err := VerifyRegistration(req.Registration, keyring)
	if err != nil {
		return RegistrationPayload{}, errInvalidRegistration
	}
	observed, err := FingerprintDigest(req.ObservedFingerprint.AnchorSchema, req.ObservedFingerprint.VolumeSerial, req.ObservedFingerprint.AnchorSHA256, req.ObservedFingerprint.BundleRelativePath)
	if err != nil || observed != registration.FingerprintSHA256 {
		return RegistrationPayload{}, errInvalidRegistration
	}
	sigDigest, err := SignatureDigestFromDigestField(req.LicenseSignatureSHA256)
	if err != nil || sigDigest != registration.LicenseSignatureSHA256 || req.InstallationID != registration.InstallationID || req.Product != registration.Product {
		return RegistrationPayload{}, errInvalidRegistration
	}
	return registration, nil
}

func SignatureDigestFromDigestField(value string) (string, error) {
	if !digestPattern.MatchString(value) {
		return "", fmt.Errorf("invalid digest")
	}
	return value, nil
}

func validID(value string) bool {
	if len(value) == 0 || len(value) > 128 || strings.TrimSpace(value) != value {
		return false
	}
	for _, r := range value {
		if r < 0x21 || r == 0x7f || r > 0x7e {
			return false
		}
	}
	return true
}

func decodeStrictJSON(data []byte, target any) error {
	if !utf8.Valid(data) {
		return errors.New("invalid UTF-8")
	}
	if hasDuplicateJSONKeys(data) {
		return errors.New("duplicate JSON key")
	}
	d := json.NewDecoder(strings.NewReader(string(data)))
	d.DisallowUnknownFields()
	if err := d.Decode(target); err != nil {
		return err
	}
	if err := d.Decode(new(any)); err != io.EOF {
		return errors.New("trailing JSON")
	}
	return nil
}

func hasDuplicateJSONKeys(data []byte) bool {
	dec := json.NewDecoder(strings.NewReader(string(data)))
	if err := scanJSONValue(dec); err != nil {
		return true
	}
	_, err := dec.Token()
	return err != io.EOF
}

func scanJSONValue(dec *json.Decoder) error {
	tok, err := dec.Token()
	if err != nil {
		return err
	}
	delim, ok := tok.(json.Delim)
	if !ok {
		return nil
	}
	switch delim {
	case '{':
		keys := make(map[string]struct{})
		for dec.More() {
			keyToken, err := dec.Token()
			if err != nil {
				return err
			}
			key, ok := keyToken.(string)
			if !ok {
				return errors.New("invalid object key")
			}
			if _, exists := keys[key]; exists {
				return errors.New("duplicate object key")
			}
			keys[key] = struct{}{}
			if err := scanJSONValue(dec); err != nil {
				return err
			}
		}
		end, err := dec.Token()
		if err != nil || end != json.Delim('}') {
			return errors.New("invalid object")
		}
	case '[':
		for dec.More() {
			if err := scanJSONValue(dec); err != nil {
				return err
			}
		}
		end, err := dec.Token()
		if err != nil || end != json.Delim(']') {
			return errors.New("invalid array")
		}
	default:
		return errors.New("unexpected JSON delimiter")
	}
	return nil
}
