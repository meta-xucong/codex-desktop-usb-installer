import { beforeEach, describe, expect, it, vi } from "vitest";

import { resolveInitialTheme } from "./theme";

function setSystemTheme(prefersDark: boolean) {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn(() => ({
      matches: prefersDark,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  });
}

describe("theme defaults", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("follows a light system preference when no choice was saved", () => {
    setSystemTheme(false);
    expect(resolveInitialTheme()).toBe("light");
  });

  it("follows a dark system preference when no choice was saved", () => {
    setSystemTheme(true);
    expect(resolveInitialTheme()).toBe("dark");
  });

  it("keeps an explicit saved choice over the system preference", () => {
    setSystemTheme(false);
    localStorage.setItem("cam.theme", "dark");
    expect(resolveInitialTheme()).toBe("dark");
  });
});
