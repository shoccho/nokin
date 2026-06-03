#[cfg(target_arch = "wasm32")]
compile_error!("Nokin is a native desktop application; WebAssembly builds are unsupported");

pub mod completion;
pub mod config;
pub mod edit;
pub mod index;
pub mod lsp;
pub mod run;
pub mod theme;
pub mod ui;
#[cfg(feature = "gtk-ui")]
pub mod vte;
pub mod workspace;
