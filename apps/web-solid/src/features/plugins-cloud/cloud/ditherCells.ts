// Ordered-dither cells for meters and columns (crates/viewport/src/dither.rs `meter_cell`, crates/ui/src/dither_bar.rs).
// Pure, so the cell rules are testable without a canvas.

/** crates/viewport/src/dither.rs BAYER_4, read row-major. */
const BAYER_4 = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
/** One dither cell, in CSS pixels. */
export const CELL = 2;

/** The ordered-dither threshold of cell `(x, y)`, in `0..1`. A cell lights when its density exceeds this. */
export const threshold = (x: number, y: number) => (BAYER_4[(y & 3) * 4 + (x & 3)] ?? 0) / 16 + 0.5 / 16;

/** How many of `columns` a fraction lights. Any use at all lights one, so a fresh period never reads as unreported. */
export const litColumns = (fraction: number, columns: number) => Math.ceil(Math.max(0, Math.min(1, fraction || 0)) * columns);

export type Cell = "on" | "faint" | "track" | "empty";

/** Cell `(x, y)` of a meter whose first `lit` columns are filled: denser toward the leading edge, sparse dust past it. */
export function meterCell(x: number, y: number, lit: number): Cell {
  const cutoff = threshold(x, y);
  if (x < lit) return 0.5 + (0.5 * (x + 1)) / lit > cutoff ? "on" : "faint";
  return cutoff < 0.3 ? "track" : "empty";
}

/**
 * Cell `(x, y)` of a column `rows` tall (y counted from the bottom) filled `height` rows: the top row is solid,
 * the body thickens toward the top, and an empty column keeps a dotted floor.
 */
export function columnCell(x: number, y: number, height: number): Cell {
  if (y < height) {
    if (y === height - 1) return "on";
    return 0.45 + (0.55 * (y + 1)) / height > threshold(x, y) ? "on" : "faint";
  }
  return y === 0 && threshold(x, y) < 0.3 ? "track" : "empty";
}

/** Rows a value fills out of `rows`, with any value above zero showing at least one. */
export const filledRows = (value: number, max: number, rows: number) => (value > 0 && max > 0 ? Math.max(1, Math.round((Math.min(value, max) / max) * rows)) : 0);
