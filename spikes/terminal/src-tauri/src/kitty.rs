//! Read-only kitty.conf importer: follows `include`, keeps the last value of
//! each key, and maps what it can onto xterm.js options (FR-1002).

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Default)]
pub struct KittyImport {
    /// Files read, in order.
    pub sources: Vec<String>,
    /// xterm.js ITerminalOptions (theme included).
    pub options: Map<String, Value>,
    /// Container padding in px (window_padding_width).
    pub padding: [f64; 4],
    pub opacity: f64,
    pub ligatures: bool,
    pub copy_on_select: bool,
    /// `map <key> send_text all <text>` lines translated for the key handler.
    pub key_remaps: Vec<Value>,
    pub mapped: Vec<(String, String)>,
    pub unmapped: Vec<(String, String, String)>,
}

const MULTI: &[&str] = &[
    "map", "mouse_map", "symbol_map", "font_features", "narrow_symbols", "env", "action_alias",
    "kitten_alias", "modify_font",
];

fn parse(path: &Path, out: &mut Vec<(String, String)>, sources: &mut Vec<String>, depth: u32) {
    if depth > 8 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else { return };
    sources.push(path.display().to_string());
    let dir = path.parent().unwrap_or(Path::new("."));
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = match line.split_once(char::is_whitespace) {
            Some((k, v)) => (k, v.trim()),
            None => (line, ""),
        };
        match k {
            "include" | "globinclude" | "envinclude" => {
                // Only plain `include` is followed; the others are reported.
                if k == "include" {
                    let p = expand(v, dir);
                    parse(&p, out, sources, depth + 1);
                } else {
                    out.push((k.to_string(), v.to_string()));
                }
            }
            _ => out.push((k.to_string(), v.to_string())),
        }
    }
}

fn expand(v: &str, dir: &Path) -> PathBuf {
    if let Some(rest) = v.strip_prefix("~/") {
        return PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest);
    }
    let p = PathBuf::from(v);
    if p.is_absolute() {
        p
    } else {
        dir.join(p)
    }
}

/// kitty matches font names fuzzily ("FiraCode-Regular Nerd Font"); CSS needs
/// the family name. Strip PostScript style suffixes.
fn family_exists(name: &str) -> bool {
    core_text::font_collection::create_for_family(name)
        .and_then(|c| c.get_descriptors())
        .map(|d| d.len() > 0)
        .unwrap_or(false)
}

fn css_family(name: &str) -> String {
    let styles = ["-Regular", "-Bold", "-Italic", "-Medium", "-Light", "-Retina", "-SemiBold"];
    let mut s = name.to_string();
    for st in styles {
        s = s.replace(st, "");
    }
    s.trim().to_string()
}

