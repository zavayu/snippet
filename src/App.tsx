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

const POPUP_WIDTH = 510;
const POPUP_MIN_HEIGHT = 112;
const POPUP_MAX_HEIGHT = 720;
const POPUP_SHADOW_MARGIN = 24;

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
  const [isSelectionExpanded, setIsSelectionExpanded] = useState(false);
  const popupContentRef = useRef<HTMLDivElement>(null);
  const lastPositionedSelectionRef = useRef<CapturedSelection | null>(null);

  useEffect(() => {
    const unlisten = Promise.all([
      listen<CapturedSelection>("selection-captured", (event) => {
        setSelection(event.payload);
        setErrorMessage(null);
        setIsSelectionExpanded(false);
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

        // Keep the window anchored while debug text is expanded or collapsed.
        // Cursor-relative placement is only needed for a newly captured selection.
        if (!shouldReposition) {
          return;
        }

        // The Rust side makes an initial placement before showing the window.
        // Reposition after the initial content-driven resize so it never extends
        // under the taskbar or off the bottom of the monitor.
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
  }, [captureState, errorMessage, isSelectionExpanded, selection]);

  return (
    <main className="min-h-screen overflow-hidden p-6 text-zinc-100">
      <section className="mx-auto w-full max-w-xl">
        <div ref={popupContentRef} className="rounded-[28px] bg-zinc-900 p-4 shadow-lg shadow-black/50">
          <header
            className="relative -mx-4 -mt-4 mb-3 flex cursor-grab select-none items-center gap-3 pl-4 pr-10 pt-4 active:cursor-grabbing"
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
            <form className="min-w-0 flex-1" onMouseDown={(event) => event.stopPropagation()}>
              <input
                aria-label="Ask about the selection"
                className="w-full bg-transparent px-1 py-2 text-[17px] text-zinc-100 outline-none placeholder:text-zinc-500"
                placeholder="Ask about this…"
                type="text"
              />
            </form>
            <button
              type="button"
              aria-label="Close Snippet"
              className="absolute right-2 top-2 grid size-8 cursor-pointer place-items-center rounded-md text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
              onMouseDown={(event) => event.stopPropagation()}
              onClick={() => void getCurrentWindow().hide()}
            >
              <span aria-hidden="true" className="text-xl font-light leading-none">×</span>
            </button>
          </header>

          <div className="mb-5 flex flex-wrap gap-2">
            {['Summarize', 'Refine', 'Explain'].map((action) => (
              <button
                key={action}
                type="button"
                className="cursor-pointer rounded-full bg-zinc-800 px-3 py-1.5 text-[10px] font-bold text-zinc-300 transition-colors hover:bg-zinc-700 hover:text-zinc-100"
              >
                {action}
              </button>
            ))}
          </div>

          {captureState === "captured" && selection && (
            <div>
              <button
                type="button"
                aria-expanded={isSelectionExpanded}
                className="flex cursor-pointer items-center gap-2 text-[11px] font-medium  text-zinc-500 transition-colors hover:text-zinc-300"
                onClick={() => setIsSelectionExpanded((expanded) => !expanded)}
              >
                <span aria-hidden="true">{isSelectionExpanded ? "−" : "+"}</span>
                Selected Text
              </button>
              {isSelectionExpanded && (
                <blockquote
                  className="selection-scroll mt-3 overflow-y-auto whitespace-pre-wrap text-sm leading-5 text-zinc-200"
                  style={{ maxHeight: POPUP_MAX_HEIGHT - (160 + POPUP_SHADOW_MARGIN * 2) }}
                >
                  {selection.text}
                </blockquote>
              )}
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
