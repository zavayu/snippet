export type ResearchSource = {
  id: number;
  title: string;
  url: string;
  excerpt: string;
  publishedAt: string | null;
};
export type ResearchInfo = {
  phase: "unsupported" | "deciding" | "answering" | "planning" | "searching" | "reading" | "summarizing";
  query: string | null;
  sources: ResearchSource[];
};

export function safeWebUrl(value: string): string | null {
  try {
    const url = new URL(value);
    return ["http:", "https:"].includes(url.protocol) && !url.username && !url.password ? url.href : null;
  } catch { return null; }
}

// Convert only text nodes, leaving code, existing links and images alone. The
// generated links always come from the retrieved source list, never model URLs.
export function researchCitations(sources: ResearchSource[]) {
  type Node = { type: string; value?: string; children?: Node[]; url?: string };
  const byId = new Map(sources.filter((source) => safeWebUrl(source.url)).map((source) => [source.id, source]));
  return () => (tree: Node) => {
    const visit = (node: Node) => {
      if (!node.children || ["link", "linkReference", "image", "code", "inlineCode"].includes(node.type)) return;
      node.children = node.children.flatMap((child) => {
        if (child.type !== "text" || !child.value) { visit(child); return [child]; }
        const parts: Node[] = [];
        let offset = 0;
        for (const match of child.value.matchAll(/\[(\d+(?:\s*,\s*\d+)*)\]/g)) {
          if (!match[1].split(",").some((id) => byId.has(Number(id)))) continue;
          if (match.index! > offset) parts.push({ type: "text", value: child.value.slice(offset, match.index) });
          parts.push({ type: "text", value: "[" });
          for (const token of match[1].split(/(\d+)/)) {
            if (!token) continue;
            const source = /^\d+$/.test(token) ? byId.get(Number(token)) : undefined;
            parts.push(source
              ? { type: "link", url: source.url, children: [{ type: "text", value: token }] }
              : { type: "text", value: token });
          }
          parts.push({ type: "text", value: "]" });
          offset = match.index! + match[0].length;
        }
        if (!parts.length) return [child];
        if (offset < child.value.length) parts.push({ type: "text", value: child.value.slice(offset) });
        return parts;
      });
    };
    visit(tree);
  };
}
export function responseWithSources(text: string, sources?: ResearchSource[]): string {
  const valid = sources?.filter((source) => safeWebUrl(source.url));
  return valid?.length ? `${text}\n\nSources\n${valid.map((source) => `[${source.id}] ${source.title} - ${source.url}`).join("\n")}` : text;
}
