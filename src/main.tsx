import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { RegionCaptureOverlay } from "./components/RegionCaptureOverlay";

const isRegionCaptureWindow = getCurrentWindow().label === "region-capture";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isRegionCaptureWindow ? <RegionCaptureOverlay /> : <App />}
  </React.StrictMode>,
);
