import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AttachmentPicker, ImageAttachmentPreview } from "./AttachmentPicker";
import { AttachFilesButton } from "./FileAttachments";

describe("context attachments", () => {
  it("keeps window attachment available when the file limit is reached", () => {
    const html = renderToStaticMarkup(createElement(AttachmentPicker, {
      conversationId: 1, filesDisabled: true, onFile() {}, onCapture() {}, onClose() {}, onBusy() {}, onLayoutChange() {},
    }));
    expect(html).toContain("Add file");
    expect(html).toContain("Add window");
    expect(html.match(/disabled=""/g)).toHaveLength(1);
  });
  it("labels the plus button as a context menu", () => {
    const html = renderToStaticMarkup(createElement(AttachFilesButton, { disabled: false, busy: false, expanded: true, onClick() {} }));
    expect(html).toContain('aria-label="Add context"');
    expect(html).toContain('aria-expanded="true"');
  });
  it("shows an escaped window title and screenshot dimensions without treating it as a live feed", () => {
    const html = renderToStaticMarkup(createElement(ImageAttachmentPreview, {
      image: { imageId: 1, source: "window", title: "<Notes>", preview: { dataUrl: "data:image/png;base64,abc", width: 800, height: 600 } },
      disabled: false, onRemove() {},
    }));
    expect(html).toContain("&lt;Notes&gt;");
    expect(html).toContain("800 × 600");
    expect(html).toContain('aria-label="Preview screenshot"');
    expect(html).toContain('aria-label="Remove captured image"');
  });
});
