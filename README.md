# Aspen

**Aspen** is a Mac-first desktop studio for photographers. v1 ships the **Deduplicate** feature: find duplicate images (RAW + common formats), keep the single sharpest / clearest frame in `Images-Good`, and place the rest in `Rejected`.

Help / docs: this README and [docs/USER_GUIDE.md](docs/USER_GUIDE.md)  
Repository: https://github.com/aniruddh02/aspen-editor

## Features (v1)

- Scan folders of Sony/Nikon/Canon/Fuji **RAW** plus JPEG, PNG, DNG, TIFF, WebP, BMP, GIF, HEIC
- Exact (blake3) + perceptual (pHash) duplicate grouping
- Exactly **one** winner per group → `Images-Good` (green); others → `Rejected` (red)
- Prefer **RAW/DNG** when scores are close
- **Portrait** (default) vs **Landscape** scoring
- **Move** or **Copy** file action
- On-disk hash cache for fast re-runs
- Live log with **Export**
- Performance profiles: Low / Medium / High

Future: **Styles** (editing looks) — placeholder in the UI only.

## Requirements (build)

- macOS 12+ (Apple Silicon recommended; tested target `aarch64-apple-darwin`)
- [Xcode Command Line Tools](https://developer.apple.com/xcode/resources/)
- [Rust](https://rustup.rs/) (stable)
- Node.js 20+ and npm

## Develop

```bash
npm install
npm run tauri dev
```

## Test

```bash
# Rust unit + integration tests
cd src-tauri && cargo test

# Frontend typecheck
npm run build
```

## Build DMG / app bundle

```bash
npm install
npm run tauri build
```

Outputs (typical):

- `src-tauri/target/release/bundle/macos/Aspen.app`
- `src-tauri/target/release/bundle/dmg/Aspen_*.dmg`

### ZIP alternate

```bash
cd src-tauri/target/release/bundle/macos
zip -r Aspen.app.zip Aspen.app
```

## Install on your Mac

1. Open the **DMG** and drag **Aspen.app** into **Applications**, or run:

```bash
./packaging/install.sh /path/to/Aspen.app
```

2. First launch (unsigned personal build): **Right-click → Open**, or:

```bash
xattr -cr "/Applications/Aspen.app"
```

3. Use **Deduplicate**: drop a folder → **Run Deduplicate**.

## Settings summary

| Setting | Default | Purpose |
|---|---|---|
| Scene | Portrait | Face/eye-weighted vs sharpness-only |
| File action | Move | Move or copy into Good/Rejected |
| Strength | Balanced | How aggressively near-duplicates merge |
| Perf | Medium | CPU threads / confirmation depth |

## License

Personal project — see repository for license terms.
