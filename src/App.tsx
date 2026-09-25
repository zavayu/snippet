import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  cursorPosition,
  getCurrentWindow,
  LogicalSize,
  monitorFromPoint,
  PhysicalPosition,
} from "@tauri-apps/api/window";
import "./App.css";
import { SHOW_DEVELOPMENT_TOOLS } from "./config";

const POPUP_WIDTH = 510;
const POPUP_MIN_HEIGHT = 112;
const POPUP_MAX_HEIGHT = 720;
const POPUP_SHADOW_MARGIN = 24;

type CaptureState = "waiting" | "captured" | "error";
type GenerationState = "idle" | "loading" | "streaming" | "complete" | "error";
type QuickAction = "explain" | "summarize" | "refine";

const QUICK_ACTIONS: QuickAction[] = ["explain", "summarize", "refine"];

const QUICK_ACTION_LABELS: Record<QuickAction, string> = {
  explain: "Explain",
  summarize: "Summarize",
  refine: "Refine",
};

type CapturedSelection = {
  text: string;
  source: "clipboard-copy" | "ui-automation";
  truncated: boolean;
  clipboardRestored: boolean;
};

type CaptureFailure = {
  message: string;
};

type PromptIntent =
  | { kind: "quick-action"; action: QuickAction }
  | { kind: "custom"; instruction: string };

type ChatMessage = {
  role: "system" | "user";
  content: string;
};

type AppConfig = {
  ollamaBaseUrl: string;
  model: string | null;
};

type SettingsSnapshot = {
  config: AppConfig;
  effectiveModel: string | null;
  modelSource: "settings" | "environment" | "none";
};

type ModelInfo = {
  name: string;
};

type ProviderStatus =
  | { state: "ready"; models: ModelInfo[] }
  | { state: "unavailable"; message: string }
  | { state: "modelNotConfigured"; models: ModelInfo[] }
  | { state: "modelMissing"; model: string; models: ModelInfo[] };

type GenerationStarted = {
  requestId: number;
};

type GenerationChunk = {
  requestId: number;
  delta: string;
};

type GenerationFinished = {
  requestId: number;
};

