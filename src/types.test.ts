import { describe, expect, it } from "vitest";
import { DEFAULT_SETTINGS, DOCS_URL } from "./types";

describe("defaults", () => {
  it("defaults scene to portrait", () => {
    expect(DEFAULT_SETTINGS.sceneMode).toBe("portrait");
  });

  it("defaults file action to copy", () => {
    expect(DEFAULT_SETTINGS.fileAction).toBe("copy");
  });

  it("points docs to public GitHub repo", () => {
    expect(DOCS_URL).toContain("github.com/aniruddh02/aspen-editor");
  });

  it("continues to Image Editing by default", () => {
    expect(DEFAULT_SETTINGS.continueToImageEditing).toBe(true);
    expect(DEFAULT_SETTINGS.lastImagesGoodPath).toBe("");
  });

  it("keeps AI off while portrait recipes default on", () => {
    expect(DEFAULT_SETTINGS.enableAiFeatures).toBe(false);
    expect(DEFAULT_SETTINGS.useAiForDedup).toBe(false);
    expect(DEFAULT_SETTINGS.useAiForEdit).toBe(false);
    expect(DEFAULT_SETTINGS.eyeSharpen).toBe(true);
    expect(DEFAULT_SETTINGS.eyeSharpenStrength).toBe("medium");
    expect(DEFAULT_SETTINGS.vignette).toBe(true);
    expect(DEFAULT_SETTINGS.subjectBlur).toBe(true);
    expect(DEFAULT_SETTINGS.noiseReduction).toBe(false);
  });

  it("keeps benchmark capture opt-in so normal runs stay fast", () => {
    expect(DEFAULT_SETTINGS.benchmarkLogging).toBe(false);
  });
});
