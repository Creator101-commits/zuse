//! Zapper avatars, shared by every client: a cast of retro pixel robots.
//!
//! Each robot is a [`Design`]: one soft body and at most one feature (an
//! antenna, ear bolts), built from rounded plates sampled at cell centres onto a 16x16 grid and filled
//! in one flat colour with white eyes on a transparent ground. Character
//! comes from the silhouette and the eyes, which a pose ([`Frame`]) can
//! blink, glance sideways or settle with a squash; antenna tips flash.
//! Mascot and colour ids are saved with each Zapper, so art and names can
//! change but ids cannot; every id saved by an earlier version still names
//! one of these.
use crate::zots::ZotAvatar;

/// The flat round face an earlier version drew for each mascot. The Slack
/// persona renderer still draws these until it learns the sprites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    /// Round eyes and a small smile.
    Smile,
    /// Tall pill eyes, no mouth.
    Tall,
    /// One wide visor with two glints.
    Visor,
    /// A single large eye.
    Cyclops,
    /// Closed, smiling eyes.
    Happy,
    /// Square shades and a flat mouth.
    Shades,
    /// One open eye, one closed, a sideways smirk.
    Wink,
    /// Closed, resting eyes and a small round mouth.
    Sleepy,
    /// Round eyes, rosy cheeks and a button mouth.
    Blush,
}

/// Grid side, in pixels.
pub const SPRITE: usize = 16;

/// A rounded rectangle in grid units: left, top, right, bottom and corner
/// radius. Cell `(x, y)` is filled when its centre is inside. Edges sit on
/// whole units so silhouettes stay symmetric and free of stray pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plate(pub f32, pub f32, pub f32, pub f32, pub f32);

impl Plate {
    fn contains(self, x: f32, y: f32) -> bool {
        let Plate(x0, y0, x1, y1, r) = self;
        // Corner radii may exceed half a side; `clamp` would panic.
        let cx = x.max(x0 + r).min(x1 - r);
        let cy = y.max(y0 + r).min(y1 - r);
        (x0..=x1).contains(&x)
            && (y0..=y1).contains(&y)
            && (x - cx).powi(2) + (y - cy).powi(2) <= r * r + 1e-4
    }
}

/// What a plate is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The robot itself, in its colour.
    Body,
    /// An antenna lamp that flashes white now and then.
    Tip,
}

/// One robot: plates in paint order, its eyes as `(x, y, w, h)` cell
/// rectangles, and the row a squash folds away (everything above it drops
/// one pixel, so the robot settles).
pub struct Design {
    pub parts: &'static [(Plate, Part)],
    pub eyes: &'static [(i8, i8, i8, i8)],
    pub waist: u8,
}

pub struct Mascot {
    pub id: &'static str,
    pub name: &'static str,
    /// One line for tooltips and accessibility.
    pub about: &'static str,
    pub face: Face,
    pub design: Design,
}

use Part::*;

