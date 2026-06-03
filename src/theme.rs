use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub default_fg: usize,
    pub default_bg: usize,
    pub comment: usize,
    pub comment_bold: bool,
    pub comment_italic: bool,
    pub number: usize,
    pub number_bold: bool,
    pub number_italic: bool,
    pub keyword: usize,
    pub keyword_bold: bool,
    pub keyword_italic: bool,
    pub string: usize,
    pub string_bold: bool,
    pub string_italic: bool,
    pub string_eol_bg: usize,
    pub preprocessor: usize,
    pub preprocessor_bold: bool,
    pub preprocessor_italic: bool,
    pub type_color: usize,
    pub type_bold: bool,
    pub type_italic: bool,
    pub function: usize,
    pub function_bold: bool,
    pub function_italic: bool,
    pub selection_fg: usize,
    pub selection_bg: usize,
    pub margin_fg: usize,
    pub caret: usize,
    pub caret_width: usize,
    pub current_line_bg: usize,
    pub current_line_visible: bool,
    pub extra_ascent: i32,
    pub extra_descent: i32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            default_fg: 0xdbdbdb,
            default_bg: 0x1c1c1c,
            comment: 0xadadad,
            comment_bold: false,
            comment_italic: false,
            number: 0x8ad1ff,
            number_bold: false,
            number_italic: false,
            keyword: 0xbf6069,
            keyword_bold: true,
            keyword_italic: false,
            string: 0x6bb37c,
            string_bold: false,
            string_italic: false,
            string_eol_bg: 0x6e006e,
            preprocessor: 0x45bde6,
            preprocessor_bold: false,
            preprocessor_italic: false,
            type_color: 0x50aab3,
            type_bold: false,
            type_italic: false,
            function: 0xcc8ad4,
            function_bold: false,
            function_italic: false,
            selection_fg: 0x000000,
            selection_bg: 0xe7a96b,
            margin_fg: 0x6e6e6e,
            caret: 0xffffff,
            caret_width: 1,
            current_line_bg: 0x262626,
            current_line_visible: true,
            extra_ascent: 0,
            extra_descent: 0,
        }
    }
}

#[cfg(feature = "gtk-ui")]
impl From<&Palette> for scintilla::Palette {
    fn from(palette: &Palette) -> Self {
        Self {
            default_fg: palette.default_fg,
            default_bg: palette.default_bg,
            comment: palette.comment,
            comment_bold: palette.comment_bold,
            comment_italic: palette.comment_italic,
            number: palette.number,
            number_bold: palette.number_bold,
            number_italic: palette.number_italic,
            keyword: palette.keyword,
            keyword_bold: palette.keyword_bold,
            keyword_italic: palette.keyword_italic,
            string: palette.string,
            string_bold: palette.string_bold,
            string_italic: palette.string_italic,
            string_eol_bg: palette.string_eol_bg,
            preprocessor: palette.preprocessor,
            preprocessor_bold: palette.preprocessor_bold,
            preprocessor_italic: palette.preprocessor_italic,
            type_color: palette.type_color,
            type_bold: palette.type_bold,
            type_italic: palette.type_italic,
            function: palette.function,
            function_bold: palette.function_bold,
            function_italic: palette.function_italic,
            selection_fg: palette.selection_fg,
            selection_bg: palette.selection_bg,
            margin_fg: palette.margin_fg,
            caret: palette.caret,
            caret_width: palette.caret_width,
            current_line_bg: palette.current_line_bg,
            current_line_visible: palette.current_line_visible,
            extra_ascent: palette.extra_ascent,
            extra_descent: palette.extra_descent,
        }
    }
}

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
    load_scheme(name).editor
}

