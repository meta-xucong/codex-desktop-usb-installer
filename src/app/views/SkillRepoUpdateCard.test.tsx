import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { managerApi } from "../../services/managerApi";
import type {
  SkillRepoApplyReport,
  SkillRepoRollbackReport,
  SkillRepoUpdateInfo,
} from "../../shared/types";
import { I18nProvider } from "../i18n";
import { SKILL_REPO_CHECK_UI_TIMEOUT_MS, SkillRepoUpdateCard } from "./SkillRepoUpdateCard";

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
  sourceKind: "domestic",
  sourceLabel: "domestic-2",
  attempts: [
    {
      sourceId: "cn-mirror-1",
      sourceKind: "domestic",
      phase: "metadata",
      result: "timeout",
      httpStatus: null,
      elapsedMs: 8_000,
    },
    {
      sourceId: "cn-mirror-2",
      sourceKind: "domestic",
      phase: "archive",
      result: "ok",
      httpStatus: 200,
      elapsedMs: 3_200,
    },
  ],
  archiveSize: 12,
  archiveSha256: "b".repeat(64),
  expected: {
    schemaVersion: 1,
    repo: "meta-xucong/skill_hub",
    branch: "master",
    commit: "a".repeat(40),
    sourceKind: "domestic",
    sourceId: "cn-mirror-2",
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
    expect(screen.getByText(/Product Design plugin.*10 skills.*installed with the update/i)).toBeInTheDocument();
  });

  it("checks then applies the frozen identity with one update click", async () => {
    api.skillRepoCheckUpdate.mockResolvedValue(INFO);
    api.skillRepoApplyUpdate.mockResolvedValue({ ...REPORT, productDesignSkillCount: 10 });
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
    await waitFor(() => expect(screen.getByText(/Current/)).toBeTruthy());
    expect(screen.getByText(/Installable 2.*conflicts 1.*skipped 0/)).toBeInTheDocument();
    expect(screen.getByText(/Product Design plugin.*10 skills/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Update now" }));
    await waitFor(() => expect(api.skillRepoApplyUpdate).toHaveBeenCalledWith(INFO.expected));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.getByText(/Update success/)).toBeTruthy();
    expect(screen.getByTestId("skill-repo-product-design-count")).toHaveTextContent("Product Design plugin is installed, with 10 skills.");
    expect(screen.queryByTestId("skill-repo-product-design-status")).not.toBeInTheDocument();
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

  it("surfaces a bounded timeout instead of keeping the checking state forever", async () => {
    vi.useFakeTimers();
    try {
      let resolveCheck!: (value: SkillRepoUpdateInfo) => void;
      const pending = new Promise<SkillRepoUpdateInfo>((resolve) => {
        resolveCheck = resolve;
      });
      api.skillRepoCheckUpdate.mockReturnValue(pending);
      render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

      openDetails();
      fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
      expect(screen.getByRole("button", { name: "Checking…" })).toBeDisabled();

      await act(async () => {
        vi.advanceTimersByTime(SKILL_REPO_CHECK_UI_TIMEOUT_MS);
        await Promise.resolve();
        await Promise.resolve();
      });

      expect(screen.getByRole("alert")).toHaveTextContent(/timed out/i);
      expect(screen.getByRole("button", { name: "Check updates" })).toBeEnabled();

      await act(async () => {
        resolveCheck(INFO);
        await pending;
        await Promise.resolve();
      });

      expect(screen.getByRole("alert")).toHaveTextContent(/timed out/i);
      expect(screen.queryByText(/Current/)).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows safe source category, phase, result, and elapsed time", async () => {
    api.skillRepoCheckUpdate.mockResolvedValue(INFO);
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
    await waitFor(() => expect(screen.getByText("Selected source: Domestic source 2")).toBeInTheDocument());

    const attempts = screen.getAllByTestId("skill-repo-source-attempt");
    expect(attempts).toHaveLength(2);
    expect(attempts[0]).toHaveTextContent(/Domestic source 1.*Metadata.*timed out.*8\.0s/s);
    expect(attempts[1]).toHaveTextContent(/Domestic source 2.*Archive.*passed.*3\.2s.*HTTP 200/s);
  });

  it("shows all-source failures as safe retryable diagnostics", async () => {
    api.skillRepoCheckUpdate
      .mockRejectedValueOnce({
        code: "SKILL_REPO_ALL_SOURCES_FAILED",
        message: "all sources failed at https://example.test/archive.zip",
        detail: "GET https://example.test/archive.zip failed with api_key=secret",
        attempts: INFO.attempts,
      })
      .mockResolvedValueOnce(INFO);
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent(/temporarily unavailable/i));

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("No local Skills were changed.");
    expect(alert).not.toHaveTextContent("https://example.test/archive.zip");
    expect(alert).not.toHaveTextContent("api_key=secret");
    expect(screen.getByTestId("skill-repo-retry")).toBeEnabled();

    fireEvent.click(screen.getByTestId("skill-repo-retry"));
    await waitFor(() => expect(api.skillRepoCheckUpdate).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.getByText(/Current/)).toBeInTheDocument());
  });

  it("shows the safe backend detail for an invalid source instead of hiding it", async () => {
    api.skillRepoCheckUpdate.mockRejectedValue({
      code: "SKILL_REPO_SOURCE_INVALID",
      message: "source invalid",
      detail: "stage=source-resource-validate; candidate=portable-resources; reason=Skill Hub 来源目录 JSON 无效",
    });
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByRole("alert")).toHaveTextContent("The update source or confirmed version is invalid");

    fireEvent.click(screen.getByText("Show details"));
    expect(screen.getByRole("alert")).toHaveTextContent("source-resource-validate");
    expect(screen.getByRole("alert")).toHaveTextContent("来源目录 JSON 无效");
  });

  it("copies Skill Hub diagnostics without URLs, accounts, or credentials", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    api.skillRepoCheckUpdate.mockRejectedValue({
      code: "SKILL_REPO_ALL_SOURCES_FAILED",
      message: "failed at https://example.test/archive.zip",
      detail: "owner/meta-xucong/skill_hub token=secret",
      attempts: INFO.attempts,
    });
    render(<I18nProvider><SkillRepoUpdateCard /></I18nProvider>);

    openDetails();
    fireEvent.click(screen.getByRole("button", { name: "Check updates" }));
    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Show details"));
    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledOnce());

    const copied = String(writeText.mock.calls[0]?.[0]);
    expect(copied).toContain("cn-mirror-1");
    expect(copied).not.toContain("https://example.test/archive.zip");
    expect(copied).not.toContain("meta-xucong");
    expect(copied).not.toContain("token=secret");
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
    const confirm = screen.getByRole("alertdialog");
    expect(confirm).toHaveTextContent("Roll back the last complete hot-update generation?");
    fireEvent.click(confirm.querySelector("button.btn.primary")!);

    await waitFor(() => expect(api.skillRepoRollback).toHaveBeenCalledOnce());
    expect(window.confirm).not.toHaveBeenCalled();
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
    fireEvent.click(screen.getByRole("alertdialog").querySelector("button.btn.primary")!);

    await waitFor(() => expect(screen.getByText(/Something didn’t finish/)).toBeInTheDocument());
    expect(screen.queryByText(/Rolled back/)).not.toBeInTheDocument();
  });
});
