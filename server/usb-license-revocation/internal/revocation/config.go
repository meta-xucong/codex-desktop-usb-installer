package revocation

import (
	"fmt"
	"os"
	"strconv"
)

func ConfigFromEnv() (Config, HTTPConfig, error) {
	get := func(name, fallback string) string {
		if value := os.Getenv(name); value != "" {
			return value
		}
		return fallback
	}
	intEnv := func(name string, fallback int) (int, error) {
		value := os.Getenv(name)
		if value == "" {
			return fallback, nil
		}
		parsed, err := strconv.Atoi(value)
		if err != nil || parsed < 1 {
			return 0, fmt.Errorf("invalid %s", name)
		}
		return parsed, nil
	}
	rate, err := intEnv("USB_REVOCATION_MAX_RATE_PER_MINUTE", 60)
	if err != nil {
		return Config{}, HTTPConfig{}, err
	}
	burst, err := intEnv("USB_REVOCATION_RATE_BURST", 10)
	if err != nil {
		return Config{}, HTTPConfig{}, err
	}
	cfg := Config{
		DatabasePath:      get("USB_REVOCATION_DATABASE", "/var/lib/codex-usb-license-revocation/state.db"),
		AuditPath:         get("USB_REVOCATION_AUDIT", "/var/lib/codex-usb-license-revocation/audit.jsonl"),
		AuditBackupPath:   get("USB_REVOCATION_AUDIT_BACKUP", "/var/backups/codex-usb-license-revocation/audit.jsonl"),
		RestoreMarkerPath: get("USB_REVOCATION_RESTORE_MARKER", "/var/lib/codex-usb-license-revocation/RESTORE-IN-PROGRESS"),
		LicenseKeysPath:   get("USB_REVOCATION_LICENSE_KEYRING", "/etc/codex-usb-license-revocation/license-keyring.json"),
		SigningKeyPath:    get("USB_REVOCATION_SIGNING_KEY", "/etc/codex-usb-license-revocation/status-signing-key"),
		SigningKeyID:      get("USB_REVOCATION_SIGNING_KEY_ID", ""),
		MaxRatePerMinute:  rate, RateBurst: burst,
	}
	adminCookieSecure, err := strconv.ParseBool(get("USB_REVOCATION_ADMIN_COOKIE_SECURE", "true"))
	if err != nil {
		return Config{}, HTTPConfig{}, fmt.Errorf("invalid USB_REVOCATION_ADMIN_COOKIE_SECURE")
	}
	httpCfg := HTTPConfig{
		ListenAddress:    get("USB_REVOCATION_LISTEN_ADDR", "127.0.0.1:8787"),
		MaxRatePerMinute: rate, RateBurst: burst,
		AdminUsername:      get("USB_REVOCATION_ADMIN_USERNAME", ""),
		AdminPassword:      get("USB_REVOCATION_ADMIN_PASSWORD", ""),
		AdminAuthStatePath: get("USB_REVOCATION_ADMIN_AUTH_STATE", "/var/lib/codex-usb-license-revocation/admin-auth.json"),
		AdminTOTPSecret:    get("USB_REVOCATION_ADMIN_TOTP_SECRET", ""),
		AdminOrigin:        get("USB_REVOCATION_ADMIN_ORIGIN", ""),
		AdminCookieSecure:  adminCookieSecure,
	}
	if httpCfg.AdminUsername != "" {
		httpCfg.modelCatalog = defaultModelCatalogRunnerConfig()
	}
	if (httpCfg.AdminUsername == "") != (httpCfg.AdminPassword == "") {
		return Config{}, HTTPConfig{}, fmt.Errorf("USB_REVOCATION_ADMIN_USERNAME and USB_REVOCATION_ADMIN_PASSWORD must be configured together")
	}
	if httpCfg.AdminUsername != "" {
		if !validID(httpCfg.AdminUsername) || len(httpCfg.AdminUsername) > 64 || len(httpCfg.AdminPassword) < 12 || len(httpCfg.AdminPassword) > 1024 {
			return Config{}, HTTPConfig{}, fmt.Errorf("USB revocation admin credentials are invalid")
		}
		if httpCfg.AdminAuthStatePath == "" {
			return Config{}, HTTPConfig{}, fmt.Errorf("USB_REVOCATION_ADMIN_AUTH_STATE is required")
		}
		if httpCfg.AdminTOTPSecret != "" {
			if _, err := decodeAdminTOTPSecret(httpCfg.AdminTOTPSecret); err != nil {
				return Config{}, HTTPConfig{}, fmt.Errorf("USB_REVOCATION_ADMIN_TOTP_SECRET is invalid")
			}
		}
		if err := validateAdminOrigin(httpCfg.AdminOrigin, httpCfg.AdminCookieSecure); err != nil {
			return Config{}, HTTPConfig{}, fmt.Errorf("USB_REVOCATION_ADMIN_ORIGIN is required and invalid: %w", err)
		}
	}
	return cfg, httpCfg, nil
}
