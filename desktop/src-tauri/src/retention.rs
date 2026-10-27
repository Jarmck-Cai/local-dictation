//! Audio expiry is independent of history rows, favorites, and history limits.

use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

pub const AUDIO_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Default)]
pub struct CleanupReport {
    pub deleted: usize,
    pub failed: usize,
    pub skipped_links: usize,
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes directory junctions, which are not all reported as symlinks.
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

fn is_audio_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    [
        ".wav",
        ".pcm",
        ".raw",
        ".tmp",
        ".wav.part",
        ".pcm.part",
        ".raw.part",
    ]
    .iter()
    .any(|extension| name.ends_with(extension))
}

/// Scan only the dedicated recording tree. Links/reparse points are never followed.
/// Future modification times are retained until they reach the normal seven-day age.
pub fn cleanup_audio_directory(directory: &Path, now: SystemTime) -> io::Result<CleanupReport> {
    if !directory.try_exists()? {
        return Ok(CleanupReport::default());
    }
    // Reject a redirected root (or a redirected ancestor) as well as links within it.
    // Checking the lexical ancestors avoids canonicalizing through a junction first.
    for ancestor in directory
        .ancestors()
        .filter(|path| !path.as_os_str().is_empty())
    {
        if is_link(&fs::symlink_metadata(ancestor)?) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Audio directory contains a link or reparse point",
            ));
        }
    }
    let root = fs::canonicalize(directory)?;
    let mut report = CleanupReport::default();
    cleanup_tree(&root, &root, now, &mut report)?;
    Ok(report)
}

fn cleanup_tree(
    root: &Path,
    directory: &Path,
    now: SystemTime,
    report: &mut CleanupReport,
) -> io::Result<()> {
    // On Windows, keep a non-delete-shared handle while walking a directory so
    // another process cannot rename/replace that directory underneath this scan.
    #[cfg(windows)]
    let _directory_guard = {
        use std::os::windows::fs::OpenOptionsExt;
        let handle = fs::OpenOptions::new()
            .access_mode(0x80) // FILE_READ_ATTRIBUTES
            .share_mode(0x3) // FILE_SHARE_READ | FILE_SHARE_WRITE, deliberately no DELETE
            .custom_flags(0x0220_0000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
            .open(directory)?;
        if is_link(&handle.metadata()?) {
            report.skipped_links += 1;
            return Ok(());
        }
        handle
    };

    if !fs::canonicalize(directory)?.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Audio path escaped the recording directory",
        ));
    }
    for entry in fs::read_dir(directory)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                report.failed += 1;
                continue;
            }
        };
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                report.failed += 1;
                continue;
            }
        };
        if is_link(&metadata) {
            report.skipped_links += 1;
            continue;
        }
        if metadata.is_dir() {
            if cleanup_tree(root, &path, now, report).is_err() {
                report.failed += 1;
            }
        } else if metadata.is_file() && is_audio_file(&path) {
            let expired = metadata
                .modified()
                .ok()
                .and_then(|mtime| now.duration_since(mtime).ok())
                .is_some_and(|age| age >= AUDIO_RETENTION);
            if expired {
                if fs::remove_file(&path).is_ok() {
                    report.deleted += 1;
                } else {
                    report.failed += 1;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, FileTimes};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "dictation-retention-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn audio(&self, relative: &str, modified: SystemTime) -> PathBuf {
            let p = self.0.join(relative);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            let file = File::create(&p).unwrap();
            file.set_times(FileTimes::new().set_modified(modified))
                .unwrap();
            p
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn orphaned_and_segment_audio_expires_without_history_rows() {
        let dir = TempDir::new();
        let now = SystemTime::now();
        let old = now - AUDIO_RETENTION - Duration::from_secs(1);
        let orphan = dir.audio("orphan.wav", old);
        let segment = dir.audio("segments/aborted.wav.tmp", old);
        let report = cleanup_audio_directory(&dir.0, now).unwrap();
        assert_eq!(report.deleted, 2);
        assert!(!orphan.exists());
        assert!(!segment.exists());
    }

    #[test]
    fn future_mtime_and_young_audio_are_preserved() {
        let dir = TempDir::new();
        let now = SystemTime::now();
        let future = dir.audio("future.wav", now + Duration::from_secs(3600));
        let young = dir.audio("young.wav", now - Duration::from_secs(60));
        let notes = dir.audio("notes.txt", now - AUDIO_RETENTION - Duration::from_secs(1));
        cleanup_audio_directory(&dir.0, now).unwrap();
        assert!(future.exists());
        assert!(young.exists());
        assert!(notes.exists());
    }

    #[cfg(windows)]
    #[test]
    fn directory_junction_cannot_extend_the_cleanup_scope() {
        use std::process::Command;
        let dir = TempDir::new();
        let outside = TempDir::new();
        let now = SystemTime::now();
        let protected = outside.audio(
            "outside.wav",
            now - AUDIO_RETENTION - Duration::from_secs(1),
        );
        let junction = dir.0.join("segments");
        let status = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside.0)
            .status()
            .unwrap();
        assert!(status.success());
        let report = cleanup_audio_directory(&dir.0, now).unwrap();
        assert!(protected.exists());
        assert_eq!(report.skipped_links, 1);
        // Remove only the known junction, so TempDir teardown cannot traverse it.
        fs::remove_dir(junction).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cannot_extend_the_cleanup_scope() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new();
        let outside = TempDir::new();
        let now = SystemTime::now();
        let protected = outside.audio(
            "outside.wav",
            now - AUDIO_RETENTION - Duration::from_secs(1),
        );
        symlink(&outside.0, dir.0.join("segments")).unwrap();
        let report = cleanup_audio_directory(&dir.0, now).unwrap();
        assert!(protected.exists());
        assert_eq!(report.skipped_links, 1);
    }
}
