//! Cascading side submenus: a menu row with children opens its submenu
//! beside the menu, aligned to the row, on whichever side has room, and kept
//! inside the window. Any depth nests; each level shows at most one submenu.
//!
//! The host view keeps one [`Submenus`] per menu and hands out a
//! [`SubmenuAccess`] to reach it. Rows are wrapped by a [`SubmenuLevel`]:
//! [`SubmenuLevel::item`] for ordinary rows, [`SubmenuLevel::trigger`] for a
//! row that owns a submenu. Pointer intent (opening on hover, the grace while
//! the pointer travels diagonally into an open submenu, clicks) is handled by
//! the wrappers; keys go through [`Submenus::key`], and the host's outside
//! dismissal asks [`Submenus::contains`] before closing.
//!
//! ```ignore
//! let menu = SubmenuLevel::root("cloud-menu", |shell: &mut Shell| &mut shell.cloud_menu);
//! let panel = self.cloud_menu.is_open(0, "performance").then(|| {
//!     let child = menu.child();
//!     card.child(child.item(0, cpu_row, cx)).into_any_element()
//! });
//! rows.push(menu.trigger(0, "performance", performance_row, panel, cx));
//! ```

use super::{CARD_INSET, CARD_RADIUS, HoverAction, HoverIntent, menu_motion};
use crate::icons::{self, icon};
use crate::theme::Theme;
use gpui::{
    AnyElement, App, Bounds, Context, Div, Edges, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, Position, SharedString, Size,
    Stateful, Style, Window, div, point, prelude::*, px,
};
use std::rc::Rc;

/// The gap between a menu's edge and its submenu.
pub const SUBMENU_GAP: f32 = 4.0;
/// How close a submenu may come to the window's edges.
pub const SUBMENU_WINDOW_MARGIN: f32 = 8.0;

/// Which side of its menu a submenu opened on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SubmenuSide {
    #[default]
    Right,
    Left,
}

/// Where a submenu of `size` goes beside its trigger row. `parent_inset` is
/// the distance from the row to its menu's edge, so the submenu clears the
/// whole menu rather than overlapping its padding. Prefers the right; flips
/// left when the right would leave the window; with no room either side it
/// takes the roomier one. Its first row lines up with the trigger, sliding
/// up only as far as the window's bottom needs.
pub fn submenu_placement(
    trigger: Bounds<Pixels>,
    size: Size<Pixels>,
    viewport: Size<Pixels>,
    parent_inset: Pixels,
) -> (Point<Pixels>, SubmenuSide) {
    let margin = px(SUBMENU_WINDOW_MARGIN);
    let offset = parent_inset + px(SUBMENU_GAP);
    let right = trigger.right() + offset;
    let left = trigger.left() - offset - size.width;
    let fits_right = right + size.width <= viewport.width - margin;
    let fits_left = left >= margin;
    let side = if fits_right {
        SubmenuSide::Right
    } else if fits_left {
        SubmenuSide::Left
    } else if viewport.width - trigger.right() >= trigger.left() {
        SubmenuSide::Right
    } else {
        SubmenuSide::Left
    };
    let x = match side {
        SubmenuSide::Right => right,
        SubmenuSide::Left => left,
    };
    let max_x = (viewport.width - margin - size.width).max(margin);
    let max_y = (viewport.height - margin - size.height).max(margin);
    // The submenu card's own inset puts its first row level with the trigger.
    let y = trigger.top() - px(CARD_INSET);
    (point(x.clamp(margin, max_x), y.clamp(margin, max_y)), side)
}

/// What a row of a menu is, for keyboard navigation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmenuRow {
    /// An ordinary row: Enter activates it.
    Item,
    /// A row that opens the submenu with this key.
    Submenu(SharedString),
}

/// The keys a cascading menu handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmenuKey {
    Up,
    Down,
    Home,
    End,
    /// Opens the highlighted row's submenu.
    Right,
    /// Closes the focused submenu, back to its parent.
    Left,
    Enter,
    Escape,
}

impl SubmenuKey {
    pub fn from_keystroke(key: &str) -> Option<Self> {
        Some(match key {
            "up" => Self::Up,
            "down" => Self::Down,
            "home" => Self::Home,
            "end" => Self::End,
            "right" => Self::Right,
            "left" => Self::Left,
            "enter" | "space" => Self::Enter,
            "escape" => Self::Escape,
            _ => return None,
        })
    }
}

