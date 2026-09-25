import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

type MarkdownResponseProps = {
  content: string;
};

export function MarkdownResponse({ content }: MarkdownResponseProps) {
  return (
    <div className="markdown-response">
      <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml>
        {content}
      </ReactMarkdown>
    </div>
  );
}
