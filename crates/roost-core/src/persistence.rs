use crate::error::{Error, Result};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Replaces `path` with `bytes` after synchronizing a unique temporary file
/// created in the destination directory.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(io_error)?;

    let (temp_path, mut temp_file) =
        create_temp_file(parent, path.file_name().unwrap_or_default())?;
    let result = (|| {
        temp_file.write_all(bytes).map_err(io_error)?;
        temp_file.sync_all().map_err(io_error)?;
        drop(temp_file);
        fs::rename(&temp_path, path).map_err(io_error)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn create_temp_file(parent: &Path, filename: &OsStr) -> Result<(PathBuf, File)> {
    let pid = std::process::id();
    loop {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = filename.to_os_string();
        temp_name.push(format!(".roost-{pid}-{counter}"));
        let temp_path = parent.join(temp_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error(error)),
        }
    }
}

fn io_error(error: std::io::Error) -> Error {
    Error::Io(error.to_string())
}
