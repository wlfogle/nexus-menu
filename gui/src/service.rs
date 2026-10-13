//! What the window can ask for, with no Tauri types in it so it can be tested
//! without a window. The commands in `main.rs` are one-line wrappers.
//!
//! The page never sends a path. A scan hands out ids and the service keeps the
//! scan; creating entries takes ids back and the engine looks each one up in
//! that scan. Editing, hiding and restoring name an entry by its desktop file
//! ID; the engine refuses an ID that could name anything else and looks the
//! file up itself, so a path from the page is never used.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard};

use nexus_menu::engine::{self, InstallResult, Item, Options, PrefixInfo, Scan, Summary};
use nexus_menu::entries::{
    self, Dirs, EntryDetail, EntryInfo, Fields, Removal, Restored, SaveError, Saved,
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

/// What happened to one entry the page asked to remove: `removal` if it worked,
/// otherwise `message` says why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoveOutcome {
    pub id: String,
    pub removal: Option<Removal>,
    pub message: Option<String>,
}

pub struct Service {
    options: Options,
    /// Where entries are read from and written to.
    dirs: Dirs,
    scan: Mutex<Option<Scan>>,
}

impl Service {
    pub fn new(options: Options) -> Self {
        let dirs = Dirs::around(options.target_dir.clone());
        Service {
            options,
            dirs,
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

    pub fn entries(&self) -> Vec<EntryInfo> {
        entries::list_entries(&self.dirs)
    }

    pub fn entry(&self, id: &str) -> Result<EntryDetail, String> {
        entries::get_entry(&self.dirs, id)
    }

    pub fn save_entry(&self, id: &str, fields: &Fields) -> Result<Saved, SaveError> {
        entries::save_entry(&self.dirs, id, fields)
    }

    /// Remove each entry in the way that fits where it came from. An id is
    /// handled once however often it is listed.
    pub fn remove_entries(&self, ids: &[String]) -> Vec<RemoveOutcome> {
        let mut seen = HashSet::new();
        ids.iter()
            .filter(|id| seen.insert(id.as_str()))
            .map(|id| match entries::remove_entry(&self.dirs, id) {
                Ok(removal) => RemoveOutcome {
                    id: id.clone(),
                    removal: Some(removal),
                    message: None,
                },
                Err(message) => RemoveOutcome {
                    id: id.clone(),
                    removal: None,
                    message: Some(message),
                },
            })
            .collect()
    }

    pub fn restore_entry(&self, id: &str) -> Result<Restored, String> {
        entries::restore_entry(&self.dirs, id)
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
        let mut service = Service::new(Options {
            target_dir: dir.join("apps"),
            catalog_path: dir.join("no-such-catalog.toml"),
            ignore_nodisplay: false,
            extra_prefixes: Vec::new(),
        });
        // Only scratch directories, so these tests never see this machine's
        // real menu, and a bug here could not touch it.
        service.dirs = Dirs::new(
            dir.join("apps"),
            dir.join("backups"),
            vec![dir.join("system")],
        );
        service
    }

    const ENTRY: &str = "[Desktop Entry]\nType=Application\nName=Fox\nExec=fox %u\nIcon=fox\n";

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
    fn a_path_or_unknown_id_is_refused_and_nothing_is_touched() {
        let dir = scratch("rm");
        let service = service_in(&dir);
        let victim = dir.join("precious.desktop");
        std::fs::write(&victim, ENTRY).unwrap();
        let victim_path = victim.display().to_string();

        let ids = vec![
            victim_path.clone(),
            "../precious.desktop".to_string(),
            "nope.desktop".to_string(),
            victim_path,
        ];
        let results = service.remove_entries(&ids);
        assert_eq!(results.len(), 3, "the repeated id is handled once");
        assert!(results
            .iter()
            .all(|r| r.removal.is_none() && r.message.is_some()));
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), ENTRY);

        assert!(service.entry("/etc/passwd").is_err());
        assert!(service.restore_entry("../precious.desktop").is_err());
        assert!(!dir.join("apps").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_entry_can_be_listed_edited_hidden_and_restored() {
        let dir = scratch("entries");
        let service = service_in(&dir);
        let system = dir.join("system/fox.desktop");
        std::fs::create_dir_all(system.parent().unwrap()).unwrap();
        std::fs::write(&system, ENTRY).unwrap();

        let listed = service.entries();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "fox.desktop");

        let mut fields = service.entry("fox.desktop").unwrap().fields;
        fields.name = "Renamed".to_string();
        let saved = service.save_entry("fox.desktop", &fields).unwrap();
        assert!(saved.created_override);
        assert_eq!(service.entries()[0].name, "Renamed");
        assert_eq!(std::fs::read_to_string(&system).unwrap(), ENTRY);

        let hidden = service.remove_entries(&["fox.desktop".to_string()]);
        assert_eq!(hidden[0].removal, Some(Removal::Hidden));
        assert!(service.entries()[0].hidden);

        assert_eq!(
            service.restore_entry("fox.desktop").unwrap(),
            Restored::OverrideRemoved
        );
        let back = &service.entries()[0];
        assert_eq!(back.name, "Fox");
        assert!(!back.hidden && !back.edited);
        assert_eq!(std::fs::read_to_string(&system).unwrap(), ENTRY);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_values_come_back_as_field_errors_and_write_nothing() {
        let dir = scratch("invalid");
        let service = service_in(&dir);
        let system = dir.join("system/fox.desktop");
        std::fs::create_dir_all(system.parent().unwrap()).unwrap();
        std::fs::write(&system, ENTRY).unwrap();

        let mut fields = service.entry("fox.desktop").unwrap().fields;
        fields.name = "x\nExec=evil".to_string();
        match service.save_entry("fox.desktop", &fields) {
            Err(SaveError::Invalid(errors)) => assert_eq!(errors[0].field, "name"),
            other => panic!("expected a field error, got {other:?}"),
        }
        assert!(!dir.join("apps").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
