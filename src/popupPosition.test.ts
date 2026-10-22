import { describe, expect, it } from "vitest";
import { popupPosition } from "./popupPosition";

const workArea = { x: 0, y: 0, width: 1920, height: 1040 };
const size = { width: 510, height: 200 };
const bounds = {
  first: { x: 400, y: 200, width: 300, height: 20 },
  last: { x: 400, y: 240, width: 150, height: 20 },
};

describe("selection popup positioning", () => {
  it("places the visible popup below the last selected line regardless of cursor", () => {
    expect(popupPosition({ bounds, cursor: { x: 1800, y: 900 }, workArea, size, scaleFactor: 1 }))
      .toEqual({ x: 376, y: 244 });
  });
  it("flips above the first selected line as the popup grows", () => {
    expect(popupPosition({ bounds, workArea, size: { width: 510, height: 850 }, scaleFactor: 1 }))
      .toEqual({ x: 376, y: 0 });
    const lower = { first: { ...bounds.first, y: 850 }, last: { ...bounds.last, y: 900 } };
    expect(popupPosition({ bounds: lower, workArea, size, scaleFactor: 1 }))
      .toEqual({ x: 376, y: 666 });
  });
  it("scales gaps without scaling physical selection coordinates twice", () => {
    const result = popupPosition({ bounds, workArea, size, scaleFactor: 2 })!;
    expect(result.x + 48).toBe(bounds.last.x);
    expect(result.y + 48).toBe(bounds.last.y + bounds.last.height + 16);
  });
  it("clamps to the selection monitor including negative desktop coordinates", () => {
    const area = { x: -1920, y: -100, width: 1920, height: 1080 };
    const selected = { first: { x: -50, y: 100, width: 20, height: 20 }, last: { x: -50, y: 120, width: 10, height: 20 } };
    expect(popupPosition({ bounds: selected, workArea: area, size, scaleFactor: 1 }))
      .toEqual({ x: -510, y: 124 });
  });
  it("uses the cursor when selection geometry is missing", () => {
    expect(popupPosition({ cursor: { x: 500, y: 300 }, workArea, size, scaleFactor: 1 }))
      .toEqual({ x: 484, y: 286 });
    expect(popupPosition({ workArea, size, scaleFactor: 1 })).toBeNull();
  });
});
