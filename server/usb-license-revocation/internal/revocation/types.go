package revocation

import "time"

const (
	ProductID            = "codex-usb-oneclick"
	RegistrationDomain   = "codex-usb-revocation-registration-v1"
	RegistrationDomainV2 = "codex-usb-revocation-registration-v2"
	RegistrationSchema   = 1
	RegistrationSchemaV2 = 2
	ResponseSchema       = 1
	RequestSchema        = 1
	MaxRequestBytes      = 64 << 10
	MaxRegistrationBytes = 16 << 10
)

type RegistrationEnvelope struct {
	PayloadB64   string `json:"payloadB64"`
	SignatureB64 string `json:"signatureB64"`
}

type RegistrationUploadRequest struct {
	Registration RegistrationEnvelope `json:"registration"`
}

type RegistrationPayload struct {
	SchemaVersion          int     `json:"schemaVersion"`
	Domain                 string  `json:"domain"`
	Product                string  `json:"product"`
	InstallationID         string  `json:"installationId"`
	LicenseKeyID           string  `json:"licenseKeyId"`
	LicenseSignatureSHA256 string  `json:"licenseSignatureSha256"`
	FingerprintSHA256      string  `json:"fingerprintSha256"`
	IssuedAt               string  `json:"issuedAt"`
	USBAssetID             *string `json:"usbAssetId,omitempty"`
}

type ObservedFingerprint struct {
	VolumeSerial       uint32 `json:"volumeSerial"`
	AnchorSHA256       string `json:"anchorSha256"`
	BundleRelativePath string `json:"bundleRelativePath"`
	AnchorSchema       uint32 `json:"anchorSchema"`
}

type CheckRequest struct {
	SchemaVersion          int                  `json:"schemaVersion"`
	RequestID              string               `json:"requestId"`
	Product                string               `json:"product"`
	InstallationID         string               `json:"installationId"`
	LicenseSignatureSHA256 string               `json:"licenseSignatureSha256"`
	Registration           RegistrationEnvelope `json:"registration"`
	ObservedFingerprint    ObservedFingerprint  `json:"observedFingerprint"`
}

type ResponsePayload struct {
	SchemaVersion          int       `json:"schemaVersion"`
	Product                string    `json:"product"`
	InstallationID         string    `json:"installationId"`
	LicenseSignatureSHA256 string    `json:"licenseSignatureSha256"`
	RequestID              string    `json:"requestId"`
	Status                 string    `json:"status"`
	StateVersion           int64     `json:"stateVersion"`
	IssuedAt               time.Time `json:"issuedAt"`
	SigningKeyID           string    `json:"signingKeyId"`
}

type SignedEnvelope struct {
	PayloadB64   string `json:"payloadB64"`
	SignatureB64 string `json:"signatureB64"`
}

type LicenseRecord struct {
	Product                string
	InstallationID         string
	KeyID                  string
	LicenseSignatureSHA256 string
	FingerprintDigest      string
	USBAssetID             string `json:"usbAssetId,omitempty"`
	Status                 string
	StateVersion           int64
	CreatedAt              time.Time
	UpdatedAt              time.Time
	RevokedAt              *time.Time
	RevokeReason           string
}

type AuditEvent struct {
	Sequence     uint64        `json:"sequence"`
	PreviousHash string        `json:"previousHash"`
	EventID      string        `json:"eventId"`
	Actor        string        `json:"actor"`
	Action       string        `json:"action"`
	Record       LicenseRecord `json:"record"`
	Reason       string        `json:"reason,omitempty"`
	OccurredAt   time.Time     `json:"occurredAt"`
	Hash         string        `json:"hash"`
}

type JournalHead struct {
	Sequence uint64 `json:"sequence"`
	Hash     string `json:"hash"`
}

type Keyring struct {
	Keys map[string]string `json:"keys"`
}

type StatusSigningKey struct {
	KeyID string
	Key   []byte
}
