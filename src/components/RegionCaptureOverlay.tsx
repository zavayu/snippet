import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";

type Point = { x: number; y: number };

type Region = {
  x: number;
  y: number;
  width: number;
  height: number;
};

function pointFor(event: ReactPointerEvent<HTMLElement>): Point {
  const bounds = event.currentTarget.getBoundingClientRect();
  return {
    x: Math.min(1, Math.max(0, (event.clientX - bounds.left) / bounds.width)),
    y: Math.min(1, Math.max(0, (event.clientY - bounds.top) / bounds.height)),
  };
}

function regionFrom(start: Point, end: Point): Region {
  return {
    x: Math.min(start.x, end.x),
    y: Math.min(start.y, end.y),
    width: Math.abs(end.x - start.x),
    height: Math.abs(end.y - start.y),
  };
}

export function RegionCaptureOverlay() {
  const startRef = useRef<Point | null>(null);
  const [region, setRegion] = useState<Region | null>(null);
  const [message, setMessage] = useState("Drag to capture a region · Esc to cancel");
  const [isSubmitting, setIsSubmitting] = useState(false);

  const cancel = () => {
    if (!isSubmitting) {
      void invoke("cancel_region_capture");
    }
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancel();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [isSubmitting]);

  const submit = async (nextRegion: Region) => {
    if (nextRegion.width < 0.01 || nextRegion.height < 0.01) {
      setMessage("Select a larger region.");
      return;
    }

    setIsSubmitting(true);
    setMessage("Capturing region…");
    try {
      await invoke("complete_region_capture", { region: nextRegion });
    } catch (error) {
      setIsSubmitting(false);
      setMessage(error instanceof Error ? error.message : String(error));
    }
  };

  const onPointerDown = (event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0 || isSubmitting) {
      return;
    }
    const point = pointFor(event);
    startRef.current = point;
    setRegion({ x: point.x, y: point.y, width: 0, height: 0 });
    setMessage("Release to capture · Esc to cancel");
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: ReactPointerEvent<HTMLElement>) => {
    const start = startRef.current;
    if (!start || isSubmitting) {
      return;
    }
    setRegion(regionFrom(start, pointFor(event)));
  };

  const onPointerUp = (event: ReactPointerEvent<HTMLElement>) => {
    const start = startRef.current;
    if (!start || isSubmitting) {
      return;
    }
    startRef.current = null;
    const nextRegion = regionFrom(start, pointFor(event));
    setRegion(nextRegion);
    void submit(nextRegion);
  };

  return (
    <main
      aria-label="Select a screen region"
      className="relative h-full w-full cursor-crosshair select-none"
      onContextMenu={(event) => {
        event.preventDefault();
        cancel();
      }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
    >
      {!region && <div className="absolute inset-0 bg-black/35" />}
      {region && (
        <div
          aria-hidden="true"
          className="absolute border border-white/90 bg-white/5 shadow-[0_0_0_9999px_rgba(0,0,0,0.35)]"
          style={{
            left: `${region.x * 100}%`,
            top: `${region.y * 100}%`,
            width: `${region.width * 100}%`,
            height: `${region.height * 100}%`,
          }}
        />
      )}
      <p className="pointer-events-none absolute bottom-6 left-1/2 -translate-x-1/2 rounded-full bg-zinc-950/85 px-4 py-2 text-xs text-zinc-100 shadow-lg">
        {message}
      </p>
    </main>
  );
}
