# Native UI Migration

Nokin is migrating from its Linux-only GTK frontend to a native Rust desktop frontend based on
Floem. The target platforms are Windows, macOS, and Linux.

## Native-Only Rule

The application must remain a desktop-native program:

- Do not add a browser runtime, WebView, HTML renderer, JavaScript runtime, or Electron-style shell.
- Do not add a WASM frontend. Nokin deliberately rejects `wasm32` builds.
- Use Floem's desktop renderer path. On desktop, it renders through native graphics APIs via
  `wgpu`, with a software renderer available as a fallback.

Floem supports WASM as an optional upstream target, but Nokin does not expose or support that
target.

## Frontend Features

The existing GTK frontend remains the default while the migration is incomplete:

```sh
cargo run
```

The new native frontend builds without GTK or Scintilla:

```sh
cargo run --no-default-features --features native-ui -- src/main.rs
```

Only one frontend may be enabled at a time.

## Migration Sequence

1. Finish the Floem workspace browser polish. The workbench shell, explorer panel, tab strip,
   status bar, file dialogs, saving, keyboard shortcuts, and workspace navigation are wired.
2. Expand the Floem rope-backed editor integration. Themes, lexical highlighting, LSP sync,
   diagnostic counts, completion queries, definition navigation, hover, references, signature
   help, diagnostics, and formatting are wired. Completion insertion, rename, code actions, and
   semantic-token rendering remain.
3. Replace GTK VTE with a Rust terminal model based on `alacritty_terminal` and a native renderer.
4. Remove `gtk-ui`, the Scintilla adapter crates, native source fetching, and GTK build
   documentation after feature parity.

Settings and theme storage follow platform config directory conventions. CI checks the native
frontend on Linux, Windows, and macOS.
