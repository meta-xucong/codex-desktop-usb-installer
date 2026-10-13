import { describe, expect, it } from "vitest";

import { selectMediaModels } from "./mediaModelSelection";

describe("selectMediaModels", () => {
  const catalog = [
    "gpt-image-2",
    "gpt-image-2.5",
    "grok-imagine-image-1.0",
    "grok-imagine-image-1.2",
    "grok-video-1.1",
    "seedance-2.0",
  ];

  it("selects the newest supported model in each available family", () => {
    expect(selectMediaModels(catalog)).toMatchObject({
      image2Model: "gpt-image-2.5",
      grokImageModel: "grok-imagine-image-1.2",
      grokVideoModel: "grok-video-1.1",
      seedanceModel: "seedance-2.0",
      minimaxVideoModel: "",
    });
  });

  it("leaves Seedance unset during unattended installation because its route ID is not in the catalog", () => {
    expect(selectMediaModels(catalog, { includeSeedance: false }).seedanceModel).toBe("");
  });

  it("does not use text models as media fallbacks", () => {
    expect(selectMediaModels(["gpt-4o", "claude-opus-4-8"])).toMatchObject({
      image2Model: "",
      grokImageModel: "",
      grokVideoModel: "",
      seedanceModel: "",
      minimaxVideoModel: "",
    });
  });
});
