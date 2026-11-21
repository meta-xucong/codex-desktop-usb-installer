import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import {
  WINDOWS_PORTABLE_EXTRACTION_SOURCE,
  type DownloadProgress,
} from "../../shared/types";
import { I18nProvider } from "../i18n";
import { ThemeProvider } from "../theme";
import { ProgressScreen } from "./ProgressScreen";

function renderProgress(source: string) {
  const dl: DownloadProgress = {
    downloaded: 16,
    total: 128,
    source,
    phase: "downloading",
  };

  return render(
    <ThemeProvider>
      <I18nProvider>
        <ProgressScreen
          scene="test"
          scopeRef={createRef<HTMLDivElement>()}
          paused={null}
          dl={dl}
          dlPct={12.5}
          dlBytes={16}
          dlSpeed={4}
          installing
          downloadStop={null}
          downloadStopBusy={false}
          failure={null}
          onResume={() => undefined}
          onPause={() => undefined}
          onCancel={() => undefined}
        />
      </I18nProvider>
    </ThemeProvider>,
  );
}

describe("ProgressScreen download source copy", () => {
  beforeEach(() => {
    localStorage.setItem("cam.lang", "zh-CN");
  });

  it("uses a clear download label instead of an empty source", () => {
    renderProgress("   ");

    expect(screen.getByText("正在下载 Codex…")).toBeInTheDocument();
    expect(screen.queryByText("正在从  下载")).not.toBeInTheDocument();
  });

  it("keeps a known download source in the phase label", () => {
    renderProgress("mirror.example");

    expect(screen.getByText("正在从 mirror.example 下载")).toBeInTheDocument();
  });
});

describe("ProgressScreen split Windows install progress", () => {
  beforeEach(() => {
    localStorage.setItem("cam.lang", "zh-CN");
  });

  function renderInstall(
    progress: DownloadProgress,
    installation: DownloadProgress,
    download: DownloadProgress | null = {
      downloaded: 100,
      total: 100,
      source: "mirror.example",
      phase: "downloading",
    },
  ) {
    return render(
      <ThemeProvider>
        <I18nProvider>
          <ProgressScreen
            scene="install-test"
            scopeRef={createRef<HTMLDivElement>()}
            paused={null}
            dl={progress}
            downloadProgress={download}
            installProgress={installation}
            dlPct={progress.total ? (progress.downloaded / progress.total) * 100 : 0}
            dlBytes={progress.downloaded}
            dlSpeed={0}
            installing
            splitWindowsInstall
            downloadStop={null}
            downloadStopBusy={false}
            failure={null}
            activity="正在安装能力包…"
            onResume={() => undefined}
            onPause={() => undefined}
            onCancel={() => undefined}
          />
        </I18nProvider>
      </ThemeProvider>,
    );
  }

  it("keeps download at 100% and marks the first of four install steps during verification", () => {
    renderInstall(
      { downloaded: 100, total: 100, source: "mirror.example", phase: "verifying" },
      { downloaded: 0, total: 0, source: "mirror.example", phase: "verifying" },
    );

    expect(screen.getByText("下载完成")).toBeInTheDocument();
    expect(screen.getByText("正在安装能力包…")).toBeInTheDocument();
    expect(screen.getByTestId("download-progress-bar")).toHaveAttribute("aria-valuenow", "100");
    const installProgress = screen.getByTestId("install-progress-bar");
    expect(installProgress).toHaveAttribute("aria-valuenow", "1");
    expect(installProgress.querySelectorAll(".install-stage-segment")).toHaveLength(4);
    expect(installProgress.querySelector('[data-state="active"] .install-stage-fill')).toHaveClass(
      "indeterminate",
    );
  });

  it("shows the second of four install steps and only the real extraction percentage", () => {
    renderInstall(
      { downloaded: 100, total: 100, source: "mirror.example", phase: "applying" },
      {
        downloaded: 35,
        total: 100,
        source: WINDOWS_PORTABLE_EXTRACTION_SOURCE,
        phase: "applying",
      },
    );

    expect(screen.getByTestId("download-progress-bar")).toHaveAttribute("aria-valuenow", "100");
    const installProgress = screen.getByTestId("install-progress-bar");
    expect(installProgress).toHaveAttribute("aria-valuenow", "2");
    expect(installProgress).toHaveAttribute("aria-valuetext", expect.stringContaining("35%"));
    expect(installProgress.querySelectorAll(".install-stage-segment")).toHaveLength(4);
    expect(installProgress.querySelectorAll('[data-state="complete"]')).toHaveLength(1);
    expect(installProgress.querySelector('[data-state="active"] .install-stage-fill')).toHaveStyle({
      width: "35%",
    });
  });

  it("finishes a download without a byte total once installation starts", () => {
    renderInstall(
      { downloaded: 0, total: 0, source: "", phase: "applying" },
      { downloaded: 0, total: 0, source: "", phase: "applying" },
      { downloaded: 256, total: 0, source: "mirror.example", phase: "downloading" },
    );

    const downloadProgress = screen.getByTestId("download-progress-bar");
    expect(downloadProgress).toHaveAttribute("aria-valuenow", "100");
    expect(downloadProgress).toHaveAttribute("aria-valuetext", "下载完成");
    expect(downloadProgress.querySelector(".bar-fill")).toHaveStyle({ width: "100%" });
    expect(downloadProgress.querySelector(".bar-fill")).not.toHaveClass("indeterminate");
    expect(screen.getByText("正在安装能力包…")).toHaveClass("progress-activity");
  });

  it("keeps an unknown transfer animated while it is still downloading", () => {
    const download: DownloadProgress = {
      downloaded: 256, total: 0, source: "mirror.example", phase: "downloading",
    };
    renderInstall(download, { downloaded: 0, total: 0, source: "", phase: "preparing" }, download);

    const downloadProgress = screen.getByTestId("download-progress-bar");
    expect(downloadProgress).not.toHaveAttribute("aria-valuenow");
    expect(downloadProgress.querySelector(".bar-fill")).toHaveClass("indeterminate");
  });

  it("holds at three of four steps while final health checks are running", () => {
    renderInstall(
      { downloaded: 100, total: 100, source: "mirror.example", phase: "finishing" },
      { downloaded: 0, total: 0, source: "mirror.example", phase: "finishing" },
    );

    const installProgress = screen.getByTestId("install-progress-bar");
    expect(installProgress).toHaveAttribute("aria-valuenow", "3");
    expect(installProgress.querySelectorAll('[data-state="complete"]')).toHaveLength(3);
    expect(installProgress.querySelector('[data-state="active"] .install-stage-fill')).toHaveClass(
      "indeterminate",
    );
  });
});
