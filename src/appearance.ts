export type Appearance = {
  mode: "system" | "light" | "dark";
  accent: string;
  lightBackground: string | null; lightForeground: string | null;
  darkBackground: string | null; darkForeground: string | null;
  uiFont: "system" | "segoe-ui" | "arial" | "georgia";
  responseFont: "same" | "segoe-ui" | "arial" | "georgia";
  codeFont: "cascadia-code" | "consolas" | "courier-new";
  uiFontSize: number; responseFontSize: number; codeFontSize: number;
  windowWidth: number; chatMaxHeight: number; settingsMaxHeight: number;
  reduceMotion: "system" | "on" | "off";
};
export const DEFAULT_APPEARANCE: Appearance = {
  windowWidth: 510, chatMaxHeight: 900, settingsMaxHeight: 680,
  mode: "system", accent: "#8b5cf6", lightBackground: null, lightForeground: null,
  darkBackground: null, darkForeground: null, uiFont: "system", responseFont: "same",
  codeFont: "cascadia-code", uiFontSize: 13, responseFontSize: 14, codeFontSize: 12, reduceMotion: "system",
};
export function resolveMode(mode: Appearance["mode"], systemDark: boolean) {
  return mode === "system" ? systemDark ? "dark" : "light" : mode;
}
export function modeColors(value: Appearance, mode: "light" | "dark") {
  return mode === "dark" ? { background: value.darkBackground ?? "#18181b", foreground: value.darkForeground ?? "#f4f4f5" }
    : { background: value.lightBackground ?? "#fafafa", foreground: value.lightForeground ?? "#18181b" };
}
function channels(hex: string) { return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)); }
export function mix(a: string, b: string, amount: number) {
  const end = channels(b);
  return "#" + channels(a).map((v, i) => Math.round(v + (end[i] - v) * amount).toString(16).padStart(2, "0")).join("");
}
export function accentInk(hex: string) {
  const rgb = channels(hex).map((v) => { const s = v / 255; return s <= .04045 ? s / 12.92 : ((s + .055) / 1.055) ** 2.4; });
  const luminance = rgb[0] * .2126 + rgb[1] * .7152 + rgb[2] * .0722;
  // Keep white text on medium accents, including the default purple.
  return luminance > .25 ? "#18181b" : "#ffffff";
}
const fonts = { system: 'Inter, "Segoe UI", system-ui, sans-serif', "segoe-ui": '"Segoe UI", sans-serif', arial: 'Arial, sans-serif', georgia: 'Georgia, serif' };
const codeFonts = { "cascadia-code": '"Cascadia Code", Consolas, monospace', consolas: 'Consolas, monospace', "courier-new": '"Courier New", monospace' };
export function appearanceVariables(value: Appearance, mode: "light" | "dark"): Record<string, string> {
  const { background, foreground } = modeColors(value, mode);
  const variables: Record<string, string> = {};
  for (const [shade, amount] of [[950, 0], [900, 0], [800, .07], [700, .17], [600, .32], [500, .48], [400, .65], [300, .8], [200, .9], [100, 1], [50, 1]]) {
    variables[`--color-zinc-${shade}`] = mix(background, foreground, amount);
  }
  variables["--color-zinc-950"] = mix(background, mode === "dark" ? "#000000" : "#ffffff", .5);
  for (const shade of [50,100,200,300,400,500,600,700,800,900,950]) {
    variables[`--color-violet-${shade}`] = shade === 500 ? value.accent : mix(value.accent, shade < 500 ? foreground : background, Math.abs(shade - 500) / 650);
  }
  return { ...variables, "--status-warning": mode === "light" ? "#92400e" : "#fde68a",
    "--status-error": mode === "light" ? "#be123c" : "#fda4af", "--accent-ink": accentInk(value.accent), "--ui-font": fonts[value.uiFont],
    "--response-font": value.responseFont === "same" ? fonts[value.uiFont] : fonts[value.responseFont],
    "--code-font": codeFonts[value.codeFont], "--ui-font-size": `${value.uiFontSize}px`,
    "--response-font-size": `${value.responseFontSize}px`, "--code-font-size": `${value.codeFontSize}px`,
    "--text-xs": `${value.uiFontSize - 1}px`, "--text-sm": `${value.uiFontSize + 1}px`, "--text-base": `${value.uiFontSize + 3}px`,
  };
}

// Monitor work-area dimensions are converted to logical pixels by the caller.
export function popupLimits(value: Appearance, settingsOpen: boolean, workArea?: { width: number; height: number }) {
  const height = settingsOpen ? value.settingsMaxHeight : value.chatMaxHeight;
  return {
    width: workArea ? Math.min(value.windowWidth, Math.max(1, workArea.width - 16)) : value.windowWidth,
    height: workArea ? Math.min(height, Math.max(112, workArea.height - 16)) : height,
  };
}
