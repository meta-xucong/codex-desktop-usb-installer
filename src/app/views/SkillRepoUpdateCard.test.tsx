import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { managerApi } from "../../services/managerApi";
import type {
  SkillRepoApplyReport,
  SkillRepoRollbackReport,
  SkillRepoUpdateInfo,
} from "../../shared/types";
import { I18nProvider } from "../i18n";
import { SkillRepoUpdateCard } from "./SkillRepoUpdateCard";

vi.mock("../../services/managerApi", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../services/managerApi")>();
  return {
    ...actual,
    managerApi: {
      ...actual.managerApi,
      skillRepoCheckUpdate: vi.fn(),
      skillRepoApplyUpdate: vi.fn(),
      skillRepoRollback: vi.fn(),
    },
  };
});

const api = vi.mocked(managerApi);
const INFO: SkillRepoUpdateInfo = {
  schemaVersion: 1,
  currentCommit: null,
  latestCommit: "a".repeat(40),
  sourceKind: "relay",
  archiveSize: 12,
  archiveSha256: "b".repeat(64),
  expected: {
    schemaVersion: 1,
    repo: "meta-xucong/skill_hub",
    branch: "master",
    commit: "a".repeat(40),
    sourceKind: "relay",
    archiveSize: 12,
    archiveSha256: "b".repeat(64),
  },
  updateAvailable: true,
  installableCount: 2,
  conflictCount: 1,
  skippedCount: 0,
  warning: "unsigned",
};
const REPORT: SkillRepoApplyReport = {
  schemaVersion: 1,
  status: "success",
  identity: INFO.expected,
  applied: [{ id: "demo", status: "applied", code: null, fileCount: 1 }],
  conflicts: [],
  skipped: [],
  failed: [],
  lastGoodGeneration: "generation",
  reloadedRequired: true,
  logPath: null,
};
const PARTIAL_REPORT: SkillRepoApplyReport = {
  ...REPORT,
  status: "partial",
  applied: [{ id: "new-skill", status: "applied", code: null, fileCount: 2 }],
  conflicts: [{
    id: "user-skill",
    status: "conflict",
    code: "SKILL_REPO_USER_CONFLICT",
    fileCount: 3,
  }],
  skipped: [{
    id: "skipped-skill",
    status: "skipped",
    code: "SKILL_REPO_LEDGER_INVALID",
    fileCount: 1,
  }],
};
const ROLLBACK_REPORT: SkillRepoRollbackReport = {
  schemaVersion: 1,
  status: "success",
  restoredGeneration: "generation",
  restoredSkills: ["new-skill", "another-skill"],
  reloadedRequired: true,
  logPath: null,
};

describe("SkillRepoUpdateCard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(window, "confirm").mockReturnValue(true);
  });

  function openDetails() {
    fireEvent.click(screen.getByRole("button", { name: "Install Skill Hub" }));
  }

  it("keeps the home page compact and opens the Skill Hub details on demand", () => {
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    expect(screen.getByRole("button", { name: "Install Skill Hub" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check updates" })).not.toBeInTheDocument();
    expect(screen.queryByText(/independently unsigned/i)).not.toBeInTheDocument();

    openDetails();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check updates" })).toBeInTheDocument();
    expect(screen.getByText(/Find and install extra Skills/)).toBeInTheDocument();
  });

  it("checks then applies the frozen expected identity", async () => {
    api.skillRepoCheckUpdate.mockResolvedValue(INFO);
    api.skillRepoApplyUpdate.mockResolvedValue(REPORT);
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
    await waitFor(() => expect(screen.getByText(/Current/)).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "Update now" }));
    await waitFor(() => expect(api.skillRepoApplyUpdate).toHaveBeenCalledWith(INFO.expected));
    expect(screen.getByText(/Update success/)).toBeTruthy();
  });

  it("does not turn the compatibility warning field into a home-page warning", async () => {
    api.skillRepoCheckUpdate.mockResolvedValue(INFO);
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
    await waitFor(() => expect(screen.getByText(/Current/)).toBeInTheDocument());

    expect(screen.queryByText("unsigned")).not.toBeInTheDocument();
    expect(screen.queryByText(/independently unsigned/i)).not.toBeInTheDocument();
  });

  it("releases the checking state when the bridge never resolves", async () => {
    vi.useFakeTimers();
    try {
      api.skillRepoCheckUpdate.mockReturnValue(new Promise<never>(() => undefined));
      render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

      openDetails();
      fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
      expect(screen.getByRole("button", { name: "Checking…" })).toBeDisabled();

      await act(async () => {
        vi.advanceTimersByTime(50_000);
        await Promise.resolve();
      });

      expect(screen.getByRole("alert")).toHaveTextContent("The operation timed out");
      expect(screen.getByRole("button", { name: "Check updates" })).toBeEnabled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows partial results with conflict and skipped counts", async () => {
    api.skillRepoCheckUpdate.mockResolvedValue(INFO);
    api.skillRepoApplyUpdate.mockResolvedValue(PARTIAL_REPORT);
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Update now" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Update now" }));

    await waitFor(() => expect(screen.getByTestId("skill-repo-apply-result")).toHaveTextContent("Update partial"));
    expect(screen.getByTestId("skill-repo-apply-counts")).toHaveTextContent(
      /Installable 1.*conflicts 1.*skipped 1/,
    );
  });

  it("uses safe localized copy when apply fails without rendering raw URL or path", async () => {
    const rawFailure = "failed to apply https://example.test/archive.zip at C:\\Users\\secret\\skills";
    api.skillRepoCheckUpdate.mockResolvedValue(INFO);
    api.skillRepoApplyUpdate.mockRejectedValue({
      code: "SKILL_REPO_STALE",
      message: rawFailure,
    });
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Update now" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Update now" }));

    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByRole("alert")).not.toHaveTextContent(rawFailure);
    expect(screen.getByRole("alert")).not.toHaveTextContent("https://example.test/archive.zip");
    expect(screen.getByRole("alert")).not.toHaveTextContent("C:\\Users\\secret\\skills");
    expect(screen.queryByRole("button", { name: "Update now" })).not.toBeInTheDocument();
  });

  it("confirms and invokes the independent rollback action", async () => {
    api.skillRepoRollback.mockResolvedValue(ROLLBACK_REPORT);
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Rollback last version" }));

    await waitFor(() => expect(api.skillRepoRollback).toHaveBeenCalledOnce());
    expect(window.confirm).toHaveBeenCalledWith(
      "Roll back the last complete hot-update generation?",
    );
    expect(screen.getByText(/Rolled back 2 Skills/)).toBeInTheDocument();
  });

  it("does not claim rollback success for a blocked report", async () => {
    api.skillRepoRollback.mockResolvedValue({
      ...ROLLBACK_REPORT,
      status: "blocked",
      restoredGeneration: null,
      restoredSkills: [],
    });
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Rollback last version" }));

    await waitFor(() => expect(screen.getByText(/Something didn’t finish/)).toBeInTheDocument());
    expect(screen.queryByText(/Rolled back/)).not.toBeInTheDocument();
  });
});