/// What a key did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmenuOutcome {
    /// The menus changed (or the key was swallowed); repaint.
    Handled,
    /// Run the row at `index` of the menu reached by `path` (submenu keys
    /// from the root; empty for the root menu).
    Activate {
        path: Vec<SharedString>,
        index: usize,
    },
    /// Escape on the root menu: close everything.
    Dismiss,
}

#[derive(Clone, Debug)]
struct OpenSubmenu {
    key: SharedString,
    /// The index of the row that opened it, in its parent menu.
    trigger: usize,
    /// Where it last painted, for the hover corridor and outside presses.
    panel: Option<Bounds<Pixels>>,
    side: SubmenuSide,
}

/// The open submenus of one menu, its keyboard highlight, and pointer intent.
/// Depth 0 is the root menu; `open[d]` is the submenu opened from depth `d`.
#[derive(Default)]
pub struct Submenus {
    open: Vec<OpenSubmenu>,
    highlight: Vec<Option<usize>>,
    focus: usize,
    intents: Vec<HoverIntent<usize>>,
}

impl Submenus {
    /// The key of the submenu opened from the menu at `depth`.
    pub fn open_key(&self, depth: usize) -> Option<&SharedString> {
        self.open.get(depth).map(|open| &open.key)
    }

    pub fn is_open(&self, depth: usize, key: &str) -> bool {
        self.open_key(depth)
            .is_some_and(|open| open.as_ref() == key)
    }

    pub fn any_open(&self) -> bool {
        !self.open.is_empty()
    }

    /// The submenu keys from the root to the deepest open submenu.
    pub fn path(&self) -> Vec<SharedString> {
        self.open.iter().map(|open| open.key.clone()).collect()
    }

    /// The menu that receives keys.
    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn highlighted(&self, depth: usize) -> Option<usize> {
        self.highlight.get(depth).copied().flatten()
    }

    pub fn is_highlighted(&self, depth: usize, index: usize) -> bool {
        self.highlighted(depth) == Some(index)
    }

    /// Which side the submenu opened from `depth` is on.
    pub fn side(&self, depth: usize) -> Option<SubmenuSide> {
        self.open.get(depth).map(|open| open.side)
    }

    /// Whether `point` falls on any open submenu: hosts skip their outside
    /// dismissal for presses there.
    pub fn contains(&self, point: Point<Pixels>) -> bool {
        self.open
            .iter()
            .filter_map(|open| open.panel)
            .any(|panel| panel.contains(&point))
    }

    fn set_highlight(&mut self, depth: usize, index: Option<usize>) {
        if self.highlight.len() <= depth {
            self.highlight.resize(depth + 1, None);
        }
        self.highlight[depth] = index;
    }

    fn intent(&mut self, depth: usize) -> &mut HoverIntent<usize> {
        if self.intents.len() <= depth {
            self.intents.resize_with(depth + 1, HoverIntent::default);
        }
        &mut self.intents[depth]
    }

    /// Open the submenu `key` from row `trigger` of the menu at `depth`,
    /// closing whatever that menu had open.
    pub fn open(&mut self, depth: usize, trigger: usize, key: impl Into<SharedString>) {
        let key = key.into();
        let depth = depth.min(self.open.len());
        let same = self
            .open
            .get(depth)
            .is_some_and(|open| open.key == key && open.trigger == trigger);
        if !same {
            self.close_from(depth);
            self.open.push(OpenSubmenu {
                key,
                trigger,
                panel: None,
                side: SubmenuSide::Right,
            });
        }
        self.set_highlight(depth, Some(trigger));
    }

    /// Close the submenu opened from `depth`, and everything under it.
    pub fn close_from(&mut self, depth: usize) {
        self.open.truncate(depth);
        self.highlight.truncate(depth + 1);
        self.focus = self.focus.min(depth);
        for intent in self.intents.iter_mut().skip(depth) {
            intent.cancel();
        }
    }

    /// Close every submenu and forget the highlight: the menu itself closed.
    pub fn close_all(&mut self) {
        self.open.clear();
        self.highlight.clear();
        self.focus = 0;
        for intent in &mut self.intents {
            intent.reset();
        }
    }

