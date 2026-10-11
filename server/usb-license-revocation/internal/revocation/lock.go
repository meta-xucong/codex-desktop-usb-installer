package revocation

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strconv"
	"time"
)

// A crash leaves the lock marker behind deliberately; operators must verify
// no process is active before removing it instead of guessing it is stale.
func acquireFileLock(ctx context.Context, path string, timeout time.Duration) (func(), error) {
	if err := os.MkdirAll(filepath.Dir(path), 0750); err != nil {
		return nil, err
	}
	deadline := time.Now().Add(timeout)
	for {
		f, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
		if err == nil {
			_, writeErr := f.WriteString(strconv.Itoa(os.Getpid()) + "\n")
			if writeErr == nil {
				writeErr = f.Sync()
			}
			closeErr := f.Close()
			if writeErr != nil || closeErr != nil {
				_ = os.Remove(path)
				return nil, errors.Join(writeErr, closeErr)
			}
			return func() { _ = os.Remove(path) }, nil
		}
		if !errors.Is(err, os.ErrExist) {
			return nil, err
		}
		if time.Now().After(deadline) {
			return nil, errors.New("state lock busy")
		}
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-time.After(20 * time.Millisecond):
		}
	}
}
