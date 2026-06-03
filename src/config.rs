use std::env;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub ui: UiSettings,
    pub editor: EditorSettings,
    pub workspace: WorkspaceSettings,
    pub terminal: TerminalSettings,
    pub lsp: LspSettings,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiThemeMode {
    System,
    ColorScheme,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiSettings {
    pub theme_mode: UiThemeMode,
    pub font_family: String,
    pub font_size: f64,
    pub scale: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditorSettings {
    pub font_family: String,
    pub font_size: f64,
    pub tab_width: usize,
    pub insert_spaces: bool,
    pub ligatures: bool,
    pub theme: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceSettings {
    pub close_tabs_on_folder_open: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalSettings {
    pub shell: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LspSettings {
    pub clangd: String,
    pub rust_analyzer: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectConfig {
    pub workspace_command: Option<String>,
    pub file_commands: Vec<(String, String)>,
    pub compiler: String,
    pub include_dirs: Vec<PathBuf>,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            close_tabs_on_folder_open: true,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui: UiSettings {
                theme_mode: UiThemeMode::System,
                font_family: "Sans".into(),
                font_size: 10.0,
                scale: 1.0,
            },
            editor: EditorSettings {
                font_family: "Monospace".into(),
                font_size: 11.0,
                tab_width: 4,
                insert_spaces: true,
                ligatures: true,
                theme: "tango-dark".into(),
            },
            workspace: WorkspaceSettings::default(),
            terminal: TerminalSettings {
                shell: default_shell(),
            },
            lsp: LspSettings {
                clangd: "clangd".into(),
                rust_analyzer: "rust-analyzer".into(),
            },
        }
    }
}

impl Settings {
    pub fn load() -> io::Result<Self> {
        let Some(directory) = config_directory() else {
            return Ok(Self::default());
        };
        load_optional(&directory.join("settings.toml"), Self::parse)
    }

    pub fn parse(input: &str) -> Self {
        let mut settings = Self::default();
        parse_entries(input, |section, key, value| match (section, key) {
            ("ui", "theme_mode") => {
                settings.ui.theme_mode =
                    UiThemeMode::parse(&unquote(value)).unwrap_or(settings.ui.theme_mode)
            }
            ("ui", "font_family") => settings.ui.font_family = unquote(value),
            ("ui", "font_size") => {
                settings.ui.font_size = value.parse().unwrap_or(settings.ui.font_size)
            }
            ("ui", "scale") => settings.ui.scale = value.parse().unwrap_or(settings.ui.scale),
            ("editor", "font_family") => settings.editor.font_family = unquote(value),
            ("editor", "font_size") => {
                settings.editor.font_size = value.parse().unwrap_or(settings.editor.font_size)
            }
            ("editor", "tab_width") => {
                settings.editor.tab_width = value.parse().unwrap_or(settings.editor.tab_width)
            }
            ("editor", "insert_spaces") => {
                settings.editor.insert_spaces =
                    value.parse().unwrap_or(settings.editor.insert_spaces)
            }
            ("editor", "ligatures") => {
                settings.editor.ligatures = value.parse().unwrap_or(settings.editor.ligatures)
            }
            ("editor", "theme") => settings.editor.theme = unquote(value),
            ("workspace", "close_tabs_on_folder_open") => {
                settings.workspace.close_tabs_on_folder_open = value
                    .parse()
                    .unwrap_or(settings.workspace.close_tabs_on_folder_open)
            }
            ("terminal", "shell") => settings.terminal.shell = unquote(value),
            ("lsp", "clangd") => settings.lsp.clangd = unquote(value),
            ("lsp", "rust_analyzer") => settings.lsp.rust_analyzer = unquote(value),
            _ => {}
        });
        settings.ui.font_size = bounded_or(settings.ui.font_size, 6.0, 32.0, 10.0);
        settings.ui.scale = bounded_or(settings.ui.scale, 0.75, 2.0, 1.0);
        settings
    }

    pub fn save(&self) -> io::Result<()> {
        let Some(directory) = config_directory() else {
            return Err(io::Error::other(
                "the platform configuration directory is unavailable",
            ));
        };
        fs::create_dir_all(&directory)?;
        fs::write(directory.join("settings.toml"), self.to_toml())
    }

    pub fn file_path() -> Option<PathBuf> {
        config_directory().map(|directory| directory.join("settings.toml"))
    }

    fn to_toml(&self) -> String {
        format!(
            "[ui]\ntheme_mode = \"{}\"\nfont_family = \"{}\"\nfont_size = {}\nscale = {}\n\n[editor]\nfont_family = \"{}\"\nfont_size = {}\ntab_width = {}\ninsert_spaces = {}\nligatures = {}\ntheme = \"{}\"\n\n[workspace]\nclose_tabs_on_folder_open = {}\n\n[terminal]\nshell = \"{}\"\n\n[lsp]\nclangd = \"{}\"\nrust_analyzer = \"{}\"\n",
            self.ui.theme_mode.as_str(),
            escape_toml(&self.ui.font_family),
            self.ui.font_size,
            self.ui.scale,
            escape_toml(&self.editor.font_family),
            self.editor.font_size,
            self.editor.tab_width,
            self.editor.insert_spaces,
            self.editor.ligatures,
            escape_toml(&self.editor.theme),
            self.workspace.close_tabs_on_folder_open,
            escape_toml(&self.terminal.shell),
            escape_toml(&self.lsp.clangd),
            escape_toml(&self.lsp.rust_analyzer),
        )
    }
}

pub fn config_directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        return env::var_os("APPDATA")
            .or_else(|| {
                env::var_os("USERPROFILE")
                    .map(|home| PathBuf::from(home).join("AppData/Roaming").into_os_string())
            })
            .map(PathBuf::from)
            .map(|directory| directory.join("nokin"));
    }
    #[cfg(target_os = "macos")]
    {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/nokin"));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        unix_config_directory(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn unix_config_directory(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    xdg_config_home
        .filter(|directory| !directory.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".config")))
        .map(|directory| directory.join("nokin"))
}

fn default_shell() -> String {
    #[cfg(target_os = "windows")]
    {
        env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    }
    #[cfg(not(target_os = "windows"))]
    {
        env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

impl UiThemeMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "color-scheme" => Some(Self::ColorScheme),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::ColorScheme => "color-scheme",
        }
    }
}

