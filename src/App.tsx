import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import "./App.css";
import {
  AppSettings,
  DEFAULT_SETTINGS,
  DeduplicateResult,
  DOCS_URL,
  ProgressEvent,
} from "./types";

type FeatureId = "deduplicate" | "styles";

interface LogLine {
  ts: string;
  message: string;
  kind?: "good" | "reject" | "plain";
}

function nowTs() {
  return new Date().toLocaleTimeString([], { hour12: false });
}

function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { id: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="segmented">
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
          className={value === o.id ? "on" : ""}
          onClick={() => onChange(o.id)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function App() {
  const [feature, setFeature] = useState<FeatureId>("deduplicate");
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [folder, setFolder] = useState<string>("");
  const [dragging, setDragging] = useState(false);
  const [running, setRunning] = useState(false);
  const [logOpen, setLogOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [logs, setLogs] = useState<LogLine[]>([]);
  const [progress, setProgress] = useState({ current: 0, total: 0 });
  const [result, setResult] = useState<DeduplicateResult | null>(null);
  const logEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const { getCurrentWebview } = await import("@tauri-apps/api/webview");
        unlisten = await getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === "over") {
            setDragging(true);
          } else if (event.payload.type === "leave") {
            setDragging(false);
          } else if (event.payload.type === "drop") {
            setDragging(false);
            const paths = event.payload.paths;
            if (paths?.length) {
              setFolder(paths[0]);
            }
          }
        });
      } catch {
        /* browser / non-tauri */
      }
    })();
    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    invoke<AppSettings>("get_settings")
      .then(setSettings)
      .catch(() => setSettings(DEFAULT_SETTINGS));
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<ProgressEvent>("dedupe-progress", (event) => {
      const ev = event.payload;
      setProgress({ current: ev.current, total: ev.total || 1 });
      const kind = ev.message.includes("→ Good")
        ? "good"
        : ev.message.includes("→ Rejected")
          ? "reject"
          : "plain";
      setLogs((prev) => [
        ...prev,
        { ts: nowTs(), message: ev.message, kind },
      ]);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logs]);

  const pct = useMemo(() => {
    if (!progress.total) return 0;
    return Math.min(100, Math.round((progress.current / progress.total) * 100));
  }, [progress]);

  async function persistSettings(next: AppSettings) {
    setSettings(next);
    try {
      await invoke("save_app_settings", { settings: next });
    } catch {
      /* ignore offline/dev */
    }
  }

  async function chooseFolder() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected === "string") {
      setFolder(selected);
    }
  }

  async function onDrop(e: React.DragEvent) {
    e.preventDefault();
    setDragging(false);
    const files = e.dataTransfer.files;
    if (files.length > 0) {
      // Tauri drag-drop often provides path via webkitGetAsEntry / tauri file drop
      const anyFile = files[0] as File & { path?: string };
      if (anyFile.path) {
        setFolder(anyFile.path);
        return;
      }
    }
    // Fallback: use dialog
    await chooseFolder();
  }

  async function runDedupe() {
    if (!folder || running) return;
    setRunning(true);
    setResult(null);
    setLogs((prev) => [
      ...prev,
      { ts: nowTs(), message: `Starting deduplicate on ${folder}` },
    ]);
    setLogOpen(true);
    try {
      const res = await invoke<DeduplicateResult>("run_deduplicate_cmd", {
        folder,
        settings,
      });
      setResult(res);
    } catch (err) {
      setLogs((prev) => [
        ...prev,
        { ts: nowTs(), message: `Error: ${String(err)}` },
      ]);
    } finally {
      setRunning(false);
    }
  }

  async function cancel() {
    await invoke("cancel_deduplicate");
  }

  async function exportLog() {
    const path = await save({
      defaultPath: `aspen-log-${Date.now()}.txt`,
      filters: [{ name: "Log", extensions: ["txt", "log"] }],
    });
    if (!path) return;
    const content = logs.map((l) => `${l.ts}  ${l.message}`).join("\n");
    await invoke("export_log", { path, content });
  }

  async function openDocs() {
    try {
      const url = await invoke<string>("get_docs_url");
      await openUrl(url || DOCS_URL);
    } catch {
      await openUrl(DOCS_URL);
    }
  }

  return (
    <div className="app-shell">
      <aside className="features-nav">
        <div className="brand-mark">
          <div className="word">ASPEN</div>
          <div className="tag">Studio</div>
        </div>
        <div className="nav-items">
          <button
            type="button"
            className={`nav-item ${feature === "deduplicate" ? "active" : ""}`}
            onClick={() => setFeature("deduplicate")}
          >
            <span className="label">Deduplicate</span>
            <span className="hint">Keep the sharpest frame</span>
          </button>
          <button type="button" className="nav-item" disabled title="Coming soon">
            <span className="label">Styles</span>
            <span className="hint">Coming soon</span>
          </button>
        </div>
      </aside>

      <div className="main-stage">
        <div className="chrome">
          {running && (
            <button type="button" className="ghost-btn" onClick={cancel}>
              Cancel
            </button>
          )}
          <button type="button" className="icon-btn" title="Documentation" onClick={openDocs}>
            ?
          </button>
          <button
            type="button"
            className="icon-btn"
            title="Settings"
            onClick={() => setSettingsOpen(true)}
          >
            ⚙
          </button>
        </div>

        {feature === "deduplicate" ? (
          <div className="workspace">
            <div>
              <div className="workspace-header">
                <h1>Deduplicate</h1>
                <p>Keep the sharpest frame. Retire the rest.</p>
              </div>
              <div className="panel">
                <div
                  className={`drop-zone ${dragging ? "active" : ""}`}
                  onDragOver={(e) => {
                    e.preventDefault();
                    setDragging(true);
                  }}
                  onDragLeave={() => setDragging(false)}
                  onDrop={onDrop}
                >
                  <h2>Drop a folder of images</h2>
                  <div className="formats">RAW · JPEG · PNG · DNG · TIFF · HEIC · …</div>
                  <button type="button" className="ghost-btn" onClick={chooseFolder}>
                    Choose Folder…
                  </button>
                </div>

                <div className="path-line">
                  Folder: {folder || "—"}
                </div>

                <button
                  type="button"
                  className="primary-btn"
                  disabled={!folder || running}
                  onClick={runDedupe}
                >
                  {running ? "Working…" : "Run Deduplicate"}
                </button>

                {running && (
                  <div className="progress">
                    <div className="progress-bar">
                      <span style={{ width: `${pct}%` }} />
                    </div>
                    <div className="path-line">
                      {progress.current} / {progress.total || "…"} ({pct}%)
                    </div>
                  </div>
                )}

                <div className="controls-row">
                  <div className="control">
                    Scene
                    <Segmented
                      value={settings.sceneMode}
                      options={[
                        { id: "portrait", label: "Portrait" },
                        { id: "landscape", label: "Landscape" },
                      ]}
                      onChange={(sceneMode) =>
                        persistSettings({ ...settings, sceneMode })
                      }
                    />
                  </div>
                  <div className="control">
                    Strength
                    <Segmented
                      value={settings.duplicateStrength}
                      options={[
                        { id: "loose", label: "Loose" },
                        { id: "balanced", label: "Balanced" },
                        { id: "strict", label: "Strict" },
                      ]}
                      onChange={(duplicateStrength) =>
                        persistSettings({ ...settings, duplicateStrength })
                      }
                    />
                  </div>
                  <div className="control">
                    Perf
                    <Segmented
                      value={settings.perfProfile}
                      options={[
                        { id: "low", label: "Low" },
                        { id: "medium", label: "Med" },
                        { id: "high", label: "High" },
                      ]}
                      onChange={(perfProfile) =>
                        persistSettings({ ...settings, perfProfile })
                      }
                    />
                  </div>
                </div>
              </div>
            </div>

            <aside className={`log-rail ${logOpen ? "open" : ""}`}>
              <button
                type="button"
                className="log-toggle"
                onClick={() => setLogOpen((v) => !v)}
              >
                {logOpen ? "Live Log ▾" : "Log"}
              </button>
              <div className="log-body panel" style={{ padding: "0.75rem" }}>
                <div className="log-lines">
                  {logs.length === 0 && (
                    <div className="log-line">Log is empty. Run Deduplicate to see activity.</div>
                  )}
                  {logs.map((l, i) => (
                    <div key={i} className={`log-line ${l.kind || ""}`}>
                      {l.ts}  {l.message}
                    </div>
                  ))}
                  <div ref={logEndRef} />
                </div>
                <div className="log-actions">
                  <button type="button" className="ghost-btn" onClick={exportLog} disabled={!logs.length}>
                    Export
                  </button>
                  <button type="button" className="ghost-btn" onClick={() => setLogs([])}>
                    Clear
                  </button>
                </div>
              </div>
            </aside>
          </div>
        ) : (
          <div className="coming-soon panel" style={{ margin: "1.25rem" }}>
            Styles are coming soon.
          </div>
        )}
      </div>

      {settingsOpen && (
        <div className="settings-drawer">
          <div style={{ display: "flex", justifyContent: "space-between" }}>
            <h2>Settings</h2>
            <button type="button" className="icon-btn" onClick={() => setSettingsOpen(false)}>
              ✕
            </button>
          </div>
          <div className="settings-section">
            <h3>App</h3>
            <button type="button" className="ghost-btn" onClick={openDocs}>
              Open documentation
            </button>
          </div>
          <div className="settings-section">
            <h3>Deduplicate</h3>
            <div className="field">
              <span>File action</span>
              <Segmented
                value={settings.fileAction}
                options={[
                  { id: "move", label: "Move" },
                  { id: "copy", label: "Copy" },
                ]}
                onChange={(fileAction) => persistSettings({ ...settings, fileAction })}
              />
            </div>
            <div className="field">
              <span>Scene mode</span>
              <Segmented
                value={settings.sceneMode}
                options={[
                  { id: "portrait", label: "Portrait" },
                  { id: "landscape", label: "Landscape" },
                ]}
                onChange={(sceneMode) => persistSettings({ ...settings, sceneMode })}
              />
            </div>
            <div className="field">
              <label>
                <input
                  type="checkbox"
                  checked={settings.includeSubfolders}
                  onChange={(e) =>
                    persistSettings({
                      ...settings,
                      includeSubfolders: e.target.checked,
                    })
                  }
                />{" "}
                Include subfolders
              </label>
            </div>
            <button
              type="button"
              className="ghost-btn"
              onClick={async () => {
                await invoke("clear_hash_cache");
                setLogs((prev) => [
                  ...prev,
                  { ts: nowTs(), message: "Hash cache cleared" },
                ]);
              }}
            >
              Clear hash cache
            </button>
          </div>
        </div>
      )}

      {result && (
        <div className="modal-backdrop">
          <div className="modal">
            <h2>Deduplicate complete</h2>
            <p>
              {result.duplicateGroups} duplicate groups resolved
              <br />
              {result.keptGood} kept in Images-Good · {result.rejected} moved/copied to Rejected
              <br />
              {result.uniqueUntouched} unique images also in Good
            </p>
            <div className="tree">
              {result.folder}
              <br />
              ├── Images-Good (green)
              <br />
              └── Rejected (red)
            </div>
            {result.errors.length > 0 && (
              <p style={{ color: "var(--aspen-reject)", fontSize: "0.85rem" }}>
                {result.errors.length} warnings — see log for details.
              </p>
            )}
            <div className="actions">
              <button
                type="button"
                className="primary-btn"
                onClick={() => openPath(result.folder)}
              >
                Open Folder
              </button>
              <button type="button" className="ghost-btn" onClick={() => setResult(null)}>
                Done
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
