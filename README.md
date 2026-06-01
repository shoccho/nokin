# Nokin

A lightweight code editor for Linux built around a fast native editor widget. Supports syntax
highlighting for most common languages, an integrated terminal, and LSP-powered code intelligence
for Rust and C/C++.

## Features

- Syntax highlighting for C, Rust, Python, JavaScript, HTML, CSS, JSON, YAML, TOML, Markdown,
  Lua, Ruby, SQL, shell scripts, and more
- Integrated terminal panel sharing the workspace directory
- LSP code intelligence: completions, hover docs, go-to-definition, find references, rename,
  inline diagnostics, and document formatting
- Go-to-definition for C even without a language server, using a workspace symbol index
- File explorer with lazy directory loading
- Tabbed editing with multi-select (Ctrl+D)
- Geany-compatible color themes — drop any Geany `.conf` theme file in and it works
- Per-project build and run commands with path placeholders
- Auto-pairing of brackets and quotes, brace-aware indentation, backspace unindent

## Requirements

```sh
sudo apt install libgtk-3-dev libvte-2.91-dev pkg-config g++
```

Also requires Rust (stable). Install from [rustup.rs](https://rustup.rs) if needed.

For LSP support: install `clangd` (C/C++) and/or `rust-analyzer` (Rust) and make sure they are
on your `PATH`.

## Building

Fetch the bundled editor library sources, then build:

```sh
./scripts/fetch-native.sh
cargo build --release
```

The binary will be at `target/release/nokin`. You can copy it anywhere on your `PATH`.

## Running

```sh
nokin                   # opens current directory as workspace
nokin /path/to/project  # opens a directory as workspace
nokin src/main.rs       # opens a single file (parent dir becomes workspace)
```

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `Ctrl+S` | Save current file |
| `F5` | Run the project command in the terminal |
| `F12` | Go to definition |
| `Ctrl+click` | Go to definition |
| `Ctrl+Space` | Trigger completion |
| `Ctrl+Shift+Space` | Show signature help |
| `Ctrl+K` | Show hover documentation |
| `Ctrl+.` | Code actions |
| `F2` | Rename symbol |
| `Ctrl+Shift+I` | Format document |
| `Ctrl+D` | Select word at cursor; repeat to add next match |
| `Ctrl+B` | Toggle file explorer |
| `Ctrl+J` | Toggle terminal panel |

References, diagnostics, and semantic token refresh are available from the Navigate menu.

## Configuration

User settings are stored at `~/.config/nokin/settings.toml`. The file is created automatically
when you save from the Settings dialog (Edit → Settings), but you can also edit it by hand:

```toml
[editor]
font_family = "Monospace"
font_size = 11.0
tab_width = 4
insert_spaces = true
theme = "tango-dark"

[workspace]
close_tabs_on_folder_open = true

[terminal]
shell = "/bin/bash"

[lsp]
clangd = "clangd"
rust_analyzer = "rust-analyzer"
```

### Project configuration

Drop a `.nokin.toml` in your project root to configure build and run commands:

```toml
[run]
workspace = "make run"

[run.files]
c = "cc ${file} -o /tmp/out && /tmp/out"
rs = "cargo run"

[c]
compiler = "cc"
include_dirs = ["include", "../shared/include"]
```

`F5` runs the command for the active file's extension, falling back to `workspace`. Commands run
in the integrated terminal. Available placeholders: `${file}`, `${file_dir}`, `${workspace}`.

## Themes

Nokin uses [Geany](https://www.geany.org/)'s color scheme format. To install a theme, place any
Geany `.conf` file in `~/.config/nokin/themes/` and select it from Edit → Settings.

A large collection of themes is available at
[github.com/geany/geany-themes](https://github.com/geany/geany-themes). Download any `.conf`
file and drop it into `~/.config/nokin/themes/`.

If you want to configure extra line spacing or caret width in a theme, add a `[styling]` section:

```ini
[styling]
line_height = 2;2
caret_width = 2
```

## License

Nokin is MIT licensed. Scintilla and Lexilla are compiled into the binary and their copyright
notice is reproduced in the `NOTICES` file as required by their license. GTK3 and VTE are
dynamically linked system libraries under LGPL v2.1+.