impl ProjectConfig {
    pub fn load(root: &Path) -> io::Result<Self> {
        load_optional(&root.join(".nokin.toml"), Self::parse)
    }

    pub fn parse(input: &str) -> Self {
        let mut config = Self {
            compiler: "cc".into(),
            ..Self::default()
        };
        parse_entries(input, |section, key, value| match section {
            "run" if key == "workspace" => config.workspace_command = Some(unquote(value)),
            "run.files" => config.file_commands.push((key.into(), unquote(value))),
            "c" if key == "compiler" => config.compiler = unquote(value),
            "c" if key == "include_dirs" => config.include_dirs = parse_string_array(value),
            _ => {}
        });
        config
    }

    pub fn command_for_extension(&self, extension: &str) -> Option<&str> {
        self.file_commands
            .iter()
            .find(|(candidate, _)| candidate == extension)
            .map(|(_, command)| command.as_str())
    }

    pub fn set_command_for_extension(&mut self, extension: &str, command: &str) {
        self.file_commands
            .retain(|(candidate, _)| candidate != extension);
        if !command.is_empty() {
            self.file_commands
                .push((extension.to_owned(), command.to_owned()));
        }
    }

    pub fn save(&self, root: &Path) -> io::Result<()> {
        fs::write(root.join(".nokin.toml"), self.to_toml())
    }

    fn to_toml(&self) -> String {
        let mut output = String::from("[run]\n");
        if let Some(command) = &self.workspace_command {
            output.push_str(&format!("workspace = \"{}\"\n", escape_toml(command)));
        }
        if !self.file_commands.is_empty() {
            output.push_str("\n[run.files]\n");
            for (extension, command) in &self.file_commands {
                output.push_str(&format!("{extension} = \"{}\"\n", escape_toml(command)));
            }
        }
        output.push_str("\n[c]\n");
        output.push_str(&format!("compiler = \"{}\"\n", escape_toml(&self.compiler)));
        output.push_str("include_dirs = [");
        for (index, directory) in self.include_dirs.iter().enumerate() {
            if index != 0 {
                output.push_str(", ");
            }
            output.push('"');
            output.push_str(&escape_toml(&directory.to_string_lossy()));
            output.push('"');
        }
        output.push_str("]\n");
        output
    }
}

fn load_optional<T>(path: &Path, parse: impl FnOnce(&str) -> T) -> io::Result<T>
where
    T: Default,
{
    match fs::read_to_string(path) {
        Ok(input) => Ok(parse(&input)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error),
    }
}

fn parse_entries(input: &str, mut visit: impl FnMut(&str, &str, &str)) {
    let mut section = "";
    for raw_line in input.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim();
        } else if let Some((key, value)) = line.split_once('=') {
            visit(section, key.trim(), value.trim());
        }
    }
}

fn unquote(value: &str) -> String {
    let value = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value);
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            match characters.next() {
                Some('\\') => output.push('\\'),
                Some('"') => output.push('"'),
                Some('n') => output.push('\n'),
                Some('t') => output.push('\t'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(character);
        }
    }
    output
}