pub fn load_scheme(name: &str) -> Scheme {
    for dir in theme_dirs() {
        let path = dir.join(format!("{name}.conf"));
        if let Ok(src) = std::fs::read_to_string(&path) {
            return Theme::parse(&src).to_scheme();
        }
    }
    Scheme::from(Palette::default())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub editor: Palette,
    pub ui: UiPalette,
    pub terminal: TerminalPalette,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiPalette {
    pub foreground: usize,
    pub background: usize,
    pub panel: usize,
    pub raised: usize,
    pub border: usize,
    pub selection_foreground: usize,
    pub selection_background: usize,
    pub accent: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalPalette {
    pub foreground: usize,
    pub background: usize,
    pub ansi: [usize; 16],
}

impl From<Palette> for Scheme {
    fn from(editor: Palette) -> Self {
        let ui = UiPalette::from(&editor);
        let terminal = TerminalPalette::from(&editor);
        Self {
            editor,
            ui,
            terminal,
        }
    }
}

impl From<&Palette> for UiPalette {
    fn from(palette: &Palette) -> Self {
        Self {
            foreground: palette.default_fg,
            background: palette.default_bg,
            panel: blend(palette.default_bg, palette.default_fg, 0.06),
            raised: blend(palette.default_bg, palette.default_fg, 0.12),
            border: blend(palette.default_bg, palette.default_fg, 0.22),
            selection_foreground: palette.selection_fg,
            selection_background: palette.selection_bg,
            accent: palette.keyword,
        }
    }
}

impl From<&Palette> for TerminalPalette {
    fn from(palette: &Palette) -> Self {
        let background = palette.default_bg;
        let foreground = palette.default_fg;
        Self {
            foreground,
            background,
            ansi: [
                background,
                palette.keyword,
                palette.string,
                palette.number,
                palette.preprocessor,
                palette.function,
                palette.type_color,
                foreground,
                blend(background, foreground, 0.35),
                blend(palette.keyword, foreground, 0.25),
                blend(palette.string, foreground, 0.25),
                blend(palette.number, foreground, 0.25),
                blend(palette.preprocessor, foreground, 0.25),
                blend(palette.function, foreground, 0.25),
                blend(palette.type_color, foreground, 0.25),
                blend(foreground, 0xffffff, 0.35),
            ],
        }
    }
}

/// Directories searched for theme `.conf` files, in order of preference:
/// 1. User config dir (`config_directory()/themes`)
/// 2. Next to the installed binary (themes/)
/// 3. Source tree colorschemes/ dir used during development
fn theme_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(config) = crate::config::config_directory() {
        dirs.push(config.join("themes"));
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
            comment_bold: comment.bold,
            comment_italic: comment.italic,
            number: number.fg,
            number_bold: number.bold,
            number_italic: number.italic,
            keyword: keyword.fg,
            keyword_bold: keyword.bold,
            keyword_italic: keyword.italic,
            string: string.fg,
            string_bold: string.bold,
            string_italic: string.italic,
            string_eol_bg: string_eol.bg.unwrap_or(0x6e006e),
            preprocessor: preprocessor.fg,
            preprocessor_bold: preprocessor.bold,
            preprocessor_italic: preprocessor.italic,
            type_color: type_style.fg,
            type_bold: type_style.bold,
            type_italic: type_style.italic,
            function: function.fg,
            function_bold: function.bold,
            function_italic: function.italic,
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

    pub fn to_scheme(&self) -> Scheme {
        Scheme::from(self.to_palette())
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
        let inherited = match &raw.fg {
            Some(ColorRef::Named(n)) if !self.colors.contains_key(n.as_str()) => {
                self.resolve_inner(n, depth + 1)
            }
            _ => None,
        };
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
            bold: raw.bold || inherited.is_some_and(|style| style.bold),
            italic: raw.italic || inherited.is_some_and(|style| style.italic),
        })
    }
}

fn blend(from: usize, to: usize, amount: f64) -> usize {
    let channel = |shift: usize| {
        let from = ((from >> shift) & 0xff_usize) as f64;
        let to = ((to >> shift) & 0xff_usize) as f64;
        (from + (to - from) * amount).round() as usize
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
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

    #[test]
    fn derives_ui_and_terminal_colors_from_editor_scheme() {
        let scheme = load_scheme("tango-dark");
        assert_eq!(scheme.ui.foreground, scheme.editor.default_fg);
        assert_eq!(scheme.ui.background, scheme.editor.default_bg);
        assert_eq!(scheme.terminal.background, scheme.editor.default_bg);
        assert_eq!(scheme.terminal.ansi[1], scheme.editor.keyword);
        assert_ne!(scheme.ui.panel, scheme.ui.background);
    }

    #[test]
    fn preserves_bold_and_italic_style_flags() {
        let palette = Theme::parse(
            "[named_styles]\n\
             default=#fff;#000;false;false\n\
             comment=#aaa;#000;false;true\n\
             keyword=#f00;#000;true;true\n\
             keyword_1=keyword\n\
             string=#0f0;#000;false;true\n\
             string_1=string\n",
        )
        .to_palette();
        assert!(palette.comment_italic);
        assert!(palette.keyword_bold);
        assert!(palette.keyword_italic);
        assert!(palette.string_italic);
    }
}
