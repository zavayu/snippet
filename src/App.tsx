import { useState } from "react";
import "./App.css";

type ResponseState = "ready" | "thinking" | "complete";

function App() {
  const [responseState, setResponseState] = useState<ResponseState>("ready");

  function previewResponse() {
    setResponseState("thinking");
    window.setTimeout(() => setResponseState("complete"), 500);
  }

  return (
    <main className="min-h-screen bg-stone-950 p-6 text-stone-100">
      <section className="mx-auto flex min-h-[calc(100vh-3rem)] max-w-xl flex-col justify-center">
        <div className="rounded-2xl border border-white/10 bg-stone-900 p-6 shadow-2xl shadow-black/30">
          <header className="mb-5 flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="grid size-9 place-items-center rounded-xl bg-emerald-400 font-bold text-stone-950">
                S
              </div>
              <div>
                <h1 className="text-lg font-semibold tracking-tight">Snippet</h1>
                <p className="text-xs text-stone-400">Local AI for selected text</p>
              </div>
            </div>
            <span className="rounded-full bg-emerald-400/10 px-2.5 py-1 text-xs font-medium text-emerald-300">
              Ollama ready
            </span>
          </header>

          <div className="rounded-xl border border-white/10 bg-stone-950/70 p-4">
            <p className="mb-2 text-xs font-medium uppercase tracking-wider text-stone-500">Selected text</p>
            <blockquote className="text-sm leading-6 text-stone-300">
              “Good design is as little design as possible.”
            </blockquote>
          </div>

          <div className="mt-4 min-h-24 rounded-xl bg-white/[0.04] p-4">
            {responseState === "ready" && (
              <p className="text-sm text-stone-400">Choose an action to ask your local model.</p>
            )}
            {responseState === "thinking" && (
              <p className="text-sm text-stone-300">Thinking<span className="animate-pulse">...</span></p>
            )}
            {responseState === "complete" && (
              <p className="text-sm leading-6 text-stone-200">
                It argues for clarity and restraint: remove every element that does not improve the result.
              </p>
            )}
          </div>

          <footer className="mt-5 flex items-center justify-between gap-3">
            <div className="flex gap-2">
              <button className="action-button" onClick={previewResponse}>Explain</button>
              <button className="action-button" onClick={previewResponse}>Summarize</button>
              <button className="action-button" onClick={previewResponse}>Rewrite</button>
            </div>
            <kbd className="hidden rounded-md border border-white/10 px-2 py-1 text-xs text-stone-500 sm:block">
              Ctrl + Shift + Space
            </kbd>
          </footer>
        </div>
        <p className="mt-4 text-center text-xs text-stone-500">
          UI preview only — clipboard capture and Ollama streaming come next.
        </p>
      </section>
    </main>
  );
}

export default App;