pub const MASCOTS: &[Mascot] = &[
    Mascot {
        id: "bolt",
        name: "Sprocket",
        about: "Square body under a single antenna bulb",
        face: Face::Smile,
        design: Design {
            parts: &[
                (Plate(7., 2., 9., 4., 0.), Body),
                (Plate(6., 0., 10., 2., 0.9), Tip),
                (Plate(2., 4., 14., 15., 3.), Body),
            ],
            eyes: &[(5, 7, 2, 3), (9, 7, 2, 3)],
            waist: 13,
        },
    },
    Mascot {
        id: "ace",
        name: "Dynamo",
        about: "Wide body with twin antenna lamps",
        face: Face::Tall,
        design: Design {
            parts: &[
                (Plate(3., 3., 4., 6., 0.), Body),
                (Plate(12., 3., 13., 6., 0.), Body),
                (Plate(2., 1., 5., 3., 0.9), Tip),
                (Plate(11., 1., 14., 3., 0.9), Tip),
                (Plate(1., 5., 15., 15., 3.5), Body),
            ],
            eyes: &[(4, 8, 2, 3), (10, 8, 2, 3)],
            waist: 13,
        },
    },
    Mascot {
        id: "brawler",
        name: "Visor",
        about: "Rounded block with one wide visor",
        face: Face::Visor,
        design: Design {
            parts: &[
                (Plate(7., 1., 9., 4., 0.), Body),
                (Plate(2., 3., 14., 15., 3.5), Body),
            ],
            eyes: &[(4, 7, 8, 2)],
            waist: 13,
        },
    },
    Mascot {
        id: "hex",
        name: "Cyclo",
        about: "Round hull with one big eye",
        face: Face::Cyclops,
        design: Design {
            parts: &[
                (Plate(7., 2., 9., 4., 0.), Body),
                (Plate(6., 0., 10., 2., 0.9), Tip),
                (Plate(2., 3., 14., 15., 6.), Body),
            ],
            eyes: &[(6, 7, 4, 4)],
            waist: 13,
        },
    },
    Mascot {
        id: "squire",
        name: "Beam",
        about: "Rounded head between two ear bolts",
        face: Face::Happy,
        design: Design {
            parts: &[
                (Plate(1., 7., 3., 12., 0.), Body),
                (Plate(13., 7., 15., 12., 0.), Body),
                (Plate(3., 4., 13., 15., 3.5), Body),
            ],
            eyes: &[(5, 7, 2, 3), (9, 7, 2, 3)],
            waist: 13,
        },
    },
    Mascot {
        id: "shade",
        name: "Shades",
        about: "Boxy body with a side antenna",
        face: Face::Shades,
        design: Design {
            parts: &[
                (Plate(11., 2., 12., 4., 0.), Body),
                (Plate(10., 0., 13., 2., 0.9), Tip),
                (Plate(2., 4., 14., 15., 1.5), Body),
            ],
            eyes: &[(4, 8, 3, 2), (9, 8, 3, 2)],
            waist: 13,
        },
    },
    Mascot {
        id: "rogue",
        name: "Wink",
        about: "Odd-sized eyes and an off-centre antenna",
        face: Face::Wink,
        design: Design {
            parts: &[
                (Plate(4., 2., 5., 4., 0.), Body),
                (Plate(3., 0., 6., 2., 0.9), Tip),
                (Plate(2., 4., 14., 15., 3.), Body),
            ],
            eyes: &[(5, 7, 2, 3), (9, 8, 3, 2)],
            waist: 13,
        },
    },
    Mascot {
        id: "viking",
        name: "Doze",
        about: "Low, wide and half asleep",
        face: Face::Sleepy,
        design: Design {
            parts: &[(Plate(1., 6., 15., 15., 4.), Body)],
            eyes: &[(4, 10, 3, 1), (9, 10, 3, 1)],
            waist: 13,
        },
    },
    Mascot {
        id: "bones",
        name: "Rosy",
        about: "Round dome under an antenna bulb",
        face: Face::Blush,
        design: Design {
            parts: &[
                (Plate(7., 2., 9., 4., 0.), Body),
                (Plate(6., 0., 10., 2., 0.9), Tip),
                (Plate(2., 3., 14., 15., 6.), Body),
            ],
            eyes: &[(5, 7, 2, 3), (9, 7, 2, 3)],
            waist: 13,
        },
    },
];

pub struct MascotColor {
    pub id: &'static str,
    pub body: u32,
    pub shade: u32,
}

