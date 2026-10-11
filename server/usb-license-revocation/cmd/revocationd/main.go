package main

import (
	"context"
	"errors"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"example.com/codex/usb-license-revocation/internal/revocation"
)

func main() {
	logger := slog.New(slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{Level: slog.LevelInfo}))
	cfg, httpCfg, err := revocation.ConfigFromEnv()
	if err != nil {
		logger.Error("service configuration invalid")
		os.Exit(2)
	}
	keys, err := revocation.LoadKeyring(cfg.LicenseKeysPath)
	if err != nil {
		logger.Error("license keyring unavailable")
		os.Exit(2)
	}
	signer, err := revocation.LoadSigningKey(cfg.SigningKeyPath, cfg.SigningKeyID)
	if err != nil {
		logger.Error("status signing key unavailable")
		os.Exit(2)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	store, err := revocation.OpenStore(ctx, cfg, keys, signer)
	cancel()
	if err != nil {
		logger.Error("service state initialization failed")
		os.Exit(1)
	}
	defer store.Close()
	server, err := revocation.NewHTTPServer(store, keys, httpCfg, logger)
	if err != nil {
		logger.Error("service listener configuration invalid")
		os.Exit(2)
	}
	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGINT, syscall.SIGTERM)
	go func() {
		<-stop
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = server.Shutdown(ctx)
	}()
	logger.Info("revocation service starting", "listen", httpCfg.ListenAddress)
	if err := server.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		logger.Error("revocation service stopped")
		os.Exit(1)
	}
}
