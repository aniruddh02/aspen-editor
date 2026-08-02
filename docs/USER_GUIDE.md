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

Expand the Live Log rail → **Export** to save a `.txt` / `.log` of the session.

## Building installers

See the main [README](../README.md#build-dmg--app-bundle).
