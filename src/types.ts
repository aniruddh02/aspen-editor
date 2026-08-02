export type SceneMode = "portrait" | "landscape";
export type FileAction = "move" | "copy";
export type PerfProfile = "low" | "medium" | "high";
export type DuplicateStrength = "loose" | "balanced" | "strict";

export interface AppSettings {
  sceneMode: SceneMode;
  fileAction: FileAction;
  perfProfile: PerfProfile;
  duplicateStrength: DuplicateStrength;
  includeSubfolders: boolean;
  enabledExtensions: string[];
}

export interface ProgressEvent {
  stage: string;
  message: string;
  current: number;
  total: number;
}

export interface DeduplicateResult {
  folder: string;
  scanned: number;
  duplicateGroups: number;
  keptGood: number;
  rejected: number;
  uniqueLeft: number;
  errors: string[];
  goodDir: string;
  rejectedDir: string;
}

export const DEFAULT_SETTINGS: AppSettings = {
  sceneMode: "portrait",
  fileAction: "move",
  perfProfile: "medium",
  duplicateStrength: "balanced",
  includeSubfolders: true,
  enabledExtensions: [
    "arw", "srf", "sr2", "nef", "nrw", "cr2", "cr3", "crw", "raf", "dng",
    "jpg", "jpeg", "png", "tif", "tiff", "webp", "bmp", "gif", "heic", "heif",
  ],
};

export const DOCS_URL = "https://github.com/aniruddh02/aspen-editor#readme";
