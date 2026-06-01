use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let scintilla = root.join("vendor/scintilla");
    let lexilla = root.join("vendor/lexilla");
    assert!(
        scintilla.join("gtk").is_dir() && lexilla.join("src").is_dir(),
        "native sources are missing; run ./scripts/fetch-native.sh from the repository root"
    );
    run_make(&scintilla.join("gtk"), &["GTK3=1", "static"]);
    run_make(&lexilla.join("src"), &["../bin/liblexilla.a"]);

    println!("cargo:rustc-link-search=native={}", scintilla.join("bin").display());
    println!("cargo:rustc-link-search=native={}", lexilla.join("bin").display());
    println!("cargo:rustc-link-lib=static:+verbatim=scintilla.a");
    println!("cargo:rustc-link-lib=static=lexilla");
    println!("cargo:rustc-link-lib=dylib=stdc++");
    for library in pkg_config_libs("gtk+-3.0", "gmodule-no-export-2.0") {
        println!("cargo:rustc-link-lib=dylib={library}");
    }
    println!("cargo:rerun-if-changed={}", scintilla.display());
    println!("cargo:rerun-if-changed={}", lexilla.display());
}

fn run_make(directory: &Path, arguments: &[&str]) {
    let status = Command::new("make")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .status()
        .unwrap_or_else(|error| panic!("failed to run make in {}: {error}", directory.display()));
    assert!(status.success(), "make failed in {}", directory.display());
}

fn pkg_config_libs(packages: &str, extra: &str) -> Vec<String> {
    let output = Command::new("pkg-config")
        .args(["--libs-only-l", packages, extra])
        .output()
        .expect("failed to run pkg-config");
    assert!(output.status.success(), "GTK3 development libraries are required");
    String::from_utf8(output.stdout)
        .expect("pkg-config output was not UTF-8")
        .split_whitespace()
        .map(|flag| flag.trim_start_matches("-l").to_string())
        .collect()
}
