//! Profile pictures for Zappers and the people working with them.
//!
//! A Zapper is either the user's own picture or one of the retro pixel robots
//! in [`zot_mascots`], drawn on a transparent ground as runs of flat quads so
//! it stays crisp from a 14px sidebar row to the 64px settings preview.
//! Robots idle (a blink, a sideways glance, an antenna flash, a one-pixel
//! settle at larger sizes) on a coarse shared clock that only
//! re-renders the views painting them, never every frame, and holds still
//! under reduced motion. People get a quiet monogram disc; an unassigned slot
//! is a dashed ring.
use base64::Engine as _;
use gpui::{prelude::*, *};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use zuse_protocol::presentation::zot_mascots::{self, Frame, SPRITE};
use zuse_protocol::zots::{AVATAR_IMAGE_MAX_LEN, Zot, ZotAvatar, avatar_image_ok};

/// Everything needed to draw one Zapper: its mascot, which is also the
/// fallback, and the user's picture when it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZapperFace {
    pub avatar: ZotAvatar,
    pub image: Option<SharedString>,
}

impl ZapperFace {
    pub fn mascot(avatar: ZotAvatar) -> Self {
        Self {
            avatar,
            image: None,
        }
    }

    pub fn of(zot: &Zot) -> Self {
        Self {
            avatar: zot
                .config
                .avatar
                .clone()
                .unwrap_or_else(|| zot_mascots::default_avatar(&zot.id)),
            image: zot.config.avatar_image.clone().map(SharedString::from),
        }
    }
}

/// A Zapper's mascot, `size` logical pixels across.
pub fn zapper_avatar(avatar: &ZotAvatar, size: f32) -> Div {
    zapper_face(&ZapperFace::mascot(avatar.clone()), size)
}

/// A Zapper's picture, `size` logical pixels across: its own image when it
/// has a readable one, otherwise its robot.
pub fn zapper_face(face: &ZapperFace, size: f32) -> Div {
    let radius = px(size * 0.2);
    if let Some(image) = face.image.as_deref().and_then(decoded) {
        return div().size(px(size)).flex_none().child(
            img(image)
                .size(px(size))
                .rounded(radius)
                .object_fit(ObjectFit::Cover),
        );
    }
    let (mascot, color) = zot_mascots::resolve(&face.avatar);
    let palette = zot_mascots::palette(color);
    let id = mascot.id;
    // A one-pixel settle is too jumpy at sidebar size.
    let squash = size >= 24.;
    div().size(px(size)).flex_none().child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let frame = if cx.reduce_motion() {
                    Frame::default()
                } else {
                    let ms = clock_ms(cx);
                    let next = zot_mascots::next_change(id, ms, squash);
                    wake(window.current_view(), next, cx);
                    let frame = zot_mascots::frame_at(id, ms);
                    Frame {
                        squash: squash && frame.squash,
                        ..frame
                    }
                };
                paint_robot(
                    &zot_mascots::pixels(mascot, &palette, frame),
                    bounds,
                    window,
                );
            },
        )
        .size(px(size)),
    )
}

/// Remember the pictures of these Zappers, so surfaces that only know a
/// Zapper's id (a message's sender line) can show its picture too.
pub fn remember_zots<'a>(zots: impl IntoIterator<Item = &'a Zot>) {
    KNOWN.with_borrow_mut(|known| {
        for zot in zots {
            match &zot.config.avatar_image {
                Some(image) => {
                    known.insert(zot.id.clone(), SharedString::from(image.clone()));
                }
                None => {
                    known.remove(&zot.id);
                }
            }
        }
    });
}

/// The face for a Zapper known by id, preferring its remembered picture.
pub fn known_face(zot_id: &str, avatar: &ZotAvatar) -> ZapperFace {
    ZapperFace {
        avatar: avatar.clone(),
        image: KNOWN.with_borrow(|known| known.get(zot_id).cloned()),
    }
}

/// Largest picture accepted before decoding.
const UPLOAD_MAX_BYTES: usize = 25 * 1024 * 1024;
/// Side of the stored square: sharp at 64 logical pixels on a 2x screen.
pub const AVATAR_IMAGE_SIDE: u32 = 128;

