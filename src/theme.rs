use std::collections::HashMap;
use std::path::PathBuf;

use scintilla::Palette;

/// All available theme names, scanned from theme directories in preference order.
pub fn list() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for dir in theme_dirs() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut found: Vec<String> = entries
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension()?.to_str()? == "conf" {
                        Some(p.file_stem()?.to_str()?.to_owned())
                    } else {
                        None
                    }
                })
                .filter(|n| !names.contains(n))
                .collect();
            found.sort();
            names.extend(found);
        }
    }
    names
}

pub fn load(name: &str) -> Palette {
    for dir in theme_dirs() {
        let path = dir.join(format!("{name}.conf"));
        if let Ok(src) = std::fs::read_to_string(&path) {
            return Theme::parse(&src).to_palette();
        }
    }
    Palette::default()
}

/// Directories searched for theme `.conf` files, in order of preference:
/// 1. User config dir  (~/.config/nokin/themes/)
/// 2. Next to the installed binary (themes/)
/// 3. Source tree colorschemes/ dir used during development
fn theme_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".config/nokin/themes"));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("themes"));
        }
    }

    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes"));

    dirs
}

struct Theme {
    colors: HashMap<String, usize>,
    styles: HashMap<String, RawStyle>,
    styling: HashMap<String, (i32, i32)>,
}

#[derive(Clone, Default)]
struct RawStyle {
    fg: Option<ColorRef>,
    bg: Option<ColorRef>,
    bold: bool,
    italic: bool,
}

#[derive(Clone)]
enum ColorRef {
    Literal(usize),
    Named(String),
}

impl Theme {
    pub fn parse(source: &str) -> Self {
        let mut colors: HashMap<String, usize> = HashMap::new();
        let mut styles: HashMap<String, RawStyle> = HashMap::new();
        let mut styling: HashMap<String, (i32, i32)> = HashMap::new();
        let mut section = "";

        for line in source.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = &line[1..line.len() - 1];
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match section {
                "named_colors" => {
                    if let Some(color) = parse_hex(value) {
                        colors.insert(key.to_owned(), color);
                    }
                }
                "named_styles" => {
                    styles.insert(key.to_owned(), parse_raw_style(value));
                }
                "styling" => {
                    let pair = parse_int_pair(value);
                    styling.insert(key.to_owned(), pair);
                }
                _ => {}
            }
        }

