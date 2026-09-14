use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"),
    );
    let site_dir = manifest_dir.join("site");
    let entrypoint = site_dir.join("index.html");

    if !entrypoint.is_file() {
        panic!(
            "{} is missing; the checked-in browser assets are required",
            entrypoint.display()
        );
    }

    println!("cargo:rerun-if-changed={}", site_dir.display());

    let mut assets = Vec::new();
    collect_assets(&site_dir, &site_dir, &mut assets);
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

