//! Helpers shared by the unit tests.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A scratch directory removed on drop, so tests need no extra crate.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = env::temp_dir().join(format!(
            "nexus-menu-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Create a valid Wine prefix at `rel`.
    pub fn prefix(&self, rel: &str) -> PathBuf {
        let dir = self.0.join(rel);
        fs::create_dir_all(dir.join("drive_c")).unwrap();
        fs::write(dir.join("system.reg"), "WINE REGISTRY Version 2\n").unwrap();
        dir
    }

    /// Create a file at `rel`, making any missing parent directories.
    pub fn file(&self, rel: &str, contents: &str) -> PathBuf {
        self.file_bytes(rel, contents.as_bytes())
    }

    /// Like [`Scratch::file`], for contents that are not text.
    pub fn file_bytes(&self, rel: &str, contents: &[u8]) -> PathBuf {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
