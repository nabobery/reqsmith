//! Canonical atomic, symlink-refusing file writes.
//!
//! Every on-disk write reqsmith performs for user data — run snapshots, binary
//! downloads, and (re)written request/formatter files — goes through this
//! module instead of `std::fs::write` so a single, audited implementation
//! owns the filesystem-safety policy:
//!
//! * **Never follow a symlink at the destination.** A predictable output
//!   filename (e.g. a run snapshot) is a classic symlink-swap target: point
//!   it at `~/.ssh/authorized_keys` and a naive writer clobbers that file.
//!   We refuse outright if the destination path is itself a symlink, using
//!   `symlink_metadata` (which — unlike `Path::exists()` — does not follow
//!   the link and so also catches *dangling* symlinks).
//! * **Atomic content.** Data is written to a fresh temp file in the same
//!   directory, `fsync`ed, then `rename`d into place. A reader never observes
//!   a half-written file, and (`std::fs::rename` maps to `MoveFileExW` with
//!   `MOVEFILE_REPLACE_EXISTING` on Windows) the rename replaces the
//!   destination directory entry rather than writing through it.
//! * **Race-free create-new.** [`write_new`] reserves the destination name
//!   with `create_new` *before* the rename, so two processes racing for the
//!   same snapshot filename can't silently overwrite each other — the loser
//!   sees [`std::io::ErrorKind::AlreadyExists`] and picks another name.
//! * **No leaked temp files.** The temp file (and, for `write_new`, the
//!   reservation) is removed on any write/rename failure.
//! * **Crash durability.** After the rename we best-effort `fsync` the
//!   containing directory so the new entry survives a power loss.
//!
//! What this module deliberately does *not* defend against: a fully
//! attacker-controlled *parent directory tree* (e.g. `.reqsmith/runs` swapped for
//! a symlink to a hostile location). Portably validating every path component
//! without a TOCTOU needs `openat`/`O_NOFOLLOW` directory handles that `std`
//! doesn't expose. Callers that create those directories (see
//! [`crate::core::storage`]) add a cheap check that the immediate parent isn't
//! a symlink; the broader hostile-parent case is out of scope and documented
//! in `docs/security-model.md`.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Whether an existing destination may be replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Fail with [`io::ErrorKind::AlreadyExists`] if the destination already
    /// exists as a regular file; reserve the name atomically otherwise.
    CreateNew,
    /// Replace an existing regular file atomically (but never a symlink).
    Overwrite,
}

/// Atomically create a new file at `path`, failing if it already exists.
///
/// Returns [`io::ErrorKind::AlreadyExists`] if `path` already names a regular
/// file (callers that generate collision-suffixed names loop on this), and a
/// non-`AlreadyExists` error if `path` is a symlink (that is always fatal — we
/// never follow it).
pub fn write_new(path: &Path, data: &[u8]) -> io::Result<()> {
    write_impl(path, data, Mode::CreateNew)
}

/// Atomically write `data` to `path`, replacing an existing regular file.
///
/// Refuses (non-`AlreadyExists` error) if `path` is a symlink.
pub fn write_replace(path: &Path, data: &[u8]) -> io::Result<()> {
    write_impl(path, data, Mode::Overwrite)
}

fn symlink_refused(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "refusing to write through pre-existing symlink at {}",
            path.display()
        ),
    )
}

fn write_impl(path: &Path, data: &[u8], mode: Mode) -> io::Result<()> {
    // 1. Never follow a symlink at the destination (both modes). For
    //    CreateNew, a pre-existing regular file is a collision.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => return Err(symlink_refused(path)),
        Ok(_) if mode == Mode::CreateNew => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("destination already exists: {}", path.display()),
            ));
        }
        _ => {}
    }

    let dir = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "destination path has no parent directory: {}",
                path.display()
            ),
        )
    })?;
    let file_name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "destination path has no valid file name: {}",
                path.display()
            ),
        )
    })?;

    // 2. For CreateNew, atomically reserve the destination name to close the
    //    TOCTOU between the existence check above and the rename below. Two
    //    racing writers: only one wins `create_new`; the other gets
    //    AlreadyExists and can pick a different name.
    let mut reserved = false;
    if mode == Mode::CreateNew {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(_) => reserved = true,
            // `create_new` also rejects an existing symlink with EEXIST; we've
            // already handled that above, so any AlreadyExists here is a real
            // collision.
            Err(e) => return Err(e),
        }
    }

    // 3. Write the payload to a unique temp file in the same directory.
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = dir.join(format!(".{file_name}.tmp-{}-{unique}", std::process::id()));

    let write_result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(data)?;
        file.sync_all()?;
        Ok(())
    })();

    // 4. Move the temp file into place, cleaning up on any failure so we never
    //    leak a temp file or a stranded reservation.
    let result = write_result.and_then(|()| std::fs::rename(&tmp_path, path));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp_path);
        if reserved {
            let _ = std::fs::remove_file(path);
        }
        return Err(e);
    }

    // 5. Best-effort fsync of the containing directory so the rename is
    //    durable across a crash. Opening a directory as a File is not
    //    supported on Windows, so treat any failure here as non-fatal.
    if let Ok(dir_file) = File::open(dir) {
        let _ = dir_file.sync_all();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_new_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.json");
        write_new(&path, b"hello").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn write_new_refuses_existing_regular_file_with_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.json");
        write_new(&path, b"first").unwrap();
        let err = write_new(&path, b"second").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        // Original content untouched.
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
    }

    #[test]
    fn write_replace_overwrites_existing_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.json");
        write_replace(&path, b"first").unwrap();
        write_replace(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
    }

    #[test]
    fn no_temp_files_left_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.json");
        write_new(&path, b"x").unwrap();
        write_replace(&path, b"y").unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn write_new_refuses_symlink_destination_without_following_it() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        std::fs::write(&real, b"original").unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let err = write_new(&link, b"attacker").unwrap_err();
        assert_ne!(
            err.kind(),
            io::ErrorKind::AlreadyExists,
            "a symlink destination must be a fatal refusal, not a collision the caller retries past"
        );
        // The file behind the symlink is untouched.
        assert_eq!(std::fs::read(&real).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[test]
    fn write_replace_refuses_symlink_destination_without_following_it() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        std::fs::write(&real, b"original").unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert!(write_replace(&link, b"attacker").is_err());
        assert_eq!(std::fs::read(&real).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[test]
    fn write_new_refuses_dangling_symlink_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("dangling");
        std::os::unix::fs::symlink(tmp.path().join("nowhere"), &link).unwrap();

        let err = write_new(&link, b"data").unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::AlreadyExists);
        // The symlink itself is left intact, not replaced.
        let meta = std::fs::symlink_metadata(&link).unwrap();
        assert!(meta.file_type().is_symlink());
    }
}
