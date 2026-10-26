//! Font files for the webview. WKWebView hides user-installed fonts
//! (`~/Library/Fonts`) from pages, so the core finds a family's files with
//! CoreText and the UI registers them with the FontFace API (P0, ADR-006 fix 2).

use std::path::Path;

use anyhow::{Result, bail};

/// `(path, style name)` for every face of `family` CoreText knows.
pub fn font_files(family: &str) -> Vec<(String, String)> {
    let Some(coll) = core_text::font_collection::create_for_family(family) else { return vec![] };
    let Some(descs) = coll.get_descriptors() else { return vec![] };
    descs.iter().filter_map(|d| Some((d.font_path()?.display().to_string(), d.style_name()))).collect()
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
}