type GenerationFailure = {
  requestId: number;
  message: string;
};

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function App() {
  const [captureState, setCaptureState] = useState<CaptureState>("waiting");
  const [selection, setSelection] = useState<CapturedSelection | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [customInstruction, setCustomInstruction] = useState("");
  const [generationState, setGenerationState] = useState<GenerationState>("idle");
  const [responseText, setResponseText] = useState("");
  const [generationError, setGenerationError] = useState<string | null>(null);
  const [settings, setSettings] = useState<SettingsSnapshot | null>(null);
  const [settingsDraft, setSettingsDraft] = useState<AppConfig | null>(null);
  const [providerStatus, setProviderStatus] = useState<ProviderStatus | null>(null);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isSettingsLoading, setIsSettingsLoading] = useState(true);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [isSelectionExpanded, setIsSelectionExpanded] = useState(false);
  const [promptPreview, setPromptPreview] = useState<ChatMessage[] | null>(null);
  const [promptPreviewError, setPromptPreviewError] = useState<string | null>(null);
  const [isPromptPreviewLoading, setIsPromptPreviewLoading] = useState(false);
  const [isPromptPreviewExpanded, setIsPromptPreviewExpanded] = useState(false);
  const popupContentRef = useRef<HTMLDivElement>(null);
  const lastPositionedSelectionRef = useRef<CapturedSelection | null>(null);
  const activeGenerationRef = useRef<number | null>(null);
  const lastIntentRef = useRef<PromptIntent | null>(null);
  const responseTextRef = useRef("");

  const showDevelopmentPreviews = SHOW_DEVELOPMENT_TOOLS;
  const isGenerating = generationState === "loading" || generationState === "streaming";
  const hasSubmittedPrompt = generationState !== "idle" || Boolean(responseText) || Boolean(generationError);
  const availableModels = providerStatus && providerStatus.state !== "unavailable"
    ? providerStatus.models
    : [];

  const refreshProviderStatus = async () => {
    setIsSettingsLoading(true);
    try {
      setProviderStatus(await invoke<ProviderStatus>("get_provider_status"));
      setSettingsError(null);
    } catch (error) {
      setSettingsError(errorText(error));
    } finally {
      setIsSettingsLoading(false);
    }
  };

  const loadSettings = async () => {
    setIsSettingsLoading(true);
    try {
      const [snapshot, status] = await Promise.all([
        invoke<SettingsSnapshot>("get_settings"),
        invoke<ProviderStatus>("get_provider_status"),
      ]);
      setSettings(snapshot);
      setSettingsDraft(snapshot.config);
      setProviderStatus(status);
      setSettingsError(null);
    } catch (error) {
      setSettingsError(errorText(error));
    } finally {
      setIsSettingsLoading(false);
    }
  };

  useEffect(() => {
    void loadSettings();
  }, []);

  useEffect(() => {
    const unlisten = Promise.all([
      listen<CapturedSelection>("selection-captured", (event) => {
        void invoke("cancel_generation");
        activeGenerationRef.current = null;
        lastIntentRef.current = null;
        setSelection(event.payload);
        setErrorMessage(null);
        setCustomInstruction("");
        setGenerationState("idle");
        responseTextRef.current = "";
        setResponseText("");
        setGenerationError(null);
        setIsSelectionExpanded(false);
        setPromptPreview(null);
        setPromptPreviewError(null);
        setIsPromptPreviewExpanded(false);
        setCaptureState("captured");
      }),
      listen<CaptureFailure>("selection-capture-failed", (event) => {
        setSelection(null);
        setErrorMessage(event.payload.message);
        setCaptureState("error");
      }),
      listen<GenerationStarted>("generation-started", (event) => {
        activeGenerationRef.current = event.payload.requestId;
        setGenerationState("loading");
        responseTextRef.current = "";
        setResponseText("");
        setGenerationError(null);
      }),
      listen<GenerationChunk>("generation-chunk", (event) => {
        if (activeGenerationRef.current !== event.payload.requestId) {
          return;
        }
        setGenerationState("streaming");
        setResponseText((response) => {
          const nextResponse = response + event.payload.delta;
          responseTextRef.current = nextResponse;
          return nextResponse;
        });
      }),
      listen<GenerationFinished>("generation-finished", (event) => {
        if (activeGenerationRef.current !== event.payload.requestId) {
          return;
        }
        activeGenerationRef.current = null;
        setGenerationState("complete");
      }),
      listen<GenerationFinished>("generation-cancelled", (event) => {
        if (activeGenerationRef.current !== event.payload.requestId) {
          return;
        }
        activeGenerationRef.current = null;
        setGenerationState("complete");
      }),
      listen<GenerationFailure>("generation-failed", (event) => {
        if (activeGenerationRef.current !== event.payload.requestId) {
          return;
        }
        activeGenerationRef.current = null;
        setGenerationState("error");
        setGenerationError(event.payload.message);
      }),
    ]);

    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        void invoke("cancel_generation");
        void getCurrentWindow().hide();
      }
    };

    window.addEventListener("keydown", closeOnEscape);

    return () => {
      window.removeEventListener("keydown", closeOnEscape);
      void unlisten.then((handlers: UnlistenFn[]) => handlers.forEach((handler) => handler()));
    };
  }, []);

  useLayoutEffect(() => {
    let cancelled = false;
    const shouldReposition = selection !== lastPositionedSelectionRef.current;
    if (shouldReposition) {
      lastPositionedSelectionRef.current = selection;
    }

    const frame = window.requestAnimationFrame(() => {
      const contentHeight = popupContentRef.current?.getBoundingClientRect().height ?? POPUP_MIN_HEIGHT;
      const nextHeight = Math.min(
        Math.max(Math.ceil(contentHeight) + POPUP_SHADOW_MARGIN * 2, POPUP_MIN_HEIGHT),
        POPUP_MAX_HEIGHT,
      );

      void (async () => {
        const popupWindow = getCurrentWindow();
        await popupWindow.setSize(new LogicalSize(POPUP_WIDTH, nextHeight));

        if (!shouldReposition) {
          return;
        }

        const cursor = await cursorPosition();
        const monitor = await monitorFromPoint(cursor.x, cursor.y);
        const popupSize = await popupWindow.outerSize();

        if (cancelled || !monitor) {
          return;
        }

        const workArea = monitor.workArea;
        const minX = workArea.position.x;
        const minY = workArea.position.y;
        const maxX = Math.max(minX, workArea.position.x + workArea.size.width - popupSize.width);
        const maxY = Math.max(minY, workArea.position.y + workArea.size.height - popupSize.height);
        const x = Math.min(Math.max(cursor.x + 14, minX), maxX);
        const y = cursor.y + 18 <= maxY
          ? cursor.y + 18
          : Math.max(minY, cursor.y - 18 - popupSize.height);

        await popupWindow.setPosition(new PhysicalPosition(x, y));
      })().catch(() => undefined);
    });

    return () => {
      cancelled = true;
      window.cancelAnimationFrame(frame);
    };
  }, [
    captureState,
    errorMessage,
    generationError,
    generationState,
    isPromptPreviewExpanded,
    isPromptPreviewLoading,
    isSelectionExpanded,
    isSettingsOpen,
    isSettingsLoading,
    promptPreview,
    promptPreviewError,
    providerStatus,
    responseText,
    selection,
    settingsError,
  ]);

  const resolveIntent = (intentOverride?: PromptIntent): PromptIntent => {
    if (intentOverride) {
      return intentOverride;
    }

    const instruction = customInstruction.trim();
    if (instruction) {
      return { kind: "custom", instruction };
    }

    return lastIntentRef.current ?? { kind: "quick-action", action: "explain" };
  };

  const startGeneration = async (intentOverride?: PromptIntent) => {
    if (!selection) {
      return;
    }

    const intent = resolveIntent(intentOverride);
    lastIntentRef.current = intent;
    activeGenerationRef.current = null;
    setGenerationState("loading");
    responseTextRef.current = "";
    setResponseText("");
    setGenerationError(null);

    try {
      const started = await invoke<GenerationStarted>("generate_for_selection", {
        request: {
          selectedText: selection.text,
          intent,
        },
      });
      activeGenerationRef.current = started.requestId;
    } catch (error) {
      setGenerationState("error");
      setGenerationError(errorText(error));
    }
  };

  const previewPrompt = async () => {
    if (!selection || !showDevelopmentPreviews) {
      return;
    }

    setIsPromptPreviewLoading(true);
    setPromptPreview(null);
    setPromptPreviewError(null);
    setIsPromptPreviewExpanded(true);

    try {
      const messages = await invoke<ChatMessage[]>("preview_prompt", {
        request: {
          selectedText: selection.text,
          intent: resolveIntent(),
        },
      });
      setPromptPreview(messages);
    } catch (error) {
      setPromptPreviewError(errorText(error));
    } finally {
      setIsPromptPreviewLoading(false);
    }
  };

  const saveSettings = async () => {
    if (!settingsDraft) {
      return;
    }

    setIsSettingsLoading(true);
    try {
      const snapshot = await invoke<SettingsSnapshot>("save_settings", { config: settingsDraft });
      setSettings(snapshot);
      setSettingsDraft(snapshot.config);
      setSettingsError(null);
      await refreshProviderStatus();
    } catch (error) {
      setSettingsError(errorText(error));
      setIsSettingsLoading(false);
    }
  };

  const closePopup = () => {
    void invoke("cancel_generation");
    void getCurrentWindow().hide();
  };

  const configuredModelIsListed = settingsDraft?.model
    && availableModels.some((model) => model.name === settingsDraft.model);

  return (
    <main className="min-h-screen overflow-hidden p-6 text-zinc-100">
      <section className="mx-auto w-full max-w-xl">
        <div ref={popupContentRef} className="rounded-[28px] bg-zinc-900 p-4 shadow-lg shadow-black/50">
          <header
            className="relative -mx-4 -mt-4 mb-3 flex cursor-grab select-none items-center gap-3 pl-4 pr-24 pt-4 active:cursor-grabbing"
            onMouseDown={(event) => {
              if (event.button === 0) {
                void getCurrentWindow().startDragging().catch(() => undefined);
              }
            }}
          >
            <div data-tauri-drag-region className="flex shrink-0 items-center gap-2">
              <div data-tauri-drag-region className="grid size-8 place-items-center rounded-lg bg-zinc-100 text-xs font-bold text-zinc-950">
                S
              </div>
            </div>
            {!hasSubmittedPrompt && (
              <form
                className="min-w-0 flex-1"
                onMouseDown={(event) => event.stopPropagation()}
                onSubmit={(event) => {
                  event.preventDefault();
                  void startGeneration();
                }}
              >
                <input
                  aria-label="Ask about the selection"
                  className="w-full bg-transparent px-1 py-2 text-[17px] text-zinc-100 outline-none placeholder:text-zinc-500"
                  placeholder="Ask about this…"
                  type="text"
                  value={customInstruction}
                  onChange={(event) => {
                    setCustomInstruction(event.target.value);
                    lastIntentRef.current = null;
                    setPromptPreview(null);
                    setPromptPreviewError(null);
                    setIsPromptPreviewExpanded(false);
                  }}
                />
              </form>
            )}
            {hasSubmittedPrompt && isGenerating && (
              <div data-tauri-drag-region className="flex min-w-0 flex-1 items-center gap-2">
                <svg
                  aria-hidden="true"
                  viewBox="0 0 24 24"
                  className="size-4 shrink-0 animate-spin fill-none"
                >
                  <circle cx="12" cy="12" r="8" className="stroke-zinc-800" strokeWidth="2.5" />
                  <path d="M20 12a8 8 0 0 0-8-8" className="stroke-zinc-400" strokeLinecap="round" strokeWidth="2.5" />
                </svg>
                <span data-tauri-drag-region className="truncate px-1 py-2 text-[17px] italic text-zinc-400">
                  {generationState === "loading" ? "Thinking…" : "Responding…"}
                </span>
              </div>
            )}
            <button
              type="button"
              aria-label="Open local AI settings"
              title="Settings"
              className="absolute right-11 top-4 grid size-8 cursor-pointer place-items-center rounded-md text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
              onMouseDown={(event) => event.stopPropagation()}
              onClick={() => {
                setIsSettingsOpen((open) => !open);
                void refreshProviderStatus();
              }}
            >
              <svg aria-hidden="true" viewBox="0 0 24 24" className="size-4 fill-none stroke-current stroke-[1.8]">
                <path strokeLinecap="round" strokeLinejoin="round" d="M10.3 3.6a1.9 1.9 0 0 1 3.4 0l.4 1a1.9 1.9 0 0 0 2 1.1l1.1-.2a1.9 1.9 0 0 1 2.4 2.4l-.2 1.1a1.9 1.9 0 0 0 1.1 2l1 .4a1.9 1.9 0 0 1 0 3.4l-1 .4a1.9 1.9 0 0 0-1.1 2l.2 1.1a1.9 1.9 0 0 1-2.4 2.4l-1.1-.2a1.9 1.9 0 0 0-2 1.1l-.4 1a1.9 1.9 0 0 1-3.4 0l-.4-1a1.9 1.9 0 0 0-2-1.1l-1.1.2a1.9 1.9 0 0 1-2.4-2.4l.2-1.1a1.9 1.9 0 0 0-1.1-2l-1-.4a1.9 1.9 0 0 1 0-3.4l1-.4a1.9 1.9 0 0 0 1.1-2l-.2-1.1a1.9 1.9 0 0 1 2.4-2.4l1.1.2a1.9 1.9 0 0 0 2-1.1l.4-1Z" />
                <circle cx="12" cy="12" r="3" />
              </svg>
            </button>
            <button
              type="button"
              aria-label="Close Snippet"
              className="absolute right-2 top-4 grid size-8 cursor-pointer place-items-center rounded-md text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
              onMouseDown={(event) => event.stopPropagation()}
              onClick={closePopup}
            >
              <span aria-hidden="true" className="text-xl font-light leading-none">×</span>
            </button>
          </header>

          {!hasSubmittedPrompt && (
            <div className="mb-5 flex flex-wrap gap-2">
              {QUICK_ACTIONS.map((action) => (
                <button
                  key={action}
                  type="button"
                  className="cursor-pointer rounded-full bg-zinc-700 px-3.5 py-1.5 text-[10px] font-extrabold text-zinc-300 transition-colors hover:bg-zinc-700 hover:text-zinc-100"
                  onClick={() => void startGeneration({ kind: "quick-action", action })}
                >
                  {QUICK_ACTION_LABELS[action]}
                </button>
              ))}
            </div>
          )}

          {isSettingsOpen && (
            <section className="mb-5 rounded-xl border border-zinc-800 bg-zinc-950/45 p-3">
              <div className="mb-3 flex items-center justify-between gap-3">
                <div>
                  <p className="text-xs font-semibold text-zinc-200">Local AI</p>
                  <p className="text-[10px] text-zinc-500">Ollama runs on your computer.</p>
                </div>
                <button
                  type="button"
                  className="text-[10px] font-medium text-zinc-400 transition-colors hover:text-zinc-100"
                  onClick={() => void refreshProviderStatus()}
                >
                  Refresh
                </button>
              </div>

              <label className="mb-3 block text-[10px] font-medium text-zinc-500">
                Ollama address
                <input
                  className="mt-1 w-full rounded-lg border border-zinc-800 bg-zinc-900 px-2.5 py-2 text-xs text-zinc-200 outline-none focus:border-zinc-600"
                  value={settingsDraft?.ollamaBaseUrl ?? ""}
                  onChange={(event) => setSettingsDraft((draft) => draft && ({
                    ...draft,
                    ollamaBaseUrl: event.target.value,
                  }))}
                />
              </label>

              <label className="block text-[10px] font-medium text-zinc-500">
                Model
                <select
                  className="mt-1 w-full rounded-lg border border-zinc-800 bg-zinc-900 px-2.5 py-2 text-xs text-zinc-200 outline-none focus:border-zinc-600"
                  value={settingsDraft?.model ?? ""}
                  onChange={(event) => setSettingsDraft((draft) => draft && ({
                    ...draft,
                    model: event.target.value || null,
                  }))}
                >
                  <option value="">Choose an installed model</option>
                  {settingsDraft?.model && !configuredModelIsListed && (
                    <option value={settingsDraft.model}>{settingsDraft.model} (not installed)</option>
                  )}
                  {availableModels.map((model) => (
                    <option key={model.name} value={model.name}>{model.name}</option>
                  ))}
                </select>
              </label>

              {settings?.modelSource === "environment" && (
                <p className="mt-2 text-[10px] leading-4 text-zinc-500">
                  `SNIPPET_OLLAMA_MODEL` is overriding the saved model for this development session.
                </p>
              )}
              {providerStatus && (
                <p className="mt-2 text-[10px] leading-4 text-zinc-500">
                  {providerStatus.state === "ready" && "Ollama is ready."}
                  {providerStatus.state === "unavailable" && providerStatus.message}
                  {providerStatus.state === "modelNotConfigured" && "Choose one of the installed models."}
                  {providerStatus.state === "modelMissing" && `${providerStatus.model} is not installed.`}
                </p>
              )}
              {settingsError && <p className="mt-2 text-[10px] leading-4 text-amber-200">{settingsError}</p>}

              <div className="mt-3 flex items-center justify-between gap-3">
                <span className="text-[10px] text-zinc-600">
                  {isSettingsLoading ? "Checking Ollama…" : ""}
                </span>
                <button
                  type="button"
                  className="rounded-full bg-zinc-100 px-3 py-1.5 text-[10px] font-bold text-zinc-950 transition-colors hover:bg-white disabled:cursor-wait disabled:opacity-60"
                  disabled={!settingsDraft || isSettingsLoading}
                  onClick={() => void saveSettings()}
                >
                  Save settings
                </button>
              </div>
            </section>
          )}

          {hasSubmittedPrompt && (
            <section className="mb-5">
              {responseText && (
                <div className="response-scroll max-h-80 overflow-y-auto whitespace-pre-wrap text-sm leading-5 text-zinc-200">
                  {responseText}
                </div>
              )}
              {generationError && <p className="text-xs leading-5 text-amber-200">{generationError}</p>}
            </section>
          )}

          {captureState === "captured" && selection && showDevelopmentPreviews && (
            <div>
              <button
                type="button"
                aria-expanded={isSelectionExpanded}
                className="flex cursor-pointer items-center gap-2 text-[11px] font-medium text-zinc-500 transition-colors hover:text-zinc-300"
                onClick={() => setIsSelectionExpanded((expanded) => !expanded)}
              >
                <span aria-hidden="true">{isSelectionExpanded ? "−" : "+"}</span>
                Selected Text
              </button>
              {isSelectionExpanded && (
                <blockquote className="preview-scroll mt-3 max-h-44 overflow-y-auto whitespace-pre-wrap rounded-lg border border-zinc-800/80 bg-zinc-950/50 p-3 text-[10px] leading-4 text-zinc-400">
                  {selection.text}
                </blockquote>
              )}

              <section className="mt-4">
                <button
                  type="button"
                  aria-expanded={isPromptPreviewExpanded}
                  className="flex cursor-pointer items-center gap-2 text-[11px] font-medium text-zinc-500 transition-colors hover:text-zinc-300"
                  onClick={() => {
                    if (isPromptPreviewExpanded) {
                      setIsPromptPreviewExpanded(false);
                      return;
                    }
                    setIsPromptPreviewExpanded(true);
                    if (!promptPreview && !promptPreviewError && !isPromptPreviewLoading) {
                      void previewPrompt();
                    }
                  }}
                >
                  <span aria-hidden="true">{isPromptPreviewExpanded ? "−" : "+"}</span>
                  Prompt Preview
                </button>
                {isPromptPreviewExpanded && (
                  <div className="mt-3">
                    {isPromptPreviewLoading && <p className="text-xs leading-5 text-zinc-500">Building prompt…</p>}
                    {promptPreviewError && <p className="text-xs leading-5 text-zinc-500">{promptPreviewError}</p>}
                    {promptPreview && (
                      <div className="preview-scroll max-h-44 space-y-3 overflow-y-auto rounded-lg border border-zinc-800/80 bg-zinc-950/50 p-3">
                        {promptPreview.map((message) => (
                          <div key={message.role}>
                            <p className="mb-1 text-[9px] font-bold uppercase tracking-wider text-zinc-600">{message.role}</p>
                            <pre className="whitespace-pre-wrap break-words font-mono text-[10px] leading-4 text-zinc-400">
                              {message.content}
                            </pre>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </section>
            </div>
          )}

          {captureState === "error" && (
            <p className="mt-3 text-xs leading-5 text-amber-200">{errorMessage}</p>
          )}
        </div>
      </section>
    </main>
  );
}

export default App;