    fn placed(&mut self, depth: usize, panel: Bounds<Pixels>, side: SubmenuSide) {
        if let Some(open) = self.open.get_mut(depth) {
            open.panel = Some(panel);
            open.side = side;
        }
    }

    /// Apply a key to the focused menu. `rows` describes the menu reached by
    /// a path of submenu keys (empty for the root).
    pub fn key(
        &mut self,
        key: SubmenuKey,
        rows: impl Fn(&[SharedString]) -> Vec<SubmenuRow>,
    ) -> SubmenuOutcome {
        let depth = self.focus.min(self.open.len());
        self.focus = depth;
        let path: Vec<SharedString> = self.open[..depth]
            .iter()
            .map(|open| open.key.clone())
            .collect();
        let menu = rows(&path);
        let step = |this: &mut Self, delta: isize| {
            let next = super::menu_step(this.highlighted(depth), menu.len(), delta);
            this.set_highlight(depth, next);
        };
        match key {
            SubmenuKey::Up => step(self, -1),
            SubmenuKey::Down => step(self, 1),
            SubmenuKey::Home => self.set_highlight(depth, (!menu.is_empty()).then_some(0)),
            SubmenuKey::End => self.set_highlight(depth, menu.len().checked_sub(1)),
            SubmenuKey::Right | SubmenuKey::Enter => {
                let Some(index) = self.highlighted(depth).filter(|ix| *ix < menu.len()) else {
                    if key == SubmenuKey::Enter {
                        return SubmenuOutcome::Handled;
                    }
                    step(self, 1);
                    return SubmenuOutcome::Handled;
                };
                match &menu[index] {
                    SubmenuRow::Submenu(child) => {
                        self.open(depth, index, child.clone());
                        let mut child_path = path;
                        child_path.push(child.clone());
                        let first = (!rows(&child_path).is_empty()).then_some(0);
                        self.focus = depth + 1;
                        self.set_highlight(depth + 1, first);
                    }
                    SubmenuRow::Item if key == SubmenuKey::Enter => {
                        return SubmenuOutcome::Activate { path, index };
                    }
                    SubmenuRow::Item => {}
                }
            }
            SubmenuKey::Left | SubmenuKey::Escape => {
                if self.open.len() > depth {
                    // A submenu the pointer opened, not yet focused.
                    self.close_from(depth);
                } else if depth > 0 {
                    self.close_from(depth - 1);
                    self.focus = depth - 1;
                } else if key == SubmenuKey::Escape {
                    self.close_all();
                    return SubmenuOutcome::Dismiss;
                }
            }
        }
        SubmenuOutcome::Handled
    }
}

/// How a host reaches its [`Submenus`].
pub type SubmenuAccess<T> = fn(&mut T) -> &mut Submenus;

/// One menu of a cascade: wraps its rows so they open, close and highlight
/// submenus. Derive child levels with [`Self::child`].
pub struct SubmenuLevel<T: 'static> {
    id: SharedString,
    depth: usize,
    access: SubmenuAccess<T>,
    radius: f32,
    parent_inset: f32,
}

impl<T: 'static> Clone for SubmenuLevel<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            ..*self
        }
    }
}

