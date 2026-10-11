package revocation

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
	"unicode/utf8"
)

var ErrInvalidFingerprint = errors.New("invalid fingerprint")

func FingerprintDigest(anchorSchema, volumeSerial uint32, anchorSHA256, bundleRelativePath string) (string, error) {
	anchorHex := strings.ToLower(anchorSHA256)
	anchor, err := hex.DecodeString(anchorHex)
	if err != nil || len(anchor) != sha256.Size || len(anchorSHA256) != sha256.Size*2 {
		return "", ErrInvalidFingerprint
	}
	path, err := canonicalBundlePath(bundleRelativePath)
	if err != nil || len(path) > int(^uint32(0)) {
		return "", ErrInvalidFingerprint
	}
	var input []byte
	input = append(input, []byte("codex-usb-fingerprint-v1")...)
	input = append(input, 0)
	var num [4]byte
	binary.BigEndian.PutUint32(num[:], anchorSchema)
	input = append(input, num[:]...)
	binary.BigEndian.PutUint32(num[:], volumeSerial)
	input = append(input, num[:]...)
	input = append(input, anchor...)
	binary.BigEndian.PutUint32(num[:], uint32(len(path)))
	input = append(input, num[:]...)
	input = append(input, path...)
	sum := sha256.Sum256(input)
	return hex.EncodeToString(sum[:]), nil
}

func canonicalBundlePath(value string) ([]byte, error) {
	if value == "" || !utf8.ValidString(value) || strings.IndexByte(value, 0) >= 0 || strings.Contains(value, ":") {
		return nil, ErrInvalidFingerprint
	}
	value = strings.ReplaceAll(value, `\`, "/")
	if strings.HasPrefix(value, "/") {
		return nil, ErrInvalidFingerprint
	}
	parts := strings.Split(value, "/")
	for i, part := range parts {
		if part == "" || part == "." || part == ".." {
			return nil, ErrInvalidFingerprint
		}
		parts[i] = asciiLower(part)
	}
	return []byte(strings.Join(parts, "/")), nil
}

func asciiLower(value string) string {
	b := []byte(value)
	for i, c := range b {
		if c >= 'A' && c <= 'Z' {
			b[i] = c + ('a' - 'A')
		}
	}
	return string(b)
}

func SignatureDigest(signatureB64 string) (string, error) {
	sig, err := decodeStdBase64(signatureB64)
	if err != nil || len(sig) != 64 {
		return "", fmt.Errorf("invalid license signature")
	}
	sum := sha256.Sum256(sig)
	return hex.EncodeToString(sum[:]), nil
}
