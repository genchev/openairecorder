//! Verified, cross-volume workspace relocation. The locator is committed last;
//! before that point all errors leave the source authoritative and untouched.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct Location {
    root: PathBuf,
}

pub fn location_file() -> PathBuf {
    if let Some(root) = std::env::var_os("RECORDER_DATA_DIR") {
        return PathBuf::from(root).join("workspace-location.json");
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("OpenAIRecorder/workspace-location.json");
    }
    crate::app::default_data_root().join("workspace-location.json")
}

pub fn resolve(initial: &Path, locator: &Path) -> Result<PathBuf> {
    match fs::read(locator) {
        Ok(bytes) => {
            let location: Location = serde_json::from_slice(&bytes)
                .context("Cannot read workspace location settings")?;
            ensure!(
                location.root.is_absolute() && location.root.is_dir(),
                "Saved workspace is unavailable: {}. Reconnect its disk or restore that folder before opening the app.",
                location.root.display()
            );
            Ok(location.root)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(initial.to_owned()),
        Err(e) => Err(e).context("Cannot read workspace location settings"),
    }
}

pub struct Moved {
    pub root: PathBuf,
    pub warning: Option<String>,
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        } // Reparse points / junctions.
    }
    false
}

fn entries(root: &Path, skip: &Path) -> Result<Vec<PathBuf>> {
    fn walk(root: &Path, dir: &Path, skip: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path == skip {
                continue;
            }
            let meta = fs::symlink_metadata(&path)?;
            ensure!(
                !is_link(&meta),
                "Linked files or junctions cannot be moved: {}",
                path.display()
            );
            ensure!(
                meta.is_file() || meta.is_dir(),
                "Unsupported workspace entry: {}",
                path.display()
            );
            found.push(path.strip_prefix(root)?.to_owned());
            if meta.is_dir() {
                walk(root, &path, skip, found)?;
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(root, root, skip, &mut found)?;
    found.sort();
    Ok(found)
}
fn same_bytes(a: &Path, b: &Path) -> Result<bool> {
    if fs::metadata(a)?.len() != fs::metadata(b)?.len() {
        return Ok(false);
    }
    let mut a = fs::File::open(a)?;
    let mut b = fs::File::open(b)?;
    let mut left = vec![0; 1024 * 1024];
    let mut right = vec![0; 1024 * 1024];
    loop {
        let n = a.read(&mut left)?;
        if n == 0 {
            return Ok(b.read(&mut right[..1])? == 0);
        }
        b.read_exact(&mut right[..n])?;
        if left[..n] != right[..n] {
            return Ok(false);
        }
    }
}
fn write_location(locator: &Path, target: &Path) -> Result<()> {
    let temp = locator.with_file_name(format!(".workspace-location-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(&Location {
            root: target.to_owned(),
        })?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, locator).context("Cannot save the new workspace location")?;
        // No fallible work after the locator commit: the target is now authoritative.
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn relocate(source: &Path, target: &Path, locator: &Path) -> Result<Moved> {
    let source = fs::canonicalize(source).context("Cannot access current workspace")?;
    let target = fs::canonicalize(target).context("Choose an existing empty folder")?;
    ensure!(target.is_dir(), "Choose a folder for the workspace");
    ensure!(
        !target.starts_with(&source) && !source.starts_with(&target),
        "Choose a separate folder, outside the current workspace and its parent folders"
    );
    let locator_parent = locator
        .parent()
        .context("Invalid workspace settings location")?;
    fs::create_dir_all(locator_parent)?;
    let locator_parent = fs::canonicalize(locator_parent)?;
    let locator = locator_parent.join(locator.file_name().context("Invalid location file")?);
    ensure!(
        !locator_parent.starts_with(&target) || locator.parent() == Some(target.as_path()),
        "The destination cannot contain the original configuration folder"
    );
    for entry in fs::read_dir(&target)? {
        ensure!(
            entry?.path() == locator,
            "Choose an empty folder; existing files will not be merged or overwritten"
        );
    }
    let files = entries(&source, &locator)?;
    let stage = target.join(format!(".openairecorder-move-{}", Uuid::new_v4()));
    fs::create_dir(&stage)?;
    let mut published = Vec::new();
    let prepare = (|| -> Result<()> {
        for relative in &files {
            let from = source.join(relative);
            let to = stage.join(relative);
            if fs::metadata(&from)?.is_dir() {
                fs::create_dir(&to)?;
            } else {
                let mut options = fs::OpenOptions::new();
                options.create_new(true).write(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut output = options
                    .open(&to)
                    .with_context(|| format!("Cannot copy {}", relative.display()))?;
                std::io::copy(&mut fs::File::open(&from)?, &mut output)?;
                output.sync_all()?;
                drop(output);
                fs::set_permissions(&to, fs::metadata(&from)?.permissions())?;
            }
        }
        ensure!(
            entries(&source, &locator)? == files,
            "Workspace changed during the move; try again when it is idle"
        );
        for relative in &files {
            let from = source.join(relative);
            let to = stage.join(relative);
            if fs::metadata(&from)?.is_file() {
                ensure!(
                    same_bytes(&from, &to)?,
                    "Copy verification failed for {}",
                    relative.display()
                );
            }
        }
        // A stale preview/background refresh must not write to the old root after
        // switching; the caller holds State::jobs and invalidates its GUI epoch.
        for entry in fs::read_dir(&stage)? {
            let path = entry?.path();
            let dest = target.join(path.file_name().unwrap());
            ensure!(
                !dest.exists(),
                "Destination changed during the move; original workspace retained"
            );
            fs::rename(&path, &dest)?;
            published.push(dest);
        }
        write_location(&locator, &target)?;
        Ok(())
    })();
    if let Err(error) = prepare {
        for path in &published {
            if path.is_dir() {
                let _ = fs::remove_dir_all(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
        let _ = fs::remove_dir_all(&stage);
        return Err(error).context("Move failed; the original workspace is still active");
    }
    let _ = fs::remove_dir(&stage);
    let cleanup = (|| -> Result<()> {
        // Remove only verified, unchanged files, never new files written by an
        // external editor during the move. Empty-directory removal is deliberate.
        for relative in files.iter().rev() {
            let old = source.join(relative);
            let new = target.join(relative);
            if fs::symlink_metadata(&old)?.is_dir() {
                fs::remove_dir(&old)?;
            } else {
                ensure!(
                    same_bytes(&old, &new)?,
                    "An old file changed during the move and was retained: {}",
                    old.display()
                );
                fs::remove_file(&old)?;
            }
        }
        if !locator.starts_with(&source) {
            fs::remove_dir(&source)?;
        }
        Ok(())
    })();
    let warning = cleanup.err().map(|e| {
        format!(
            "The new workspace is active, but some old files remain at {}: {e}",
            source.display()
        )
    });
    Ok(Moved {
        root: target,
        warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("recorder-move-{}", Uuid::new_v4()));
        let source = base.join("original");
        let target = base.join("other-disk-location");
        let locator = source.join("workspace-location.json");
        fs::create_dir_all(source.join("session/chunks")).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(source.join("session/audio"), b"audio samples").unwrap();
        fs::write(source.join("session/chunks/000001.wav"), b"pending audio").unwrap();
        fs::write(
            source.join("session/notes.md"),
            "# Бележки\n- [ ] Follow up",
        )
        .unwrap();
        fs::write(source.join("global-actions.json"), b"[]").unwrap();
        fs::write(source.join("settings.json"), br#"{"theme":"dark"}"#).unwrap();
        crate::secrets::save(&source, "fixture-key-not-real").unwrap();
        (base, source, target, locator)
    }
    #[test]
    fn relocation_preserves_all_data_and_key_and_remembers_multiple_moves() {
        let (base, source, target, locator) = setup();
        let result = relocate(&source, &target, &locator).unwrap();
        assert!(result.warning.is_none());
        assert_eq!(result.root, fs::canonicalize(&target).unwrap());
        assert_eq!(resolve(&source, &locator).unwrap(), result.root);
        assert_eq!(
            fs::read_to_string(target.join("session/notes.md")).unwrap(),
            "# Бележки\n- [ ] Follow up"
        );
        assert_eq!(
            fs::read(target.join("session/chunks/000001.wav")).unwrap(),
            b"pending audio"
        );
        assert_eq!(
            crate::secrets::load(&target).unwrap(),
            "fixture-key-not-real"
        );
        assert_eq!(fs::read_dir(&source).unwrap().count(), 1); // Only the locator remains.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(target.join("gemini-key.bin"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let third = base.join("third");
        fs::create_dir(&third).unwrap();
        relocate(&target, &third, &locator).unwrap();
        assert!(!target.exists());
        assert_eq!(
            resolve(&source, &locator).unwrap(),
            fs::canonicalize(&third).unwrap()
        );
        // Moving back to the original location must preserve its locator.
        relocate(&third, &source, &locator).unwrap();
        assert!(!third.exists());
        assert_eq!(
            resolve(&source, &locator).unwrap(),
            fs::canonicalize(&source).unwrap()
        );
        assert_eq!(
            fs::read(source.join("session/audio")).unwrap(),
            b"audio samples"
        );
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn fixed_app_configuration_outside_the_workspace_survives_moves() {
        let (base, source, target, _) = setup();
        let locator = base.join("app-config/workspace-location.json");
        relocate(&source, &target, &locator).unwrap();
        assert!(!source.exists());
        assert!(locator.exists());
        assert_eq!(
            resolve(&source, &locator).unwrap(),
            fs::canonicalize(&target).unwrap()
        );
        assert_eq!(
            crate::secrets::load(&target).unwrap(),
            "fixture-key-not-real"
        );
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn destinations_cannot_merge_overwrite_or_nest_the_workspace() {
        let (base, source, target, locator) = setup();
        fs::write(target.join("keep.txt"), b"unrelated").unwrap();
        assert!(relocate(&source, &target, &locator).is_err());
        assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"unrelated");
        assert!(relocate(&source, &source, &locator).is_err());
        assert!(relocate(&source, &source.join("session"), &locator).is_err());
        assert!(relocate(&source, &base, &locator).is_err());
        assert!(source.join("session/audio").exists());
        assert!(!locator.exists());
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn failed_location_commit_rolls_back_copied_files_without_deleting_source() {
        let (base, source, target, locator) = setup();
        fs::create_dir(&locator).unwrap(); // Forces the final atomic file rename to fail.
        assert!(relocate(&source, &target, &locator).is_err());
        assert!(source.join("session/audio").exists());
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(
            crate::secrets::load(&source).unwrap(),
            "fixture-key-not-real"
        );
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn missing_or_invalid_saved_location_never_silently_creates_an_empty_workspace() {
        let (base, source, target, locator) = setup();
        assert_eq!(resolve(&source, &locator).unwrap(), source);
        relocate(&source, &target, &locator).unwrap();
        fs::rename(&target, base.join("offline-drive")).unwrap();
        assert!(resolve(&source, &locator).is_err());
        fs::write(&locator, b"invalid config").unwrap();
        assert!(resolve(&source, &locator).is_err());
        fs::remove_dir_all(base).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn linked_files_are_rejected_without_copying_outside_the_workspace() {
        let (base, source, target, locator) = setup();
        fs::write(base.join("outside"), b"unrelated").unwrap();
        std::os::unix::fs::symlink(base.join("outside"), source.join("linked")).unwrap();
        assert!(relocate(&source, &target, &locator).is_err());
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(fs::read(base.join("outside")).unwrap(), b"unrelated");
        fs::remove_dir_all(base).unwrap();
    }
}
