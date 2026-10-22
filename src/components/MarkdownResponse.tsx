import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { researchCitations, type ResearchSource } from "../research";
import { WebLink } from "./WebLink";

type MarkdownResponseProps = {
  content: string;
  sources?: ResearchSource[];
};

export function MarkdownResponse({ content, sources }: MarkdownResponseProps) {
  const allowed = new Set(sources?.map((source) => source.url));
  return (
    <div className="markdown-response">
      <ReactMarkdown remarkPlugins={sources ? [remarkGfm, researchCitations(sources)] : [remarkGfm]} skipHtml
        components={{
          a: ({ href, children }) => href && (!sources || allowed.has(href))
            ? <WebLink href={href}>{children}</WebLink> : <span>{children}</span>,
          // Research evidence can supply links, never remote image loads.
          ...(sources ? { img: ({ alt }: { alt?: string }) => <span>{alt}</span> } : {}),
        }}>
        {content}
      </ReactMarkdown>
    </div>
  );
}
