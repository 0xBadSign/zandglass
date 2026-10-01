//! Owner-only file helpers. Permissions are set when the file or directory is
//! created (mode `0700` / `0600` on Unix), never by a chmod afterwards, so
//! there is no window in which a secret is readable by others.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

/// Creates `dir` and any missing parents, each with mode `0700` on Unix.
pub fn create_private_dir(dir: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))
}

fn private_options() -> OpenOptions {
    let mut opts = OpenOptions::new();
    opts.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts
}

/// Creates a new file with mode `0600` and writes `data`. Fails when the
/// file already exists.
pub fn write_new_private(path: &Path, data: &[u8]) -> Result<()> {
    let mut f = private_options()
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(data)
        .with_context(|| format!("writing {}", path.display()))
}

/// Creates an empty file with mode `0600` unless it exists already.
pub fn touch_private(path: &Path) -> Result<()> {
    match private_options().create_new(true).open(path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e).with_context(|| format!("creating {}", path.display())),
    }
}

/// Writes `data` to `path` atomically (temporary file, then rename) with mode
/// `0600` from the moment the file exists.
pub fn write_private_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    write_new_private(&tmp, data)?;
    fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn dirs_and_files_are_private_from_creation() {
        let t = tempfile::tempdir().unwrap();
        let deep = t.path().join("a/b/c");
        create_private_dir(&deep).unwrap();
        assert_eq!(mode(&deep), 0o700);
        assert_eq!(mode(&t.path().join("a")), 0o700);
        let f = deep.join("x");
        write_private_atomic(&f, b"1").unwrap();
        assert_eq!(mode(&f), 0o600);
        write_private_atomic(&f, b"2").unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"2");
        assert_eq!(mode(&f), 0o600);
        assert!(write_new_private(&f, b"3").is_err());
        let g = deep.join("db");
        touch_private(&g).unwrap();
        touch_private(&g).unwrap();
        assert_eq!(mode(&g), 0o600);
    }
}
