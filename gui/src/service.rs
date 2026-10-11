//! What the window can ask for, with no Tauri types in it so it can be tested
//! without a window. The commands in `main.rs` are one-line wrappers.
//!
//! The page never sends a path. A scan hands out ids and the service keeps the
//! scan; creating entries takes ids back and the engine looks each one up in
//! that scan. Removing takes an id the engine listed and re-checks it against
//! the set of files it is allowed to delete.

use std::sync::{Mutex, MutexGuard};

use nexus_menu::engine::{
    self, Generated, InstallResult, Item, Options, PrefixInfo, RemoveResult, Scan, Summary,
};
use serde::Serialize;

/// What the page needs from a scan.
#[derive(Debug, Clone, Serialize)]
pub struct ScanView {
    pub items: Vec<Item>,
    pub summary: Summary,
    /// Where entries will be written, for display.
    pub target_dir: String,
}

pub struct Service {
    options: Options,
    scan: Mutex<Option<Scan>>,
}

impl Service {
    pub fn new(options: Options) -> Self {
        Service {
            options,
            scan: Mutex::new(None),
        }
    }

    /// A panic while the lock was held cannot leave the scan half-written,
    /// since it is only ever replaced whole, so a poisoned lock is still good.
    fn lock(&self) -> MutexGuard<'_, Option<Scan>> {
        self.scan.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Look at this machine, and keep the result for [`Service::install`].
    pub fn scan(&self) -> Result<ScanView, String> {
        let scan = Scan::run(&self.options).map_err(|e| format!("{e:#}"))?;
        let view = ScanView {
            items: scan.items.clone(),
            summary: scan.summary.clone(),
            target_dir: self.options.target_dir.display().to_string(),
        };
        *self.lock() = Some(scan);
        Ok(view)
    }

    pub fn install(
        &self,
        ids: &[String],
        force: bool,
        dry_run: bool,
    ) -> Result<Vec<InstallResult>, String> {
        match self.lock().as_ref() {
            Some(scan) => Ok(scan.install(ids, force, dry_run)),
            None => Err("nothing has been scanned yet".to_string()),
        }
    }

    pub fn list_generated(&self) -> Vec<Generated> {
        engine::list_generated(&self.options.target_dir)
    }

    pub fn remove(&self, ids: &[String]) -> Vec<RemoveResult> {
        engine::remove_generated(&self.options.target_dir, ids)
    }

    pub fn prefixes(&self) -> Vec<PrefixInfo> {
        engine::list_prefixes(&self.options.extra_prefixes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nexus-menu-gui-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn service_in(dir: &std::path::Path) -> Service {
        Service::new(Options {
            target_dir: dir.join("apps"),
            catalog_path: dir.join("no-such-catalog.toml"),
            ignore_nodisplay: false,
            extra_prefixes: Vec::new(),
        })
    }

    #[test]
    fn installing_before_any_scan_is_refused_and_writes_nothing() {
        let dir = scratch("noscan");
        let service = service_in(&dir);
        let result = service.install(&["native:ls".to_string()], false, false);
        assert_eq!(result.unwrap_err(), "nothing has been scanned yet");
        assert!(!dir.join("apps").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_id_the_scan_did_not_hand_out_is_refused_and_writes_nothing() {
        let dir = scratch("badid");
        let service = service_in(&dir);
        // A real scan of this machine, read-only.
        let view = service.scan().expect("scan runs");
        assert_eq!(view.target_dir, dir.join("apps").display().to_string());

        let results = service
            .install(
                &[
                    "native:no-such-program".to_string(),
                    "/etc/passwd".to_string(),
                ],
                true,
                false,
            )
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results
            .iter()
            .all(|r| r.status == engine::Status::Failed && r.path.is_none()));
        assert!(!dir.join("apps").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dry_run_of_every_scanned_item_writes_nothing() {
        let dir = scratch("dry");
        let service = service_in(&dir);
        let view = service.scan().expect("scan runs");
        let ids: Vec<String> = view.items.iter().map(|i| i.id.clone()).collect();
        let results = service.install(&ids, false, true).unwrap();
        assert_eq!(results.len(), ids.len());
        assert!(results.iter().all(|r| r.status != engine::Status::Created));
        assert!(!dir.join("apps").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_a_path_that_is_not_a_generated_entry_is_refused() {
        let dir = scratch("rm");
        let service = service_in(&dir);
        let victim = dir.join("precious.desktop");
        std::fs::write(&victim, "[Desktop Entry]\nName=Precious\n").unwrap();
        let results = service.remove(&[victim.display().to_string()]);
        assert!(!results[0].removed);
        assert!(victim.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
