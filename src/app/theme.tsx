import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { withViewTransition } from "./viewTransition";

export type ThemeMode = "system" | "light" | "dark";
type Resolved = "light" | "dark";

interface ThemeCtx {
  mode: ThemeMode;
  resolved: Resolved;
  setMode: (m: ThemeMode) => void;
}

const Ctx = createContext<ThemeCtx | null>(null);
const LS_KEY = "cam.theme";

function readMode(): ThemeMode {
  const v = localStorage.getItem(LS_KEY);
  // A fresh manager should respect the host operating system.  An explicit
  // user choice remains authoritative once it has been saved.
  return v === "system" || v === "light" || v === "dark" ? v : "system";
}

function systemPrefersDark(): boolean {
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;
}

/** The resolved theme for first paint — main.tsx stamps this on <html> before
 *  React mounts, so a light-theme user never sees a dark first frame (the bare
 *  `:root` carries the dark tokens). */
export function resolveInitialTheme(): Resolved {
  const mode = readMode();
  return mode === "system" ? (systemPrefersDark() ? "dark" : "light") : mode;
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeState] = useState<ThemeMode>(readMode);
  const [systemDark, setSystemDark] = useState<boolean>(systemPrefersDark);

  // Track the OS appearance so "system" mode stays live. When the switch is
  // actually visible (mode === system) it cross-fades like a manual theme
  // change, instead of hard-cutting — same withViewTransition + synchronous
  // stamp as setMode, so the transition's "after" snapshot is already themed.
  useEffect(() => {
    const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!mq) return;
    const onChange = () => {
      if (readMode() === "system") {
        withViewTransition(() => {
          setSystemDark(mq.matches);
          document.documentElement.dataset.theme = mq.matches ? "dark" : "light";
        });
      } else {
        setSystemDark(mq.matches);
      }
    };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const resolved: Resolved = mode === "system" ? (systemDark ? "dark" : "light") : mode;

  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
  }, [resolved]);

  const setMode = useCallback((m: ThemeMode) => {
    localStorage.setItem(LS_KEY, m);
    withViewTransition(() => {
      setModeState(m);
      // Set the attribute synchronously too, so the transition's "after"
      // snapshot already reflects the new theme — the effect below is passive
      // and would otherwise run after the snapshot is captured.
      document.documentElement.dataset.theme =
        m === "system" ? (systemPrefersDark() ? "dark" : "light") : m;
    });
  }, []);

  const value = useMemo(() => ({ mode, resolved, setMode }), [mode, resolved, setMode]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useTheme(): ThemeCtx {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useTheme must be used within ThemeProvider");
  return ctx;
}
