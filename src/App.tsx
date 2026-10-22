import { useEffect, useLayoutEffect, useReducer, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { disable as disableAutostart, enable as enableAutostart, isEnabled as isAutostartEnabled } from "@tauri-apps/plugin-autostart";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  cursorPosition,
  currentMonitor,
  getCurrentWindow,
  LogicalSize,
  monitorFromPoint,
  PhysicalPosition,
} from "@tauri-apps/api/window";
import "./App.css";
import { SHOW_DEVELOPMENT_TOOLS } from "./config";
import { UpdatesPanel } from "./components/UpdatesPanel";
import { updateController, useUpdates } from "./useUpdates";
import { conversationReducer, isTurnActive, type GenerationEvent, type TurnIdentity } from "./conversation";
import { ConversationPanel } from "./components/ConversationPanel";

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
  hasCompletedOnboarding: boolean;
  automaticUpdates: boolean;
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

type GenerationStarted = GenerationEvent;

type GenerationChunk = GenerationEvent & {
  delta: string;
};

type GenerationFinished = GenerationEvent;

type GenerationFailure = GenerationEvent & {
  message: string;
};

type ScreenReadFinished = {
  requestId: number;
  imageId: number;
  text: string;
};

type SettingsPage = "general" | "ocr" | "shortcuts" | "updates";
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
  const conversationIdRef = useRef(Date.now());
  const nextTurnIdRef = useRef(0);
  const activeTurnRef = useRef<TurnIdentity | null>(null);
  const activeRequestPromiseRef = useRef<Promise<unknown>>(Promise.resolve());
  const [conversation, dispatchConversation] = useReducer(conversationReducer, {
    conversationId: conversationIdRef.current, expanded: false, turns: [],
  });
  const [captureState, setCaptureState] = useState<CaptureState>("waiting");
  const [selection, setSelection] = useState<CapturedSelection | null>(null);
  const [image, setImage] = useState<ImageCaptureEvent | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [customInstruction, setCustomInstruction] = useState("");
  const [ocrState, setOcrState] = useState<OcrState>("idle");
  const [ocrSource, setOcrSource] = useState<ImageCaptureEvent["source"] | null>(null);
  const [ocrError, setOcrError] = useState<string | null>(null);
  const [queuedOcrIntent, setQueuedOcrIntent] = useState<PromptIntent | null>(null);
  const [settings, setSettings] = useState<SettingsSnapshot | null>(null);
  const [settingsDraft, setSettingsDraft] = useState<AppConfig | null>(null);
  const [providerStatus, setProviderStatus] = useState<ProviderStatus | null>(null);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isOnboardingOpen, setIsOnboardingOpen] = useState(false);
  const [onboardingStartAtLogin, setOnboardingStartAtLogin] = useState(true);
  const [settingsPage, setSettingsPage] = useState<SettingsPage>("general");
  const [recordingShortcut, setRecordingShortcut] = useState<ShortcutKey | null>(null);
  const [isSettingsLoading, setIsSettingsLoading] = useState(true);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [isAutostartLoading, setIsAutostartLoading] = useState(true);
  const [autostartError, setAutostartError] = useState<string | null>(null);
  const [isSelectionExpanded, setIsSelectionExpanded] = useState(false);
  const [isAttachedTextExpanded, setIsAttachedTextExpanded] = useState(false);
  const [promptPreview, setPromptPreview] = useState<ChatMessage[] | null>(null);
  const [promptPreviewError, setPromptPreviewError] = useState<string | null>(null);
  const [isPromptPreviewLoading, setIsPromptPreviewLoading] = useState(false);
  const [isPromptPreviewExpanded, setIsPromptPreviewExpanded] = useState(false);
  const [isPopupSurfaceVisible, setIsPopupSurfaceVisible] = useState(false);
  const [popupOpenSequence, setPopupOpenSequence] = useState(0);
  const popupContentRef = useRef<HTMLDivElement>(null);
  const popupPositionRequestRef = useRef<{ cursor: Promise<PhysicalPosition | null> } | null>(null);
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
  const initialInputRef = useRef<HTMLInputElement>(null);

  const showDevelopmentPreviews = SHOW_DEVELOPMENT_TOOLS;
  const currentTurn = conversation.turns[conversation.turns.length - 1];
  const generationState = currentTurn?.status ?? "idle";
  const isGenerating = isTurnActive(currentTurn);
  const updates = useUpdates(settings?.config.automaticUpdates ?? false);
  const hasSubmittedPrompt = conversation.turns.length > 0;
  const availableModels = providerStatus && providerStatus.state !== "unavailable"
    ? providerStatus.models
    : [];

  const resetConversation = () => {
    const conversationId = ++conversationIdRef.current;
    activeTurnRef.current = null;
    dispatchConversation({ type: "reset", conversationId });
    void invoke("reset_conversation", { conversationId }).catch((error) => {
      if (conversationIdRef.current === conversationId) {
        setErrorMessage(errorText(error));
        setCaptureState("error");
      }
    });
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
    setIsSelectionExpanded(false);
    setIsAttachedTextExpanded(false);
    setPromptPreview(null);
    setPromptPreviewError(null);
    setIsPromptPreviewLoading(false);
    setIsPromptPreviewExpanded(false);
    setCaptureState("waiting");
  };

  const finishTurn = (event: GenerationEvent, status: "complete" | "stopped" | "error", message?: string) => {
    if (!event.context) return;
    dispatchConversation(status === "error"
      ? { type: "error", identity: event.context, message: message ?? "Generation failed." }
      : { type: status, event });
    if (activeTurnRef.current?.conversationId === event.context.conversationId
      && activeTurnRef.current.turnId === event.context.turnId) {
      activeTurnRef.current = null;
    }
  };

  const submitTurn = async (
    kind: "initial" | "follow-up", prompt: string, request: Record<string, unknown>,
    retry = false, reference?: string,
  ) => {
    if (activeTurnRef.current || updateController.getSnapshot().phase === "installing" || !prompt.trim()) return;
    const identity = { conversationId: conversationIdRef.current, turnId: ++nextTurnIdRef.current };
    activeTurnRef.current = identity;
    dispatchConversation({ type: "submit", identity, prompt: prompt.trim(), reference, kind, retry });
    try {
      const pending = invoke<GenerationStarted>(kind === "initial" ? "generate_for_selection" : "continue_conversation", {
        request: { ...request, identity },
      });
      activeRequestPromiseRef.current = pending.catch(() => undefined);
      const started = await pending;
      // The event can arrive before the command resolves. The reducer accepts
      // this acknowledgement only while the same turn is still active.
      dispatchConversation({ type: "started", event: started });
    } catch (error) {
      finishTurn({ requestId: 0, context: identity }, "error", errorText(error));
    }
  };

  const requestPopupPosition = () => {
    // Capture the opening location before the pointer moves onto a popup control.
    popupPositionRequestRef.current = { cursor: cursorPosition().catch(() => null) };
    setPopupOpenSequence((sequence) => sequence + 1);
  };

  const openChatPopup = () => {
    requestPopupPosition();
    setIsSettingsOpen(false);
    setSettingsPage("general");
    setIsPopupSurfaceVisible(false);
    window.requestAnimationFrame(() => setIsPopupSurfaceVisible(true));
  };

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
      if (!snapshot.config.hasCompletedOnboarding) {
        setIsOnboardingOpen(true);
        setIsSettingsOpen(true);
        setSettingsPage("general");
      }
      setProviderStatus(status);
      setSettingsError(null);
    } catch (error) {
      setSettingsError(errorText(error));
    } finally {
      setIsSettingsLoading(false);
    }
  };

  const loadAutostart = async () => {
    setIsAutostartLoading(true);
    try {
      const enabled = await isAutostartEnabled();
      setAutostartEnabled(enabled);
      setOnboardingStartAtLogin(enabled || !settings?.config.hasCompletedOnboarding);
      setAutostartError(null);
    } catch (error) {
      setAutostartError(errorText(error));
    } finally {
      setIsAutostartLoading(false);
    }
  };

  useEffect(() => {
    void loadSettings();
    void loadAutostart();
  }, []);

  useEffect(() => {
    const unlisten = Promise.all([
      listen("popup-opened", () => {
        resetConversation();
        openChatPopup();
      }),
      listen("open-settings", () => {
        setIsSettingsOpen(true);
        setSettingsPage("general");
        void loadSettings();
        void loadAutostart();
      }),
      listen("open-updates", () => {
        // Opening update settings must preserve a response already in progress.
        requestPopupPosition();
        setIsPopupSurfaceVisible(true);
        setIsSettingsOpen(true);
        setSettingsPage("updates");
        void updateController.check();
      }),
      listen<SelectionCaptureEvent>("selection-captured", (event) => {
        const capturedSelection = event.payload.selection;
        resetConversation();
        openChatPopup();
        setSelection(capturedSelection);
        setCaptureState("captured");
        if (event.payload.action) {
          void startGenerationRef.current?.(
            { kind: "quick-action", action: event.payload.action },
            capturedSelection,
            true,
          );
        }
      }),
      listen<ImageCaptureEvent>("image-captured", (event) => {
        resetConversation();
        openChatPopup();
        ocrImageIdRef.current = event.payload.autoRead ? event.payload.imageId : null;
        setImage(event.payload.autoRead ? null : event.payload);
        setOcrState(event.payload.autoRead ? "reading" : "idle");
        setOcrSource(event.payload.autoRead ? event.payload.source : null);
        setCaptureState("captured");
        if (event.payload.autoRead) {
          void startScreenReadRef.current?.(event.payload.imageId, event.payload.source);
        }
      }),
      listen<CaptureFailure>("selection-capture-failed", (event) => {
        resetConversation();
        openChatPopup();
        setErrorMessage(event.payload.message);
        setCaptureState("error");
      }),
      listen<GenerationStarted>("generation-started", (event) => {
        dispatchConversation({ type: "started", event: event.payload });
      }),
      listen<GenerationChunk>("generation-chunk", (event) => {
        dispatchConversation({ type: "chunk", event: event.payload, delta: event.payload.delta });
      }),
      listen<GenerationFinished>("generation-finished", (event) => {
        finishTurn(event.payload, "complete");
      }),
      listen<GenerationFinished>("generation-cancelled", (event) => {
        finishTurn(event.payload, "stopped");
      }),
      listen<GenerationFailure>("generation-failed", (event) => {
        if (event.payload.context) finishTurn(event.payload, "error", event.payload.message);
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
          const conversationId = conversationIdRef.current;
          window.queueMicrotask(() => {
            if (conversationIdRef.current === conversationId) {
              void startGenerationRef.current?.(pendingIntent, screenText, true);
            }
          });
        }
      }),
      listen<GenerationFailure & { imageId: number }>("screen-read-failed", (event) => {
        if (ocrImageIdRef.current !== event.payload.imageId) {
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
        closePopup();
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
    // Keep an opening request pending if a newer layout cancels this pass.
    const positionRequest = popupPositionRequestRef.current;

    const frame = window.requestAnimationFrame(() => {
      void (async () => {
        const popupWindow = getCurrentWindow();
        const cursor = positionRequest ? await positionRequest.cursor : null;
        if (cancelled) {
          return;
        }
        const monitor = cursor
          ? await monitorFromPoint(cursor.x, cursor.y)
          : await currentMonitor();
        if (cancelled) {
          return;
        }
        const maxHeight = monitor
          ? Math.min(POPUP_MAX_HEIGHT, Math.max(POPUP_MIN_HEIGHT, monitor.workArea.size.height / monitor.scaleFactor - 16))
          : POPUP_MAX_HEIGHT;
        popupContentRef.current?.style.setProperty("--popup-max-height", `${maxHeight}px`);
        const contentHeight = popupContentRef.current?.getBoundingClientRect().height ?? POPUP_MIN_HEIGHT;
        const desiredHeight = Math.max(Math.ceil(contentHeight) + POPUP_SHADOW_MARGIN * 2, POPUP_MIN_HEIGHT);
        const nextHeight = Math.min(desiredHeight, maxHeight);
        await popupWindow.setSize(new LogicalSize(POPUP_WIDTH, nextHeight));

        if (cancelled || (positionRequest && popupPositionRequestRef.current !== positionRequest)) {
          return;
        }

        const popupSize = await popupWindow.outerSize();

        const position = !positionRequest ? await popupWindow.outerPosition() : null;
        if (cancelled || (positionRequest && popupPositionRequestRef.current !== positionRequest)) {
          return;
        }
        if (positionRequest) popupPositionRequestRef.current = null;
        if (!monitor) {
          return;
        }

        const workArea = monitor.workArea;
        const minX = workArea.position.x;
        const minY = workArea.position.y;
        const maxX = Math.max(minX, workArea.position.x + workArea.size.width - popupSize.width);
        const maxY = Math.max(minY, workArea.position.y + workArea.size.height - popupSize.height);
        if (position) {
          // Growing into conversation mode may require a small adjustment to
          // keep the composer on-screen. Otherwise preserve the window anchor.
          const x = Math.min(Math.max(position.x, minX), maxX);
          const y = Math.min(Math.max(position.y, minY), maxY);
          if (x !== position.x || y !== position.y) await popupWindow.setPosition(new PhysicalPosition(x, y));
          return;
        }
        if (!cursor) return;
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
    popupOpenSequence,
    conversation,
    captureState,
    errorMessage,
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
    selection,
    settingsError,
    updates.phase,
    updates.version,
    updates.notes,
    updates.error,
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
    if (updateController.getSnapshot().phase === "installing") return;
    if (ocrState === "reading" && !bypassOcrWait) {
      const intent = resolveIntent(intentOverride);
      pendingOcrIntentRef.current = intent;
      setQueuedOcrIntent(intent);
      return;
    }

    const intent = resolveIntent(intentOverride);
    const activeSelection = selectionOverride ?? selection;
    const activeImage = selectionOverride ? null : image;
    if (!activeSelection && !activeImage && intent.kind !== "custom") {
      return;
    }

    lastIntentRef.current = intent;
    const prompt = intent.kind === "custom" ? intent.instruction : QUICK_ACTION_LABELS[intent.action];
    await submitTurn("initial", prompt, {
      selectedText: activeSelection?.text ?? null,
      imageId: activeImage?.imageId ?? null,
      intent,
    }, Boolean(currentTurn), activeImage ? "Screenshot attached" : activeSelection ? "Reference text attached" : undefined);
  };

  startGenerationRef.current = startGeneration;

  const startScreenRead = async (imageId: number, source: ImageCaptureEvent["source"]) => {
    if (updateController.getSnapshot().phase === "installing") return;
    ocrImageIdRef.current = imageId;
    setImage(null);
    setOcrState("reading");
    setOcrSource(source);
    setOcrError(null);

    try {
      await invoke<GenerationStarted>("read_screen", {
        request: { imageId, conversationId: conversationIdRef.current },
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

  const setAutostart = async (enabled: boolean) => {
    setIsAutostartLoading(true);
    try {
      if (enabled) {
        await enableAutostart();
      } else {
        await disableAutostart();
      }
      setAutostartEnabled(enabled);
      setAutostartError(null);
      return true;
    } catch (error) {
      setAutostartError(errorText(error));
      return false;
    } finally {
      setIsAutostartLoading(false);
    }
  };

  const finishOnboarding = async () => {
    if (!settingsDraft || !(settingsDraft.model || settings?.effectiveModel)) {
      setSettingsError("Choose an installed Ollama model before finishing setup.");
      return;
    }

    setIsSettingsLoading(true);
    try {
      const config = { ...settingsDraft, hasCompletedOnboarding: true };
      const snapshot = await invoke<SettingsSnapshot>("save_settings", { config });
      setSettings(snapshot);
      setSettingsDraft(snapshot.config);
      setSettingsError(null);
      if (!await setAutostart(onboardingStartAtLogin)) {
        return;
      }
      setIsOnboardingOpen(false);
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
    popupPositionRequestRef.current = null;
    resetConversation();
    setIsPopupSurfaceVisible(false);
    void invoke("discard_image_capture");
    void getCurrentWindow().hide();
  };

  const startNewConversation = () => {
    resetConversation();
    setIsSettingsOpen(false);
    void invoke("discard_image_capture");
    window.requestAnimationFrame(() => initialInputRef.current?.focus());
  };

  const configuredModelIsListed = settingsDraft?.model
    && availableModels.some((model) => model.name === settingsDraft.model);
  const configuredOcrModelIsListed = settingsDraft
    && availableModels.some((model) => model.name === settingsDraft.ocrModel);
  const canFinishOnboarding = Boolean(settingsDraft?.model || settings?.effectiveModel);

  return (
    <main className="min-h-screen overflow-hidden p-6 text-zinc-100">
      <section className="mx-auto w-full max-w-xl">
        <div
          ref={popupContentRef}
          className={`popup-surface ${isPopupSurfaceVisible ? "popup-surface--enter" : "popup-surface--hidden"} rounded-[28px] bg-zinc-900 p-4 shadow-lg shadow-black/50`}
        >
          <header
            className={`relative -mx-4 -mt-4 mb-3 flex cursor-grab select-none items-center gap-3 pl-4 pt-4 active:cursor-grabbing ${conversation.expanded && !isSettingsOpen ? "pr-32" : "pr-24"}`}
            onMouseDown={(event) => {
              if (event.button === 0) {
                void getCurrentWindow().startDragging().catch(() => undefined);
              }
            }}
          >
            <div data-tauri-drag-region className="flex shrink-0 items-center gap-2">
              <img data-tauri-drag-region src="/snippet-icon.png" alt="Snippet" draggable={false} width={32} height={32} className="size-8 object-contain" />
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
                  ref={initialInputRef}
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
            {!isSettingsOpen && hasSubmittedPrompt && !isGenerating && conversation.expanded && (
              <span data-tauri-drag-region className="min-w-0 flex-1 px-1 py-2 text-[17px] text-zinc-400">
                Conversation
              </span>
            )}
            {conversation.expanded && !isSettingsOpen && <button
              type="button"
              aria-label="New conversation"
              title="New conversation"
              className="absolute right-20 top-4 grid size-8 cursor-pointer place-items-center rounded-md text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
              onMouseDown={(event) => event.stopPropagation()}
              onClick={startNewConversation}
            >
              <svg aria-hidden="true" viewBox="0 0 24 24" className="size-4 fill-none stroke-current stroke-[1.8]" strokeLinecap="round" strokeLinejoin="round">
                <rect x="3" y="3" width="18" height="18" rx="3" />
                <path d="M12 8v8m-4-4h8" />
              </svg>
            </button>}
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
              {updates.version && !isSettingsOpen && <span aria-label="Update available" className="absolute right-0 top-0 size-2 rounded-full bg-violet-400" />}
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
                    className="cursor-pointer rounded-full bg-violet-500 px-3.5 py-1.5 text-sm font-normal text-white transition-colors hover:bg-violet-400"
                    onClick={() => void startScreenRead(image.imageId, image.source)}
                  >
                    {image.source === "region" ? "Read region" : "Read screen"}
                  </button>
                )}
                {(selection || image || ocrState === "reading") && QUICK_ACTIONS.map((action) => (
                  <button
                    key={action}
                    type="button"
                    className="cursor-pointer rounded-full bg-zinc-700 px-3.5 py-1.5 text-sm font-normal text-zinc-300 transition-colors hover:bg-zinc-700 hover:text-zinc-100"
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
              {isOnboardingOpen && (
                <section className="mb-4 rounded-xl border border-violet-400/30 bg-violet-500/10 p-3">
                  <p className="text-sm font-semibold text-zinc-100">Welcome to Snippet</p>
                  <p className="mt-1 text-xs leading-5 text-zinc-400">
                    Snippet runs locally through Ollama. Install Ollama, choose a model below, then finish setup.
                  </p>
                  <button
                    type="button"
                    className="mt-2 text-xs font-medium text-violet-200 underline decoration-violet-300/50 underline-offset-2 hover:text-violet-100"
                    onClick={() => void openUrl("https://ollama.com/download")}
                  >
                    Download Ollama
                  </button>
                  <label className="mt-3 flex cursor-pointer items-center justify-between gap-3 rounded-lg border border-zinc-700/80 bg-zinc-950/30 px-2.5 py-2">
                    <span className="text-xs text-zinc-200">Launch Snippet when I sign in</span>
                    <input
                      aria-label="Launch Snippet when I sign in"
                      className="size-3.5 cursor-pointer accent-violet-400"
                      type="checkbox"
                      checked={onboardingStartAtLogin}
                      disabled={isAutostartLoading}
                      onChange={(event) => setOnboardingStartAtLogin(event.target.checked)}
                    />
                  </label>
                  <button
                    type="button"
                    className="mt-3 w-full rounded-lg bg-violet-500 px-3 py-2 text-xs font-bold text-white transition-colors hover:bg-violet-400 disabled:cursor-not-allowed disabled:opacity-45"
                    disabled={!canFinishOnboarding || isSettingsLoading || isAutostartLoading}
                    onClick={() => void finishOnboarding()}
                  >
                    Finish setup
                  </button>
                </section>
              )}
              <div className="mb-3 flex items-center justify-between gap-3">
                <p className="text-sm font-semibold text-zinc-200">{settingsPage === "updates" ? "Updates" : "Local AI"}</p>
                {settingsPage !== "updates" && <button
                  type="button"
                  className="text-xs font-medium text-zinc-400 transition-colors hover:text-zinc-100"
                  onClick={() => void refreshProviderStatus()}
                >
                  Refresh
                </button>}
              </div>

              <div className="mb-4 flex gap-1 rounded-lg bg-zinc-950/50 p-1">
                {([
                  ["general", "General"],
                  ["ocr", "Read screen"],
                  ["shortcuts", "Shortcuts"],
                  ["updates", "Updates"],
                ] as const).map(([page, label]) => (
                  <button
                    key={page}
                    type="button"
                    className={`flex-1 whitespace-nowrap rounded-md px-2 py-1.5 text-xs font-medium transition-colors ${settingsPage === page ? "bg-zinc-700 text-zinc-100" : "text-zinc-500 hover:text-zinc-300"}`}
                    onClick={() => setSettingsPage(page)}
                  >
                    {label}
                  </button>
                ))}
              </div>

              {settingsPage === "updates" && (
                <UpdatesPanel update={updates}
                  automatic={settingsDraft?.automaticUpdates ?? true}
                  onAutomaticChange={(enabled) => setSettingsDraft((draft) => draft && ({ ...draft, automaticUpdates: enabled }))}
                  busy={isGenerating || ocrState === "reading"} />
              )}

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

              <label className="mt-3 flex cursor-pointer items-center justify-between gap-3 rounded-lg border border-zinc-800 bg-zinc-900 px-2.5 py-2">
                <span>
                  <span className="block text-sm font-medium text-zinc-200">Launch at sign-in</span>
                  <span className="block text-xs text-zinc-500">Keep Snippet ready in the background.</span>
                </span>
                <input
                  aria-label="Launch at sign-in"
                  className="size-3.5 cursor-pointer accent-zinc-100 disabled:cursor-not-allowed"
                  type="checkbox"
                  checked={autostartEnabled}
                  disabled={isAutostartLoading}
                  onChange={(event) => void setAutostart(event.target.checked)}
                />
              </label>
              {autostartError && <p className="mt-2 text-xs text-rose-300">{autostartError}</p>}
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

              {settingsPage !== "updates" && settings?.modelSource === "environment" && (
                <p className="mt-3 text-xs text-zinc-400">
                  `SNIPPET_OLLAMA_MODEL` is overriding the saved model.
                </p>
              )}
              {settingsPage !== "updates" && providerStatus && (
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
                  {isSettingsLoading ? "Saving / loading settings…" : settingsPage === "updates" ? "Preferences apply when saved." : ""}
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
            <ConversationPanel
              conversation={conversation}
              onContinue={() => dispatchConversation({ type: "expand" })}
              onNewConversation={startNewConversation}
              onSubmit={(instruction) => void submitTurn("follow-up", instruction, { instruction })}
              onRetry={() => {
                if (currentTurn?.kind === "follow-up") {
                  void submitTurn("follow-up", currentTurn.prompt, { instruction: currentTurn.prompt }, true);
                } else {
                  void startGeneration(lastIntentRef.current ?? undefined);
                }
              }}
              onStop={() => {
                const identity = activeTurnRef.current;
                void activeRequestPromiseRef.current.then(() => {
                  if (identity && activeTurnRef.current === identity) void invoke("cancel_generation");
                });
              }}
            />
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
