//! Font files for the webview. WKWebView hides user-installed fonts
//! (`~/Library/Fonts`) from pages, so the core finds a family's files with
//! CoreText and the UI registers them with the FontFace API (P0, ADR-006 fix 2).
//! The UI font (FiraCode Nerd Font Mono) is also bundled in the `.app`
//! (`Contents/Resources/fonts`), used when it is not installed.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// The bundled UI font family.
pub const BUNDLED_FAMILY: &str = "FiraCode Nerd Font Mono";

/// `(path, style name)` for every face of `family` CoreText knows, else the
/// bundled files for the UI font.
pub fn font_files(family: &str) -> Vec<(String, String)> {
    let installed = installed_files(family);
    if installed.is_empty() && family == BUNDLED_FAMILY {
        return bundled_dir().map(|d| bundled_files(&d)).unwrap_or_default();
    }
    installed
}

fn installed_files(family: &str) -> Vec<(String, String)> {
    let Some(coll) = core_text::font_collection::create_for_family(family) else { return vec![] };
    let Some(descs) = coll.get_descriptors() else { return vec![] };
    descs.iter().filter_map(|d| Some((d.font_path()?.display().to_string(), d.style_name()))).collect()
}

/// `Contents/Resources/fonts` next to the binary in a bundle.
fn bundled_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let d = exe.parent()?.parent()?.join("Resources").join("fonts");
    d.is_dir().then_some(d)
}

/// `FiraCodeNerdFontMono-SemiBold.ttf` → style `SemiBold`.
fn bundled_files(dir: &Path) -> Vec<(String, String)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<(String, String)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let stem = p.file_stem()?.to_str()?.to_string();
            let style = stem.rsplit_once('-')?.1.to_string();
            (p.extension()? == "ttf").then(|| (p.display().to_string(), style))
        })
        .collect();
    v.sort();
    v
}

/// Reads a font file. Only font files: this is reachable from the webview.
pub fn read_font(path: &str) -> Result<Vec<u8>> {
    let p = Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "woff" | "woff2") {
        bail!("not a font file: {path}");
    }
    Ok(std::fs::read(p)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_system_fonts_and_refuses_other_files() {
        let menlo = font_files("Menlo");
        assert!(menlo.iter().any(|(p, _)| p.ends_with(".ttc")), "{menlo:?}");
        assert!(font_files("No Such Family 123").is_empty());
        assert!(read_font("/etc/passwd").is_err());
        assert!(!read_font(&menlo[0].0).unwrap().is_empty());
    }

    #[test]
    fn bundled_styles_from_file_names() {
        let d = tempfile::tempdir().unwrap();
        for f in ["FiraCodeNerdFontMono-Regular.ttf", "FiraCodeNerdFontMono-SemiBold.ttf", "OFL.txt"] {
            std::fs::write(d.path().join(f), "x").unwrap();
        }
        let styles: Vec<String> = bundled_files(d.path()).into_iter().map(|(_, s)| s).collect();
        assert_eq!(styles, ["Regular", "SemiBold"]);
    }
}
