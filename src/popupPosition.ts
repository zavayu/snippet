export type ScreenRect = { x: number; y: number; width: number; height: number };
export type SelectionBounds = { first: ScreenRect; last: ScreenRect };

// All coordinates and sizes are physical pixels. Only visual gaps and the
// transparent window inset start in logical pixels and need display scaling.
export function popupPosition({ bounds, cursor, workArea, size, scaleFactor }: {
  bounds?: SelectionBounds | null;
  cursor?: { x: number; y: number } | null;
  workArea: ScreenRect;
  size: { width: number; height: number };
  scaleFactor: number;
}): { x: number; y: number } | null {
  if (!bounds && !cursor) return null;
  const inset = 24 * scaleFactor;
  const gap = 8 * scaleFactor;
  const bottom = workArea.y + workArea.height;
  const maxX = Math.max(workArea.x, workArea.x + workArea.width - size.width);
  const maxY = Math.max(workArea.y, bottom - size.height);
  const x = bounds ? bounds.last.x - inset : cursor!.x + 8 * scaleFactor - inset;
  const below = bounds
    ? bounds.last.y + bounds.last.height + gap - inset
    : cursor!.y + 10 * scaleFactor - inset;
  const above = bounds
    ? bounds.first.y - gap - size.height + inset
    : cursor!.y - 10 * scaleFactor - size.height + inset;
  const y = below <= maxY ? below : above;
  return {
    x: Math.round(Math.min(Math.max(x, workArea.x), maxX)),
    y: Math.round(Math.min(Math.max(y, workArea.y), maxY)),
  };
}
