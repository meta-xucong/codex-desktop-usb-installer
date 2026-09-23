import { useCallback, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { errorCode } from "../../services/managerApi";
import type {
  DownloadProgress,
  OperationPhase,
  OperationSnapshot,
} from "../../shared/types";
import {
  contextualFailure,
  messageFailure,
  type FailureSurface,
} from "../errorCopy";
import { useI18n } from "../i18n";
import { useCountUp } from "../useCountUp";
import type { DownloadStopIntent } from "./ProgressScreen";

export type StartDlListenOptions = {
  /** When reattaching, bind to this operation id and reject foreign events. */
  operationId?: string | null;
  /** Keep the last known progress (reattach) instead of clearing to empty. */
  preserveProgress?: boolean;
};

/** The live-download state machine shared by the Mac and Windows homes: the
 *  progress bytes + eased readouts, the pause/cancel intent, and the backend
 *  stop request. Platform differences are injected — the event channel name and
 *  the pause/cancel commands. Errors surface through `onError` so the host view
 *  can drive its own banner (with optional raw detail disclosure). */
export function useDownloadProgress(opts: {
  eventName: string;
  pauseDownload: (operationId: string) => Promise<boolean>;
  cancelDownload: (operationId: string) => Promise<boolean>;
  getOperationSnapshot: () => Promise<OperationSnapshot | null>;
  onError: (failure: FailureSurface | null) => void;
}) {
  const { eventName, pauseDownload, cancelDownload, getOperationSnapshot, onError } = opts;
  const { t } = useI18n();

  const [dl, setDl] = useState<DownloadProgress | null>(null);
  const [speed, setSpeed] = useState(0);
  const dlSample = useRef<{ t: number; bytes: number } | null>(null);
  const [downloadStop, setDownloadStop] = useState<DownloadStopIntent | null>(null);
  const [downloadStopBusy, setDownloadStopBusy] = useState(false);
  const downloadStopRef = useRef<DownloadStopIntent | null>(null);
  // Monotonic generation for stop IPC. resetStop invalidates a response from an
  // operation that settled while the command or its snapshot probe was pending.
  const downloadStopRequestIdRef = useRef(0);
  // Latest live progress, read at pause time to snapshot the paused figures
  // (the `dl` state is cleared when the perform/install call unwinds).
  const dlRef = useRef<DownloadProgress | null>(null);
  // Active operation id: latched from the first tagged event, or set on reattach.
  // Events carrying a *different* id are dropped so a late packet from op A
  // cannot paint onto op B after a new task started (or after reload).
  const activeOperationIdRef = useRef<string | null>(null);

  // Smoothly roll the live figures instead of snapping on every progress event.
  const dlPctTarget = dl && dl.total > 0 ? Math.min(100, (dl.downloaded / dl.total) * 100) : 0;
  const dlPct = useCountUp(dlPctTarget);
  const dlBytes = useCountUp(dl?.downloaded ?? 0);
  const dlSpeed = useCountUp(speed);

  const onDlProgress = useCallback((event: { payload: DownloadProgress }) => {
    const p = event.payload;
    const eventOpId = typeof p.operationId === "string" && p.operationId ? p.operationId : null;
    const activeId = activeOperationIdRef.current;

    if (eventOpId) {
      if (activeId && eventOpId !== activeId) {
        // Late event for a previous operation — ignore.
        return;
      }
      if (!activeId) {
        // First tagged event for this listen session: latch the id.
        activeOperationIdRef.current = eventOpId;
      }
    } else if (activeId) {
      // We already bound to a concrete op (reattach); untagged events are foreign.
      return;
    }

    setDl(p);
    dlRef.current = p;
    const now = Date.now();
    const prev = dlSample.current;
    if (!prev) {
      dlSample.current = { t: now, bytes: p.downloaded };
    } else if (now > prev.t + 400) {
      setSpeed((p.downloaded - prev.bytes) / ((now - prev.t) / 1000));
      dlSample.current = { t: now, bytes: p.downloaded };
    }
  }, []);

  const startDlListen = useCallback(
    async (options?: StartDlListenOptions) => {
      activeOperationIdRef.current = options?.operationId ?? null;
      if (!options?.preserveProgress) {
        setDl(null);
        dlRef.current = null;
        setSpeed(0);
        dlSample.current = null;
      }
      try {
        return await listen<DownloadProgress>(eventName, onDlProgress);
      } catch {
        // Non-Tauri (web preview): no event bus — nothing to clean up.
        return () => {};
      }
    },
    [eventName, onDlProgress],
  );

  /** Seed progress state from a backend snapshot (reload reattach). */
  const applySnapshotProgress = useCallback((
    progress: DownloadProgress | null | undefined,
    phase?: OperationPhase,
  ) => {
    if (!progress) return;
    const next = phase ? { ...progress, phase } : progress;
    setDl(next);
    dlRef.current = next;
    setSpeed(0);
    dlSample.current = { t: Date.now(), bytes: next.downloaded };
  }, []);

  const clearActiveOperation = useCallback(() => {
    activeOperationIdRef.current = null;
  }, []);

  const requestDownloadStop = useCallback(
    async (intent: DownloadStopIntent) => {
      // React state does not disable the button until the next render. The ref
      // closes that gap so a fast double-click cannot enqueue two stop commands.
      if (downloadStopRef.current) return;
      const requestId = ++downloadStopRequestIdRef.current;
      // Capture the owner synchronously. A late IPC response must never infer a
      // newly-started operation as its target.
      const requestedOperationId = activeOperationIdRef.current;
      onError(null);
      setDownloadStop(intent);
      setDownloadStopBusy(true);
      downloadStopRef.current = intent;

      const isCurrent = () =>
        downloadStopRequestIdRef.current === requestId && downloadStopRef.current === intent;
      const clearCurrent = () => {
        if (!isCurrent()) return false;
        downloadStopRef.current = null;
        setDownloadStop(null);
        setDownloadStopBusy(false);
        return true;
      };
      const probeSnapshot = async () => {
        try {
          return { available: true as const, snapshot: await getOperationSnapshot() };
        } catch {
          return { available: false as const, snapshot: null };
        }
      };
      const uninterruptibleFailure = () =>
        messageFailure(
          t("progress.stopUninterruptible"),
          "download_stop_uninterruptible",
          false,
        );

      // Snapshot is authoritative when available. This closes the UI-event lag
      // window where the final progress packet has not arrived but the backend
      // has already entered committing/finishing.
      const before = await probeSnapshot();
      if (!isCurrent()) return;
      const operationId = requestedOperationId ?? before.snapshot?.id ?? null;
      if (!operationId) {
        clearCurrent();
        onError(
          messageFailure(
            t("progress.stopDeliveryFailed", { action: t(`progress.${intent}`) }),
            "download_stop_not_delivered",
          ),
        );
        return;
      }
      // If this renderer was already bound to A but the backend now reports B,
      // the request is stale. Let A's completion/reset drive the UI; do not arm B.
      if (before.snapshot && before.snapshot.id !== operationId) {
        clearCurrent();
        return;
      }
      if (!activeOperationIdRef.current) {
        activeOperationIdRef.current = operationId;
      }
      if (before.snapshot && !before.snapshot.cancellable) {
        clearCurrent();
        onError(uninterruptibleFailure());
        return;
      }

      try {
        const active =
          intent === "pause"
            ? await pauseDownload(operationId)
            : await cancelDownload(operationId);
        if (!active) {
          const after = await probeSnapshot();
          if (!isCurrent()) return;
          // A successful empty snapshot means the operation settled before the
          // stop response. Do not paint a stale failure over its result screen.
          if (before.snapshot && after.available && !after.snapshot) {
            clearCurrent();
            return;
          }
          const uninterruptible = Boolean(after.snapshot && !after.snapshot.cancellable);
          clearCurrent();
          onError(
            uninterruptible
              ? uninterruptibleFailure()
              : messageFailure(
                  t("progress.stopRejected", { action: t(`progress.${intent}`) }),
                  "download_stop_rejected",
                ),
          );
        }
      } catch (cause) {
        const after = await probeSnapshot();
        if (!isCurrent()) return;
        if (before.snapshot && after.available && !after.snapshot) {
          clearCurrent();
          return;
        }
        if (after.snapshot && !after.snapshot.cancellable) {
          clearCurrent();
          onError(uninterruptibleFailure());
          return;
        }
        const delivered = errorCode(cause) !== null;
        clearCurrent();
        onError(
          contextualFailure(
            cause,
            t,
            t(delivered ? "progress.stopRejected" : "progress.stopDeliveryFailed", {
              action: t(`progress.${intent}`),
            }),
            delivered ? "download_stop_rejected" : "download_stop_not_delivered",
          ),
        );
      }
    },
    [pauseDownload, cancelDownload, getOperationSnapshot, onError, t],
  );

  // Clear the transfer + stop state when a perform/install call unwinds.
  const resetStop = useCallback(() => {
    downloadStopRequestIdRef.current += 1;
    setDl(null);
    dlRef.current = null;
    setSpeed(0);
    dlSample.current = null;
    setDownloadStop(null);
    setDownloadStopBusy(false);
    downloadStopRef.current = null;
    activeOperationIdRef.current = null;
  }, []);

  return {
    dl,
    setDl,
    dlRef,
    dlPct,
    dlBytes,
    dlSpeed,
    downloadStop,
    downloadStopBusy,
    downloadStopRef,
    activeOperationIdRef,
    startDlListen,
    applySnapshotProgress,
    clearActiveOperation,
    requestDownloadStop,
    resetStop,
  };
}