impl<T: 'static> SubmenuLevel<T> {
    pub fn root(id: impl Into<SharedString>, access: SubmenuAccess<T>) -> Self {
        Self {
            id: id.into(),
            depth: 0,
            access,
            radius: CARD_RADIUS,
            parent_inset: CARD_INSET,
        }
    }

    /// The level of the submenus this level's triggers open.
    pub fn child(&self) -> Self {
        Self {
            depth: self.depth + 1,
            ..self.clone()
        }
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    /// The corner radius of submenu cards (default [`CARD_RADIUS`]).
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// The padding between this menu's rows and its card's edge (default
    /// [`CARD_INSET`]), which a submenu clears.
    pub fn parent_inset(mut self, inset: f32) -> Self {
        self.parent_inset = inset;
        self
    }

    fn row_id(&self, index: usize) -> ElementId {
        ElementId::Name(format!("{}-{}-{index}", self.id, self.depth).into())
    }

    /// Pointer intent over row `index`: highlight it, and open its submenu
    /// (or close a sibling's) once the pointer settles rather than while it
    /// heads diagonally into an open submenu.
    fn hover(
        access: SubmenuAccess<T>,
        depth: usize,
        index: usize,
        key: Option<SharedString>,
        pointer: Point<Pixels>,
        moved: bool,
        this: &mut T,
        cx: &mut Context<T>,
    ) {
        let menus = access(this);
        let changed = menus.focus != depth || !menus.is_highlighted(depth, index);
        menus.focus = depth;
        menus.set_highlight(depth, Some(index));
        let open = menus.open.get(depth);
        let current = open.map(|open| open.trigger);
        let bounds = open.and_then(|open| open.panel);
        let left = open.is_some_and(|open| open.side == SubmenuSide::Left);
        let intent = menus.intent(depth);
        let action = if moved {
            intent.moved(current.as_ref(), &index, pointer, bounds, left)
        } else {
            intent.enter(current.as_ref(), &index, pointer, bounds, left)
        };
        let apply = move |menus: &mut Submenus| {
            match &key {
                Some(key) => menus.open(depth, index, key.clone()),
                None => menus.close_from(depth),
            }
            menus.intent(depth).record_origin(pointer);
        };
        match action {
            HoverAction::None if changed => cx.notify(),
            HoverAction::None => {}
            HoverAction::Open => {
                apply(menus);
                cx.notify();
            }
            HoverAction::Defer => {
                if changed {
                    cx.notify();
                }
                access(this).intent(depth).defer(cx, move |this, cx| {
                    let menus = access(this);
                    if menus.open.len() > depth && menus.intent(depth).pending() == Some(&index) {
                        menus.intent(depth).cancel();
                        apply(menus);
                        cx.notify();
                    }
                });
            }
        }
    }

    fn tracked(
        &self,
        index: usize,
        key: Option<SharedString>,
        row: impl IntoElement,
        cx: &mut Context<T>,
    ) -> Stateful<Div> {
        let (access, depth) = (self.access, self.depth);
        let enter_key = key.clone();
        div()
            .id(self.row_id(index))
            .w_full()
            .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                if *hovered {
                    let pointer = window.mouse_position();
                    Self::hover(
                        access,
                        depth,
                        index,
                        enter_key.clone(),
                        pointer,
                        false,
                        this,
                        cx,
                    );
                } else {
                    access(this).intent(depth).leave(&index);
                }
            }))
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    Self::hover(
                        access,
                        depth,
                        index,
                        key.clone(),
                        event.position,
                        true,
                        this,
                        cx,
                    );
                }),
            )
            .child(row)
    }

    /// An ordinary row at `index` of this menu. The row keeps its own click.
    pub fn item(&self, index: usize, row: impl IntoElement, cx: &mut Context<T>) -> Stateful<Div> {
        self.tracked(index, None, row, cx)
    }

    /// A row at `index` that opens submenu `key`. `panel` is the submenu's
    /// card, rendered by the caller only while open (see
    /// [`Submenus::is_open`]); it is placed beside this menu. End the row
    /// with [`submenu_chevron`].
    pub fn trigger(
        &self,
        index: usize,
        key: impl Into<SharedString>,
        row: impl IntoElement,
        panel: Option<AnyElement>,
        cx: &mut Context<T>,
    ) -> Stateful<Div> {
        let key = key.into();
        let (access, depth) = (self.access, self.depth);
        let click_key = key.clone();
        let mut wrapper = self
            .tracked(index, Some(key.clone()), row, cx)
            .relative()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                let menus = access(this);
                menus.intent(depth).reset();
                menus.focus = depth;
                // Hover has usually opened it already; a click keeps it
                // open rather than toggling it shut under the pointer.
                menus.open(depth, index, click_key.clone());
                cx.notify();
                cx.stop_propagation();
            }));
        if let Some(panel) = panel {
            let entity = cx.entity().downgrade();
            let on_placed: Rc<dyn Fn(Bounds<Pixels>, SubmenuSide, &mut App)> =
                Rc::new(move |bounds, side, cx| {
                    let _ = entity.update(cx, |this, _| access(this).placed(depth, bounds, side));
                });
            let card = crate::frost::floating_glass(self.radius, crate::frost::MENU_BLUR, panel);
            let content = menu_motion(
                SharedString::from(format!("{}-{}-{key}-submenu", self.id, depth)),
                None,
                div()
                    .occlude()
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .child(card),
            );
            wrapper = wrapper.child(
                gpui::deferred(SubmenuLayer {
                    content: div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .child(content)
                        .into_any_element(),
                    parent_inset: px(self.parent_inset),
                    on_placed,
                })
                .priority(2 + depth),
            );
        }
        wrapper
    }
}

