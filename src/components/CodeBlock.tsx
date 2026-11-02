import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { highlightCode } from "../syntaxHighlight";

export function CodeBlock({ code, language = "" }: { code: string; language?: string }) {
  const [status, setStatus] = useState<"idle" | "copied" | "error">("idle");
  const timeout = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (timeout.current) clearTimeout(timeout.current); }, []);
  const { label, tokens } = useMemo(() => highlightCode(code, language), [code, language]);
  const copyLabel = status === "copied" ? "Copied code" : status === "error" ? "Copy failed - retry" : "Copy code";
  return <figure className="response-code-block" aria-label={`${label} code block`}>
    <figcaption className="response-code-language">{label}</figcaption>
    <pre><code className={language ? `language-${language}` : undefined}>{tokens.map((token, index) => token.kind
      ? <span key={index} className={`syntax-${token.kind}`}>{token.text}</span> : token.text)}</code></pre>
    <div className="response-code-actions">
      <button type="button" className="response-code-copy" title={copyLabel} aria-label={copyLabel} onClick={async () => {
        try { await invoke("copy_response", { text: code }); setStatus("copied"); }
        catch { setStatus("error"); }
        if (timeout.current) clearTimeout(timeout.current);
        timeout.current = setTimeout(() => setStatus("idle"), 2500);
      }}>
        <svg aria-hidden="true" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          {status === "copied" ? <path d="m5 12 4.5 4.5L19 7" /> : <><rect x="8" y="8" width="11" height="11" rx="2" /><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" /></>}
        </svg>
      </button>
      <span role="status" className={status === "error" ? "text-warning" : ""}>{status === "copied" ? "Copied" : status === "error" ? "Copy failed. Try again." : ""}</span>
    </div>
  </figure>;
}
