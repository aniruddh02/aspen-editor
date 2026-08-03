# Aspen User Guide

## What Deduplicate does

1. Scans the folder you select (optionally including subfolders).
2. Ignores anything already inside `Images-Good` or `Rejected`.
3. Groups exact duplicates and visually similar images.
4. For each group of 2+, scores sharpness (and face/eye clarity in Portrait mode).
5. Puts **exactly one** winner in `Images-Good` and the rest in `Rejected`.
6. Leaves unique (non-duplicate) images where they are.

## Supported formats

- RAW: ARW, NEF, CR2, CR3, RAF, DNG, and related
- Raster: JPEG, PNG, TIFF, WebP, BMP, GIF, HEIC/HEIF

## Scene mode

- **Portrait** (default): weights face/eye clarity more heavily when ranking duplicates.
- **Landscape**: sharpness-only ranking (faster).

## Move vs Copy

- **Move**: originals are relocated into Good/Rejected.
- **Copy**: originals stay put; copies are written into Good/Rejected.

## Duplicate strength

- **Loose**: merges more near-duplicates
- **Balanced**: default
- **Strict**: only very similar images merge

## Performance

- **Low**: fewer threads, no face scoring
- **Medium**: default for ~50–500 images
- **High**: more threads, confirmatory dHash

## Hash cache

Aspen stores blake3/pHash metadata under Application Support. Re-runs skip unchanged files. Use **Clear hash cache** in Settings if needed.

## Export log

Expand the Live Log rail → **Export** to save the current structured log. Aspen also keeps rotated
JSONL logs under `~/Library/Logs/Aspen`.

## Continue to Image Editing

**Continue to Image Editing** is checked by default. After Deduplicate finishes, choose the primary
continuation button to open Image Editing with `Images-Good` already selected.

Aspen saves the latest `Images-Good` path even when continuation is unchecked. You can always
replace it with **Choose Folder**.

## Prepare Lightroom Classic

1. Install Node.js: `brew install node`.
2. Install the Lightroom bridge:
   `npx -y @mskalski/lightroom-mcp install-plugin`.
3. Fully restart Lightroom Classic.
4. Open **File → Plug-in Manager → Lightroom MCP** and click **Start Server**.

## Image Editing

1. Confirm the source is your `Images-Good` folder.
2. Choose the portrait recipes and Small, Medium, or High strength.
3. Leave **Use AI** off for deterministic Lightroom-only processing.
4. Select **Process with Lightroom**.
5. Open the resulting `Processed-Images` folder from the completion dialog.

Later processing passes create numbered folders instead of replacing earlier output.

Eye sharpen uses high masking to protect skin. Vignette uses Lightroom post-crop vignetting.
Subject blur is skipped with a warning when the installed Lightroom MCP does not expose Lens Blur
or subject masks; Aspen never substitutes a global blur.

## Optional local AI

Install Ollama with `brew install ollama`, then run `ollama pull qwen3:1.7b`. In Aspen Settings,
turn on **Enable AI features**, then turn on **Use AI** in Image Editing.

Chat can interpret bounded requests such as “less vignette” or “sharper eyes.” **Apply feedback**
creates the next `Processed-Images-N` folder. The context is limited to 20 messages and
approximately 8,000 tokens, and auto-clears according to the settings.

When the master AI setting is off, Aspen does not contact or start Ollama.

## Automatic settings

All setting changes save automatically and are reused on the next processing run. The complete
default and behavior reference is in the main [README](../README.md#settings-reference).

## Building installers

See the main [README](../README.md#build-dmg--app-bundle).
