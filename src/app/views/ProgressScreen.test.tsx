import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import type { DownloadProgress } from "../../shared/types";
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
