import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
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
import { MarkdownResponse } from "./components/MarkdownResponse";
import { SHOW_DEVELOPMENT_TOOLS } from "./config";

const POPUP_WIDTH = 510;
const POPUP_MIN_HEIGHT = 112;
const POPUP_MAX_HEIGHT = 900;
const POPUP_SHADOW_MARGIN = 24;
const POPUP_CONTENT_INSET = 24;
const POPUP_CURSOR_GAP_X = 8;
const POPUP_CURSOR_GAP_Y = 10;
const POPUP_WINDOW_OFFSET_X = POPUP_CURSOR_GAP_X - POPUP_CONTENT_INSET;
const POPUP_WINDOW_OFFSET_Y = POPUP_CURSOR_GAP_Y - POPUP_CONTENT_INSET;

type CaptureState = "waiting" | "captured" | "error";
type GenerationState = "idle" | "loading" | "streaming" | "complete" | "error";
type OcrState = "idle" | "reading" | "ready" | "error";
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

type ShortcutConfig = {
  openPopup: string;
  summarize: string;
  explain: string;
  refine: string;
  captureScreen: string;
  captureRegion: string;
  readScreen: string;
  readRegion: string;
};

type SelectionCaptureEvent = {
  selection: CapturedSelection;
  action: QuickAction | null;
};

