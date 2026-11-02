import { useEffect, useLayoutEffect, useState } from "react";
import { syntaxVariables } from "./syntaxTheme";
import { appearanceVariables, resolveMode, type Appearance } from "./appearance";

export function useAppearance(value: Appearance) {
  const [systemDark, setSystemDark] = useState(() => matchMedia("(prefers-color-scheme: dark)").matches);
  const [systemMotion, setSystemMotion] = useState(() => matchMedia("(prefers-reduced-motion: reduce)").matches);
  useEffect(() => {
    const dark = matchMedia("(prefers-color-scheme: dark)");
    const motion = matchMedia("(prefers-reduced-motion: reduce)");
    const updateDark = () => setSystemDark(dark.matches);
    const updateMotion = () => setSystemMotion(motion.matches);
    dark.addEventListener("change", updateDark); motion.addEventListener("change", updateMotion);
    updateDark(); updateMotion();
    return () => { dark.removeEventListener("change", updateDark); motion.removeEventListener("change", updateMotion); };
  }, []);
  const mode = resolveMode(value.mode, systemDark);
  useLayoutEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = mode;
    root.dataset.reduceMotion = String(value.reduceMotion === "system" ? systemMotion : value.reduceMotion === "on");
    root.style.colorScheme = mode;
    const variables = appearanceVariables(value, mode);
    Object.assign(variables, syntaxVariables(mode, variables["--color-zinc-950"]));
    for (const [key, entry] of Object.entries(variables)) root.style.setProperty(key, entry);
  }, [value, mode, systemMotion]);
  return mode;
}
