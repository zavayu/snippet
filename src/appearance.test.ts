import { describe, expect, it } from "vitest";
import { accentInk, appearanceVariables, DEFAULT_APPEARANCE, modeColors, popupLimits, resolveMode } from "./appearance";

describe("appearance", () => {
  it.each(["light", "dark"] as const)("keeps warning and error text readable on %s surfaces", (mode) => {
    const vars = appearanceVariables(DEFAULT_APPEARANCE, mode);
    const luminance = (hex: string) => {
      const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4);
      return c[0] * .2126 + c[1] * .7152 + c[2] * .0722;
    };
    for (const status of ["--status-warning", "--status-error"]) {
      for (const surface of ["--color-zinc-900", "--color-zinc-800"]) {
        const a = luminance(vars[status]), b = luminance(vars[surface]);
        expect((Math.max(a, b) + .05) / (Math.min(a, b) + .05)).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
  it("uses separate chat and settings height limits", () => {
    const value = { ...DEFAULT_APPEARANCE, windowWidth: 800, chatMaxHeight: 1200, settingsMaxHeight: 1000 };
    expect(popupLimits(value, false)).toEqual({ width: 800, height: 1200 });
    expect(popupLimits(value, true)).toEqual({ width: 800, height: 1000 });
  });
  it("fits the monitor work area without enlarging smaller preferences", () => {
    const workArea = { width: 700, height: 800 };
    expect(popupLimits({ ...DEFAULT_APPEARANCE, windowWidth: 1000, chatMaxHeight: 1400 }, false, workArea)).toEqual({ width: 684, height: 784 });
    expect(popupLimits(DEFAULT_APPEARANCE, true, workArea)).toEqual({ width: 510, height: 680 });
  });
  it("follows system changes only in system mode", () => {
    expect(resolveMode("system", true)).toBe("dark");
    expect(resolveMode("system", false)).toBe("light");
    expect(resolveMode("dark", false)).toBe("dark");
    expect(resolveMode("light", true)).toBe("light");
  });
  it("keeps custom light and dark colors independent", () => {
    const value = { ...DEFAULT_APPEARANCE, darkBackground: "#111111", lightForeground: "#222222" };
    expect(modeColors(value, "dark")).toEqual({ background: "#111111", foreground: "#f4f4f5" });
    expect(modeColors(value, "light")).toEqual({ background: "#fafafa", foreground: "#222222" });
  });
  it("reverses existing surface and text tokens for light mode", () => {
    const light = appearanceVariables(DEFAULT_APPEARANCE, "light");
    const dark = appearanceVariables(DEFAULT_APPEARANCE, "dark");
    expect(light["--color-zinc-900"]).toBe("#fafafa");
    expect(light["--color-zinc-100"]).toBe("#18181b");
    expect(dark["--color-zinc-900"]).toBe("#18181b");
    expect(dark["--color-zinc-100"]).toBe("#f4f4f5");
  });
  it("chooses readable text on light and dark accents", () => {
    expect(accentInk(DEFAULT_APPEARANCE.accent)).toBe("#ffffff");
    expect(accentInk("#ffcc00")).toBe("#18181b");
    expect(accentInk("#331166")).toBe("#ffffff");
  });
  it("applies typography to interface, responses and code independently", () => {
    const vars = appearanceVariables({ ...DEFAULT_APPEARANCE, uiFontSize: 18, responseFontSize: 22, codeFontSize: 20, responseFont: "georgia", codeFont: "consolas" }, "dark");
    expect(vars["--ui-font-size"]).toBe("18px");
    expect(vars["--text-xs"]).toBe("17px");
    expect(vars["--response-font-size"]).toBe("22px");
    expect(vars["--code-font-size"]).toBe("20px");
    expect(vars["--response-font"]).toContain("Georgia");
    expect(vars["--code-font"]).toContain("Consolas");
  });
});