type ImageCaptureEvent = {
  imageId: number;
  source: "screen" | "region";
  preview: {
    dataUrl: string;
    width: number;
    height: number;
  };
  autoRead: boolean;
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
  ocrModel: string;
  ocrNumPredict: number;
  ocrNumCtx: number;
  thinking: boolean;
  visionEnabled: boolean;
  shortcuts: ShortcutConfig;
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

type ScreenReadFinished = {
  requestId: number;
  imageId: number;
  text: string;
};

type SettingsPage = "general" | "ocr" | "shortcuts";
type ShortcutKey = keyof ShortcutConfig;

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta"]);

function shortcutKeyFromCode(code: string) {
  if (/^Key[A-Z]$/.test(code)) {
    return code.slice(3);
  }
  if (/^Digit[0-9]$/.test(code)) {
    return code.slice(5);
  }
  if (/^F(?:[1-9]|1[0-9]|2[0-4])$/.test(code)) {
    return code;
  }

  return {
    Space: "Space",
    Enter: "Enter",
    Tab: "Tab",
    Backspace: "Backspace",
    Delete: "Delete",
    Insert: "Insert",
    Home: "Home",
    End: "End",
    PageUp: "PageUp",
    PageDown: "PageDown",
    ArrowUp: "ArrowUp",
    ArrowDown: "ArrowDown",
    ArrowLeft: "ArrowLeft",
    ArrowRight: "ArrowRight",
  }[code];
}

function shortcutFromKeypress(event: ReactKeyboardEvent) {
  if (MODIFIER_KEYS.has(event.key)) {
    return null;
  }

  const key = shortcutKeyFromCode(event.code);
  const modifiers = [
    event.ctrlKey && "Ctrl",
    event.altKey && "Alt",
    event.shiftKey && "Shift",
    event.metaKey && "Super",
  ].filter(Boolean);
  return key && modifiers.length > 0 ? [...modifiers, key].join("+") : null;
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function App() {
  const [captureState, setCaptureState] = useState<CaptureState>("waiting");
  const [selection, setSelection] = useState<CapturedSelection | null>(null);
  const [image, setImage] = useState<ImageCaptureEvent | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [customInstruction, setCustomInstruction] = useState("");
  const [generationState, setGenerationState] = useState<GenerationState>("idle");
  const [ocrState, setOcrState] = useState<OcrState>("idle");
  const [ocrSource, setOcrSource] = useState<ImageCaptureEvent["source"] | null>(null);
  const [ocrError, setOcrError] = useState<string | null>(null);
  const [queuedOcrIntent, setQueuedOcrIntent] = useState<PromptIntent | null>(null);
  const [responseText, setResponseText] = useState("");
  const [generationError, setGenerationError] = useState<string | null>(null);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "error">("idle");
  const [settings, setSettings] = useState<SettingsSnapshot | null>(null);
  const [settingsDraft, setSettingsDraft] = useState<AppConfig | null>(null);
  const [providerStatus, setProviderStatus] = useState<ProviderStatus | null>(null);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [settingsPage, setSettingsPage] = useState<SettingsPage>("general");
  const [recordingShortcut, setRecordingShortcut] = useState<ShortcutKey | null>(null);
  const [isSettingsLoading, setIsSettingsLoading] = useState(true);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [isSelectionExpanded, setIsSelectionExpanded] = useState(false);
  const [isAttachedTextExpanded, setIsAttachedTextExpanded] = useState(false);
  const [promptPreview, setPromptPreview] = useState<ChatMessage[] | null>(null);
  const [promptPreviewError, setPromptPreviewError] = useState<string | null>(null);
  const [isPromptPreviewLoading, setIsPromptPreviewLoading] = useState(false);
  const [isPromptPreviewExpanded, setIsPromptPreviewExpanded] = useState(false);
  const [isPopupSurfaceVisible, setIsPopupSurfaceVisible] = useState(false);
  const popupContentRef = useRef<HTMLDivElement>(null);
  const shouldPositionPopupRef = useRef(false);
  const activeGenerationRef = useRef<number | null>(null);
  const lastIntentRef = useRef<PromptIntent | null>(null);
  const pendingOcrIntentRef = useRef<PromptIntent | null>(null);
  const ocrImageIdRef = useRef<number | null>(null);
  const startGenerationRef = useRef<(
    intentOverride?: PromptIntent,
    selectionOverride?: CapturedSelection,
    bypassOcrWait?: boolean,
  ) => Promise<void> | null>(null);
  const startScreenReadRef = useRef<(
    imageId: number,
    source: ImageCaptureEvent["source"],
  ) => Promise<void> | null>(null);
  const responseTextRef = useRef("");
  const copyResetTimeoutRef = useRef<number | null>(null);

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

  useEffect(() => () => {
    if (copyResetTimeoutRef.current !== null) {
      window.clearTimeout(copyResetTimeoutRef.current);
    }
  }, []);

  useEffect(() => {
    const unlisten = Promise.all([
      listen("popup-opened", () => {
        shouldPositionPopupRef.current = true;
        setIsPopupSurfaceVisible(false);
        window.requestAnimationFrame(() => setIsPopupSurfaceVisible(true));
        void invoke("cancel_generation");
        activeGenerationRef.current = null;
        lastIntentRef.current = null;
        pendingOcrIntentRef.current = null;
        ocrImageIdRef.current = null;
        setSelection(null);
        setImage(null);
        setOcrState("idle");
        setOcrSource(null);
        setOcrError(null);
        setQueuedOcrIntent(null);
        setErrorMessage(null);
        setCustomInstruction("");
        setGenerationState("idle");
        responseTextRef.current = "";
        setResponseText("");
        setGenerationError(null);
        setCopyState("idle");
        setIsSelectionExpanded(false);
        setIsAttachedTextExpanded(false);
        setPromptPreview(null);
        setPromptPreviewError(null);
        setIsPromptPreviewLoading(false);
        setIsPromptPreviewExpanded(false);
        setCaptureState("waiting");
      }),
      listen<SelectionCaptureEvent>("selection-captured", (event) => {
        const capturedSelection = event.payload.selection;
        shouldPositionPopupRef.current = true;
        setIsPopupSurfaceVisible(false);
        window.requestAnimationFrame(() => setIsPopupSurfaceVisible(true));
        void invoke("cancel_generation");
        activeGenerationRef.current = null;
        lastIntentRef.current = null;
        pendingOcrIntentRef.current = null;
        ocrImageIdRef.current = null;
        setSelection(capturedSelection);
        setImage(null);
        setOcrState("idle");
        setOcrSource(null);
        setOcrError(null);
        setQueuedOcrIntent(null);
        setErrorMessage(null);
        setCustomInstruction("");
        setGenerationState("idle");
        responseTextRef.current = "";
        setResponseText("");
        setGenerationError(null);
        setCopyState("idle");
        setIsSelectionExpanded(false);
        setIsAttachedTextExpanded(false);
        setPromptPreview(null);
        setPromptPreviewError(null);
        setIsPromptPreviewExpanded(false);
        setCaptureState("captured");
        if (event.payload.action) {
          void startGeneration(
            { kind: "quick-action", action: event.payload.action },
            capturedSelection,
          );
        }
      }),
      listen<ImageCaptureEvent>("image-captured", (event) => {
        shouldPositionPopupRef.current = true;
        setIsPopupSurfaceVisible(false);
        window.requestAnimationFrame(() => setIsPopupSurfaceVisible(true));
        void invoke("cancel_generation");
        activeGenerationRef.current = null;
        lastIntentRef.current = null;
        pendingOcrIntentRef.current = null;
        ocrImageIdRef.current = event.payload.autoRead ? event.payload.imageId : null;
        setSelection(null);
        setImage(event.payload.autoRead ? null : event.payload);
        setOcrState(event.payload.autoRead ? "reading" : "idle");
        setOcrSource(event.payload.autoRead ? event.payload.source : null);
        setOcrError(null);
        setQueuedOcrIntent(null);
        setErrorMessage(null);
        setCustomInstruction("");
        setGenerationState("idle");
        responseTextRef.current = "";
        setResponseText("");
        setGenerationError(null);
        setCopyState("idle");
        setIsSelectionExpanded(false);
        setIsAttachedTextExpanded(false);
        setPromptPreview(null);
        setPromptPreviewError(null);
        setIsPromptPreviewLoading(false);
        setIsPromptPreviewExpanded(false);
        setCaptureState("captured");
        if (event.payload.autoRead) {
          void startScreenReadRef.current?.(event.payload.imageId, event.payload.source);
        }
      }),
      listen<CaptureFailure>("selection-capture-failed", (event) => {
        setIsPopupSurfaceVisible(false);
        window.requestAnimationFrame(() => setIsPopupSurfaceVisible(true));
        setSelection(null);
        setIsAttachedTextExpanded(false);
        pendingOcrIntentRef.current = null;
        ocrImageIdRef.current = null;
        setOcrState("error");
        setOcrError(event.payload.message);
        setQueuedOcrIntent(null);
        setErrorMessage(event.payload.message);
        setCaptureState("error");
      }),
      listen<GenerationStarted>("generation-started", (event) => {
        activeGenerationRef.current = event.payload.requestId;
        setGenerationState("loading");
        responseTextRef.current = "";
        setResponseText("");
        setGenerationError(null);
        setCopyState("idle");
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
      listen<ScreenReadFinished>("screen-read-finished", (event) => {
        if (ocrImageIdRef.current !== event.payload.imageId) {
          return;
        }
        ocrImageIdRef.current = null;
        const screenText: CapturedSelection = {
          text: event.payload.text,
          source: "ui-automation",
          truncated: false,
          clipboardRestored: true,
        };
        const pendingIntent = pendingOcrIntentRef.current;
        pendingOcrIntentRef.current = null;
        setSelection(screenText);
        setIsAttachedTextExpanded(false);
        setOcrState("ready");
        setOcrError(null);
        setQueuedOcrIntent(null);
        void invoke("discard_image_capture");
        if (pendingIntent) {
          window.queueMicrotask(() => {
            void startGenerationRef.current?.(pendingIntent, screenText, true);
          });
        }
      }),
      listen<GenerationFailure>("screen-read-failed", (event) => {
        if (ocrImageIdRef.current === null) {
          return;
        }
        ocrImageIdRef.current = null;
        pendingOcrIntentRef.current = null;
        setOcrState("error");
        setOcrError(event.payload.message);
        setQueuedOcrIntent(null);
      }),
    ]);

    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setIsPopupSurfaceVisible(false);
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
    const shouldReposition = shouldPositionPopupRef.current;
    shouldPositionPopupRef.current = false;

    const frame = window.requestAnimationFrame(() => {
      const contentHeight = popupContentRef.current?.getBoundingClientRect().height ?? POPUP_MIN_HEIGHT;
      const desiredHeight = Math.max(
        Math.ceil(contentHeight) + POPUP_SHADOW_MARGIN * 2,
        POPUP_MIN_HEIGHT,
      );

      void (async () => {
        const popupWindow = getCurrentWindow();
        const cursor = await cursorPosition();
        const monitor = await monitorFromPoint(cursor.x, cursor.y);
        const maxHeight = monitor
          ? Math.min(POPUP_MAX_HEIGHT, Math.max(POPUP_MIN_HEIGHT, monitor.workArea.size.height - 16))
          : POPUP_MAX_HEIGHT;
        const nextHeight = Math.min(desiredHeight, maxHeight);
        await popupWindow.setSize(new LogicalSize(POPUP_WIDTH, nextHeight));

        if (!shouldReposition) {
          return;
        }

        const popupSize = await popupWindow.outerSize();

        if (cancelled || !monitor) {
          return;
        }

        const workArea = monitor.workArea;
        const minX = workArea.position.x;
        const minY = workArea.position.y;
        const maxX = Math.max(minX, workArea.position.x + workArea.size.width - popupSize.width);
        const maxY = Math.max(minY, workArea.position.y + workArea.size.height - popupSize.height);
        const x = Math.min(Math.max(cursor.x + POPUP_WINDOW_OFFSET_X, minX), maxX);
        const y = cursor.y + POPUP_WINDOW_OFFSET_Y <= maxY
          ? cursor.y + POPUP_WINDOW_OFFSET_Y
          : Math.max(minY, cursor.y - POPUP_WINDOW_OFFSET_Y - popupSize.height);

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
    ocrError,
    ocrState,
    queuedOcrIntent,
    isAttachedTextExpanded,
    isPromptPreviewExpanded,
    isPromptPreviewLoading,
    isSelectionExpanded,
    isSettingsOpen,
    isSettingsLoading,
    settingsPage,
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

  const startGeneration = async (
    intentOverride?: PromptIntent,
    selectionOverride?: CapturedSelection,
    bypassOcrWait = false,
  ) => {
    if (ocrState === "reading" && !bypassOcrWait) {
      const intent = resolveIntent(intentOverride);
      pendingOcrIntentRef.current = intent;
      setQueuedOcrIntent(intent);
      return;
    }

    const intent = resolveIntent(intentOverride);
    const activeSelection = selectionOverride ?? selection;
    if (!activeSelection && !image && intent.kind !== "custom") {
      return;
    }

    lastIntentRef.current = intent;
    activeGenerationRef.current = null;
    setGenerationState("loading");
    responseTextRef.current = "";
    setResponseText("");
    setGenerationError(null);
    setCopyState("idle");

    try {
      const started = await invoke<GenerationStarted>("generate_for_selection", {
        request: {
          selectedText: activeSelection?.text ?? null,
          imageId: image?.imageId ?? null,
          intent,
        },
      });
      activeGenerationRef.current = started.requestId;
    } catch (error) {
      setGenerationState("error");
      setGenerationError(errorText(error));
    }
  };

  startGenerationRef.current = startGeneration;

  const startScreenRead = async (imageId: number, source: ImageCaptureEvent["source"]) => {
    ocrImageIdRef.current = imageId;
    setImage(null);
    setOcrState("reading");
    setOcrSource(source);
    setOcrError(null);

    try {
      await invoke<GenerationStarted>("read_screen", {
        request: { imageId },
      });
    } catch (error) {
      if (ocrImageIdRef.current !== imageId) {
        return;
      }
      ocrImageIdRef.current = null;
      pendingOcrIntentRef.current = null;
      setOcrState("error");
      setOcrError(errorText(error));
      setQueuedOcrIntent(null);
    }
  };

  startScreenReadRef.current = startScreenRead;

  const previewPrompt = async () => {
    if ((!selection && !image) || !showDevelopmentPreviews) {
      return;
    }

    setIsPromptPreviewLoading(true);
    setPromptPreview(null);
    setPromptPreviewError(null);
    setIsPromptPreviewExpanded(true);

    try {
      const messages = await invoke<ChatMessage[]>("preview_prompt", {
        request: {
          selectedText: selection?.text ?? null,
          hasImage: Boolean(image),
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

  const updateShortcut = (key: ShortcutKey, value: string) => {
    setSettingsDraft((draft) => draft && ({
      ...draft,
      shortcuts: {
        ...draft.shortcuts,
        [key]: value,
      },
    }));
  };

  const recordShortcut = (key: ShortcutKey, event: ReactKeyboardEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Escape") {
      setRecordingShortcut(null);
      return;
    }
    if (!event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey
      && (event.key === "Backspace" || event.key === "Delete")) {
      updateShortcut(key, "");
      setRecordingShortcut(null);
      return;
    }

    const shortcut = shortcutFromKeypress(event);
    if (shortcut) {
      updateShortcut(key, shortcut);
      setRecordingShortcut(null);
    }
  };

  const closePopup = () => {
    setIsPopupSurfaceVisible(false);
    void invoke("cancel_generation");
    void invoke("discard_image_capture");
    void getCurrentWindow().hide();
  };

  const copyResponse = async () => {
    if (!responseText) {
      return;
    }

    try {
      await invoke("copy_response", { text: responseText });
      setCopyState("copied");
      if (copyResetTimeoutRef.current !== null) {
        window.clearTimeout(copyResetTimeoutRef.current);
      }
      copyResetTimeoutRef.current = window.setTimeout(() => {
        setCopyState("idle");
        copyResetTimeoutRef.current = null;
      }, 2500);
    } catch {
      setCopyState("error");
    }
  };

  const configuredModelIsListed = settingsDraft?.model
    && availableModels.some((model) => model.name === settingsDraft.model);
  const configuredOcrModelIsListed = settingsDraft
    && availableModels.some((model) => model.name === settingsDraft.ocrModel);

  return (
    <main className="min-h-screen overflow-hidden p-6 text-zinc-100">
      <section className="mx-auto w-full max-w-xl">
        <div
          ref={popupContentRef}
          className={`popup-surface ${isPopupSurfaceVisible ? "popup-surface--enter" : "popup-surface--hidden"} rounded-[28px] bg-zinc-900 p-4 shadow-lg shadow-black/50`}
        >
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
            {isSettingsOpen && (
              <span data-tauri-drag-region className="min-w-0 flex-1 px-1 py-2 text-[17px] text-zinc-100">
                Settings
              </span>
            )}
            {!isSettingsOpen && !hasSubmittedPrompt && (
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
                  placeholder={selection || image ? "Ask about this…" : "Ask me anything"}
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
            {!isSettingsOpen && hasSubmittedPrompt && isGenerating && (
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
              aria-label={isSettingsOpen ? "Close settings" : "Open local AI settings"}
              title={isSettingsOpen ? "Back" : "Settings"}
              className="absolute right-11 top-4 grid size-8 cursor-pointer place-items-center rounded-md text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
              onMouseDown={(event) => event.stopPropagation()}
              onClick={() => {
                if (isSettingsOpen) {
                  setIsSettingsOpen(false);
                } else {
                  setIsSettingsOpen(true);
                  setSettingsPage("general");
                  void refreshProviderStatus();
                }
              }}
            >
              {isSettingsOpen ? (
                <svg aria-hidden="true" viewBox="0 0 24 24" className="size-5 fill-none stroke-current stroke-[1.8]">
                  <path strokeLinecap="round" strokeLinejoin="round" d="m14.5 5-7 7 7 7" />
                </svg>
              ) : (
                <svg aria-hidden="true" viewBox="0 0 24 24" className="size-4 fill-none stroke-current stroke-[1.8]">
                  <path strokeLinecap="round" strokeLinejoin="round" d="M10.3 3.6a1.9 1.9 0 0 1 3.4 0l.4 1a1.9 1.9 0 0 0 2 1.1l1.1-.2a1.9 1.9 0 0 1 2.4 2.4l-.2 1.1a1.9 1.9 0 0 0 1.1 2l1 .4a1.9 1.9 0 0 1 0 3.4l-1 .4a1.9 1.9 0 0 0-1.1 2l.2 1.1a1.9 1.9 0 0 1-2.4 2.4l-1.1-.2a1.9 1.9 0 0 0-2 1.1l-.4 1a1.9 1.9 0 0 1-3.4 0l-.4-1a1.9 1.9 0 0 0-2-1.1l-1.1.2a1.9 1.9 0 0 1-2.4-2.4l.2-1.1a1.9 1.9 0 0 0-1.1-2l-1-.4a1.9 1.9 0 0 1 0-3.4l1-.4a1.9 1.9 0 0 0 1.1-2l-.2-1.1a1.9 1.9 0 0 1 2.4-2.4l1.1.2a1.9 1.9 0 0 0 2-1.1l.4-1Z" />
                  <circle cx="12" cy="12" r="3" />
                </svg>
              )}
            </button>
            <button
              type="button"
              aria-label="Close Snippet"
              className="absolute right-2 top-4 grid size-8 cursor-pointer place-items-center rounded-md text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
              onMouseDown={(event) => event.stopPropagation()}
              onClick={closePopup}
            >
              <svg aria-hidden="true" viewBox="0 0 24 24" className="size-5 fill-none stroke-current stroke-[1.8]">
                <path strokeLinecap="round" d="m7 7 10 10M17 7 7 17" />
              </svg>
            </button>
          </header>

          {!isSettingsOpen && !hasSubmittedPrompt && (
            <div>
              {ocrState !== "idle" && (
                <>
                  <div className="flex items-center gap-2 rounded-lg border border-zinc-800 bg-zinc-950/50 p-2">
                    {ocrState === "reading" && (
                      <svg aria-hidden="true" viewBox="0 0 24 24" className="size-3.5 shrink-0 animate-spin fill-none">
                        <circle cx="12" cy="12" r="8" className="stroke-zinc-800" strokeWidth="2.5" />
                        <path d="M20 12a8 8 0 0 0-8-8" className="stroke-zinc-400" strokeLinecap="round" strokeWidth="2.5" />
                      </svg>
                    )}
                    <span className="min-w-0 flex-1 text-[10px] text-zinc-400">
                      {ocrState === "reading" && `Reading ${ocrSource === "region" ? "region" : "screen"}…${queuedOcrIntent ? " Your request will send when it is ready." : ""}`}
                      {ocrState === "ready" && `${ocrSource === "region" ? "Region" : "Screen"} text attached`}
                      {ocrState === "error" && ocrError}
                    </span>
                    {ocrState === "ready" && (
                      <div className="flex items-center gap-1">
                        {selection && (
                          <button
                            type="button"
                            aria-expanded={isAttachedTextExpanded}
                            aria-label={isAttachedTextExpanded ? "Hide attached text" : "Show attached text"}
                            title={isAttachedTextExpanded ? "Hide attached text" : "Show attached text"}
                            className="grid size-6 cursor-pointer place-items-center rounded text-zinc-500 transition-colors hover:bg-zinc-800 hover:text-zinc-200"
                            onClick={() => setIsAttachedTextExpanded((expanded) => !expanded)}
                          >
                            <svg aria-hidden="true" viewBox="0 0 24 24" className="size-3.5 fill-none stroke-current stroke-[2]">
                              <path strokeLinecap="round" strokeLinejoin="round" d={isAttachedTextExpanded ? "m7 15 5-5 5 5" : "m7 9 5 5 5-5"} />
                            </svg>
                          </button>
                        )}
                        <button
                          type="button"
                          aria-label="Remove extracted screen text"
                          className="grid size-6 cursor-pointer place-items-center rounded text-zinc-500 transition-colors hover:bg-zinc-800 hover:text-zinc-200"
                          onClick={() => {
                            setSelection(null);
                            setOcrState("idle");
                            setOcrSource(null);
                            setOcrError(null);
                            setIsAttachedTextExpanded(false);
                          }}
                        >
                          ×
                        </button>
                      </div>
                    )}
                  </div>
                  {ocrState === "ready" && isAttachedTextExpanded && selection && (
                    <blockquote className="preview-scroll mt-2 max-h-44 overflow-y-auto whitespace-pre-wrap rounded-lg border border-zinc-800/80 bg-zinc-950/50 p-3 text-xs leading-5 text-zinc-400">
                      {selection.text}
                    </blockquote>
                  )}
                  <div className="mb-3" />
                </>
              )}
              {image && (
                <div className="mb-3 flex items-center gap-2 rounded-lg border border-zinc-800 bg-zinc-950/50 p-2">
                  <img
                    alt={image.source === "region" ? "Captured region" : "Captured screen"}
                    className="size-10 rounded object-cover"
                    src={image.preview.dataUrl}
                  />
                  <span className="min-w-0 flex-1 text-[10px] text-zinc-400">
                    {image.source === "region" ? "Region capture" : "Screen capture"} · {image.preview.width} × {image.preview.height}
                  </span>
                  <button
                    type="button"
                    aria-label="Remove captured image"
                    className="grid size-6 cursor-pointer place-items-center rounded text-zinc-500 transition-colors hover:bg-zinc-800 hover:text-zinc-200"
                    onClick={() => {
                      setImage(null);
                      void invoke("discard_image_capture");
                    }}
                  >
                    ×
                  </button>
                </div>
              )}
              <div className="flex flex-wrap gap-2">
                {image && (
                  <button
                    type="button"
                    className="cursor-pointer rounded-full bg-violet-500 px-3.5 py-1.5 text-[10px] font-extrabold text-white transition-colors hover:bg-violet-400"
                    onClick={() => void startScreenRead(image.imageId, image.source)}
                  >
                    {image.source === "region" ? "Read region" : "Read screen"}
                  </button>
                )}
                {(selection || image || ocrState === "reading") && QUICK_ACTIONS.map((action) => (
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
            </div>
          )}

          {isSettingsOpen && (
            <section className="settings-scroll min-h-64 px-1 pr-2 pt-1">
              <div className="mb-3 flex items-center justify-between gap-3">
                <p className="text-sm font-semibold text-zinc-200">Local AI</p>
                <button
                  type="button"
                  className="text-xs font-medium text-zinc-400 transition-colors hover:text-zinc-100"
                  onClick={() => void refreshProviderStatus()}
                >
                  Refresh
                </button>
              </div>

              <div className="mb-4 flex gap-1 rounded-lg bg-zinc-950/50 p-1">
                {([
                  ["general", "General"],
                  ["ocr", "Read screen"],
                  ["shortcuts", "Shortcuts"],
                ] as const).map(([page, label]) => (
                  <button
                    key={page}
                    type="button"
                    className={`flex-1 rounded-md px-2 py-1.5 text-xs font-medium transition-colors ${settingsPage === page ? "bg-zinc-700 text-zinc-100" : "text-zinc-500 hover:text-zinc-300"}`}
                    onClick={() => setSettingsPage(page)}
                  >
                    {label}
                  </button>
                ))}
              </div>

              {settingsPage === "general" && (
                <>
              <label className="mb-4 block text-xs font-medium text-zinc-400">
                Ollama address
                <input
                  className="mt-1 w-full rounded-lg border border-zinc-800 bg-zinc-900 px-2.5 py-2 text-sm text-zinc-200 outline-none focus:border-zinc-600"
                  value={settingsDraft?.ollamaBaseUrl ?? ""}
                  onChange={(event) => setSettingsDraft((draft) => draft && ({
                    ...draft,
                    ollamaBaseUrl: event.target.value,
                  }))}
                />
              </label>

              <label className="block text-xs font-medium text-zinc-400">
                Model
                <select
                  className="mt-1 w-full rounded-lg border border-zinc-800 bg-zinc-900 px-2.5 py-2 text-sm text-zinc-200 outline-none focus:border-zinc-600"
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
                </>
              )}

              {settingsPage === "ocr" && (
                <>
              <label className="mt-3 block text-xs font-medium text-zinc-400">
                Read screen model
                <select
                  className="mt-1 w-full rounded-lg border border-zinc-800 bg-zinc-900 px-2.5 py-2 text-sm text-zinc-200 outline-none focus:border-zinc-600"
                  value={settingsDraft?.ocrModel ?? ""}
                  onChange={(event) => setSettingsDraft((draft) => draft && ({
                    ...draft,
                    ocrModel: event.target.value,
                  }))}
                >
                  {settingsDraft && !configuredOcrModelIsListed && (
                    <option value={settingsDraft.ocrModel}>{settingsDraft.ocrModel} (not installed)</option>
                  )}
                  {availableModels.map((model) => (
                    <option key={model.name} value={model.name}>{model.name}</option>
                  ))}
                </select>
              </label>

              <label className="mt-5 block text-xs font-medium text-zinc-400">
                Output limit <span className="float-right text-zinc-200">{settingsDraft?.ocrNumPredict ?? 0} tokens</span>
                <input
                  aria-label="OCR output limit"
                  className="mt-2 w-full accent-violet-400"
                  type="range"
                  min="256"
                  max="8192"
                  step="256"
                  value={settingsDraft?.ocrNumPredict ?? 2048}
                  onChange={(event) => setSettingsDraft((draft) => draft && ({
                    ...draft,
                    ocrNumPredict: Number(event.target.value),
                  }))}
                />
              </label>

              <label className="mt-5 block text-xs font-medium text-zinc-400">
                Context window <span className="float-right text-zinc-200">{settingsDraft?.ocrNumCtx ?? 0} tokens</span>
                <input
                  aria-label="OCR context window"
                  className="mt-2 w-full accent-violet-400"
                  type="range"
                  min="4096"
                  max="32768"
                  step="4096"
                  value={settingsDraft?.ocrNumCtx ?? 16384}
                  onChange={(event) => setSettingsDraft((draft) => draft && ({
                    ...draft,
                    ocrNumCtx: Number(event.target.value),
                  }))}
                />
              </label>
                </>
              )}

              {settingsPage === "general" && (
                <>
              <label className="mt-3 flex cursor-pointer items-center justify-between gap-3 rounded-lg border border-zinc-800 bg-zinc-900 px-2.5 py-2">
                <span>
                  <span className="block text-sm font-medium text-zinc-200">Enable thinking</span>
                </span>
                <input
                  aria-label="Enable thinking"
                  className="size-3.5 cursor-pointer accent-zinc-100"
                  type="checkbox"
                  checked={settingsDraft?.thinking ?? false}
                  onChange={(event) => setSettingsDraft((draft) => draft && ({
                    ...draft,
                    thinking: event.target.checked,
                  }))}
                />
              </label>

              <label className="mt-3 flex cursor-pointer items-center justify-between gap-3 rounded-lg border border-zinc-800 bg-zinc-900 px-2.5 py-2">
                <span>
                  <span className="block text-sm font-medium text-zinc-200">Enable direct image analysis</span>
                </span>
                <input
                  aria-label="Enable direct image analysis"
                  className="size-3.5 cursor-pointer accent-zinc-100"
                  type="checkbox"
                  checked={settingsDraft?.visionEnabled ?? false}
                  onChange={(event) => setSettingsDraft((draft) => draft && ({
                    ...draft,
                    visionEnabled: event.target.checked,
                  }))}
                />
              </label>
                </>
              )}

              {settingsPage === "shortcuts" && (
              <div className="mt-4">
                <p className="text-sm font-medium text-zinc-200">Shortcuts</p>
                <p className="mt-1 text-xs text-zinc-400">
                  Click a shortcut, then press keys.
                </p>
                <div className="mt-2 grid grid-cols-2 gap-2">
                  {([
                    ["openPopup", "Open popup"],
                    ["summarize", "Summarize"],
                    ["explain", "Explain"],
                    ["refine", "Refine"],
                    ["captureScreen", "Capture screen"],
                    ["captureRegion", "Capture region"],
                    ["readScreen", "Read screen"],
                    ["readRegion", "Read region"],
                  ] as const).map(([key, label]) => (
                    <label key={key} className="text-xs font-medium text-zinc-400">
                      {label}
                      <div className="mt-1 flex gap-1">
                        <button
                          type="button"
                          aria-label={`Record shortcut for ${label}`}
                          className={`min-w-0 flex-1 rounded-lg border px-2.5 py-2 text-left text-sm outline-none transition-colors ${recordingShortcut === key ? "border-violet-400 bg-violet-500/10 text-violet-100" : "border-zinc-800 bg-zinc-900 text-zinc-200 hover:border-zinc-600"}`}
                          onClick={() => setRecordingShortcut((current) => current === key ? null : key)}
                          onBlur={() => setRecordingShortcut((current) => current === key ? null : current)}
                          onKeyDown={(event) => {
                            if (recordingShortcut === key) {
                              recordShortcut(key, event);
                            }
                          }}
                        >
                          {recordingShortcut === key ? "Press shortcut…" : settingsDraft?.shortcuts[key] || "Disabled"}
                        </button>
                        <button
                          type="button"
                          aria-label={`Disable ${label} shortcut`}
                          title="Disable shortcut"
                          className="grid size-8 shrink-0 place-items-center rounded-lg border border-zinc-800 bg-zinc-900 text-zinc-500 transition-colors hover:border-zinc-600 hover:text-zinc-200"
                          onClick={() => {
                            updateShortcut(key, "");
                            setRecordingShortcut(null);
                          }}
                        >
                          ×
                        </button>
                      </div>
                    </label>
                  ))}
                </div>
              </div>
              )}

              {settings?.modelSource === "environment" && (
                <p className="mt-3 text-xs text-zinc-400">
                  `SNIPPET_OLLAMA_MODEL` is overriding the saved model.
                </p>
              )}
              {providerStatus && (
                <p className="mt-3 text-xs text-zinc-400">
                  {providerStatus.state === "ready" && "Ollama is ready."}
                  {providerStatus.state === "unavailable" && providerStatus.message}
                  {providerStatus.state === "modelNotConfigured" && "Choose one of the installed models."}
                  {providerStatus.state === "modelMissing" && `${providerStatus.model} is not installed.`}
                </p>
              )}
              {settingsError && <p className="mt-3 text-xs leading-5 text-amber-200">{settingsError}</p>}

              <div className="mt-3 flex items-center justify-between gap-3">
                <span className="text-xs text-zinc-500">
                  {isSettingsLoading ? "Checking Ollama…" : ""}
                </span>
                <button
                  type="button"
                  className="rounded-full bg-zinc-100 px-3.5 py-2 text-xs font-bold text-zinc-950 transition-colors hover:bg-white disabled:cursor-wait disabled:opacity-60"
                  disabled={!settingsDraft || isSettingsLoading}
                  onClick={() => void saveSettings()}
                >
                  Save settings
                </button>
              </div>
            </section>
          )}

          {!isSettingsOpen && hasSubmittedPrompt && (
            <section className="mb-2">
              {responseText && (
                <div className="response-scroll max-h-80 overflow-y-auto text-sm leading-5 text-zinc-200">
                  <MarkdownResponse content={responseText} />
                  {generationState === "complete" && (
                    <button
                      type="button"
                      aria-label={copyState === "copied" ? "Copied response" : "Copy response"}
                      title={copyState === "copied" ? "Copied" : "Copy response"}
                      className="mt-1 inline-flex cursor-pointer items-center rounded-md p-1 text-zinc-500 transition-colors hover:bg-zinc-800 hover:text-zinc-200"
                      onClick={() => void copyResponse()}
                    >
                      {copyState === "copied" ? (
                        <svg aria-hidden="true" viewBox="0 0 24 24" className="size-4 fill-none stroke-current stroke-[2]">
                          <path strokeLinecap="round" strokeLinejoin="round" d="m5 12 4.5 4.5L19 7" />
                        </svg>
                      ) : (
                        <svg aria-hidden="true" viewBox="0 0 24 24" className="size-4 fill-none stroke-current stroke-[1.8]">
                          <rect x="8" y="8" width="11" height="11" rx="2" />
                          <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" />
                        </svg>
                      )}
                    </button>
                  )}
                </div>
              )}
              {generationError && <p className="text-xs leading-5 text-amber-200">{generationError}</p>}
            </section>
          )}

          {!isSettingsOpen && captureState === "captured" && selection && showDevelopmentPreviews && (
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

          {!isSettingsOpen && captureState === "error" && (
            <p className="mt-3 text-xs leading-5 text-amber-200">{errorMessage}</p>
          )}
        </div>
      </section>
    </main>
  );
}

export default App;
