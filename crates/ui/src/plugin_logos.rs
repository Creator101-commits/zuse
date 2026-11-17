//! Plugin logos where only a plugin id is at hand: `@Linear` chips in the
//! composer and transcript, and plugin tool rows. The id resolves to the
//! domain its logo is stored under: first from plugins the `@` menu has
//! offered (the live catalog), then from the bundled catalog. Anything else
//! keeps the generic plug glyph.
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use gpui::{ImageFormat, SharedString};

/// Plugins seen in the live catalog this session: id → (name, domain).
static KNOWN: LazyLock<RwLock<HashMap<String, (String, String)>>> = LazyLock::new(Default::default);

/// Remember the live catalog's plugins, so chips for plugins the bundled
/// catalog doesn't list still find their logo.
pub(crate) fn remember(plugins: &[crate::composer::MentionPlugin]) {
    let mut known = KNOWN.write().unwrap();
    for plugin in plugins {
        known.insert(
            plugin.id.clone(),
            (plugin.name.clone(), plugin.domain.clone()),
        );
    }
}

/// The name the live catalog gave `plugin_id`, if it was seen.
pub(crate) fn known_name(plugin_id: &str) -> Option<String> {
    KNOWN
        .read()
        .unwrap()
        .get(plugin_id)
        .map(|(name, _)| name.clone())
        .filter(|name| !name.trim().is_empty())
}

/// The domain `plugin_id`'s logo is stored under.
pub(crate) fn domain(plugin_id: &str) -> Option<String> {
    let remembered = KNOWN
        .read()
        .unwrap()
        .get(plugin_id)
        .map(|(_, domain)| domain.clone())
        .filter(|domain| !domain.is_empty());
    remembered.or_else(|| zuse_protocol::plugin_tools::bundled_domain(plugin_id).map(Into::into))
}

/// The asset path of `plugin_id`'s logo, when the app ships one.
pub(crate) fn logo_path(plugin_id: &str) -> Option<SharedString> {
    let path = format!("plugin-icons/{}", domain(plugin_id)?);
    zuse_icons::load_plugin_icon(&path).map(|_| path.into())
}

/// `plugin_id`'s logo as an image to paint straight into a text layout,
/// where no `img` element can sit. One image per logo, so it decodes once.
pub(crate) fn logo_image(plugin_id: &str) -> Option<Arc<gpui::Image>> {
    static CACHE: LazyLock<Mutex<HashMap<SharedString, Arc<gpui::Image>>>> =
        LazyLock::new(Default::default);
    let path = logo_path(plugin_id)?;
    let mut cache = CACHE.lock().unwrap();
    if let Some(image) = cache.get(&path) {
        return Some(image.clone());
    }
    let bytes = zuse_icons::load_plugin_icon(&path)?;
    let image = Arc::new(gpui::Image::from_bytes(image_format(bytes), bytes.to_vec()));
    cache.insert(path, image.clone());
    Some(image)
}

/// The bundled logos are PNG, JPEG or SVG.
fn image_format(bytes: &[u8]) -> ImageFormat {
    if bytes.starts_with(b"\x89PNG") {
        ImageFormat::Png
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        ImageFormat::Jpeg
    } else {
        ImageFormat::Svg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_plugins_have_a_logo_and_unknown_ones_fall_back() {
        assert_eq!(
            logo_path("linear").as_deref(),
            Some("plugin-icons/linear.app")
        );
        assert!(logo_image("linear").is_some());
        assert_eq!(logo_path("my-custom-server"), None);
        assert!(logo_image("my-custom-server").is_none());
    }

    #[test]
    fn plugins_from_the_live_catalog_resolve_too() {
        remember(&[crate::composer::MentionPlugin {
            id: "test-github-plugin".into(),
            name: "GitHub".into(),
            domain: "github.com".into(),
        }]);
        assert_eq!(
            logo_path("test-github-plugin").as_deref(),
            Some("plugin-icons/github.com")
        );
        assert_eq!(known_name("test-github-plugin").as_deref(), Some("GitHub"));
    }

    #[test]
    fn logo_formats_follow_their_bytes() {
        assert_eq!(image_format(b"\x89PNG\r\n"), ImageFormat::Png);
        assert_eq!(image_format(&[0xFF, 0xD8, 0xFF]), ImageFormat::Jpeg);
        assert_eq!(image_format(b"<svg"), ImageFormat::Svg);
    }
}
