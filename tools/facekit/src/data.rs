//! The photo folders the commands read, and what to say when one is
//! missing.
//!
//! The photos are not part of the repository: they are large, and anyone
//! can download them again. A command that needs them therefore checks
//! its folder first and, when the photos are not there, says where the
//! instructions for getting them are, instead of failing with a bare
//! "No such file or directory".

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

/// Where the instructions for getting the photos are, from the
/// repository root.
pub const INSTRUCTIONS: &str = "tools/facekit/data/README.md";

/// The photos (`jpg`, `jpeg`, `png`) directly in `dir`, sorted by name.
///
/// # Errors
///
/// When the folder does not exist or holds no photo; the message names
/// the instructions for getting the photos.
pub fn photos(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut photos: Vec<PathBuf> = entries(dir)?
        .into_iter()
        .filter(|path| is_photo(path))
        .collect();
    if photos.is_empty() {
        bail!(missing(dir, "holds no photos (jpg, jpeg or png)"));
    }
    photos.sort();
    Ok(photos)
}

/// The subfolders of `dir`, one per person, sorted by name.
///
/// # Errors
///
/// When the folder does not exist or has no subfolder; the message names
/// the instructions for getting the photos.
pub fn people(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut people: Vec<PathBuf> = entries(dir)?
        .into_iter()
        .filter(|path| path.is_dir())
        .collect();
    if people.is_empty() {
        bail!(missing(dir, "holds no folders of photos, one per person"));
    }
    people.sort();
    Ok(people)
}

/// Whether `path` has the extension of a photo.
pub fn is_photo(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png"
            )
        })
}

/// Everything directly in `dir`.
fn entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let Ok(listing) = fs::read_dir(dir) else {
        bail!(missing(dir, "does not exist or cannot be read"));
    };
    Ok(listing
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect())
}

/// The message for a folder without the photos a command needs.
fn missing(dir: &Path, problem: &str) -> String {
    format!(
        "{} {problem}.\n\
         The photos are not part of the repository. {INSTRUCTIONS} says \
         how to download or make them.",
        dir.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty folder of its own for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("facekit-data-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_folder_points_to_the_instructions() {
        let error = photos(Path::new("no/such/folder")).unwrap_err().to_string();
        assert!(error.contains("no/such/folder"), "{error}");
        assert!(error.contains(INSTRUCTIONS), "{error}");
        let error = people(Path::new("no/such/folder")).unwrap_err().to_string();
        assert!(error.contains(INSTRUCTIONS), "{error}");
    }

    #[test]
    fn an_empty_folder_points_to_the_instructions() {
        let dir = scratch("empty");
        fs::write(dir.join("notes.txt"), "not a photo").unwrap();
        let error = photos(&dir).unwrap_err().to_string();
        assert!(error.contains(INSTRUCTIONS), "{error}");
        let error = people(&dir).unwrap_err().to_string();
        assert!(error.contains(INSTRUCTIONS), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn photos_and_people_are_found_and_sorted() {
        let dir = scratch("found");
        for name in ["b.JPG", "a.png", "c.txt"] {
            fs::write(dir.join(name), "").unwrap();
        }
        for name in ["Zoe", "Adam"] {
            fs::create_dir(dir.join(name)).unwrap();
        }
        assert_eq!(
            photos(&dir).unwrap(),
            [dir.join("a.png"), dir.join("b.JPG")]
        );
        assert_eq!(people(&dir).unwrap(), [dir.join("Adam"), dir.join("Zoe")]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
