import { mix } from "./appearance";

function luminance(hex: string) {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((v) => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4);
  return c[0] * .2126 + c[1] * .7152 + c[2] * .0722;
}
export function contrastRatio(a: string, b: string) {
  const x = luminance(a), y = luminance(b);
  return (Math.max(x, y) + .05) / (Math.min(x, y) + .05);
}
function readable(color: string, background: string) {
  const target = contrastRatio("#ffffff", background) > contrastRatio("#000000", background) ? "#ffffff" : "#000000";
  for (let step = 0; step <= 20; step++) {
    const candidate = mix(color, target, step / 20);
    if (contrastRatio(candidate, background) >= 4.5) return candidate;
  }
  return target;
}
export function syntaxVariables(mode: "light" | "dark", background: string) {
  const palette = mode === "dark"
    ? { keyword: "#c4b5fd", string: "#86efac", number: "#fdba74", title: "#93c5fd", comment: "#a1a1aa", property: "#f9a8d4", meta: "#67e8f9" }
    : { keyword: "#6d28d9", string: "#166534", number: "#9a3412", title: "#1d4ed8", comment: "#52525b", property: "#9d174d", meta: "#155e75" };
  return Object.fromEntries(Object.entries(palette).map(([key, color]) => [`--syntax-${key}`, readable(color, background)]));
}
