// This is intentionally a compile-time development flag, never enabled in a
// release build. Override it in an untracked `.env.development.local` file.
export const SHOW_DEVELOPMENT_TOOLS =
  import.meta.env.DEV && import.meta.env.VITE_SHOW_DEVELOPMENT_TOOLS === "true";
