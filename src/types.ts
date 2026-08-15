export type SceneMode = "portrait" | "landscape";
export type FileAction = "move" | "copy";
export type PerfProfile = "low" | "medium" | "high";
export type DuplicateStrength = "loose" | "balanced" | "strict";
export type EditStrength = "small" | "medium" | "high";

export interface AppSettings {
  sceneMode: SceneMode;
  fileAction: FileAction;
  perfProfile: PerfProfile;
  duplicateStrength: DuplicateStrength;
  includeSubfolders: boolean;
  enabledExtensions: string[];
  continueToImageEditing: boolean;
  lastImagesGoodPath: string;
  enableAiFeatures: boolean;
  useAiForDedup: boolean;
  useAiForEdit: boolean;
  eyeSharpen: boolean;
  eyeSharpenStrength: EditStrength;
  vignette: boolean;
  vignetteStrength: EditStrength;
  subjectBlur: boolean;
  subjectBlurStrength: EditStrength;
  optimalCrop: boolean;
  whiteBalance: boolean;
  colorTone: boolean;
  exposureNormalize: boolean;
  noiseReduction: boolean;
  ollamaModel: string;
  ollamaTemperature: number;
  chatAutoClearAfterRun: boolean;
  chatAutoClearOnLeave: boolean;
  chatAutoClearOnAiOff: boolean;
  verboseLogging: boolean;
  includeFullPathsInLogs: boolean;
  includeChatPromptsInLogs: boolean;
  benchmarkLogging: boolean;
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
  uniqueUntouched: number;
  errors: string[];
  goodDir: string;
  rejectedDir: string;
  aiReranked: number;
  benchmarkLog: string | null;
}

export interface ImageEditProgress {
  runId: string;
  stage: string;
  message: string;
  current: number;
  total: number;
  level: string;
}

export interface ImageEditResult {
  runId: string;
  sourcePath: string;
  outputPath: string;
  processed: number;
  warnings: string[];
  usedAi: boolean;
}

export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

export const DEFAULT_SETTINGS: AppSettings = {
  sceneMode: "portrait",
  fileAction: "copy",
  perfProfile: "medium",
  duplicateStrength: "balanced",
  includeSubfolders: true,
  continueToImageEditing: true,
  lastImagesGoodPath: "",
  enableAiFeatures: false,
  useAiForDedup: false,
  useAiForEdit: false,
  eyeSharpen: true,
  eyeSharpenStrength: "medium",
  vignette: true,
  vignetteStrength: "medium",
  subjectBlur: true,
  subjectBlurStrength: "medium",
  optimalCrop: true,
  whiteBalance: true,
  colorTone: true,
  exposureNormalize: true,
  noiseReduction: false,
  ollamaModel: "",
  ollamaTemperature: 0.2,
  chatAutoClearAfterRun: true,
  chatAutoClearOnLeave: true,
  chatAutoClearOnAiOff: true,
  verboseLogging: false,
  includeFullPathsInLogs: false,
  includeChatPromptsInLogs: false,
  benchmarkLogging: false,
  enabledExtensions: [
    "arw", "srf", "sr2", "nef", "nrw", "cr2", "cr3", "crw", "raf", "dng",
    "jpg", "jpeg", "png", "tif", "tiff", "webp", "bmp", "gif", "heic", "heif",
  ],
};

export const DOCS_URL = "https://github.com/aniruddh02/aspen-editor#readme";