pub const COLORS: &[MascotColor] = &[
    MascotColor {
        id: "coral",
        body: 0xef6b5f,
        shade: 0xc94a45,
    },
    MascotColor {
        id: "yellow",
        body: 0xf6c94a,
        shade: 0xe0a92c,
    },
    MascotColor {
        id: "green",
        body: 0x7fd05a,
        shade: 0x5aae3c,
    },
    MascotColor {
        id: "lime",
        body: 0xc3e45a,
        shade: 0x9ec43a,
    },
    MascotColor {
        id: "violet",
        body: 0xa98bf5,
        shade: 0x8263dc,
    },
    MascotColor {
        id: "teal",
        body: 0x5fd0c0,
        shade: 0x36ab9c,
    },
    MascotColor {
        id: "blue",
        body: 0x6ea8ff,
        shade: 0x4b82e0,
    },
    MascotColor {
        id: "pink",
        body: 0xff8fbf,
        shade: 0xe2669c,
    },
];

fn hash(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// A stable avatar for a seed, so Zots saved without one keep their look.
pub fn default_avatar(seed: &str) -> ZotAvatar {
    let value = hash(seed);
    ZotAvatar {
        mascot: MASCOTS[value as usize % MASCOTS.len()].id.into(),
        color: COLORS[(value >> 8) as usize % COLORS.len()].id.into(),
    }
}

/// The mascot and colour an avatar draws, unknown ids falling back to the
/// first of each so a Zapper saved by a newer client still has a face.
pub fn resolve(avatar: &ZotAvatar) -> (&'static Mascot, &'static MascotColor) {
    let mascot = MASCOTS
        .iter()
        .find(|m| m.id == avatar.mascot)
        .unwrap_or(&MASCOTS[0]);
    let color = COLORS
        .iter()
        .find(|c| c.id == avatar.color)
        .unwrap_or(&COLORS[0]);
    (mascot, color)
}

/// The colours one robot is drawn in, as `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub body: u32,
    pub eye: u32,
}

pub fn palette(color: &MascotColor) -> Palette {
    Palette {
        body: color.body,
        eye: 0xffffff,
    }
}

/// The idle pose at one instant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frame {
    pub blink: bool,
    /// Eyes glance this many cells sideways, where the body allows.
    pub gaze: i8,
    /// Antenna tips light up.
    pub flash: bool,
    /// The head settles one pixel onto the body.
    pub squash: bool,
}

/// Animation changes land on this grid, so a screen of avatars wakes the
/// renderer at most a few times a second.
pub const FRAME_MS: u64 = 200;
/// Idle beats, in frames. Each cycle places its event at a pseudo-random
/// point, so blinks come every few seconds without a metronome feel.
const BLINK_CYCLE: u64 = 30;
const GLANCE_CYCLE: u64 = 55;
const SETTLE_CYCLE: u64 = 70;
const FLASH_CYCLE: u64 = 23;

fn beat(id: &str, salt: u64, cycle: u64) -> u64 {
    u64::from(hash(&format!("{id}:{salt}:{cycle}")))
}

/// The pose of mascot `id` at `ms` since the shared animation epoch. Every
/// robot runs its own pseudo-random schedule, so a list of them never moves
/// in unison.
pub fn frame_at(id: &str, ms: u64) -> Frame {
    let step = ms / FRAME_MS + u64::from(hash(id) % 997);
    let at = |cycle_len: u64, salt: u64, span: u64| {
        let cycle = step / cycle_len;
        let start = beat(id, salt, cycle) % (cycle_len - span);
        (step % cycle_len).checked_sub(start).filter(|d| *d < span)
    };
    let gaze = match at(GLANCE_CYCLE, 2, 7) {
        Some(_) if beat(id, 3, step / GLANCE_CYCLE).is_multiple_of(2) => 1,
        Some(_) => -1,
        None => 0,
    };
    Frame {
        blink: at(BLINK_CYCLE, 1, 1).is_some(),
        gaze,
        flash: at(FLASH_CYCLE, 4, 2).is_some(),
        squash: at(SETTLE_CYCLE, 5, 2).is_some(),
    }
}