/// The trailing › of a row that opens a submenu.
pub fn submenu_chevron(theme: &Theme) -> AnyElement {
    icon(icons::ALT_ARROW_RIGHT)
        .size(px(12.0))
        .flex_none()
        .text_color(theme.text_muted)
        .into_any_element()
}

/// Lays its content out over the trigger row, then moves it beside the row
/// by [`submenu_placement`] once its size is known.
struct SubmenuLayer {
    content: AnyElement,
    parent_inset: Pixels,
    on_placed: Rc<dyn Fn(Bounds<Pixels>, SubmenuSide, &mut App)>,
}

impl IntoElement for SubmenuLayer {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for SubmenuLayer {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let child = self.content.request_layout(window, cx);
        let zero: gpui::Length = px(0.0).into();
        let layout = window.request_layout(
            Style {
                position: Position::Absolute,
                inset: Edges {
                    top: zero,
                    right: zero,
                    bottom: zero,
                    left: zero,
                },
                ..Style::default()
            },
            [child],
            cx,
        );
        (layout, child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let laid_out = window.layout_bounds(*child);
        let (origin, side) = submenu_placement(
            bounds,
            laid_out.size,
            window.viewport_size(),
            self.parent_inset,
        );
        (self.on_placed)(Bounds::new(origin, laid_out.size), side, cx);
        window.with_element_offset(origin - laid_out.origin, |window| {
            self.content.prepaint(window, cx)
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut LayoutId,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    fn path(menus: &Submenus) -> Vec<String> {
        menus.path().iter().map(|key| key.to_string()).collect()
    }

    fn row(x: f32, y: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(240.0), px(32.0)))
    }

    #[test]
    fn opens_right_aligned_to_the_row() {
        let (origin, side) = submenu_placement(
            row(100.0, 200.0),
            size(px(220.0), px(120.0)),
            size(px(1200.0), px(800.0)),
            px(6.0),
        );
        assert_eq!(side, SubmenuSide::Right);
        assert_eq!(origin.x, px(340.0 + 6.0 + SUBMENU_GAP));
        assert_eq!(origin.y, px(200.0 - CARD_INSET));
    }

    #[test]
    fn flips_left_near_the_right_edge() {
        let (origin, side) = submenu_placement(
            row(900.0, 200.0),
            size(px(220.0), px(120.0)),
            size(px(1200.0), px(800.0)),
            px(6.0),
        );
        assert_eq!(side, SubmenuSide::Left);
        assert_eq!(origin.x, px(900.0 - 6.0 - SUBMENU_GAP - 220.0));
    }

    #[test]
    fn stays_inside_the_window() {
        let viewport = size(px(600.0), px(400.0));
        let panel = size(px(260.0), px(300.0));
        // No room either side: the roomier side, clamped to the margin.
        let (origin, side) = submenu_placement(row(180.0, 20.0), panel, viewport, px(6.0));
        assert_eq!(side, SubmenuSide::Right);
        assert_eq!(origin.x, px(600.0 - SUBMENU_WINDOW_MARGIN - 260.0));
        // Near the bottom it slides up; near the top it holds the margin.
        let (origin, _) = submenu_placement(row(20.0, 360.0), panel, viewport, px(6.0));
        assert_eq!(origin.y, px(400.0 - SUBMENU_WINDOW_MARGIN - 300.0));
        let (origin, _) = submenu_placement(row(20.0, 0.0), panel, viewport, px(6.0));
        assert_eq!(origin.y, px(SUBMENU_WINDOW_MARGIN));
        // Taller than the window: pinned to the top margin.
        let (origin, _) = submenu_placement(
            row(20.0, 100.0),
            size(px(200.0), px(500.0)),
            viewport,
            px(6.0),
        );
        assert_eq!(origin.y, px(SUBMENU_WINDOW_MARGIN));
    }

    /// Root: Performance › (CPU, Memory › (Used, Free)), Sleep, Details.
    fn rows(path: &[SharedString]) -> Vec<SubmenuRow> {
        match path.iter().map(|key| key.as_ref()).collect::<Vec<_>>()[..] {
            [] => vec![
                SubmenuRow::Submenu("performance".into()),
                SubmenuRow::Item,
                SubmenuRow::Item,
            ],
            ["performance"] => vec![SubmenuRow::Item, SubmenuRow::Submenu("memory".into())],
            ["performance", "memory"] => vec![SubmenuRow::Item, SubmenuRow::Item],
            _ => Vec::new(),
        }
    }

    #[test]
    fn arrows_walk_into_and_out_of_nested_submenus() {
        let mut menus = Submenus::default();
        assert_eq!(menus.key(SubmenuKey::Down, rows), SubmenuOutcome::Handled);
        assert_eq!(menus.highlighted(0), Some(0));
        menus.key(SubmenuKey::Up, rows);
        assert_eq!(menus.highlighted(0), Some(2), "wraps");
        menus.key(SubmenuKey::Down, rows);
        // → opens Performance and focuses its first row.
        menus.key(SubmenuKey::Right, rows);
        assert!(menus.is_open(0, "performance"));
        assert_eq!((menus.focus(), menus.highlighted(1)), (1, Some(0)));
        // ↓ then Enter opens Memory, a level deeper.
        menus.key(SubmenuKey::Down, rows);
        menus.key(SubmenuKey::Enter, rows);
        assert_eq!(path(&menus), ["performance", "memory"]);
        assert_eq!(menus.focus(), 2);
        menus.key(SubmenuKey::Down, rows);
        assert_eq!(
            menus.key(SubmenuKey::Enter, rows),
            SubmenuOutcome::Activate {
                path: vec!["performance".into(), "memory".into()],
                index: 1,
            }
        );
        // ← closes back to the parent, keeping its trigger highlighted.
        menus.key(SubmenuKey::Left, rows);
        assert_eq!(path(&menus), ["performance"]);
        assert_eq!((menus.focus(), menus.highlighted(1)), (1, Some(1)));
        // Esc closes a level at a time, then everything.
        assert_eq!(menus.key(SubmenuKey::Escape, rows), SubmenuOutcome::Handled);
        assert!(!menus.any_open());
        assert_eq!((menus.focus(), menus.highlighted(0)), (0, Some(0)));
        assert_eq!(menus.key(SubmenuKey::Escape, rows), SubmenuOutcome::Dismiss);
        assert_eq!(menus.highlighted(0), None);
    }

    #[test]
    fn enter_on_root_items_activates_and_one_submenu_per_level() {
        let mut menus = Submenus::default();
        menus.key(SubmenuKey::End, rows);
        assert_eq!(
            menus.key(SubmenuKey::Enter, rows),
            SubmenuOutcome::Activate {
                path: Vec::new(),
                index: 2,
            }
        );
        // → on an item does nothing.
        menus.key(SubmenuKey::Right, rows);
        assert!(!menus.any_open());
        // Opening a sibling at a level replaces that level and below.
        menus.open(0, 0, "performance");
        menus.open(1, 1, "memory");
        menus.open(0, 1, "other");
        assert_eq!(path(&menus), ["other"]);
        // A submenu opened by the pointer closes on ← from its parent.
        menus.close_all();
        menus.open(0, 0, "performance");
        assert_eq!(menus.focus(), 0);
        menus.key(SubmenuKey::Left, rows);
        assert!(!menus.any_open());
    }

    #[test]
    fn presses_on_a_placed_submenu_are_inside() {
        let mut menus = Submenus::default();
        menus.open(0, 0, "performance");
        let panel = Bounds::new(point(px(400.0), px(100.0)), size(px(200.0), px(120.0)));
        menus.placed(0, panel, SubmenuSide::Left);
        assert_eq!(menus.side(0), Some(SubmenuSide::Left));
        assert!(menus.contains(panel.center()));
        assert!(!menus.contains(point(px(10.0), px(10.0))));
        menus.close_all();
        assert!(!menus.contains(panel.center()));
    }

    #[test]
    fn keys_map_from_keystrokes() {
        assert_eq!(SubmenuKey::from_keystroke("right"), Some(SubmenuKey::Right));
        assert_eq!(
            SubmenuKey::from_keystroke("escape"),
            Some(SubmenuKey::Escape)
        );
        assert_eq!(SubmenuKey::from_keystroke("a"), None);
    }
}
