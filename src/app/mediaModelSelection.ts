/** Select the newest supported media routes from the live upstream catalog. */
export function selectMediaModels(models: string[], options: { includeSeedance?: boolean } = {}) {
  const version = (id: string) =>
    [...id.matchAll(/\d+(?:\.\d+)*/g)]
      .map(([value]) => value.split(".").map(Number))
      .flat();
  const newestModel = (matches: (id: string) => boolean) =>
    models
      .filter(matches)
      .sort((left, right) => {
        const a = version(left);
        const b = version(right);
        for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
          const difference = (b[index] ?? 0) - (a[index] ?? 0);
          if (difference !== 0) return difference;
        }
        return left.localeCompare(right);
      })[0] ?? "";
  const lower = (id: string) => id.toLowerCase();
  return {
    mediaGenerationAgentId: "",
    image2Model: newestModel((id) => /^gpt-image-\d/i.test(id)),
    grokImageModel: newestModel((id) => {
      const name = lower(id);
      return name.includes("grok") && (name.includes("image") || name.includes("imagine")) && !name.includes("video");
    }),
    grokVideoModel: newestModel((id) => {
      const name = lower(id);
      return name.includes("grok") && name.includes("video");
    }),
    // The model catalog has the model ID, but not Seedance's account-specific
    // route ID. The manual connection flow can select it and ask for that ID;
    // unattended first install must leave it unset.
    seedanceModel: options.includeSeedance === false ? "" : newestModel((id) => lower(id).includes("seedance")),
    // The shared 404token route does not currently expose a supported MiniMax
    // adapter profile, so never configure it from a catalog name alone.
    minimaxVideoModel: "",
  };
}
