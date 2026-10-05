//! Build script: embeds every file in `site/` into the server executable.
//!
//! It writes `embedded_assets.rs` into Cargo's `OUT_DIR`. That file defines
//! `EMBEDDED_ASSETS`, a table of paths and `include_bytes!` calls.
//! `server/main.rs` includes it.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"));
    let site_dir = manifest_dir.join("site");
    let entrypoint = site_dir.join("index.html");

    if !entrypoint.is_file() {
        panic!(
            "{} is missing; the checked-in browser assets are required",
            entrypoint.display()
        );
    }

    // Run this script again when a file in `site/` changes.
    println!("cargo:rerun-if-changed={}", site_dir.display());

    let mut assets = Vec::new();
    collect_assets(&site_dir, &site_dir, &mut assets);
    // Sort by path, so that every build generates the same table.
    assets.sort_by(|left, right| left.0.cmp(&right.0));

    let mut generated = String::from("static EMBEDDED_ASSETS: &[EmbeddedAsset] = &[\n");
    for (relative_path, absolute_path) in assets {
        let absolute_path = absolute_path
            .to_str()
            .expect("asset paths must be valid UTF-8");
        generated.push_str(&format!(
            "    EmbeddedAsset {{ path: {relative_path:?}, bytes: include_bytes!({absolute_path:?}) }},\n"
        ));
    }
    generated.push_str("];\n");

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set"))
        .join("embedded_assets.rs");
    fs::write(output, generated).expect("failed to generate the embedded asset table");
}

/// Add every file below `directory` to `assets`, as a pair of the path
/// relative to `root` (with `/` as separator) and the absolute path.
fn collect_assets(root: &Path, directory: &Path, assets: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| entry.expect("cannot read an asset directory entry"))
        .collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_assets(root, &path, assets);
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("asset must be inside the site directory")
                .to_string_lossy()
                .replace('\\', "/");
            assets.push((relative, path));
        }
    }
}