pub fn import(conf: &Path) -> KittyImport {
    let mut lines = Vec::new();
    let mut imp = KittyImport { opacity: 1.0, ..Default::default() };
    parse(conf, &mut lines, &mut imp.sources, 0);

    let mut last: BTreeMap<String, String> = BTreeMap::new();
    let mut multi: Vec<(String, String)> = Vec::new();
    for (k, v) in lines {
        if MULTI.contains(&k.as_str()) {
            multi.push((k, v));
        } else {
            last.insert(k, v);
        }
    }

    let o = &mut imp.options;
    let mut theme = Map::new();
    let mut ansi: Vec<Value> = vec![Value::Null; 16];
    let mut ext: BTreeMap<u32, String> = BTreeMap::new();
    // kitty defaults that differ from xterm.js defaults.
    o.insert("scrollback".into(), json!(2000));
    o.insert("fontSize".into(), json!(11.0));
    o.insert("cursorStyle".into(), json!("block"));
    o.insert("cursorBlink".into(), json!(true));
    o.insert("macOptionIsMeta".into(), json!(false));
    imp.ligatures = true;

    for (k, v) in &last {
        let ok = |imp_m: &mut Vec<(String, String)>, to: &str| imp_m.push((format!("{k} {v}"), to.into()));
        match k.as_str() {
            "font_family" => {
                // Resolve like kitty does: an exact family name, else Menlo.
                // SPIKE_FONT forces a family (used for the FiraCode comparison).
                let forced = std::env::var("SPIKE_FONT").ok();
                let fam = if let Some(f) = forced.clone() {
                    f
                } else if family_exists(v) {
                    v.clone()
                } else {
                    imp.unmapped.push((k.clone(), v.clone(), format!(
                        "kitty cannot resolve this name and falls back to Menlo (kitty only takes monospaced families, e.g. \"{} Mono\")", css_family(v))));
                    "Menlo".to_string()
                };
                o.insert("fontFamily".into(), json!(format!("\"{fam}\", monospace")));
                ok(&mut imp.mapped, &format!("fontFamily \"{fam}\"{}", if forced.is_some() { " (forced by SPIKE_FONT)" } else { "" }));
            }
            "bold_font" => {
                o.insert("fontWeightBold".into(), json!("bold"));
                ok(&mut imp.mapped, "fontWeightBold bold (same family)");
            }
            "italic_font" | "bold_italic_font" if v == "auto" => ok(&mut imp.mapped, "browser picks the italic face"),
            "font_size" => {
                let s: f64 = v.parse().unwrap_or(11.0);
                o.insert("fontSize".into(), json!(s));
                ok(&mut imp.mapped, "fontSize (pt == CSS px on macOS)");
            }
            "adjust_line_height" => {
                if let Some(p) = v.strip_suffix('%') {
                    let r: f64 = p.parse().unwrap_or(100.0) / 100.0;
                    o.insert("lineHeight".into(), json!(r));
                    ok(&mut imp.mapped, "lineHeight");
                } else {
                    imp.unmapped.push((k.clone(), v.clone(), "absolute px line height: needs measured cell height".into()));
                }
            }
            "disable_ligatures" => {
                imp.ligatures = v != "always";
                ok(&mut imp.mapped, if imp.ligatures { "ligatures addon loaded" } else { "ligatures addon NOT loaded" });
            }
            "cursor_shape" => {
                let s = match v.as_str() { "beam" => "bar", "underline" => "underline", _ => "block" };
                o.insert("cursorStyle".into(), json!(s));
                ok(&mut imp.mapped, &format!("cursorStyle {s}"));
            }
            "cursor_shape_unfocused" => {
                let s = match v.as_str() { "beam" => "bar", "underline" => "underline", "hollow" => "outline", "unchanged" => "block", _ => "outline" };
                o.insert("cursorInactiveStyle".into(), json!(s));
                ok(&mut imp.mapped, "cursorInactiveStyle");
            }
            "cursor_blink_interval" => {
                let blink = v != "0";
                o.insert("cursorBlink".into(), json!(blink));
                ok(&mut imp.mapped, "cursorBlink (on/off only, not the interval)");
            }
            "cursor_beam_thickness" => {
                o.insert("cursorWidth".into(), json!(v.parse::<f64>().unwrap_or(1.5)));
                ok(&mut imp.mapped, "cursorWidth");
            }
            "scrollback_lines" => {
                o.insert("scrollback".into(), json!(v.parse::<i64>().unwrap_or(2000).max(0)));
                ok(&mut imp.mapped, "scrollback");
            }
            "macos_option_as_alt" => {
                let meta = matches!(v.as_str(), "yes" | "both");
                o.insert("macOptionIsMeta".into(), json!(meta));
                if matches!(v.as_str(), "left" | "right") {
                    imp.unmapped.push((k.clone(), v.clone(), "xterm.js cannot make only one Option key Meta".into()));
                } else {
                    ok(&mut imp.mapped, &format!("macOptionIsMeta {meta}"));
                }
            }
            "copy_on_select" => {
                imp.copy_on_select = v != "no";
                ok(&mut imp.mapped, "onSelectionChange -> clipboard (app code)");
            }
            "enable_audio_bell" => ok(&mut imp.mapped, "xterm.js has no audible bell by default"),
            "window_padding_width" => {
                let n: Vec<f64> = v.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                imp.padding = match n.len() {
                    1 => [n[0]; 4],
                    2 => [n[0], n[1], n[0], n[1]],
                    3 => [n[0], n[1], n[2], n[1]],
                    4 => [n[0], n[1], n[2], n[3]],
                    _ => [0.0; 4],
                };
                ok(&mut imp.mapped, "CSS padding of the pane (pt == px)");
            }
            "background_opacity" => {
                imp.opacity = v.parse().unwrap_or(1.0);
                ok(&mut imp.mapped, "allowTransparency + rgba background (needs a transparent window)");
            }
            "foreground" => { theme.insert("foreground".into(), json!(v)); }
            "background" => { theme.insert("background".into(), json!(v)); }
            "selection_background" => { theme.insert("selectionBackground".into(), json!(v)); }
            "selection_foreground" => { theme.insert("selectionForeground".into(), json!(v)); }
            "cursor" => { theme.insert("cursor".into(), json!(v)); }
            "cursor_text_color" => { theme.insert("cursorAccent".into(), json!(v)); }
            c if c.starts_with("color") && c[5..].parse::<u32>().is_ok() => {
                let n: u32 = c[5..].parse().unwrap();
                if n < 16 {
                    ansi[n as usize] = json!(v);
                } else {
                    ext.insert(n, v.clone());
                }
            }
            "url_color" | "url_style" => imp.unmapped.push((k.clone(), v.clone(), "xterm.js links have no color option".into())),
            "active_tab_background" | "active_tab_foreground" | "inactive_tab_background" | "inactive_tab_foreground"
            | "tab_bar_background" | "active_border_color" | "inactive_border_color" | "tab_bar_style" => {
                imp.unmapped.push((k.clone(), v.clone(), "app chrome, not the terminal (theme derivation §9.1)".into()))
            }
            "term" => imp.unmapped.push((k.clone(), v.clone(), "xterm.js is not kitty: TERM stays xterm-256color".into())),
            "shell_integration" => imp.unmapped.push((k.clone(), v.clone(), "kitty shell integration needs kitty".into())),
            "editor" => imp.unmapped.push((k.clone(), v.clone(), "candidate default for the FR-1102 editor command".into())),
            "allow_remote_control" | "listen_on" => {
                imp.unmapped.push((k.clone(), v.clone(), "not a pane option; enables FR-1004 'Open in kitty' via kitty @".into()))
            }
            _ => imp.unmapped.push((k.clone(), v.clone(), "no xterm.js equivalent".into())),
        }
    }
    let colors = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
    for (i, c) in ansi.iter().enumerate() {
        if !c.is_null() {
            let name = if i < 8 { colors[i].to_string() } else { format!("bright{}{}", colors[i - 8][..1].to_uppercase(), &colors[i - 8][1..]) };
            theme.insert(name, c.clone());
        }
    }
    if !ext.is_empty() {
        // extendedAnsi starts at color16.
        let max = *ext.keys().max().unwrap();
        let arr: Vec<Value> = (16..=max).map(|n| ext.get(&n).map(|s| json!(s)).unwrap_or(Value::Null)).collect();
        theme.insert("extendedAnsi".into(), json!(arr));
        imp.mapped.push((format!("color16..color{max}"), "theme.extendedAnsi".into()));
    }
    if !theme.is_empty() {
        imp.mapped.push(("foreground/background/selection/cursor/color0-15".into(), "theme".into()));
    }
    if std::env::var("SPIKE_LIGATURES").is_ok() {
        imp.ligatures = true;
    }
    if imp.opacity < 1.0 {
        if let Some(Value::String(bg)) = theme.get("background").cloned() {
            let a = (imp.opacity * 255.0).round() as u8;
            theme.insert("background".into(), json!(format!("{bg}{a:02x}")));
            o.insert("allowTransparency".into(), json!(true));
        }
    }
    o.insert("theme".into(), Value::Object(theme));

    for (k, v) in multi {
        if k == "map" {
            // map alt+left send_text all \x1b\x62
            let parts: Vec<&str> = v.split_whitespace().collect();
            if parts.len() >= 4 && parts[1] == "send_text" {
                let text = unescape(&parts[3..].join(" "));
                imp.key_remaps.push(json!({ "key": parts[0], "text": text }));
                imp.mapped.push((format!("map {v}"), "key remap in the pane key handler".into()));
                continue;
            }
            imp.unmapped.push((k, v, "kitty action, not a terminal key".into()));
        } else if k == "modify_font" {
            imp.unmapped.push((k, v, "TODO: cell_height % -> lineHeight, cell_width -> letterSpacing, baseline: none".into()));
        } else {
            imp.unmapped.push((k, v, "no xterm.js equivalent".into()));
        }
    }
    imp
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1] == b'x' {
            if let Ok(n) = u8::from_str_radix(&s[i + 2..i + 4], 16) {
                out.push(n as char);
                i += 4;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}
