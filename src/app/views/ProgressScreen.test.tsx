import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import type { DownloadProgress } from "../../shared/types";
import { WINDOWS_PORTABLE_EXTRACTION_SOURCE } from "../../shared/types";
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

  function renderInstall(progress: DownloadProgress, installation: DownloadProgress) {
    const download: DownloadProgress = {
      downloaded: 100,
      total: 100,
      source: "mirror.example",
      phase: "downloading",
    };
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
            onResume={() => undefined}
            onPause={() => undefined}
            onCancel={() => undefined}
          />
        </I18nProvider>
      </ThemeProvider>,
    );
  }

  it("keeps download at 100% while verification has no install percentage", () => {
    renderInstall(
      { downloaded: 100, total: 100, source: "mirror.example", phase: "verifying" },
      { downloaded: 0, total: 0, source: "mirror.example", phase: "verifying" },
    );

    expect(screen.getByText("下载完成")).toBeInTheDocument();
    expect(screen.getByTestId("download-progress-bar")).toHaveAttribute("aria-valuenow", "100");
    expect(screen.getByTestId("install-progress-bar")).not.toHaveAttribute("aria-valuenow");
    expect(screen.getByTestId("install-progress-bar").firstChild).toHaveClass("indeterminate");
  });

  it("shows only the real extraction percentage during portable unpack", () => {
    renderInstall(
      { downloaded: 100, total: 100, source: "mirror.example", phase: "applying" },
      {
        downloaded: 35,
        total: 100,
        source: WINDOWS_PORTABLE_EXTRACTION_SOURCE,
        phase: "applying",
      },
    );

    expect(screen.getByText("正在解压程序文件")).toBeInTheDocument();
    expect(screen.getByTestId("download-progress-bar")).toHaveAttribute("aria-valuenow", "100");
    expect(screen.getByTestId("install-progress-bar")).toHaveAttribute("aria-valuenow", "35");
  });
});