fn escape_toml(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn parse_string_array(value: &str) -> Vec<PathBuf> {
    value
        .trim_matches(|character| character == '[' || character == ']')
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(unquote)
        .map(PathBuf::from)
        .collect()
}

fn bounded_or(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    value
        .is_finite()
        .then(|| value.clamp(min, max))
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_settings() {
        let settings = Settings::parse(
            "[editor]\nfont_family = \"Iosevka\"\nfont_size = 12.5\ntab_width = 2\ninsert_spaces = false\ntheme = \"tango-dark\"\n[terminal]\nshell = \"/bin/bash\"\n",
        );
        assert_eq!(settings.editor.font_family, "Iosevka");
        assert_eq!(settings.editor.font_size, 12.5);
        assert_eq!(settings.editor.tab_width, 2);
        assert!(!settings.editor.insert_spaces);
        assert_eq!(settings.editor.theme, "tango-dark");
        assert_eq!(settings.terminal.shell, "/bin/bash");
        assert_eq!(settings.lsp.clangd, "clangd");
        assert_eq!(settings.lsp.rust_analyzer, "rust-analyzer");
    }

    #[test]
    fn serializes_settings_for_round_trip() {
        let settings = Settings::parse(
            "[ui]\ntheme_mode = \"color-scheme\"\nfont_family = \"Inter\"\nfont_size = 11\nscale = 1.25\n[editor]\nfont_family = \"JetBrains Mono\"\nfont_size = 12.5\ntab_width = 2\ninsert_spaces = false\ntheme = \"tango-dark\"\n[workspace]\nclose_tabs_on_folder_open = false\n[terminal]\nshell = \"/bin/zsh\"\n",
        );
        assert_eq!(settings.ui.theme_mode, UiThemeMode::ColorScheme);
        assert_eq!(settings.workspace.close_tabs_on_folder_open, false);
        assert_eq!(Settings::parse(&settings.to_toml()), settings);
    }

    #[test]
    fn defaults_and_bounds_new_ui_settings() {
        let defaults = Settings::parse("[editor]\ntheme = \"tango-dark\"\n");
        assert_eq!(defaults.ui.theme_mode, UiThemeMode::System);
        assert_eq!(defaults.ui.scale, 1.0);

        let bounded = Settings::parse("[ui]\nfont_size = 100\nscale = 0.1\n");
        assert_eq!(bounded.ui.font_size, 32.0);
        assert_eq!(bounded.ui.scale, 0.75);

        let finite = Settings::parse("[ui]\nfont_size = NaN\nscale = inf\n");
        assert_eq!(finite.ui.font_size, 10.0);
        assert_eq!(finite.ui.scale, 1.0);
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn resolves_xdg_config_directory_with_home_fallback() {
        assert_eq!(
            unix_config_directory(Some("/tmp/xdg".into()), Some("/tmp/home".into())),
            Some(PathBuf::from("/tmp/xdg/nokin"))
        );
        assert_eq!(
            unix_config_directory(None, Some("/tmp/home".into())),
            Some(PathBuf::from("/tmp/home/.config/nokin"))
        );
    }

    #[test]
    fn parses_project_commands_and_includes() {
        let config = ProjectConfig::parse(
            "[run]\nworkspace = \"make run\"\n[run.files]\nc = \"cc ${file}\"\n[c]\ncompiler = \"clang\"\ninclude_dirs = [\"include\", \"../shared/include\"]\n",
        );
        assert_eq!(config.workspace_command.as_deref(), Some("make run"));
        assert_eq!(config.command_for_extension("c"), Some("cc ${file}"));
        assert_eq!(config.compiler, "clang");
        assert_eq!(
            config.include_dirs,
            vec![PathBuf::from("include"), PathBuf::from("../shared/include")]
        );
    }

    #[test]
    fn updates_and_serializes_project_commands() {
        let mut config = ProjectConfig::parse(
            "[run]\nworkspace = \"make run\"\n[run.files]\nc = \"cc ${file}\"\n[c]\ncompiler = \"clang\"\ninclude_dirs = [\"include\"]\n",
        );
        config.set_command_for_extension("c", "clang \"${file}\"");
        config.set_command_for_extension("rs", "cargo run");
        let serialized = config.to_toml();
        let parsed = ProjectConfig::parse(&serialized);
        assert_eq!(parsed.workspace_command.as_deref(), Some("make run"));
        assert_eq!(parsed.command_for_extension("c"), Some("clang \"${file}\""));
        assert_eq!(parsed.command_for_extension("rs"), Some("cargo run"));
        assert_eq!(parsed.compiler, "clang");
        assert_eq!(parsed.include_dirs, vec![PathBuf::from("include")]);
    }
}
