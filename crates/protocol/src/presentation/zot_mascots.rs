//! Zapper avatars, shared by every client: a round, flat-coloured face.
//!
//! Each mascot picks a [`Face`]; each colour fills the disc. Mascot and colour
//! ids are saved with each Zapper, so art and names can change but ids cannot.
use crate::zots::ZotAvatar;

/// The features drawn on an avatar's disc.
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

pub struct Mascot {
    pub id: &'static str,
    pub name: &'static str,
    pub face: Face,
}

pub struct MascotColor {
    pub id: &'static str,
    pub body: u32,
    pub shade: u32,
}

pub const MASCOTS: &[Mascot] = &[
    Mascot {
        id: "bolt",
        name: "Sunny",
        face: Face::Smile,
    },
    Mascot {
        id: "ace",
        name: "Pip",
        face: Face::Tall,
    },
    Mascot {
        id: "brawler",
        name: "Visor",
        face: Face::Visor,
    },
    Mascot {
        id: "hex",
        name: "Cyclo",
        face: Face::Cyclops,
    },
    Mascot {
        id: "squire",
        name: "Beam",
        face: Face::Happy,
    },
    Mascot {
        id: "shade",
        name: "Shades",
        face: Face::Shades,
    },
    Mascot {
        id: "rogue",
        name: "Wink",
        face: Face::Wink,
    },
    Mascot {
        id: "viking",
        name: "Doze",
        face: Face::Sleepy,
    },
    Mascot {
        id: "bones",
        name: "Rosy",
        face: Face::Blush,
    },
];

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
}