/// Turn an uploaded picture into a stored avatar: centre-cropped to a square,
/// scaled down to [`AVATAR_IMAGE_SIDE`], and encoded as a small PNG, or as a
/// JPEG when a photo would make the PNG too large. Returns a `data:` URL.
pub fn prepare_image(bytes: &[u8]) -> Result<String, &'static str> {
    if bytes.len() > UPLOAD_MAX_BYTES {
        return Err("That image is too large. Pick one under 25 MB.");
    }
    let source = image::load_from_memory(bytes)
        .map_err(|_| "Zuse couldn't read that image. Try a PNG or JPEG.")?;
    let (width, height) = (source.width(), source.height());
    let side = width.min(height);
    if side == 0 {
        return Err("That image is empty.");
    }
    let square = source.crop_imm((width - side) / 2, (height - side) / 2, side, side);
    let target = side.min(AVATAR_IMAGE_SIDE);
    let square = if side > target {
        square.resize_exact(target, target, image::imageops::FilterType::Lanczos3)
    } else {
        square
    };
    let url = |mime: &str, data: &[u8]| {
        format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(data)
        )
    };
    let mut png = Vec::new();
    square
        .to_rgba8()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|_| "Zuse couldn't prepare that image.")?;
    let png = url("image/png", &png);
    // Pictures with transparency or flat colour stay PNG; photos are far
    // smaller as JPEG.
    if png.len() <= AVATAR_IMAGE_MAX_LEN / 2 {
        return Ok(png);
    }
    let rgb = image::DynamicImage::ImageRgba8(flatten(square.to_rgba8())).to_rgb8();
    for quality in [88, 75, 60, 45] {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality)
            .encode_image(&rgb)
            .map_err(|_| "Zuse couldn't prepare that image.")?;
        let jpeg = url("image/jpeg", &jpeg);
        if avatar_image_ok(&jpeg) {
            return Ok(jpeg);
        }
    }
    if avatar_image_ok(&png) {
        Ok(png)
    } else {
        Err("That image is too detailed to use as a picture.")
    }
}

/// Composite transparent pixels over the dark avatar tile, since JPEG has
/// no alpha.
fn flatten(mut rgba: image::RgbaImage) -> image::RgbaImage {
    for pixel in rgba.pixels_mut() {
        let alpha = f32::from(pixel[3]) / 255.;
        for (channel, back) in pixel.0.iter_mut().take(3).zip([0x14u8, 0x14, 0x14]) {
            *channel = (f32::from(*channel) * alpha + f32::from(back) * (1. - alpha)).round() as u8;
        }
        pixel[3] = 255;
    }
    rgba
}

thread_local! {
    static KNOWN: RefCell<HashMap<String, SharedString>> = RefCell::new(HashMap::new());
    static DECODED: RefCell<HashMap<u64, Option<Arc<Image>>>> = RefCell::new(HashMap::new());
}

/// The picture behind a stored `data:` URL, decoded once per distinct value.
fn decoded(url: &str) -> Option<Arc<Image>> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    let key = hasher.finish();
    DECODED.with_borrow_mut(|cache| {
        if let Some(image) = cache.get(&key) {
            return image.clone();
        }
        // A handful of Zappers is normal; never let edits grow this forever.
        if cache.len() >= 64 {
            cache.clear();
        }
        let image = decode_url(url).map(Arc::new);
        cache.insert(key, image.clone());
        image
    })
}

fn decode_url(url: &str) -> Option<Image> {
    if !avatar_image_ok(url) {
        return None;
    }
    let (format, data) = if let Some(data) = url.strip_prefix("data:image/png;base64,") {
        (ImageFormat::Png, data)
    } else {
        (
            ImageFormat::Jpeg,
            url.strip_prefix("data:image/jpeg;base64,")?,
        )
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    Some(Image::from_bytes(format, bytes))
}

/// The idle clock: one shared epoch, and one timer that wakes only the views
/// whose robots change pose next. Views drop off when they stop painting a
/// robot, so a hidden sidebar costs nothing.
struct AvatarClock {
    epoch: Instant,
    wakes: HashMap<EntityId, u64>,
    timer: Option<u64>,
    generation: u64,
}

impl Global for AvatarClock {}

impl Default for AvatarClock {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            wakes: HashMap::new(),
            timer: None,
            generation: 0,
        }
    }
}

fn clock_ms(cx: &mut App) -> u64 {
    cx.default_global::<AvatarClock>()
        .epoch
        .elapsed()
        .as_millis() as u64
}

fn wake(view: EntityId, at: u64, cx: &mut App) {
    let clock = cx.default_global::<AvatarClock>();
    let due = clock.wakes.entry(view).or_insert(at);
    *due = (*due).min(at);
    if clock.timer.is_some_and(|timer| timer <= at) {
        return;
    }
    arm(at, cx);
}

fn arm(at: u64, cx: &mut App) {
    let clock = cx.default_global::<AvatarClock>();
    clock.timer = Some(at);
    clock.generation = clock.generation.wrapping_add(1);
    let generation = clock.generation;
    let delay = at.saturating_sub(clock.epoch.elapsed().as_millis() as u64);
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(delay))
            .await;
        cx.update(|cx| fire(generation, cx));
    })
    .detach();
}

