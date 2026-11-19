//! A column chart drawn in the app's ordered-dither style: the same 2px
//! Bayer cells as the usage meter. Each column is solid at its floor and
//! dissolves toward its value, capped by one solid row; empty columns show
//! only a sparse dotted floor. Columns stack segments in their own colors.
use crate::dither::threshold;
use crate::tooltip::TooltipExt as _;
use gpui::{prelude::*, *};

const CELL: f32 = 2.0;

/// One column: its stacked segments from the floor up, and the text its
/// tooltip and accessibility label read.
#[derive(Clone, Debug, Default)]
pub struct Column {
    pub segments: Vec<(f64, Hsla)>,
    pub label: SharedString,
}
impl Column {
    pub fn total(&self) -> f64 {
        self.segments.iter().map(|(v, _)| v.max(0.0)).sum()
    }
}

/// How one cell of a column paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnCell {
    /// The top row of a filled column.
    Cap,
    On,
    Faint,
    /// The dotted floor of an empty column.
    Floor,
    Empty,
}

/// Rows of `rows` a column filled to `fraction` of the chart lights. Any
/// value at all lights one row so small days never read as empty.
pub fn lit_rows(fraction: f64, rows: usize) -> usize {
    (fraction.clamp(0.0, 1.0) * rows as f64).ceil() as usize
}

/// Cell `(x, r)` of a column lit to `lit` rows, `r` counted up from the floor.
pub fn column_cell(x: usize, r: usize, lit: usize) -> ColumnCell {
    if lit == 0 {
        return if r == 0 && threshold(x, r) < 0.55 {
            ColumnCell::Floor
        } else {
            ColumnCell::Empty
        };
    }
    if r >= lit {
        return ColumnCell::Empty;
    }
    if r + 1 == lit {
        return ColumnCell::Cap;
    }
    // Dense at the floor, thinning toward the value.
    let depth = 1.0 - r as f32 / lit as f32;
    if 0.3 + 0.7 * depth > threshold(x, r) {
        ColumnCell::On
    } else {
        ColumnCell::Faint
    }
}

fn paint_column(
    bounds: Bounds<Pixels>,
    segments: &[(f64, Hsla)],
    max: f64,
    floor: Hsla,
    window: &mut Window,
) {
    let scale = window.scale_factor().max(1.0);
    let snap = |value: Pixels| px((f32::from(value) * scale).round() / scale);
    let columns = (f32::from(bounds.size.width) / CELL) as usize;
    let rows = (f32::from(bounds.size.height) / CELL) as usize;
    if columns == 0 || rows == 0 {
        return;
    }
    let left = snap(bounds.origin.x);
    let bottom = snap(bounds.origin.y + bounds.size.height);
    let total: f64 = segments.iter().map(|(v, _)| v.max(0.0)).sum();
    let lit = lit_rows(if max > 0.0 { total / max } else { 0.0 }, rows);
    // The row where each segment ends, counted from the floor.
    let mut ends = Vec::with_capacity(segments.len());
    let mut sum = 0.0;
    for (value, color) in segments {
        sum += value.max(0.0);
        let end = if total > 0.0 {
            (sum / total * lit as f64).round() as usize
        } else {
            0
        };
        ends.push((end, *color));
    }
    for r in 0..rows {
        let color = ends
            .iter()
            .find(|(end, _)| r < *end)
            .or(ends.last())
            .map(|(_, c)| *c)
            .unwrap_or(floor);
        for x in 0..columns {
            let fill = match column_cell(x, r, lit) {
                ColumnCell::Cap | ColumnCell::On => color,
                ColumnCell::Faint => color.opacity(0.22),
                ColumnCell::Floor => floor,
                ColumnCell::Empty => continue,
            };
            let at = point(
                left + px(x as f32 * CELL),
                bottom - px((r + 1) as f32 * CELL),
            );
            window.paint_quad(gpui::fill(Bounds::new(at, size(px(CELL), px(CELL))), fill));
        }
    }
}

/// A row of dithered columns `height` tall, scaled so `max` fills it. Each
/// column shows its label on hover; `hover` tints the hovered column's slot.
pub fn columns(
    id: &'static str,
    data: Vec<Column>,
    max: f64,
    height: f32,
    floor: Hsla,
    hover: Hsla,
) -> Div {
    let mut row = div()
        .flex()
        .items_end()
        .gap(px(CELL))
        .h(px(height))
        .w_full();
    for (i, column) in data.into_iter().enumerate() {
        let segments = column.segments.clone();
        row = row.child(
            div()
                .id((id, i))
                .role(Role::Image)
                .aria_label(column.label.clone())
                .relative()
                .flex_1()
                .min_w(px(CELL))
                .max_w(px(28.))
                .h_full()
                .rounded(px(2.))
                .hover(move |v| v.bg(hover))
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            paint_column(bounds, &segments, max, floor, window)
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
                .tip(column.label),
        );
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_light_to_their_value_and_cap_the_top() {
        assert_eq!(lit_rows(0.0, 60), 0);
        assert_eq!(lit_rows(0.001, 60), 1);
        assert_eq!(lit_rows(0.5, 60), 30);
        assert_eq!(lit_rows(3.0, 60), 60);
        for x in 0..8 {
            assert_eq!(column_cell(x, 29, 30), ColumnCell::Cap);
            assert_eq!(column_cell(x, 30, 30), ColumnCell::Empty);
            assert!(matches!(
                column_cell(x, 3, 30),
                ColumnCell::On | ColumnCell::Faint
            ));
            assert_ne!(column_cell(x, 1, 0), ColumnCell::Floor);
        }
        // Denser at the floor than near the value.
        let on = |r0: usize, r1: usize| {
            (0..8)
                .flat_map(|x| (r0..r1).map(move |r| column_cell(x, r, 40)))
                .filter(|c| *c == ColumnCell::On)
                .count()
        };
        assert!(on(0, 4) > on(32, 36));
        // An empty column is a sparse dotted floor.
        let floor = (0..8)
            .filter(|x| column_cell(*x, 0, 0) == ColumnCell::Floor)
            .count();
        assert!(floor > 0 && floor < 8);
    }
}
