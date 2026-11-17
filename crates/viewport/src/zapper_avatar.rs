//! Profile pictures for Zappers and the people working with them.
//!
//! A Zapper is a flat colour disc with a simple face, drawn as vectors so it
//! stays crisp from a 14px sidebar row to the 64px settings preview. People
//! get a quiet monogram disc; an unassigned slot is a dashed ring.
use gpui::{prelude::*, *};
use zuse_protocol::presentation::zot_mascots::{self, Face};
use zuse_protocol::zots::ZotAvatar;

const INK: u32 = 0x161616;

/// A Zapper's face, `size` logical pixels across.
pub fn zapper_avatar(avatar: &ZotAvatar, size: f32) -> Div {
    let (mascot, color) = zot_mascots::resolve(avatar);
    let (face, body, shade) = (mascot.face, color.body, color.shade);
    div().size(px(size)).flex_none().child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| paint(face, body, shade, bounds, window),
        )
        .size(px(size)),
    )
}

/// A person: their first initial on a neutral disc.
pub fn person_avatar(name: &str, size: f32) -> Div {
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".into());
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(0x3a3a3a))
        .text_color(rgb(0xe8e8e8))
        .text_size(px((size * 0.5).max(7.)))
        .line_height(px(size))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initial)
}

/// The empty assignee slot.
pub fn unassigned_avatar(size: f32) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .border_1()
        .border_dashed()
        .border_color(rgb(0x5a5a5a))
}

fn paint(face: Face, body: u32, shade: u32, bounds: Bounds<Pixels>, window: &mut Window) {
    let s = f32::from(bounds.size.width.min(bounds.size.height));
    let o = bounds.origin;
    let at = |x: f32, y: f32| point(o.x + px(x * s), o.y + px(y * s));
    let disc = |window: &mut Window, x: f32, y: f32, r: f32, color: Hsla| {
        window.paint_quad(
            fill(
                Bounds::new(at(x - r, y - r), size(px(2. * r * s), px(2. * r * s))),
                color,
            )
            .corner_radii(px(r * s)),
        );
    };
    let pill = |window: &mut Window, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Hsla| {
        window.paint_quad(
            fill(Bounds::new(at(x, y), size(px(w * s), px(h * s))), color)
                .corner_radii(px(radius * s)),
        );
    };
    // Degrees clockwise from 3 o'clock, so 90 points down.
    let arc = |window: &mut Window, x: f32, y: f32, r: f32, from: f32, to: f32, width: f32| {
        let steps = 14;
        let mut path = PathBuilder::stroke(px((width * s).max(1.)));
        for i in 0..=steps {
            let t = (from + (to - from) * i as f32 / steps as f32).to_radians();
            let p = at(x + r * t.cos(), y + r * t.sin());
            if i == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, rgb(INK));
        }
    };
    let ink: Hsla = rgb(INK).into();
    let body_c: Hsla = rgb(body).into();
    disc(window, 0.5, 0.5, 0.5, body_c);
    match face {
        Face::Smile => {
            disc(window, 0.37, 0.44, 0.072, ink);
            disc(window, 0.63, 0.44, 0.072, ink);
            arc(window, 0.5, 0.53, 0.14, 25., 155., 0.065);
        }
        Face::Tall => {
            pill(window, 0.325, 0.36, 0.11, 0.22, 0.055, ink);
            pill(window, 0.565, 0.36, 0.11, 0.22, 0.055, ink);
        }
        Face::Visor => {
            pill(window, 0.2, 0.37, 0.6, 0.19, 0.095, ink);
            disc(window, 0.37, 0.465, 0.042, body_c);
            disc(window, 0.63, 0.465, 0.042, body_c);
        }
        Face::Cyclops => {
            disc(window, 0.5, 0.46, 0.19, ink);
            disc(window, 0.555, 0.405, 0.06, body_c);
        }
        Face::Happy => {
            arc(window, 0.36, 0.49, 0.085, 200., 340., 0.07);
            arc(window, 0.64, 0.49, 0.085, 200., 340., 0.07);
            arc(window, 0.5, 0.57, 0.08, 25., 155., 0.06);
        }
        Face::Shades => {
            pill(window, 0.235, 0.38, 0.235, 0.15, 0.05, ink);
            pill(window, 0.53, 0.38, 0.235, 0.15, 0.05, ink);
            pill(window, 0.44, 0.405, 0.12, 0.04, 0.02, ink);
            pill(window, 0.42, 0.65, 0.16, 0.045, 0.0225, ink);
        }
        Face::Wink => {
            disc(window, 0.37, 0.44, 0.072, ink);
            pill(window, 0.55, 0.425, 0.16, 0.05, 0.025, ink);
            arc(window, 0.53, 0.53, 0.12, 30., 120., 0.065);
        }
        Face::Sleepy => {
            arc(window, 0.36, 0.42, 0.085, 20., 160., 0.065);
            arc(window, 0.64, 0.42, 0.085, 20., 160., 0.065);
            disc(window, 0.5, 0.65, 0.05, ink);
        }
        Face::Blush => {
            // Pink cheeks read on every colour except pink, where the shade does.
            let cheek: Hsla = if body == 0xff8fbf {
                rgb(shade).into()
            } else {
                Hsla::from(rgb(0xff6b8b)).opacity(0.55)
            };
            disc(window, 0.27, 0.58, 0.075, cheek);
            disc(window, 0.73, 0.58, 0.075, cheek);
            disc(window, 0.37, 0.45, 0.068, ink);
            disc(window, 0.63, 0.45, 0.068, ink);
            disc(window, 0.5, 0.6, 0.042, ink);
        }
    }
}