/// When the pose of `id` next changes, in ms since the epoch. `squash`
/// says whether settling is drawn at this size. Never earlier than the next
/// frame boundary after `ms`.
pub fn next_change(id: &str, ms: u64, squash: bool) -> u64 {
    let now = frame_at(id, ms);
    let mut at = (ms / FRAME_MS + 1) * FRAME_MS;
    loop {
        let next = frame_at(id, at);
        if next.blink != now.blink
            || next.gaze != now.gaze
            || next.flash != now.flash
            || (squash && next.squash != now.squash)
        {
            return at;
        }
        at += FRAME_MS;
    }
}

/// One robot drawn in `palette` and posed as `frame`: `0xRRGGBB` per cell,
/// `None` where it is transparent.
pub type Pixels = [[Option<u32>; SPRITE]; SPRITE];

fn layout(design: &Design) -> [[Option<Part>; SPRITE]; SPRITE] {
    let mut grid = [[None; SPRITE]; SPRITE];
    for (shape, part) in design.parts {
        for (y, row) in grid.iter_mut().enumerate() {
            for (x, cell) in row.iter_mut().enumerate() {
                if shape.contains(x as f32 + 0.5, y as f32 + 0.5) {
                    *cell = Some(*part);
                }
            }
        }
    }
    // A lone pixel left by sampling reads as dirt, not detail.
    let lone: Vec<(usize, usize)> = (0..SPRITE)
        .flat_map(|y| (0..SPRITE).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            grid[y][x].is_some()
                && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|(dx, dy)| {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    nx < 0
                        || ny < 0
                        || nx >= SPRITE as isize
                        || ny >= SPRITE as isize
                        || grid[ny as usize][nx as usize].is_none()
                })
        })
        .collect();
    for (x, y) in lone {
        grid[y][x] = None;
    }
    grid
}

