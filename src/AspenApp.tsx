import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import "./App.css";
import {
  AppSettings,
  ChatMessage,
  DEFAULT_SETTINGS,
  DeduplicateResult,
  DOCS_URL,
  EditStrength,
  ImageEditProgress,
  ImageEditResult,
  ProgressEvent,
} from "./types";

type FeatureId = "deduplicate" | "image-edit" | "styles";
type SettingPatch = Partial<AppSettings>;

interface LogLine {
  ts: string;
  feature: string;
  level: "info" | "warn" | "error" | "debug";
  message: string;
  kind?: "good" | "reject" | "plain";
}

function nowTs() {
  return new Date().toLocaleTimeString([], { hour12: false });
}

function redactPaths(message: string) {
  return message.replace(/\/(?:Users|Volumes|private|tmp)\/[^\s,;)]+/g, "[path]");
}

function latestUserFeedback(messages: ChatMessage[]) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index].role === "user") return messages[index].content;
  }
  return undefined;
}

function Segmented<T extends string>({
  value,
  options,
  disabled,
  onChange,
}: {
  value: T;
  options: { id: T; label: string }[];
  disabled?: boolean;
  onChange: (value: T) => void;
}) {
  return (
    <div className={`segmented ${disabled ? "disabled" : ""}`}>
      {options.map((option) => (
        <button
          key={option.id}
          type="button"
          className={value === option.id ? "on" : ""}
          disabled={disabled}
          onClick={() => onChange(option.id)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function Check({
  checked,
  disabled,
  label,
  detail,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  label: string;
  detail?: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className={`check-row ${disabled ? "disabled" : ""}`}>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>
        <strong>{label}</strong>
        {detail && <small>{detail}</small>}
      </span>
    </label>
  );
}

function RecipeControl({
  label,
  detail,
  enabled,
  strength,
  onEnabled,
  onStrength,
}: {
  label: string;
  detail: string;
  enabled: boolean;
  strength: EditStrength;
  onEnabled: (value: boolean) => void;
  onStrength: (value: EditStrength) => void;
}) {
  return (
    <div className="recipe-row">
      <Check checked={enabled} label={label} detail={detail} onChange={onEnabled} />
      <Segmented
        value={strength}
        disabled={!enabled}
        options={[
          { id: "small", label: "Small" },
          { id: "medium", label: "Medium" },
          { id: "high", label: "High" },
        ]}
        onChange={onStrength}
      />
    </div>
  );
}

export default function AspenApp() {
  const [feature, setFeatureState] = useState<FeatureId>("deduplicate");
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const settingsRef = useRef(settings);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [folder, setFolder] = useState("");
  const [editSource, setEditSource] = useState("");
  const [dragging, setDragging] = useState(false);
  const [running, setRunning] = useState(false);
  const [editRunning, setEditRunning] = useState(false);
  const [logOpen, setLogOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [logs, setLogs] = useState<LogLine[]>([]);
  const [progress, setProgress] = useState({ current: 0, total: 0 });
  const [result, setResult] = useState<DeduplicateResult | null>(null);
  const [editResult, setEditResult] = useState<ImageEditResult | null>(null);
  const [chat, setChat] = useState<ChatMessage[]>([]);
  const [chatInput, setChatInput] = useState("");
  const [chatSending, setChatSending] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const logEndRef = useRef<HTMLDivElement>(null);

  function addLog(
    message: string,
    featureName: string = feature,
    level: LogLine["level"] = "info",
    kind: LogLine["kind"] = "plain",
  ) {
    setLogs((previous) => [
      ...previous.slice(-(2_000 - 1)),
      { ts: nowTs(), message, feature: featureName, level, kind },
    ]);
    const diskMessage = settingsRef.current.includeFullPathsInLogs
      ? message
      : redactPaths(message);
    void invoke("record_ui_event", {
      feature: featureName,
      action: "ui.event",
      message: diskMessage,
      level,
    }).catch((error) => console.warn("Structured UI logging unavailable", error));
  }

  function updateSettings(patch: SettingPatch) {
    setSettings((current) => {
      const next = { ...current, ...patch };
      settingsRef.current = next;
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => {
        invoke("save_app_settings", { settings: next }).catch((error) => {
          addLog(`Settings save failed: ${String(error)}`, "app", "error");
        });
      }, 100);
      return next;
    });
  }

  function setFeature(next: FeatureId) {
    if (
      feature === "image-edit" &&
      next !== "image-edit" &&
      settings.chatAutoClearOnLeave
    ) {
      setChat([]);
    }
    addLog(`Navigated from ${feature} to ${next}`, "app");
    setFeatureState(next);
  }

  useEffect(() => {
    invoke<AppSettings>("get_settings")
      .then((saved) => {
        const next = { ...DEFAULT_SETTINGS, ...saved };
        settingsRef.current = next;
        setSettings(next);
        setEditSource(next.lastImagesGoodPath || "");
      })
      .catch((error) => {
        addLog(`Settings load failed; defaults restored: ${String(error)}`, "app", "error");
      });
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === "over") setDragging(true);
          if (event.payload.type === "leave") setDragging(false);
          if (event.payload.type === "drop") {
            setDragging(false);
            const path = event.payload.paths?.[0];
            if (path) {
              if (feature === "image-edit") setEditSource(path);
              else setFolder(path);
              addLog(`Folder selected: ${path}`, feature);
            }
          }
        }),
      )
      .then((cleanup) => {
        unlisten = cleanup;
      })
      .catch((error) => addLog(`Drag/drop unavailable: ${String(error)}`, "app", "warn"));
    return () => unlisten?.();
  }, [feature]);

  useEffect(() => {
    let stopDedupe: (() => void) | undefined;
    let stopEdit: (() => void) | undefined;
    void listen<ProgressEvent>("dedupe-progress", ({ payload }) => {
      setProgress({ current: payload.current, total: payload.total || 1 });
      addLog(
        payload.message,
        "deduplicate",
        "info",
        payload.message.includes("→ Good")
          ? "good"
          : payload.message.includes("→ Rejected")
            ? "reject"
            : "plain",
      );
    }).then((cleanup) => {
      stopDedupe = cleanup;
    });
    void listen<ImageEditProgress>("image-edit-progress", ({ payload }) => {
      setProgress({ current: payload.current, total: payload.total || 1 });
      addLog(
        payload.message,
        "image-edit",
        payload.level === "warn" ? "warn" : "info",
      );
    }).then((cleanup) => {
      stopEdit = cleanup;
    });
    return () => {
      stopDedupe?.();
      stopEdit?.();
    };
  }, []);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logs]);

  const pct = useMemo(
    () =>
      progress.total
        ? Math.min(100, Math.round((progress.current / progress.total) * 100))
        : 0,
    [progress],
  );

  async function chooseFolder(target: "dedupe" | "edit") {
    addLog("Folder dialog opened", target === "edit" ? "image-edit" : "deduplicate");
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") {
      addLog("Folder dialog cancelled", target === "edit" ? "image-edit" : "deduplicate");
      return;
    }
    if (target === "edit") setEditSource(selected);
    else setFolder(selected);
  }

  async function runDedupe() {
    if (!folder || running) return;
    setRunning(true);
    setResult(null);
    setProgress({ current: 0, total: 0 });
    setLogOpen(true);
    addLog(`Starting Deduplicate on ${folder}`, "deduplicate");
    try {
      const completed = await invoke<DeduplicateResult>("run_deduplicate_cmd", {
        folder,
        settings: settingsRef.current,
      });
      setResult(completed);
      setEditSource(completed.goodDir);
      updateSettings({ lastImagesGoodPath: completed.goodDir });
    } catch (error) {
      addLog(`Deduplicate failed: ${String(error)}`, "deduplicate", "error");
    } finally {
      setRunning(false);
    }
  }

  async function runEdit(feedback = "") {
    if (!editSource || editRunning) return;
    setEditRunning(true);
    setEditResult(null);
    setProgress({ current: 0, total: 0 });
    setLogOpen(true);
    addLog(`Starting Image Editing on ${editSource}`, "image-edit");
    try {
      const completed = await invoke<ImageEditResult>("run_image_edit_cmd", {
        request: {
          sourcePath: editSource,
          settings: settingsRef.current,
          feedback,
        },
      });
      setEditResult(completed);
      completed.warnings.forEach((warning) => addLog(warning, "image-edit", "warn"));
      if (settingsRef.current.chatAutoClearAfterRun) setChat([]);
    } catch (error) {
      addLog(`Image Editing failed: ${String(error)}`, "image-edit", "error");
    } finally {
      setEditRunning(false);
    }
  }

  async function sendMessage() {
    const content = chatInput.trim();
    if (!content || chatSending) return;
    const next: ChatMessage[] = [
      ...chat,
      { role: "user" as const, content },
    ].slice(-20);
    setChat(next);
    setChatInput("");
    setChatSending(true);
    try {
      const assistant = await invoke<string>("send_ai_chat", {
        model: settings.ollamaModel,
        temperature: settings.ollamaTemperature,
        messages: next,
      });
      setChat((current) =>
        [...current, { role: "assistant" as const, content: assistant }].slice(-20),
      );
    } catch (error) {
      addLog(`AI chat failed: ${String(error)}`, "image-edit", "error");
    } finally {
      setChatSending(false);
    }
  }

  async function refreshModels() {
    try {
      const available = await invoke<string[]>("list_ollama_models_cmd");
      setModels(available);
      if (available.length && !available.includes(settings.ollamaModel)) {
        updateSettings({ ollamaModel: available[0] });
      }
    } catch (error) {
      addLog(`Ollama model discovery failed: ${String(error)}`, "image-edit", "warn");
    }
  }

  async function exportLog() {
    const path = await save({
      defaultPath: `aspen-log-${Date.now()}.jsonl`,
      filters: [{ name: "Structured log", extensions: ["jsonl", "log"] }],
    });
    if (!path) return;
    const content = logs
      .map((line) =>
        JSON.stringify({
          ...line,
          message: settings.includeFullPathsInLogs
            ? line.message
            : redactPaths(line.message),
        }),
      )
      .join("\n");
    await invoke("export_log", { path, content });
  }

  async function openDocs() {
    try {
      await openUrl((await invoke<string>("get_docs_url")) || DOCS_URL);
    } catch (error) {
      addLog(`Documentation open failed: ${String(error)}`, "app", "error");
      await openUrl(DOCS_URL);
    }
  }

  const latestFeedback = latestUserFeedback(chat);

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
          <button
            type="button"
            className={`nav-item ${feature === "image-edit" ? "active" : ""}`}
            onClick={() => setFeature("image-edit")}
          >
            <span className="label">Image Editing</span>
            <span className="hint">Lightroom portrait finish</span>
          </button>
          <button type="button" className="nav-item" disabled>
            <span className="label">Styles</span>
            <span className="hint">Coming later</span>
          </button>
        </div>
      </aside>

      <main className="main-stage">
        <div className="chrome">
          {running && (
            <button
              type="button"
              className="ghost-btn"
              onClick={() => invoke("cancel_deduplicate")}
            >
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

        {feature === "deduplicate" && (
          <WorkspaceWithLog log={renderLogRail()}>
            <div className="workspace-header">
              <h1>Deduplicate</h1>
              <p>Keep the sharpest frame. Retire the rest.</p>
            </div>
            <div className="panel">
              <DropZone
                dragging={dragging}
                title="Drop a folder of images"
                path={folder}
                onChoose={() => chooseFolder("dedupe")}
              />
              <button
                type="button"
                className="primary-btn"
                disabled={!folder || running}
                onClick={runDedupe}
              >
                {running ? "Working…" : "Run Deduplicate"}
              </button>
              {running && <Progress current={progress.current} total={progress.total} pct={pct} />}
              <div className="controls-row">
                <div className="control">
                  Scene
                  <Segmented
                    value={settings.sceneMode}
                    options={[
                      { id: "portrait", label: "Portrait" },
                      { id: "landscape", label: "Landscape" },
                    ]}
                    onChange={(sceneMode) => updateSettings({ sceneMode })}
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
                    onChange={(duplicateStrength) => updateSettings({ duplicateStrength })}
                  />
                </div>
                <div className="control">
                  Performance
                  <Segmented
                    value={settings.perfProfile}
                    options={[
                      { id: "low", label: "Low" },
                      { id: "medium", label: "Medium" },
                      { id: "high", label: "High" },
                    ]}
                    onChange={(perfProfile) => updateSettings({ perfProfile })}
                  />
                </div>
              </div>
              <div className="handoff-check">
                <Check
                  checked={settings.continueToImageEditing}
                  label="Continue to Image Editing"
                  detail="Use Images-Good as the Lightroom source after this run."
                  onChange={(continueToImageEditing) =>
                    updateSettings({ continueToImageEditing })
                  }
                />
              </div>
            </div>
          </WorkspaceWithLog>
        )}

        {feature === "image-edit" && (
          <WorkspaceWithLog log={renderLogRail()}>
            <div className="workspace-header">
              <h1>Image Editing</h1>
              <p>Apply a tasteful portrait finish through Lightroom Classic.</p>
            </div>
            <div className={`panel edit-layout ${settings.useAiForEdit ? "with-chat" : ""}`}>
              <section>
                <DropZone
                  dragging={dragging}
                  title="Choose Images-Good"
                  path={editSource}
                  onChoose={() => chooseFolder("edit")}
                />
                <div className="ai-run-toggle">
                  <Check
                    checked={settings.useAiForEdit}
                    disabled={!settings.enableAiFeatures}
                    label="Use AI for this edit"
                    detail={
                      settings.enableAiFeatures
                        ? "Combine your chat feedback with the checked Lightroom recipes."
                        : "Enable AI features in Settings to unlock chat."
                    }
                    onChange={(useAiForEdit) => {
                      updateSettings({ useAiForEdit });
                      if (!useAiForEdit && settings.chatAutoClearOnAiOff) setChat([]);
                    }}
                  />
                </div>
                <div className="recipe-list">
                  <RecipeControl
                    label="Eye sharpen"
                    detail="Masked global sharpening that protects skin."
                    enabled={settings.eyeSharpen}
                    strength={settings.eyeSharpenStrength}
                    onEnabled={(eyeSharpen) => updateSettings({ eyeSharpen })}
                    onStrength={(eyeSharpenStrength) => updateSettings({ eyeSharpenStrength })}
                  />
                  <RecipeControl
                    label="Slight vignette"
                    detail="Post-crop edge falloff."
                    enabled={settings.vignette}
                    strength={settings.vignetteStrength}
                    onEnabled={(vignette) => updateSettings({ vignette })}
                    onStrength={(vignetteStrength) => updateSettings({ vignetteStrength })}
                  />
                  <RecipeControl
                    label="Blur around subject"
                    detail="Uses Lens Blur or subject-mask fallback when MCP supports it."
                    enabled={settings.subjectBlur}
                    strength={settings.subjectBlurStrength}
                    onEnabled={(subjectBlur) => updateSettings({ subjectBlur })}
                    onStrength={(subjectBlurStrength) => updateSettings({ subjectBlurStrength })}
                  />
                  <div className="compact-checks">
                    {[
                      ["optimalCrop", "Optimal crop"],
                      ["whiteBalance", "White balance"],
                      ["colorTone", "Color tone"],
                      ["exposureNormalize", "Exposure normalize"],
                      ["noiseReduction", "Noise reduction"],
                    ].map(([key, label]) => (
                      <Check
                        key={key}
                        checked={Boolean(settings[key as keyof AppSettings])}
                        label={label}
                        onChange={(value) => updateSettings({ [key]: value } as SettingPatch)}
                      />
                    ))}
                  </div>
                </div>
                <button
                  type="button"
                  className="primary-btn"
                  disabled={!editSource || editRunning}
                  onClick={() => runEdit()}
                >
                  {editRunning ? "Processing…" : "Process with Lightroom"}
                </button>
                {editRunning && (
                  <Progress current={progress.current} total={progress.total} pct={pct} />
                )}
              </section>

              {settings.enableAiFeatures && settings.useAiForEdit && (
                <section className="chat-panel">
                  <div className="chat-header">
                    <strong>AI edit chat</strong>
                    <button type="button" className="ghost-btn" onClick={() => setChat([])}>
                      Clear
                    </button>
                  </div>
                  <div className="chat-messages">
                    {!chat.length && (
                      <p>Describe a change such as “less vignette and sharper eyes.”</p>
                    )}
                    {chat.map((message, index) => (
                      <div key={`${message.role}-${index}`} className={`chat-message ${message.role}`}>
                        {message.content}
                      </div>
                    ))}
                  </div>
                  <textarea
                    value={chatInput}
                    maxLength={4_000}
                    placeholder="Describe the finish…"
                    onChange={(event) => setChatInput(event.target.value)}
                  />
                  <div className="chat-actions">
                    <button
                      type="button"
                      className="ghost-btn"
                      disabled={!chatInput.trim() || chatSending}
                      onClick={sendMessage}
                    >
                      {chatSending ? "Thinking…" : "Send"}
                    </button>
                    <button
                      type="button"
                      className="primary-btn"
                      disabled={!latestFeedback || editRunning}
                      onClick={() => runEdit(latestFeedback)}
                    >
                      Apply feedback
                    </button>
                  </div>
                </section>
              )}
            </div>
          </WorkspaceWithLog>
        )}
      </main>

      {settingsOpen && (
        <div className="settings-drawer">
          <div className="drawer-heading">
            <h2>Settings</h2>
            <button type="button" className="icon-btn" onClick={() => setSettingsOpen(false)}>
              ✕
            </button>
          </div>
          <section className="settings-section">
            <h3>Deduplicate</h3>
            <div className="field">
              <span>File action</span>
              <Segmented
                value={settings.fileAction}
                options={[
                  { id: "move", label: "Move" },
                  { id: "copy", label: "Copy" },
                ]}
                onChange={(fileAction) => updateSettings({ fileAction })}
              />
            </div>
            <Check
              checked={settings.includeSubfolders}
              label="Include subfolders"
              onChange={(includeSubfolders) => updateSettings({ includeSubfolders })}
            />
            <button
              type="button"
              className="ghost-btn"
              onClick={() =>
                invoke("clear_hash_cache")
                  .then(() => addLog("Hash cache cleared", "deduplicate"))
                  .catch((error) =>
                    addLog(`Cache clear failed: ${String(error)}`, "deduplicate", "error"),
                  )
              }
            >
              Clear hash cache
            </button>
          </section>
          <section className="settings-section">
            <h3>Image Editing</h3>
            <p className="settings-note">
              Lightroom MCP starts when processing begins. Exports use JPEG 90 and the next
              available Processed-Images folder.
            </p>
          </section>
          <section className="settings-section">
            <h3>AI / Models</h3>
            <Check
              checked={settings.enableAiFeatures}
              label="Enable AI features"
              detail="Off means Aspen never contacts Ollama."
              onChange={(enableAiFeatures) => {
                updateSettings({
                  enableAiFeatures,
                  ...(!enableAiFeatures ? { useAiForEdit: false } : {}),
                });
                if (!enableAiFeatures) setChat([]);
                else void refreshModels();
              }}
            />
            <div className={`ai-settings ${!settings.enableAiFeatures ? "disabled" : ""}`}>
              <label className="field">
                <span>Ollama model</span>
                <select
                  disabled={!settings.enableAiFeatures}
                  value={settings.ollamaModel}
                  onChange={(event) => updateSettings({ ollamaModel: event.target.value })}
                >
                  {[...new Set([settings.ollamaModel, ...models])].map((model) => (
                    <option key={model}>{model}</option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Temperature: {settings.ollamaTemperature.toFixed(1)}</span>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.1"
                  disabled={!settings.enableAiFeatures}
                  value={settings.ollamaTemperature}
                  onChange={(event) =>
                    updateSettings({ ollamaTemperature: Number(event.target.value) })
                  }
                />
              </label>
              <Check
                checked={settings.chatAutoClearAfterRun}
                disabled={!settings.enableAiFeatures}
                label="Clear chat after run"
                onChange={(chatAutoClearAfterRun) => updateSettings({ chatAutoClearAfterRun })}
              />
              <Check
                checked={settings.chatAutoClearOnLeave}
                disabled={!settings.enableAiFeatures}
                label="Clear chat when leaving Image Editing"
                onChange={(chatAutoClearOnLeave) => updateSettings({ chatAutoClearOnLeave })}
              />
              <Check
                checked={settings.chatAutoClearOnAiOff}
                disabled={!settings.enableAiFeatures}
                label="Clear chat when Use AI is turned off"
                onChange={(chatAutoClearOnAiOff) =>
                  updateSettings({ chatAutoClearOnAiOff })
                }
              />
            </div>
          </section>
          <section className="settings-section">
            <h3>Diagnostics</h3>
            <Check
              checked={settings.verboseLogging}
              label="Verbose logging"
              onChange={(verboseLogging) => updateSettings({ verboseLogging })}
            />
            <Check
              checked={settings.includeFullPathsInLogs}
              label="Include full paths in exports"
              onChange={(includeFullPathsInLogs) => updateSettings({ includeFullPathsInLogs })}
            />
            <Check
              checked={settings.includeChatPromptsInLogs}
              label="Include chat prompts in exports"
              onChange={(includeChatPromptsInLogs) =>
                updateSettings({ includeChatPromptsInLogs })
              }
            />
            <div className="drawer-actions">
              <button
                type="button"
                className="ghost-btn"
                onClick={async () => openPath(await invoke<string>("get_logs_dir"))}
              >
                Open Logs
              </button>
              <button type="button" className="ghost-btn" onClick={exportLog}>
                Export Run
              </button>
              <button
                type="button"
                className="ghost-btn"
                onClick={() =>
                  invoke("clear_app_logs").then(() => {
                    setLogs([]);
                  })
                }
              >
                Clear Logs
              </button>
            </div>
          </section>
          <button type="button" className="ghost-btn" onClick={openDocs}>
            Open documentation
          </button>
        </div>
      )}

      {result && (
        <div className="modal-backdrop">
          <div className="modal">
            <h2>Deduplicate complete</h2>
            <p>
              {result.duplicateGroups} groups resolved · {result.keptGood} kept ·{" "}
              {result.rejected} rejected · {result.uniqueLeft} unique untouched
            </p>
            <div className="tree">
              {result.folder}
              <br />├── Images-Good
              <br />└── Rejected
            </div>
            <div className="actions">
              {settings.continueToImageEditing && (
                <button
                  type="button"
                  className="primary-btn"
                  onClick={() => {
                    setResult(null);
                    setFeature("image-edit");
                  }}
                >
                  Continue to Image Editing
                </button>
              )}
              <button type="button" className="ghost-btn" onClick={() => openPath(result.folder)}>
                Open Folder
              </button>
              <button type="button" className="ghost-btn" onClick={() => setResult(null)}>
                Done
              </button>
            </div>
          </div>
        </div>
      )}

      {editResult && (
        <div className="modal-backdrop">
          <div className="modal">
            <h2>Image Editing complete</h2>
            <p>
              {editResult.processed} images exported to
              <br />
              <strong>{editResult.outputPath}</strong>
            </p>
            {editResult.warnings.length > 0 && (
              <p className="warning-text">{editResult.warnings.length} warnings — see log.</p>
            )}
            <div className="actions">
              <button
                type="button"
                className="primary-btn"
                onClick={() => openPath(editResult.outputPath)}
              >
                Open Processed Images
              </button>
              <button type="button" className="ghost-btn" onClick={() => setEditResult(null)}>
                Done
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );

  function renderLogRail() {
    return (
      <aside className={`log-rail ${logOpen ? "open" : ""}`}>
        <button type="button" className="log-toggle" onClick={() => setLogOpen((open) => !open)}>
          {logOpen ? "Live Log ▾" : "Log"}
        </button>
        <div className="log-body panel">
          <div className="log-lines">
            {!logs.length && <div className="log-line">Log is empty.</div>}
            {logs.map((line, index) => (
              <div
                key={`${line.ts}-${index}`}
                className={`log-line ${line.kind || ""} ${line.level}`}
              >
                {line.ts} [{line.feature}] {line.message}
              </div>
            ))}
            <div ref={logEndRef} />
          </div>
          <div className="log-actions">
            <button type="button" className="ghost-btn" disabled={!logs.length} onClick={exportLog}>
              Export
            </button>
            <button type="button" className="ghost-btn" onClick={() => setLogs([])}>
              Clear
            </button>
          </div>
        </div>
      </aside>
    );
  }
}

function WorkspaceWithLog({
  children,
  log,
}: {
  children: React.ReactNode;
  log: React.ReactNode;
}) {
  return (
    <div className="workspace">
      <div>{children}</div>
      {log}
    </div>
  );
}

function DropZone({
  dragging,
  title,
  path,
  onChoose,
}: {
  dragging: boolean;
  title: string;
  path: string;
  onChoose: () => void;
}) {
  return (
    <>
      <div className={`drop-zone compact ${dragging ? "active" : ""}`}>
        <h2>{title}</h2>
        <div className="formats">RAW · JPEG · PNG · DNG · TIFF · HEIC · …</div>
        <button type="button" className="ghost-btn" onClick={onChoose}>
          Choose Folder…
        </button>
      </div>
      <div className="path-line">Folder: {path || "—"}</div>
    </>
  );
}

function Progress({
  current,
  total,
  pct,
}: {
  current: number;
  total: number;
  pct: number;
}) {
  return (
    <div className="progress">
      <div className="progress-bar">
        <span style={{ width: `${pct}%` }} />
      </div>
      <div className="path-line">
        {current} / {total || "…"} ({pct}%)
      </div>
    </div>
  );
}
