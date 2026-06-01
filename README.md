# Nokin

Nokin is a Linux-only GTK3 code editor prototype written in Rust. This repository contains the
application core and a native GTK3 shell. Pinned Scintilla and Lexilla sources are fetched locally
and intentionally excluded from Git.

## Current implementation

- GTK3 window with lazy file explorer, notebook editor, and vertically split terminal regions.
- Explorer single-click file selection opens a tab or focuses the existing tab for that path.
- Scintilla `5.6.2` editor widget with Lexilla `5.4.8` highlighting for common languages and
  formats, plain-text fallback, initial-file loading, line-number margin, and fractional font
  sizing.
- Geany-derived `abc-dark` styling for every buffer, plus C token and local-function styling,
  folding markers, lexer fold properties, indentation guides, tab indentation, backspace unindent,
  and brace-aware newline indentation.
- Basic File, Edit, View, Build, and Navigate application menus.
- Runtime-loaded GTK3 VTE `2.91` boundary with an `abc-dark` terminal palette, persistent
  workspace-rooted shell spawn, and command injection API. The UI shows a fallback when VTE is
  not installed.
- Workspace resolution from `nokin [path]`.
- User and project configuration parsing.
- Shell-escaped F5 command selection and placeholder expansion.
- `Ctrl+S` saves the active buffer, `F5` injects the selected command into the terminal,
  and `F12` or `Ctrl+click` performs syntactic C go-to-definition from the workspace symbol index.
- `Ctrl+B` toggles the explorer and `Ctrl+J` toggles the terminal panel.
- `Ctrl+D` selects the word at the caret and adds the next matching occurrence on repeated presses.
- Build-menu command configuration for the workspace fallback and the active file extension,
  persisted to project-local `.nokin.toml`.
- An Edit-menu settings dialog persists font, indentation, and shell settings.
- Paired delimiters, closing-delimiter skipping, indentation helpers, and C identifier completion.
- C workspace walking, transitive include resolution, and a Tree-sitter C symbol index.
- Lazy LSP integration for Rust through `rust-analyzer` and C/C++ through `clangd`: definitions,
  completion lists, hover calltips, references, rename workspace edits, document formatting, and
  diagnostic squiggles. Tree-sitter C navigation remains the fallback.

The visible GTK shell is usable for opening, editing, saving, and running files. Explorer
create/rename/delete actions, dirty close prompts, paired-delimiter and completion event handlers,
split synchronization, and navigation UI remain to be wired.

## Native dependencies

Install Linux development dependencies:

```sh
sudo apt install libgtk-3-dev libvte-2.91-dev pkg-config g++
```

Fetch the pinned native sources before the first build:

```sh
./scripts/fetch-native.sh
cargo build
```

The fetch script downloads the official Scintilla `5.6.2` and Lexilla `5.4.8` source archives,
verifies pinned SHA-256 checksums, and extracts them under the ignored `vendor/` directory. The
native build compiles their C++ sources into the application and keeps raw pointer handling in
dedicated audited FFI crates.
GTK3 VTE `2.91` is likewise exposed through a narrow runtime-loaded FFI module. Rust protects
application logic; GTK3, VTE, Scintilla, Lexilla, and Tree-sitter remain native-code
dependencies. GTK3 Rust bindings are intentionally avoided in the current shell.

Nokin application code is licensed under the MIT License. Fetched native dependencies retain
their upstream licenses.

## Configuration

User settings live at `~/.config/nokin/settings.toml`:

```toml
[editor]
font_family = "Monospace"
font_size = 11.0
tab_width = 4
insert_spaces = true

[terminal]
shell = "/bin/bash"

[lsp]
clangd = "clangd"
rust_analyzer = "rust-analyzer"
```

Project settings live at `.nokin.toml`:

```toml
[run]
workspace = "make run"

[run.files]
c = "cc ${file} -o /tmp/nokin-run && /tmp/nokin-run"

[c]
compiler = "cc"
include_dirs = ["include", "../shared/include"]
```

Project run commands are trusted workspace configuration and execute in the integrated shell.
Supported path placeholders are `${file}`, `${file_dir}`, and `${workspace}`.

Tree-sitter fallback navigation is syntactic and may be imperfect around preprocessing,
conditional compilation, generated headers, and compile flags. LSP shortcuts are `Ctrl+Space`
for completion, `Ctrl+Shift+Space` for signature help, `Ctrl+K` for hover, `Ctrl+.` for code
actions, `F2` for rename, and `Ctrl+Shift+I` for formatting. References, diagnostics refresh,
and semantic-token refresh are available from the Navigate menu. Server commands are configurable
in the Settings dialog. Code actions that require executing an LSP command without a workspace edit
are reported but are not executed automatically.

## Verification

```sh
cargo test
cargo run -- .
```