        Self {
            colors,
            styles,
            styling,
        }
    }

    pub fn to_palette(&self) -> Palette {
        let default = self.resolve("default");
        let comment = self.resolve("comment");
        let number = self
            .resolve_opt("number_1")
            .or_else(|| self.resolve_opt("number"))
            .unwrap_or(default);
        let keyword = self
            .resolve_opt("keyword_1")
            .or_else(|| self.resolve_opt("keyword"))
            .unwrap_or(default);
        let string = self
            .resolve_opt("string_1")
            .or_else(|| self.resolve_opt("string"))
            .unwrap_or(default);
        let string_eol = self.resolve_opt("string_eol").unwrap_or(default);
        let preprocessor = self.resolve_opt("preprocessor").unwrap_or(comment);
        let type_style = self.resolve_opt("type").unwrap_or(default);
        let function = self.resolve_opt("function").unwrap_or(default);
        let selection = self.resolve_opt("selection").unwrap_or(StyleValues {
            fg: 0x000000_usize,
            bg: Some(0x404040_usize),
            bold: false,
            italic: false,
        });
        let margin = self.resolve_opt("margin_line_number").unwrap_or(comment);
        let caret = self.resolve_opt("caret").unwrap_or(StyleValues {
            fg: default.fg,
            bg: None,
            bold: false,
            italic: false,
        });
        let current_line = self.resolve_opt("current_line").unwrap_or(StyleValues {
            fg: 0xffffff,
            bg: Some(0x262626),
            bold: true,
            italic: false,
        });
        let (extra_ascent, extra_descent) =
            self.styling.get("line_height").copied().unwrap_or((0, 0));
        let caret_width = self
            .styling
            .get("caret_width")
            .map(|&(w, _)| w)
            .unwrap_or(1);

        Palette {
            default_fg: default.fg,
            default_bg: default.bg.unwrap_or(0x1c1c1c),
            comment: comment.fg,
            number: number.fg,
            keyword: keyword.fg,
            keyword_bold: keyword.bold,
            string: string.fg,
            string_eol_bg: string_eol.bg.unwrap_or(0x6e006e),
            preprocessor: preprocessor.fg,
            type_color: type_style.fg,
            function: function.fg,
            selection_fg: selection.fg,
            selection_bg: selection.bg.unwrap_or(0x404040),
            margin_fg: margin.fg,
            caret: caret.fg,
            caret_width: caret_width.max(1) as usize,
            current_line_bg: current_line.bg.unwrap_or(0x262626),
            current_line_visible: current_line.bold,
            extra_ascent,
            extra_descent,
        }
    }

    fn resolve(&self, name: &str) -> StyleValues {
        self.resolve_opt(name).unwrap_or_default()
    }

    fn resolve_opt(&self, name: &str) -> Option<StyleValues> {
        self.resolve_inner(name, 0)
    }

    fn resolve_inner(&self, name: &str, depth: usize) -> Option<StyleValues> {
        if depth > 16 {
            return None;
        }
        let raw = self.styles.get(name)?;
        let fg = match &raw.fg {
            Some(ColorRef::Literal(v)) => *v,
            Some(ColorRef::Named(n)) => {
                if let Some(&color) = self.colors.get(n.as_str()) {
                    color
                } else {
                    self.resolve_inner(n, depth + 1)
                        .map(|s| s.fg)
                        .unwrap_or(0x808080_usize)
                }
            }
            None => self
                .resolve_inner("default", depth + 1)
                .map(|s| s.fg)
                .unwrap_or(0x808080_usize),
        };
        let bg = match &raw.bg {
            Some(ColorRef::Literal(v)) => Some(*v),
            Some(ColorRef::Named(n)) => {
                if let Some(&color) = self.colors.get(n.as_str()) {
                    Some(color)
                } else {
                    self.resolve_inner(n, depth + 1).and_then(|s| s.bg)
                }
            }
            None => None,
        };
        Some(StyleValues {
            fg,
            bg,
            bold: raw.bold,
            italic: raw.italic,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct StyleValues {
    fg: usize,
    bg: Option<usize>,
    bold: bool,
    italic: bool,
}

fn parse_raw_style(value: &str) -> RawStyle {
    if !value.contains(';') {
        let (first, modifiers) = value
            .split_once(',')
            .map(|(a, m)| (a.trim(), m))
            .unwrap_or((value.trim(), ""));
        let bold = modifiers.split(',').any(|m| m.trim() == "bold");
        let italic = modifiers.split(',').any(|m| m.trim() == "italic");
        let fg = if first.starts_with('#') {
            parse_hex(first).map(ColorRef::Literal)
        } else {
            Some(ColorRef::Named(first.to_owned()))
        };
        return RawStyle {
            fg,
            bg: None,
            bold,
            italic,
        };
    }

    let parts: Vec<&str> = value.splitn(4, ';').collect();
    let fg = parts.first().and_then(|s| parse_color_ref(s.trim()));
    let bg = parts.get(1).and_then(|s| parse_color_ref(s.trim()));
    let bold = parts
        .get(2)
        .map(|s| s.trim().trim_end_matches(';') == "true")
        .unwrap_or(false);
    let italic = parts
        .get(3)
        .map(|s| s.trim().trim_end_matches(';') == "true")
        .unwrap_or(false);
    RawStyle {
        fg,
        bg,
        bold,
        italic,
    }
}

fn parse_color_ref(s: &str) -> Option<ColorRef> {
    if s.is_empty() {
        return None;
    }
    if let Some(color) = parse_hex(s) {
        Some(ColorRef::Literal(color))
    } else {
        Some(ColorRef::Named(s.to_owned()))
    }
}

fn parse_int_pair(s: &str) -> (i32, i32) {
    let mut parts = s.splitn(2, ';');
    let a = parts
        .next()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let b = parts
        .next()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    (a, b)
}

fn parse_hex(s: &str) -> Option<usize> {
    let s = s.strip_prefix('#')?;
    match s.len() {
        3 => {
            let r = usize::from_str_radix(&s[0..1], 16).ok()?;
            let g = usize::from_str_radix(&s[1..2], 16).ok()?;
            let b = usize::from_str_radix(&s[2..3], 16).ok()?;
            Some((r << 20) | (r << 16) | (g << 12) | (g << 8) | (b << 4) | b)
        }
        6 => usize::from_str_radix(s, 16).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_and_long_hex_colors() {
        assert_eq!(parse_hex("#fff"), Some(0xffffff));
        assert_eq!(parse_hex("#1c1c1c"), Some(0x1c1c1c));
        assert_eq!(parse_hex("#abc"), Some(0xaabbcc));
        assert_eq!(parse_hex("nocolor"), None);
    }

    #[test]
    fn resolves_alias_chain_to_palette() {
        let palette = load("tango-dark");
        assert_ne!(palette.default_bg, 0);
        assert_ne!(palette.default_fg, 0);
        assert_ne!(palette.string, palette.comment);
    }

    #[test]
    fn default_themes_load_without_panic() {
        for name in ["tango-dark"] {
            let palette = load(name);
            assert_ne!(
                palette.default_fg, palette.default_bg,
                "theme {name}: fg==bg"
            );
        }
    }
}
