<div align="center">

<img src="public/aspen-logo.png" alt="Aspen logo" width="128" height="128" />

# Aspen

### Mac-first photo studio for selecting & finishing portrait shoots

[![macOS](https://img.shields.io/badge/macOS-12%2B-1f4d3a?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/aniruddh02/aspen-editor)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?style=for-the-badge&logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-stable-DEA584?style=for-the-badge&logo=rust&logoColor=black)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React-19-61DAFB?style=for-the-badge&logo=react&logoColor=black)](https://react.dev)
[![Author](https://img.shields.io/badge/Author-Aniruddh%20Tiwari-2f9e6f?style=for-the-badge&logo=github&logoColor=white)](https://github.com/aniruddh02)

[📘 User guide](docs/USER_GUIDE.md) · [📦 Repository](https://github.com/aniruddh02/aspen-editor) · [🙋 Contributor](#-author--contributor)

<br />

### ⬇️ Download (no build required)

[![Download DMG](https://img.shields.io/badge/Download-Aspen%20v0.1.0%20DMG-2f9e6f?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/aniruddh02/aspen-editor/releases/latest/download/Aspen_0.1.0_aarch64.dmg)
[![All Releases](https://img.shields.io/badge/Releases-GitHub-181717?style=for-the-badge&logo=github)](https://github.com/aniruddh02/aspen-editor/releases)

| File | Platform | Link |
|---|---|---|
| 💿 **Aspen_0.1.0_aarch64.dmg** | Apple Silicon Mac | [Download DMG](https://github.com/aniruddh02/aspen-editor/releases/latest/download/Aspen_0.1.0_aarch64.dmg) |
| 🗜️ **Aspen.app.zip** | Apple Silicon Mac | [Download ZIP](https://github.com/aniruddh02/aspen-editor/releases/latest/download/Aspen.app.zip) |

Open the DMG → drag **Aspen** into Applications.

> ⚠️ **First launch on macOS:** Aspen is ad-hoc signed (not Apple-notarized). You may see *“Apple could not verify Aspen”* — that is expected. Go to **System Settings → Privacy & Security → Open Anyway**, or **Right-click Aspen → Open**.
>
> If you still see *“Aspen.app is damaged”* from an older download, remove quarantine and try again:
> ```bash
> xattr -cr "/Applications/Aspen.app"
> ```

</div>

---

> ✨ Start with **Deduplicate** to keep one best frame from every burst.  
> Then continue to **Image Editing** for a repeatable Lightroom Classic finish.  
> Local AI through Ollama is available — but **never required**.

---

## 🧭 Workflow

1. 📁 Drop a shoot into **Deduplicate**.
2. 🎯 Aspen groups exact and perceptual duplicates, moves the winner from each group to `Images-Good`, and moves the rest to `Rejected`.
3. ➡️ **Continue to Image Editing** opens Lightroom processing with `Images-Good` already selected.
4. 🎨 Aspen applies the checked portrait recipes and exports JPEG 90 files to `Processed-Images`.
5. 🔁 A later run or AI feedback pass uses `Processed-Images-2`, `Processed-Images-3`, and so on — previous exports are never overwritten.

```text
Shoot folder
├── Images-Good      ✅ keepers (green Finder tag)
├── Rejected         ❌ duplicates (red Finder tag)
└── Processed-Images 🖼️ Lightroom exports (versioned)
```

---

## 🚀 Capabilities

| 🎨 Area | ✅ Included |
|---|---|
| 🔍 **Deduplicate** | RAW + raster discovery, blake3 exact matches, pHash near-duplicates, quality winner selection, Finder color tags |
| 🖌️ **Image Editing** | Lightroom Classic import, Develop settings, and export through `@mskalski/lightroom-mcp` |
| 🤖 **Optional AI** | Local Ollama chat and constrained edit-plan adjustments — no cloud API |
| 📊 **Diagnostics** | Structured run IDs, bounded live log, JSONL disk logs, export, five-file rotation |

---

## 🧰 Runtime dependencies

| 🧩 Dependency | ⏱️ Required when | 📥 Install |
|---|---|---|
| 🌲 Aspen | Always | Install the DMG or build locally |
| 📷 Lightroom Classic | Image Editing | Install through Adobe Creative Cloud |
| 🔌 Lightroom MCP plugin | Image Editing | `npx -y @mskalski/lightroom-mcp install-plugin` |
| 🟢 Node.js | Running MCP through `npx` | `brew install node` |
| 🦙 Ollama | Only when AI features are enabled | `brew install ollama` |
| 🧠 Qwen3 1.7B | Default local AI model | `ollama pull qwen3:1.7b` |

> 💡 After installing the plugin, fully restart Lightroom Classic, open **File → Plug-in Manager → Lightroom MCP**, and click **Start Server**.  
> Bulk Image Editing does **not** need Ollama.

---

## ⚙️ Settings reference

> 💾 Every checkbox, segmented control, slider, and dropdown is **saved automatically**. There is no Save button. The next Deduplicate or Image Editing run uses the saved values.

### 🔍 Deduplicate

| 🎛️ Setting | ⭐ Default | 📝 Details |
|---|---|---|
| 👤 Scene | Portrait | Portrait weights face/eye clarity while selecting a winner. Landscape uses sharpness scoring. |
| 🧲 Duplicate strength | Balanced | Loose groups more near-duplicates; Balanced is the normal threshold; Strict groups only very similar frames. |
| ⚡ Performance | Medium | Controls worker count, face scoring, preview batch size, and confirmatory near-duplicate checks. |
| 📂 File action | Move | Move reorganizes originals; Copy preserves originals and writes copies to the result folders. |
| 📁 Include subfolders | On | Recursively scans nested folders while skipping Aspen output folders. |
| ➡️ Continue to Image Editing | On | Adds the primary continuation action after Deduplicate and preselects `Images-Good`. |

Deduplicate always stores the latest successful `Images-Good` path. It remains the default Image Editing source even when **Continue to Image Editing** is unchecked.

### 🖌️ Image Editing

| 🎛️ Setting | ⭐ Default | 📝 Details |
|---|---|---|
| 📁 Source path | Latest `Images-Good` | Filled automatically after Deduplicate; choose another folder at any time. |
| 🤖 Use AI | Off | Off builds a deterministic plan from the controls and never contacts Ollama. On combines chat feedback with the checked recipes. |
| 👁️ Eye sharpen | On / Medium | Masked global Lightroom sharpening. Small: 25/1.0/25/70; Medium: 40/1.0/35/80; High: 55/1.2/40/85 for Amount/Radius/Detail/Masking. |
| 🌑 Slight vignette | On / Medium | Post-crop vignette. Small: −10/50/50; Medium: −20/45/60; High: −35/40/70 for Amount/Midpoint/Feather. |
| 🌫️ Blur around subject | On / Medium | Prefers Lens Blur or an inverted subject-mask softening recipe. If MCP can’t do either, Aspen skips safely and logs a warning. |
| ✂️ Optimal crop | On | Preserves the source crop when the MCP has no composition-analysis tool and records a warning. |
| 🌡️ White balance | On | Applies Lightroom Auto white balance. |
| 🎨 Color tone | On | Adds mild contrast and vibrance designed to remain skin-safe. |
| ☀️ Exposure normalize | On | Normalizes base exposure and gently recovers highlights/shadows. |
| 🔇 Noise reduction | Off | Enables moderate luminance and color noise reduction for high-ISO work. |
| 📤 Export | JPEG 90 | Writes to the next available `Processed-Images` version folder. |

### 🤖 AI and chat

| 🎛️ Setting | ⭐ Default | 📝 Details |
|---|---|---|
| 🔐 Enable AI features | Off | Master privacy/resource switch. Off hides chat, forces Use AI off, and prevents all Ollama requests. |
| 🧠 Model | `qwen3:1.7b` | Active installed Ollama model. Refresh happens when AI is enabled. |
| 🌡️ Temperature | 0.2 | Low variance keeps generated edit-plan JSON predictable. |
| 🧹 Clear chat after run | On | Removes context after a completed processing pass. |
| 🚪 Clear chat when leaving | On | Removes context when navigating away from Image Editing. |
| 📏 Context limit | 20 messages / ~8k tokens | Older messages are dropped before requests; image data is never stored in chat. |

AI output is constrained to strength steps and small exposure/contrast/vibrance deltas. Values are clamped before Lightroom receives them.

### 🩺 Diagnostics

| 🎛️ Setting | ⭐ Default | 📝 Details |
|---|---|---|
| 🔊 Verbose logging | Off | Enables additional per-file and protocol detail. |
| 🗂️ Include full paths in exports | Off | Keeps parent paths out of exported diagnostics unless explicitly enabled. |
| 💬 Include chat prompts in exports | Off | Prompt text remains excluded unless explicitly enabled. |
| 📂 Open Logs | — | Opens `~/Library/Logs/Aspen`. |
| 📤 Export Run | — | Saves the visible structured run log as JSONL. |
| 🗑️ Clear Logs | — | Clears the live ring buffer and rotated Aspen log files. |

> 🔒 Aspen never logs Lightroom authentication tokens, image bytes, model weights, or hidden chain-of-thought.

---

## 🖼️ Supported image formats

Sony, Nikon, Canon, and Fuji **RAW** formats (ARW, NEF, CR2, CR3, RAF, DNG) plus JPEG, PNG, TIFF, WebP, BMP, GIF, HEIC, and HEIF.

---

## 🛠️ Develop and test

Requirements: macOS 12+, Xcode Command Line Tools, stable Rust, Node.js 20+, and npm.

```bash
npm install
npm test
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
npm run tauri dev
```

---

## 📦 Build and install

Prefer the prebuilt installer? Use the [latest GitHub Release](https://github.com/aniruddh02/aspen-editor/releases/latest) — no local build needed.

To build from source:

```bash
npm run tauri build
```

Outputs land under `src-tauri/target/release/bundle/` (`Aspen.app` and `Aspen_*.dmg`). Drag Aspen into Applications.

**Gatekeeper note:** builds use ad-hoc signing (`signingIdentity: "-"`), which prevents the misleading “is damaged” dialog. First open still needs **Open Anyway** / Right-click → Open until the app is Apple-notarized (requires a paid Developer ID). To clear a quarantine flag from an older download:

```bash
xattr -cr "/Applications/Aspen.app"
```

---

## 🆘 Troubleshooting

- 🔌 **MCP cannot connect:** restart Lightroom Classic, open Plug-in Manager, and click **Start Server**.
- 🦙 **Ollama unavailable:** verify `ollama list` and `ollama pull qwen3:1.7b`; AI-off processing still works.
- 📁 **No images found:** choose a folder containing supported files directly; Image Editing does not recurse.
- 🩺 **Need diagnostics:** enable Verbose logging, reproduce once, then use **Export Run**.

---

## 🙋 Author & contributor

<div align="center">

<img src="public/aspen-logo.png" alt="Aspen logo" width="64" height="64" />

**Aspen is created and maintained by [Aniruddh Tiwari](https://github.com/aniruddh02).**

[![GitHub](https://img.shields.io/badge/GitHub-aniruddh02-181717?style=for-the-badge&logo=github)](https://github.com/aniruddh02)
[![Repo](https://img.shields.io/badge/Repo-aspen--editor-2f9e6f?style=for-the-badge&logo=git&logoColor=white)](https://github.com/aniruddh02/aspen-editor)

Sole contributor · Personal project · Ideas welcome via GitHub Issues

</div>

---

## 📄 License

Personal project — see the repository for license terms.
