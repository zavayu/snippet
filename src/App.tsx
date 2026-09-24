import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  cursorPosition,
  getCurrentWindow,
  LogicalSize,
  monitorFromPoint,
  PhysicalPosition,
} from "@tauri-apps/api/window";
import "./App.css";

const POPUP_WIDTH = 440;
const POPUP_MIN_HEIGHT = 220;
const POPUP_MAX_HEIGHT = 540;

type CaptureState = "waiting" | "captured" | "error";

type CapturedSelection = {
  text: string;
  source: "clipboard-copy" | "ui-automation";
  truncated: boolean;
  clipboardRestored: boolean;
};

type CaptureFailure = {
  message: string;
};

function App() {
  const [captureState, setCaptureState] = useState<CaptureState>("waiting");
  const [selection, setSelection] = useState<CapturedSelection | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const popupContentRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const unlisten = Promise.all([
      listen<CapturedSelection>("selection-captured", (event) => {
        setSelection(event.payload);
        setErrorMessage(null);
        setCaptureState("captured");
      }),
      listen<CaptureFailure>("selection-capture-failed", (event) => {
        setSelection(null);
        setErrorMessage(event.payload.message);
        setCaptureState("error");
      }),
    ]);

    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
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
    const frame = window.requestAnimationFrame(() => {
      const contentHeight = popupContentRef.current?.getBoundingClientRect().height ?? POPUP_MIN_HEIGHT;
      const nextHeight = Math.min(
        Math.max(Math.ceil(contentHeight) + 24, POPUP_MIN_HEIGHT),
        POPUP_MAX_HEIGHT,
      );

      void (async () => {
        const popupWindow = getCurrentWindow();
        await popupWindow.setSize(new LogicalSize(POPUP_WIDTH, nextHeight));

        // The Rust side makes an initial placement before showing the window.
        // Reposition after the content-driven resize so a tall popup never
        // extends under the taskbar or off the bottom of the monitor.
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
        const belowCursor = cursor.y + 18;
        const x = Math.min(Math.max(cursor.x + 14, minX), maxX);
        const y = belowCursor <= maxY
          ? belowCursor
          : Math.max(minY, cursor.y - 18 - popupSize.height);

        await popupWindow.setPosition(new PhysicalPosition(x, y));
      })().catch(() => undefined);
    });

    return () => {
      cancelled = true;
      window.cancelAnimationFrame(frame);
    };
  }, [captureState, errorMessage, selection]);

  return (
    <main className="bg-zinc-900 p-3 text-zinc-100">
      <section className="mx-auto w-full max-w-xl">
        <div ref={popupContentRef} className="rounded-xl border border-zinc-700 bg-zinc-900 p-4 shadow-2xl shadow-black/30">
          <header
            className="mb-3 flex cursor-grab select-none items-center justify-between active:cursor-grabbing"
            onMouseDown={(event) => {
              if (event.button === 0) {
                void getCurrentWindow().startDragging().catch(() => undefined);
              }
            }}
          >
            <div data-tauri-drag-region className="flex items-center gap-3">
              <div data-tauri-drag-region className="grid size-8 place-items-center rounded-lg bg-zinc-100 text-sm font-bold text-zinc-950">
                S
              </div>
              <div data-tauri-drag-region>
                <h1 data-tauri-drag-region className="text-sm font-semibold tracking-tight">Snippet</h1>
                <p data-tauri-drag-region className="text-xs text-zinc-400">Local AI for selected text</p>
              </div>
            </div>
            <span data-tauri-drag-region className="rounded-full border border-zinc-700 bg-zinc-800 px-2.5 py-1 text-xs font-medium text-zinc-300">
              Capture ready
            </span>
          </header>

          <div className="rounded-lg border border-zinc-700 bg-zinc-950 p-3">
            <p className="mb-1 text-xs font-medium uppercase tracking-wider text-zinc-500">Selected text</p>
            {captureState === "captured" && selection ? (
              <blockquote className="max-h-80 overflow-y-auto whitespace-pre-wrap text-sm leading-5 text-zinc-200">
                {selection.text}
              </blockquote>
            ) : (
              <p className="text-sm leading-5 text-zinc-400">
                Select text in another application, then press the Snippet shortcut.
              </p>
            )}
          </div>

          <div className="mt-3 text-xs leading-5 text-zinc-400">
            {captureState === "waiting" && (
              <p>Waiting for a selection.</p>
            )}
            {captureState === "captured" && selection && (
              <p>
                Captured with {selection.source === "clipboard-copy" ? "Copy" : "UI Automation"}.
                {selection.truncated ? " The preview was shortened to 20,000 characters." : ""}
              </p>
            )}
            {captureState === "error" && (
              <p className="text-amber-200">{errorMessage}</p>
            )}
          </div>

          <footer className="mt-3 flex items-center justify-between gap-3 border-t border-zinc-800 pt-3">
            <p className="text-xs text-zinc-500">Press Escape to hide Snippet.</p>
            <kbd className="hidden rounded-md border border-zinc-700 px-2 py-1 text-xs text-zinc-500 sm:block">
              Ctrl + Shift + Space
            </kbd>
          </footer>
        </div>
      </section>
    </main>
  );
}

export default App;