fn fire(generation: u64, cx: &mut App) {
    let clock = cx.default_global::<AvatarClock>();
    if clock.generation != generation {
        return;
    }
    clock.timer = None;
    // Timers can land a hair early; a view due within a few ms goes now.
    let now = clock.epoch.elapsed().as_millis() as u64 + 4;
    let mut due = Vec::new();
    clock.wakes.retain(|view, at| {
        let ready = *at <= now;
        if ready {
            due.push(*view);
        }
        !ready
    });
    let next = clock.wakes.values().min().copied();
    for view in due {
        cx.notify(view);
    }
    if let Some(next) = next {
        arm(next, cx);
    }
}

fn paint_robot(pixels: &zot_mascots::Pixels, bounds: Bounds<Pixels>, window: &mut Window) {
    let side = f32::from(bounds.size.width.min(bounds.size.height));
    let cell = side / SPRITE as f32;
    let origin = bounds.origin;
    // Same-coloured neighbours in a row become one quad: a robot is about
    // sixty quads rather than two hundred.
    for (y, row) in pixels.iter().enumerate() {
        let mut x = 0;
        while x < SPRITE {
            let Some(color) = row[x] else {
                x += 1;
                continue;
            };
            let start = x;
            while x < SPRITE && row[x] == Some(color) {
                x += 1;
            }
            window.paint_quad(fill(
                Bounds::new(
                    point(
                        origin.x + px(start as f32 * cell),
                        origin.y + px(y as f32 * cell),
                    ),
                    size(px((x - start) as f32 * cell), px(cell)),
                ),
                rgb(color),
            ));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui::*` brings its own `test` attribute; these are plain tests.
    use std::prelude::rust_2024::test;

    fn picture(width: u32, height: u32, noisy: bool) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| {
            if noisy {
                let n = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) as u8;
                image::Rgba([n, n.wrapping_mul(7), n.wrapping_mul(13), 255])
            } else if x < width / 2 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 0])
            }
        });
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    fn side_of(url: &str) -> (u32, u32) {
        let image = decode_url(url).expect("stored picture decodes");
        let decoded = image::load_from_memory(&image.bytes).unwrap();
        (decoded.width(), decoded.height())
    }

    #[test]
    fn uploads_are_cropped_square_downscaled_and_small() {
        let flat = prepare_image(&picture(900, 300, false)).unwrap();
        assert!(flat.starts_with("data:image/png;base64,"));
        assert_eq!(side_of(&flat), (AVATAR_IMAGE_SIDE, AVATAR_IMAGE_SIDE));

        let photo = prepare_image(&picture(200, 150, true)).unwrap();
        assert!(
            photo.starts_with("data:image/jpeg;base64,"),
            "noisy photos become JPEG"
        );
        assert!(avatar_image_ok(&photo));
        assert_eq!(side_of(&photo), (AVATAR_IMAGE_SIDE, AVATAR_IMAGE_SIDE));

        // Small pictures are cropped but never scaled up.
        let tiny = prepare_image(&picture(40, 64, false)).unwrap();
        assert_eq!(side_of(&tiny), (40, 40));

        assert!(prepare_image(b"not an image").is_err());
    }

    #[test]
    fn faces_prefer_a_readable_picture_and_fall_back_to_the_mascot() {
        let avatar = ZotAvatar {
            mascot: "hex".into(),
            color: "teal".into(),
        };
        assert!(decoded("data:image/png;base64,???").is_none());
        let url = prepare_image(&picture(64, 64, false)).unwrap();
        assert!(decoded(&url).is_some());

        let mut zot: Zot = serde_json::from_str(
            r#"{"id": "z1", "revision": 1,
            "config": {"name": "Ops", "purpose": "", "homeDeviceId": "d", "repositories": [],
                "coordinatorModel": {"harness": "claude-code", "model": null},
                "workerDeviceId": "d", "maxConcurrentWorkers": 4, "maxFollowups": 3,
                "avatar": {"mascot": "hex", "color": "teal"}},
            "status": "active", "coordinatorChatId": "c", "previousCoordinatorChats": [],
            "workers": [], "decisions": []}"#,
        )
        .unwrap();
        assert_eq!(ZapperFace::of(&zot), ZapperFace::mascot(avatar.clone()));
        zot.config.avatar_image = Some(url.clone());
        remember_zots([&zot]);
        assert_eq!(
            known_face("z1", &avatar).image.as_deref(),
            Some(url.as_str())
        );
        assert_eq!(ZapperFace::of(&zot).image.as_deref(), Some(url.as_str()));
        zot.config.avatar_image = None;
        remember_zots([&zot]);
        assert_eq!(known_face("z1", &avatar).image, None);
    }
}
