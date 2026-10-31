import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { FileAttachments, type AttachedFile } from "./FileAttachments";

function render(files: AttachedFile[], disabled = false) {
  return renderToStaticMarkup(createElement(FileAttachments, {
    conversationId: 1, files, disabled, onChange: () => {}, onBusy: () => {}, onLayoutChange: () => {},
  }));
}

describe("file attachment controls", () => {
  it("offers a multiple-file picker without exposing file contents or paths", () => {
    const html = render([{ fileId: "file-1", name: "notes.md", sizeBytes: 100, totalLines: 3 }]);
    expect(html).toContain('type="file"');
    expect(html).toContain('multiple=""');
    expect(html).not.toContain('aria-label="Attach files"');
    expect(html).toContain("notes.md");
    expect(html).toContain('aria-label="Remove notes.md"');
    expect(html).toContain("previously read excerpts remain");
  });
  it("disables add and remove while a response is active", () => {
    const html = render([{ fileId: "file-1", name: "source.rs", sizeBytes: 123, totalLines: 12 }], true);
    expect(html.match(/disabled=""/g)).toHaveLength(2);
  });
  it("escapes untrusted filenames in the attachment list", () => {
    const html = render(Array.from({ length: 5 }, (_, i) => ({ fileId: `file-${i}`, name: i ? `${i}.txt` : "<script>.txt", sizeBytes: 10, totalLines: 1 })));
    expect(html).toContain("&lt;script&gt;.txt");
    expect(html).not.toContain("<script>");
    expect(html).not.toContain('aria-label="Attach files"');
  });
});
