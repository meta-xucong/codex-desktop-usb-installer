import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { listen } from "@tauri-apps/api/event";

import type { DownloadProgress, OperationSnapshot } from "../../shared/types";
import { I18nProvider } from "../i18n";
import { useDownloadProgress } from "./useDownloadProgress";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

const listenMock = vi.mocked(listen);

function wrapper({ children }: { children: React.ReactNode }) {
  return <I18nProvider>{children}</I18nProvider>;
}

const ACTIVE_SNAPSHOT: OperationSnapshot = {
  id: "op-active",
  kind: "update",
  phase: "downloading",
  progress: { downloaded: 10, total: 100, source: "mirror.test" },
  paused: false,
  cancellable: true,
  interruptible: true,
};

describe("useDownloadProgress", () => {
  let onProgress: ((event: { payload: DownloadProgress }) => void) | undefined;

  beforeEach(() => {
    localStorage.setItem("cam.lang", "en");
    onProgress = undefined;
    listenMock.mockReset();
    listenMock.mockImplementation(async (_event, handler) => {
      onProgress = handler as (event: { payload: DownloadProgress }) => void;
      return () => {
        onProgress = undefined;
      };
    });
  });

  it("latches the first operation id and rejects late events from older ops", async () => {
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "mac://download-progress",
          pauseDownload: vi.fn(async () => true),
          cancelDownload: vi.fn(async () => true),
          getOperationSnapshot: vi.fn(async () => null),
          onError: vi.fn(),
        }),
      { wrapper },
    );

    await act(async () => {
      await result.current.startDlListen();
    });

    act(() => {
      onProgress?.({
        payload: {
          downloaded: 10,
          total: 100,
          source: "a.test",
          operationId: "op-new",
        },
      });
    });
    expect(result.current.dl?.downloaded).toBe(10);
    expect(result.current.activeOperationIdRef.current).toBe("op-new");

    // Late event from a previous operation must not overwrite live progress.
    act(() => {
      onProgress?.({
        payload: {
          downloaded: 999,
          total: 100,
          source: "old.test",
          operationId: "op-old",
        },
      });
    });
    expect(result.current.dl?.downloaded).toBe(10);

    // Same op continues to update.
    act(() => {
      onProgress?.({
        payload: {
          downloaded: 40,
          total: 100,
          source: "a.test",
          operationId: "op-new",
        },
      });
    });
    expect(result.current.dl?.downloaded).toBe(40);
  });

  it("rebuilds a listener bound to a reattached operation id", async () => {
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "win://download-progress",
          pauseDownload: vi.fn(async () => true),
          cancelDownload: vi.fn(async () => true),
          getOperationSnapshot: vi.fn(async () => null),
          onError: vi.fn(),
        }),
      { wrapper },
    );

    result.current.applySnapshotProgress({
      downloaded: 25,
      total: 200,
      source: "mirror.test",
      operationId: "reattach-1",
    });

    await act(async () => {
      await result.current.startDlListen({
        operationId: "reattach-1",
        preserveProgress: true,
      });
    });

    expect(result.current.dl?.downloaded).toBe(25);
    expect(result.current.activeOperationIdRef.current).toBe("reattach-1");

    // Foreign op rejected.
    act(() => {
      onProgress?.({
        payload: {
          downloaded: 1,
          total: 200,
          source: "x",
          operationId: "other",
        },
      });
    });
    expect(result.current.dl?.downloaded).toBe(25);

    // Matching op accepted.
    act(() => {
      onProgress?.({
        payload: {
          downloaded: 80,
          total: 200,
          source: "mirror.test",
          operationId: "reattach-1",
        },
      });
    });
    expect(result.current.dl?.downloaded).toBe(80);

    // Untagged events rejected while bound to a concrete id.
    act(() => {
      onProgress?.({
        payload: {
          downloaded: 5,
          total: 200,
          source: "x",
        },
      });
    });
    expect(result.current.dl?.downloaded).toBe(80);
  });

  it("keeps Windows download bytes separate from install phases and extraction", async () => {
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "win://download-progress",
          pauseDownload: vi.fn(async () => true),
          cancelDownload: vi.fn(async () => true),
          getOperationSnapshot: vi.fn(async () => null),
          onError: vi.fn(),
        }),
      { wrapper },
    );

    await act(async () => {
      await result.current.startDlListen();
    });

    act(() => {
      onProgress?.({
        payload: {
          downloaded: 100,
          total: 100,
          source: "mirror.test",
          phase: "downloading",
          operationId: "op-windows",
        },
      });
    });
    expect(result.current.downloadProgress).toMatchObject({ downloaded: 100, total: 100 });
    expect(result.current.installProgress).toBeNull();

    act(() => {
      onProgress?.({
        payload: {
          downloaded: 100,
          total: 100,
          source: "mirror.test",
          phase: "verifying",
          operationId: "op-windows",
        },
      });
    });
    expect(result.current.downloadProgress).toMatchObject({ downloaded: 100, total: 100 });
    expect(result.current.installProgress).toMatchObject({
      downloaded: 0,
      total: 0,
      phase: "verifying",
    });

    act(() => {
      onProgress?.({
        payload: {
          downloaded: 35,
          total: 100,
          source: "U盘文件解压",
          phase: "applying",
          operationId: "op-windows",
        },
      });
    });
    expect(result.current.installProgress).toMatchObject({
      downloaded: 35,
      total: 100,
      source: "U盘文件解压",
      phase: "applying",
    });

    act(() => {
      onProgress?.({
        payload: {
          downloaded: 100,
          total: 100,
          source: "mirror.test",
          phase: "committing",
          operationId: "op-windows",
        },
      });
    });
    expect(result.current.downloadProgress).toMatchObject({ downloaded: 100, total: 100 });
    expect(result.current.installProgress).toMatchObject({
      downloaded: 0,
      total: 0,
      phase: "committing",
    });

    act(() => {
      onProgress?.({
        payload: {
          downloaded: 100,
          total: 100,
          source: "mirror.test",
          phase: "finishing",
          operationId: "op-windows",
        },
      });
    });
    expect(result.current.installProgress).toMatchObject({
      downloaded: 0,
      total: 0,
      phase: "finishing",
    });

    act(() => result.current.resetStop());
    expect(result.current.downloadProgress).toBeNull();
    expect(result.current.installProgress).toBeNull();
  });

  it("clears split progress on settlement before the next operation", async () => {
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "win://download-progress",
          pauseDownload: vi.fn(async () => true),
          cancelDownload: vi.fn(async () => true),
          getOperationSnapshot: vi.fn(async () => null),
          onError: vi.fn(),
        }),
      { wrapper },
    );
    await act(async () => result.current.startDlListen());
    act(() => {
      onProgress?.({
        payload: {
          downloaded: 50,
          total: 100,
          source: "U盘文件解压",
          phase: "applying",
          operationId: "op-settled",
        },
      });
    });
    expect(result.current.installProgress).not.toBeNull();

    // OfflineHome invokes resetStop from the operation settlement cleanup for
    // both resolved and rejected installs.
    act(() => result.current.resetStop());
    expect(result.current.dl).toBeNull();
    expect(result.current.downloadProgress).toBeNull();
    expect(result.current.installProgress).toBeNull();

    await act(async () => result.current.startDlListen());
    expect(result.current.downloadProgress).toBeNull();
    expect(result.current.installProgress).toBeNull();
  });

  it("resetStop clears the active operation id", async () => {
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "mac://download-progress",
          pauseDownload: vi.fn(async () => true),
          cancelDownload: vi.fn(async () => true),
          getOperationSnapshot: vi.fn(async () => null),
          onError: vi.fn(),
        }),
      { wrapper },
    );

    await act(async () => {
      await result.current.startDlListen({ operationId: "x" });
    });
    act(() => {
      result.current.applySnapshotProgress({ downloaded: 100, total: 100, source: "cached" });
    });
    expect(result.current.activeOperationIdRef.current).toBe("x");
    expect(result.current.dlRef.current?.downloaded).toBe(100);

    act(() => {
      result.current.resetStop();
    });
    expect(result.current.activeOperationIdRef.current).toBeNull();
    expect(result.current.dl).toBeNull();
    expect(result.current.dlRef.current).toBeNull();
  });

  it("startDlListen registers the platform event channel", async () => {
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "mac://download-progress",
          pauseDownload: vi.fn(async () => true),
          cancelDownload: vi.fn(async () => true),
          getOperationSnapshot: vi.fn(async () => null),
          onError: vi.fn(),
        }),
      { wrapper },
    );

    await act(async () => {
      await result.current.startDlListen();
    });

    await waitFor(() => {
      expect(listenMock).toHaveBeenCalledWith("mac://download-progress", expect.any(Function));
    });
  });

  it("classifies a false stop response in an uncancellable phase separately", async () => {
    const onError = vi.fn();
    const cancelDownload = vi.fn(async () => false);
    const getOperationSnapshot = vi.fn(async () => ({
      id: "op-finishing",
      kind: "update" as const,
      phase: "committing" as const,
      progress: null,
      paused: false,
      cancellable: false,
      interruptible: false,
    }));
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "mac://download-progress",
          pauseDownload: vi.fn(async () => false),
          cancelDownload,
          getOperationSnapshot,
          onError,
        }),
      { wrapper },
    );

    await act(async () => {
      await result.current.requestDownloadStop("cancel");
    });

    expect(getOperationSnapshot).toHaveBeenCalledTimes(1);
    expect(cancelDownload).not.toHaveBeenCalled();
    expect(onError).toHaveBeenLastCalledWith(
      expect.objectContaining({
        code: "download_stop_uninterruptible",
        message: "The task has entered an uninterruptible install phase. Wait for it to finish.",
        recoverable: false,
      }),
    );
    expect(result.current.downloadStopBusy).toBe(false);
  });

  it("classifies a structured command rejection as delivered but rejected", async () => {
    const onError = vi.fn();
    const cancelDownload = vi.fn(async (_operationId: string) => {
      throw { code: "operation_busy", message: "backend refused" };
    });
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "win://download-progress",
          pauseDownload: vi.fn(async () => true),
          cancelDownload,
          getOperationSnapshot: vi.fn(async () => ACTIVE_SNAPSHOT),
          onError,
        }),
      { wrapper },
    );

    await act(async () => {
      await result.current.requestDownloadStop("cancel");
    });

    expect(onError).toHaveBeenLastCalledWith(
      expect.objectContaining({
        code: "download_stop_rejected",
        message: "The backend rejected the Cancel request. The task is still running; try again.",
        detail: "backend refused",
        recoverable: true,
      }),
    );
    expect(cancelDownload).toHaveBeenCalledWith(ACTIVE_SNAPSHOT.id);
    expect(result.current.downloadStopBusy).toBe(false);
  });

  it("deduplicates stop requests before React can disable the button", async () => {
    let resolvePause: ((active: boolean) => void) | undefined;
    const pauseDownload = vi.fn(
      () => new Promise<boolean>((resolve) => (resolvePause = resolve)),
    );
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "mac://download-progress",
          pauseDownload,
          cancelDownload: vi.fn(async () => true),
          getOperationSnapshot: vi.fn(async () => ACTIVE_SNAPSHOT),
          onError: vi.fn(),
        }),
      { wrapper },
    );

    let first: Promise<void> | undefined;
    let duplicate: Promise<void> | undefined;
    act(() => {
      first = result.current.requestDownloadStop("pause");
      duplicate = result.current.requestDownloadStop("pause");
    });
    await waitFor(() => expect(pauseDownload).toHaveBeenCalledTimes(1));
    await act(async () => {
      resolvePause?.(false);
      await Promise.all([first, duplicate]);
    });

    expect(result.current.downloadStopBusy).toBe(false);
  });

  it("ignores a stop response after the owning operation resets", async () => {
    const onError = vi.fn();
    let resolvePause: ((active: boolean) => void) | undefined;
    const pauseDownload = vi.fn(
      () => new Promise<boolean>((resolve) => (resolvePause = resolve)),
    );
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "mac://download-progress",
          pauseDownload,
          cancelDownload: vi.fn(async () => true),
          getOperationSnapshot: vi.fn(async () => ACTIVE_SNAPSHOT),
          onError,
        }),
      { wrapper },
    );

    let request: Promise<void> | undefined;
    await act(async () => {
      request = result.current.requestDownloadStop("pause");
      await Promise.resolve();
    });
    expect(pauseDownload).toHaveBeenCalledTimes(1);

    act(() => result.current.resetStop());
    await act(async () => {
      resolvePause?.(false);
      await request;
    });

    expect(onError).toHaveBeenCalledTimes(1);
    expect(onError).toHaveBeenLastCalledWith(null);
    expect(result.current.downloadStopBusy).toBe(false);
  });

  it("treats a successful empty post-stop snapshot as an already-ended operation", async () => {
    const onError = vi.fn();
    const getOperationSnapshot = vi
      .fn<() => Promise<OperationSnapshot | null>>()
      .mockResolvedValueOnce(ACTIVE_SNAPSHOT)
      .mockResolvedValueOnce(null);
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "win://download-progress",
          pauseDownload: vi.fn(async () => true),
          cancelDownload: vi.fn(async () => false),
          getOperationSnapshot,
          onError,
        }),
      { wrapper },
    );

    await act(async () => {
      await result.current.requestDownloadStop("cancel");
    });

    expect(getOperationSnapshot).toHaveBeenCalledTimes(2);
    expect(onError).toHaveBeenCalledTimes(1);
    expect(onError).toHaveBeenLastCalledWith(null);
    expect(result.current.downloadStopBusy).toBe(false);
  });

  it("refuses an unbound pre-arm cancel instead of targeting an unknown operation", async () => {
    const onError = vi.fn();
    const cancelDownload = vi.fn(async () => false);
    const getOperationSnapshot = vi.fn(async () => null);
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "mac://download-progress",
          pauseDownload: vi.fn(async () => false),
          cancelDownload,
          getOperationSnapshot,
          onError,
        }),
      { wrapper },
    );

    await act(async () => {
      await result.current.requestDownloadStop("cancel");
    });

    expect(getOperationSnapshot).toHaveBeenCalledTimes(1);
    expect(cancelDownload).not.toHaveBeenCalled();
    expect(onError).toHaveBeenLastCalledWith(
      expect.objectContaining({
        code: "download_stop_not_delivered",
        recoverable: true,
      }),
    );
    expect(result.current.downloadStopBusy).toBe(false);
  });

  it("does not retarget an old renderer request when the backend reports a new operation", async () => {
    const onError = vi.fn();
    const cancelDownload = vi.fn(async (_operationId: string) => true);
    const getOperationSnapshot = vi.fn(async () => ({
      ...ACTIVE_SNAPSHOT,
      id: "op-new",
    }));
    const { result } = renderHook(
      () =>
        useDownloadProgress({
          eventName: "mac://download-progress",
          pauseDownload: vi.fn(async () => true),
          cancelDownload,
          getOperationSnapshot,
          onError,
        }),
      { wrapper },
    );

    await act(async () => {
      await result.current.startDlListen({ operationId: "op-old" });
      await result.current.requestDownloadStop("cancel");
    });

    expect(getOperationSnapshot).toHaveBeenCalledTimes(1);
    expect(cancelDownload).not.toHaveBeenCalled();
    expect(result.current.downloadStopBusy).toBe(false);
  });
});