pub fn pixels(mascot: &Mascot, palette: &Palette, frame: Frame) -> Pixels {
    let grid = layout(&mascot.design);
    let body = |x: isize, y: isize| -> bool {
        x >= 0
            && y >= 0
            && x < SPRITE as isize
            && y < SPRITE as isize
            && grid[y as usize][x as usize] == Some(Body)
    };
    let mut out: Pixels = [[None; SPRITE]; SPRITE];
    for (y, row) in out.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            *cell = grid[y][x].map(|part| match part {
                Tip if frame.flash => palette.eye,
                _ => palette.body,
            });
        }
    }
    for &(ex, ey, w, h) in mascot.design.eyes {
        let (ex, ey, w, h) = (ex as isize, ey as isize, w as isize, h as isize);
        // Glance only if the whole eye stays inside the body with a margin.
        let fits = |dx: isize| {
            (ey - 1..ey + h + 1).all(|y| (ex + dx - 1..ex + dx + w + 1).all(|x| body(x, y)))
        };
        let dx = match frame.gaze as isize {
            0 => 0,
            gaze if fits(gaze) => gaze,
            _ => 0,
        };
        for y in ey..ey + h {
            // Closed eyes keep only their bottom row; a slit shuts.
            if frame.blink && (h == 1 || y != ey + h - 1) {
                continue;
            }
            for x in ex + dx..ex + dx + w {
                if body(x, y) {
                    out[y as usize][x as usize] = Some(palette.eye);
                }
            }
        }
    }
    if frame.squash {
        let waist = mascot.design.waist as usize;
        out.copy_within(0..waist, 1);
        out[0] = [None; SPRITE];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mascot_ids_are_unique() {
        let mut ids: Vec<_> = MASCOTS.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MASCOTS.len());
    }

    #[test]
    fn defaults_are_stable_and_unknown_ids_still_draw() {
        assert_eq!(default_avatar("zot"), default_avatar("zot"));
        let unknown = ZotAvatar {
            mascot: "gone".into(),
            color: "gone".into(),
        };
        let (mascot, color) = resolve(&unknown);
        assert_eq!((mascot.id, color.id), (MASCOTS[0].id, COLORS[0].id));
    }

    /// Every mascot id an earlier version could save, and the robot it now
    /// draws. Ids are durable; this list may only grow.
    const LEGACY: [(&str, &str); 9] = [
        ("bolt", "Sprocket"),
        ("ace", "Dynamo"),
        ("brawler", "Visor"),
        ("hex", "Cyclo"),
        ("squire", "Beam"),
        ("shade", "Shades"),
        ("rogue", "Wink"),
        ("viking", "Doze"),
        ("bones", "Rosy"),
    ];

    #[test]
    fn every_saved_mascot_id_maps_to_its_robot() {
        for (id, name) in LEGACY {
            let avatar = ZotAvatar {
                mascot: id.into(),
                color: "teal".into(),
            };
            assert_eq!(resolve(&avatar).0.name, name, "{id}");
        }
        for color in [
            "coral", "yellow", "green", "lime", "violet", "teal", "blue", "pink",
        ] {
            assert!(COLORS.iter().any(|c| c.id == color), "{color}");
        }
    }

    #[test]
    fn robots_fill_their_grid_have_faces_and_differ() {
        let palette = palette(&COLORS[5]);
        let mut seen = std::collections::HashSet::new();
        for mascot in MASCOTS {
            assert!(!mascot.about.is_empty());
            let still = pixels(mascot, &palette, Frame::default());
            assert!(seen.insert(still), "{} repeats another robot", mascot.id);
            let filled = still.iter().flatten().filter(|c| c.is_some()).count();
            assert!(
                filled > SPRITE * SPRITE * 2 / 5,
                "{} is too small",
                mascot.id
            );
            // Two flat tones: plating and lit eyes.
            for color in still.iter().flatten().flatten() {
                assert!([palette.body, palette.eye].contains(color), "{}", mascot.id);
            }
            let grid = layout(&mascot.design);
            for &(x, y, w, h) in mascot.design.eyes {
                for yy in y..y + h {
                    for xx in x..x + w {
                        assert_eq!(
                            grid[yy as usize][xx as usize],
                            Some(Part::Body),
                            "{}",
                            mascot.id
                        );
                    }
                }
            }
            // Sampling leaves no stray single pixels.
            for y in 0..SPRITE {
                for x in 0..SPRITE {
                    if grid[y][x].is_some() {
                        let near =
                            [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
                                .iter()
                                .any(|(dx, dy)| {
                                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                                    (0..SPRITE as i32).contains(&nx)
                                        && (0..SPRITE as i32).contains(&ny)
                                        && grid[ny as usize][nx as usize].is_some()
                                });
                        assert!(near, "{} has a lone pixel at {x},{y}", mascot.id);
                    }
                }
            }
        }
    }

    #[test]
    fn idle_loop_blinks_glances_flashes_and_settles_on_a_coarse_grid() {
        let palette = palette(&COLORS[0]);
        for mascot in MASCOTS {
            let frames: Vec<_> = (0..600)
                .map(|i| frame_at(mascot.id, i * FRAME_MS))
                .collect();
            assert!(frames.iter().any(|f| f.blink));
            assert!(frames.iter().any(|f| f.gaze == 1) && frames.iter().any(|f| f.gaze == -1));
            assert!(frames.iter().any(|f| f.flash) && frames.iter().any(|f| f.squash));
            // A blink is one frame, every few seconds.
            let blinks = frames.iter().filter(|f| f.blink).count();
            assert!((10..=30).contains(&blinks), "{}: {blinks}", mascot.id);
            assert!(frames.windows(2).all(|w| !(w[0].blink && w[1].blink)));
            // Most of the time it simply rests.
            let busy = frames.iter().filter(|f| **f != Frame::default()).count();
            assert!(busy < frames.len() / 3, "{} fidgets", mascot.id);
            for ms in [0, 1, 199, 200, 12_345] {
                let next = next_change(mascot.id, ms, true);
                assert!(next > ms && next % FRAME_MS == 0);
                assert!(next_change(mascot.id, ms, false) >= next);
            }
            let open = Frame::default();
            let shut = Frame {
                blink: true,
                ..open
            };
            assert_ne!(
                pixels(mascot, &palette, open),
                pixels(mascot, &palette, shut),
                "{}",
                mascot.id
            );
            // Settling drops everything above the waist by one pixel.
            let settled = pixels(
                mascot,
                &palette,
                Frame {
                    squash: true,
                    ..open
                },
            );
            let still = pixels(mascot, &palette, open);
            let waist = mascot.design.waist as usize;
            assert!(settled[0].iter().all(Option::is_none));
            assert_eq!(settled[1..=waist], still[..waist]);
            assert_eq!(settled[waist + 1..], still[waist + 1..]);
            let flash = pixels(
                mascot,
                &palette,
                Frame {
                    flash: true,
                    ..open
                },
            );
            let tips = mascot
                .design
                .parts
                .iter()
                .any(|(_, part)| *part == Part::Tip);
            assert_eq!(flash != still, tips, "{}", mascot.id);
        }
        assert!(MASCOTS.iter().any(|m| {
            pixels(m, &palette, Frame::default())
                != pixels(
                    m,
                    &palette,
                    Frame {
                        gaze: 1,
                        ..Frame::default()
                    },
                )
        }));
        // Each robot keeps its own schedule.
        let blinks = |id: &str| -> Vec<bool> {
            (0..200).map(|i| frame_at(id, i * FRAME_MS).blink).collect()
        };
        assert!(
            MASCOTS
                .iter()
                .all(|m| m.id == MASCOTS[0].id || blinks(m.id) != blinks(MASCOTS[0].id))
        );
    }

    #[test]
    fn configs_saved_before_custom_images_still_load() {
        use crate::zots::{ZotConfig, avatar_image_ok};
        let old = r#"{"name":"Ops","purpose":"","homeDeviceId":"d","repositories":[],
            "coordinatorModel":{"harness":"claude-code","model":null},"workerDeviceId":"d",
            "maxConcurrentWorkers":4,"maxFollowups":3,"avatar":{"mascot":"hex","color":"pink"}}"#;
        let config: ZotConfig = serde_json::from_str(old).unwrap();
        assert_eq!(config.avatar_image, None);
        assert_eq!(resolve(config.avatar.as_ref().unwrap()).0.name, "Cyclo");
        // Without an image the saved shape is unchanged, so older clients
        // and engines read what newer ones write.
        let saved = serde_json::to_value(&config).unwrap();
        assert!(saved.get("avatarImage").is_none());
        assert_eq!(
            saved["avatar"],
            serde_json::json!({"mascot":"hex","color":"pink"})
        );
        let image = "data:image/png;base64,iVBORw0KGgo=";
        let with = ZotConfig {
            avatar_image: Some(image.into()),
            ..config
        };
        let text = serde_json::to_string(&with).unwrap();
        assert!(text.contains(r#""avatarImage":"data:image/png;base64,iVBORw0KGgo=""#));
        assert_eq!(serde_json::from_str::<ZotConfig>(&text).unwrap(), with);
        // The avatar itself never gains fields, so personas stay small.
        let avatar: ZotAvatar =
            serde_json::from_str(r#"{"mascot":"bolt","color":"teal","future":1}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&avatar).unwrap(),
            r#"{"mascot":"bolt","color":"teal"}"#
        );

        assert!(avatar_image_ok(image));
        assert!(avatar_image_ok("data:image/jpeg;base64,/9j/4AAQ"));
        for bad in [
            "",
            "data:image/png;base64,",
            "data:image/svg+xml;base64,PHN2Zz4=",
            "https://example.com/a.png",
            "data:image/png;base64,abc\"<x>",
        ] {
            assert!(!avatar_image_ok(bad), "{bad}");
        }
        let huge = format!(
            "data:image/png;base64,{}",
            "A".repeat(crate::zots::AVATAR_IMAGE_MAX_LEN)
        );
        assert!(!avatar_image_ok(&huge));
    }
}
