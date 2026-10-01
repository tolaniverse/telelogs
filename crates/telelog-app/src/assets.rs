use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

/// Telelogs' own icons, layered over gpui-kit's component icons.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/*.svg"]
struct Embedded;

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match Embedded::get(path) {
            Some(file) => Ok(Some(file.data)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths: Vec<SharedString> = Embedded::iter()
            .filter(|name| name.starts_with(path))
            .map(|name| name.to_string().into())
            .collect();
        paths.extend(gpui_kit::assets::Assets.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
