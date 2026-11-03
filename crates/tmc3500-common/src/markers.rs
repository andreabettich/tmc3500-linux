//! Partial-send markers: `<state_dir>/partial-job-<id>` records that a job's data was already
//! (partly) sent to the printer. A job that comes back with a marker is held instead of being
//! sent again (cupsd restarts interrupted jobs from the start: that would be a silent duplicate).
//! `tmc3500ctl release-held-jobs` removes the marker when an operator releases the job.
use crate::alert::write_atomic_mode;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Marker file for `job_id` (characters outside `[A-Za-z0-9_-]` are replaced by `_`).
pub fn marker_path(state_dir: &Path, job_id: &str) -> PathBuf {
    let id: String = job_id
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    state_dir.join(format!("partial-job-{id}"))
}

pub fn write_marker(state_dir: &Path, job_id: &str, text: &str) -> std::io::Result<()> {
    write_atomic_mode(&marker_path(state_dir, job_id), text.as_bytes(), 0o660)
}

/// The marker text, if a marker exists (a symlink is treated as a marker too, never followed).
pub fn read_marker(state_dir: &Path, job_id: &str) -> Option<String> {
    let p = marker_path(state_dir, job_id);
    let md = std::fs::symlink_metadata(&p).ok()?;
    if !md.is_file() {
        return Some(String::from("(unreadable marker)"));
    }
    let mut s = String::new();
    match std::fs::File::open(&p) {
        Ok(f) => {
            let _ = f.take(4096).read_to_string(&mut s);
        }
        Err(_) => s.push_str("(unreadable marker)"),
    }
    Some(s)
}

pub fn remove_marker(state_dir: &Path, job_id: &str) -> std::io::Result<()> {
    match std::fs::remove_file(marker_path(state_dir, job_id)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_remove() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(read_marker(d.path(), "42"), None);
        write_marker(d.path(), "42", "job 42 partially sent (10 bytes)").unwrap();
        assert!(read_marker(d.path(), "42").unwrap().contains("10 bytes"));
        assert_eq!(
            marker_path(d.path(), "../x").file_name().unwrap(),
            "partial-job-___x"
        );
        remove_marker(d.path(), "42").unwrap();
        remove_marker(d.path(), "42").unwrap();
        assert_eq!(read_marker(d.path(), "42"), None);
    }
}
