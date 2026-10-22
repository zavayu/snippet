import { openUrl } from "@tauri-apps/plugin-opener";
import { type ReactNode } from "react";
import { safeWebUrl } from "../research";

export function WebLink({ href, children }: { href: string; children: ReactNode }) {
  const url = safeWebUrl(href);
  if (!url) return <span>{children}</span>;
  return <a href={url} target="_blank" rel="noreferrer noopener" onClick={(event) => {
    event.preventDefault();
    void openUrl(url).catch(() => undefined);
  }}>{children}</a>;
}
