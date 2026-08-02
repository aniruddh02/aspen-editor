import { describe, expect, it } from "vitest";
import { DEFAULT_SETTINGS, DOCS_URL } from "./types";

describe("defaults", () => {
  it("defaults scene to portrait", () => {
    expect(DEFAULT_SETTINGS.sceneMode).toBe("portrait");
  });

  it("defaults file action to move", () => {
    expect(DEFAULT_SETTINGS.fileAction).toBe("move");
  });

  it("points docs to public GitHub repo", () => {
    expect(DOCS_URL).toContain("github.com/aniruddh02/aspen-editor");
  });
});
