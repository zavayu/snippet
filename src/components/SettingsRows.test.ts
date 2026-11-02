import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { SettingsGroup, SettingsRow, SettingsToggle } from "./SettingsRows";

describe("settings controls", () => {
  it.each([true, false])("exposes the switch state %s and its helper description", (checked) => {
    const html = renderToStaticMarkup(createElement(SettingsToggle, {
      label: "Thinking", description: "Let supported models reason before answering.", checked, onChange() {},
    }));
    expect(html).toContain('role="switch"');
    expect(html).toContain(`aria-checked="${checked}"`);
    const helperId = html.match(/aria-describedby="([^"]+)"/)?.[1];
    expect(helperId).toBeDefined();
    expect(html).toContain(`<p id="${helperId}"`);
    expect(html).toContain("Let supported models reason before answering.");
  });

  it("keeps unavailable controls disabled", () => {
    const html = renderToStaticMarkup(createElement(SettingsToggle, { label: "Launch at sign-in", checked: false, disabled: true, onChange() {} }));
    expect(html).toContain('disabled=""');
  });

  it("associates a field with its row label inside a named group", () => {
    const html = renderToStaticMarkup(createElement(SettingsGroup, { title: "Local AI", children:
      createElement(SettingsRow, { label: "Model", controlId: "model", children:
        createElement("select", { id: "model", defaultValue: "qwen" }, createElement("option", { value: "qwen" }, "qwen")),
      }),
    }));
    expect(html).toContain('aria-label="Local AI"');
    expect(html).toContain('<label for="model"');
    expect(html).toContain('<select id="model"');
  });
});
