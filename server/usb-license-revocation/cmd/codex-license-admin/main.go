package main

import (
	"context"
	"crypto/ed25519"
	"flag"
	"fmt"
	"log"
	"os"
	"os/user"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"example.com/codex/usb-license-revocation/internal/revocation"
)

func main() {
	if len(os.Args) < 2 {
		usage()
	}
	switch os.Args[1] {
	case "keygen":
		keygen(os.Args[2:])
	case "import":
		importRegistration(os.Args[2:])
	case "revoke":
		changeState("revoke", os.Args[2:])
	case "reactivate":
		changeState("reactivate", os.Args[2:])
	case "inspect":
		inspect(os.Args[2:])
	case "backup":
		backup(os.Args[2:])
	case "verify":
		verify(os.Args[2:])
	case "recover":
		recoverState(os.Args[2:])
	default:
		usage()
	}
}

func usage() {
	fmt.Fprintln(os.Stderr, "usage: codex-license-admin {keygen|import|revoke|reactivate|inspect|backup|recover|verify} [options]")
	os.Exit(2)
}

func keygen(args []string) {
	fs := flag.NewFlagSet("keygen", flag.ExitOnError)
	out := fs.String("out", "", "status signing private-key file")
	id := fs.String("key-id", "", "public signing key identifier")
	_ = fs.Parse(args)
	if *out == "" || !regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`).MatchString(*id) || strings.TrimSpace(*id) != *id {
		usage()
	}
	if err := os.MkdirAll(filepath.Dir(*out), 0700); err != nil {
		fatal("cannot create key directory")
	}
	publicB64, _, err := revocation.GenerateSigningKey(*out)
	if err != nil {
		fatal("cannot generate signing key")
	}
	fmt.Printf("keyId: %s\npublicKeyB64: %s\nprivateKeyFile: %s\n", *id, publicB64, *out)
}

func importRegistration(args []string) {
	fs := flag.NewFlagSet("import", flag.ExitOnError)
	file := fs.String("registration-file", "", "signed USB registration envelope JSON")
	initial := fs.String("initial-status", "", "must be active for a new record")
	actor := fs.String("actor", currentActor(), "administrator identity for audit")
	_ = fs.Parse(args)
	if *file == "" || *initial != "active" {
		fatal("import requires --registration-file and --initial-status active")
	}
	store, keys := openAdminStore()
	defer store.Close()
	f, err := os.Open(*file)
	if err != nil {
		fatal("cannot read registration file")
	}
	envelope, _, err := revocation.ReadRegistrationEnvelope(f, keys)
	_ = f.Close()
	if err != nil {
		fatal("registration proof rejected")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	record, err := store.Import(ctx, envelope, *actor, time.Now().UTC())
	if err != nil {
		fatal("registration import failed")
	}
	fmt.Printf("imported installationId=%s status=%s stateVersion=%d\n", record.InstallationID, record.Status, record.StateVersion)
}

func changeState(action string, args []string) {
	fs := flag.NewFlagSet(action, flag.ExitOnError)
	id := fs.String("installation-id", "", "installation ID from registration")
	reason := fs.String("reason", "", "short non-sensitive audit reason")
	actor := fs.String("actor", currentActor(), "administrator identity for audit")
	_ = fs.Parse(args)
	if *id == "" || *reason == "" {
		fatal(action + " requires --installation-id and --reason")
	}
	store, _ := openAdminStore()
	defer store.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	record, changed, err := store.ChangeState(ctx, *id, action, *actor, *reason, time.Now().UTC())
	if err != nil {
		fatal(action + " failed")
	}
	result := "updated"
	if !changed {
		result = "already_at_target"
	}
	fmt.Printf("%s installationId=%s status=%s stateVersion=%d result=%s\n", action, record.InstallationID, record.Status, record.StateVersion, result)
}

func inspect(args []string) {
	fs := flag.NewFlagSet("inspect", flag.ExitOnError)
	id := fs.String("installation-id", "", "installation ID")
	_ = fs.Parse(args)
	if *id == "" {
		usage()
	}
	store, _ := openAdminStore()
	defer store.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	record, err := store.Inspect(ctx, *id)
	if err != nil {
		fatal("license not available")
	}
	fmt.Printf("product=%s installationId=%s keyId=%s status=%s stateVersion=%d createdAt=%s updatedAt=%s revokedAt=%s revokeReason=%s\n", record.Product, record.InstallationID, record.KeyID, record.Status, record.StateVersion, record.CreatedAt.Format(time.RFC3339), record.UpdatedAt.Format(time.RFC3339), formatTime(record.RevokedAt), record.RevokeReason)
}

func backup(args []string) {
	fs := flag.NewFlagSet("backup", flag.ExitOnError)
	out := fs.String("out-dir", "", "new directory on independent backup storage")
	_ = fs.Parse(args)
	if *out == "" {
		usage()
	}
	store, _ := openAdminStore()
	defer store.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	if err := store.Backup(ctx, *out); err != nil {
		fatal("backup failed")
	}
	fmt.Printf("backup created: %s\n", *out)
}

func verify(args []string) {
	fs := flag.NewFlagSet("verify", flag.ExitOnError)
	_ = fs.Parse(args)
	store, _ := openAdminStoreAllowUnknown()
	defer store.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if err := store.VerifyRecovery(ctx); err != nil {
		fatal("state is unknown; service must remain unavailable")
	}
	if _, err := os.Stat(configuredRestoreMarker()); err == nil {
		fmt.Println("state and audit history verified; restore marker remains and still disables API signing")
	} else {
		fmt.Println("state and audit history verified")
	}
}

func recoverState(args []string) {
	fs := flag.NewFlagSet("recover", flag.ExitOnError)
	_ = fs.Parse(args)
	store, _ := openAdminStoreAllowUnknown()
	defer store.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	if err := store.Recover(ctx); err != nil {
		fatal("recovery could not prove complete state; keep service stopped")
	}
	fmt.Println("database replayed from verified audit history; restore marker remains")
}

func openAdminStore() (*revocation.Store, map[string]ed25519.PublicKey) {
	store, keys := openAdminStoreAllowUnknown()
	if !store.IsHealthy() {
		store.Close()
		fatal("service state is unknown")
	}
	return store, keys
}

func openAdminStoreAllowUnknown() (*revocation.Store, map[string]ed25519.PublicKey) {
	cfg, _, err := revocation.ConfigFromEnv()
	if err != nil {
		fatal("service configuration invalid")
	}
	keys, err := revocation.LoadKeyring(cfg.LicenseKeysPath)
	if err != nil {
		fatal("license keyring unavailable")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	store, err := revocation.OpenStore(ctx, cfg, keys, revocation.StatusSigningKey{})
	if err != nil {
		fatal("service state unavailable")
	}
	return store, keys
}

func configuredRestoreMarker() string {
	cfg, _, err := revocation.ConfigFromEnv()
	if err != nil {
		return ""
	}
	return cfg.RestoreMarkerPath
}

func currentActor() string {
	if v := os.Getenv("SUDO_USER"); v != "" {
		return v
	}
	if u, err := user.Current(); err == nil && u.Username != "" {
		return strings.ReplaceAll(u.Username, "\\", "_")
	}
	return "local-admin"
}

func formatTime(v *time.Time) string {
	if v == nil {
		return "-"
	}
	return v.UTC().Format(time.RFC3339)
}
func fatal(message string) { log.Print(message); os.Exit(1) }
