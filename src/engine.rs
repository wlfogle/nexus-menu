//! Scanning, creating and removing menu entries, returning data.
//!
//! Nothing here prints. The command line formats these results for a
//! terminal and the desktop app shows them in a window, so both do exactly
//! the same work.
//!
//! The first half is the building blocks the command line composes itself
//! (`find_candidates`, `scan_wine`, `write_file`, ...). The second half,
//! [`Scan`] and the functions after it, is the id-based interface for a
//! caller that cannot hand paths around: a scan hands out stable ids, and
//! everything that changes the disk takes ids back and looks the real thing
//! up itself.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use crate::catalog::{AppTemplate, Catalog};
use crate::registry::{self, Registry};
use crate::{desktop, prefixes, scanner, winapps};

// ---------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------

/// A catalogued application that is installed but has no menu entry.
pub struct Candidate {
    pub template: AppTemplate,
    pub bin_path: PathBuf,
}

/// Where the user's own catalog lives by default.
pub fn default_user_catalog_path() -> PathBuf {
    registry::config_home().join("nexus-menu/catalog.toml")
}

/// Collect catalogued applications that are installed but unreachable from
/// the menu.
pub fn find_candidates(catalog: &Catalog, registry: &Registry) -> (Vec<Candidate>, usize, usize) {
    let mut candidates = Vec::new();
    let mut installed = 0usize;
    let mut covered = 0usize;

    for template in catalog.apps.values() {
        let Some(bin_path) = scanner::lookup(&template.bin) else {
            continue;
        };
        installed += 1;
        if registry.covers(&template.bin).is_some() {
            covered += 1;
            continue;
        }
        candidates.push(Candidate {
            template: template.clone(),
            bin_path,
        });
    }

    candidates.sort_by(|a, b| a.template.bin.cmp(&b.template.bin));
    (candidates, installed, covered)
}

/// The `--prefix` arguments that name nothing on disk.
pub fn missing_prefix_args(extra: &[PathBuf]) -> Vec<&Path> {
    extra
        .iter()
        .map(PathBuf::as_path)
        .filter(|p| !p.exists())
        .collect()
}

/// Every Wine prefix the tool can see: the home directory, `$WINEPREFIX`,
/// `--prefix`, what each launcher declares, and the local disks.
pub fn discover_prefixes(extra: &[PathBuf]) -> Vec<prefixes::Prefix> {
    let home = prefixes::home();
    let env_prefix = std::env::var_os("WINEPREFIX")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    let drives = prefixes::drive_roots(&home);
    prefixes::discover(&home, env_prefix.as_deref(), extra, &drives)
}

/// What a scan of the Windows apps in your prefixes turned up.
pub struct WineScan {
    pub tools: winapps::Tools,
    /// Prefixes searched.
    pub prefixes: usize,
    /// Prefixes nothing can safely be launched from.
    pub unhandled: Vec<prefixes::Prefix>,
    /// User-installed apps found.
    pub found: usize,
    /// Of those, how many an existing entry already launches.
    pub covered: usize,
    /// Apps whose launcher is not installed, so no entry can start them.
    pub no_launcher: Vec<winapps::WinApp>,
    /// The rest: installed, launchable, and unreachable from the menu.
    pub candidates: Vec<winapps::WinApp>,
}

pub fn scan_wine(target_dir: &Path, ignore_nodisplay: bool, extra: &[PathBuf]) -> WineScan {
    let registry = Registry::build(Some(target_dir), ignore_nodisplay);
    let tools = winapps::Tools::detect(&prefixes::home());
    let all = discover_prefixes(extra);
    let found = winapps::collect(&all, &tools);

    let total = found.apps.len();
    let mut covered = 0usize;
    let mut no_launcher = Vec::new();
    let mut candidates = Vec::new();
    for app in found.apps {
        if winapps::covered(&app, &registry) {
            covered += 1;
        } else if !winapps::launchable(&app, &tools) {
            no_launcher.push(app);
        } else {
            candidates.push(app);
        }
    }
    candidates.sort_by_key(|a| (a.owner, a.name.to_lowercase()));

    WineScan {
        tools,
        prefixes: all.len(),
        unhandled: found.unhandled,
        found: total,
        covered,
        no_launcher,
        candidates,
    }
}

/// `path` with the home directory shown as `~`.
pub fn tilde(path: &Path) -> String {
    match path.strip_prefix(prefixes::home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// What happened (or would have happened) to one file.
pub enum Outcome {
    Created(PathBuf),
    Would(PathBuf),
    Exists(PathBuf),
}

/// Render and write one entry. Never overwrites unless `force`.
pub fn write_entry(
    template: &AppTemplate,
    bin_path: &Path,
    dir: &Path,
    force: bool,
    dry_run: bool,
) -> Result<Outcome> {
    let filename = format!("{}.desktop", sanitize_stem(&template.bin));
    write_file(
        dir,
        &filename,
        &desktop::render(template, bin_path),
        force,
        dry_run,
    )
}

/// Write one `.desktop` file. Never overwrites unless `force`, and never
/// leaves a partial file behind.
pub fn write_file(
    dir: &Path,
    filename: &str,
    contents: &str,
    force: bool,
    dry_run: bool,
) -> Result<Outcome> {
    let path = dir.join(filename);

    if path.exists() && !force {
        return Ok(Outcome::Exists(path));
    }
    if dry_run {
        return Ok(Outcome::Would(path));
    }

    fs::create_dir_all(dir)
        .with_context(|| format!("creating target directory {}", dir.display()))?;

    // Write to a temporary file in the same directory, then rename, so a
    // crash can never leave a half-written entry in the menu.
    let tmp = path.with_extension("desktop.tmp");
    fs::write(&tmp, contents).with_context(|| format!("writing {}", tmp.display()))?;
    if let Err(err) = fs::rename(&tmp, &path) {
        // Do not leave the partial file lying around in an applications dir.
        let _ = fs::remove_file(&tmp);
        return Err(err).with_context(|| format!("installing {}", path.display()));
    }

    Ok(Outcome::Created(path))
}

/// Keep generated filenames to characters that are safe as a desktop file ID.
pub fn sanitize_stem(bin: &str) -> String {
    let cleaned: String = bin
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "kappfinder-entry".to_string()
    } else {
        cleaned
    }
}

/// Ask the desktop environment to re-read the applications directory.
/// Entirely best-effort: most environments pick changes up via inotify.
/// Returns whether `update-desktop-database` ran successfully.
pub fn refresh_desktop_database(dir: &Path) -> bool {
    matches!(
        Command::new("update-desktop-database").arg(dir).status(),
        Ok(status) if status.success()
    )
}

/// The entries this tool generated that it is allowed to delete: the ones in
/// `target_dir` or in the user's own applications directory, never anywhere
/// else.
pub fn owned_generated(target_dir: &Path) -> Vec<PathBuf> {
    let registry = Registry::build(Some(target_dir), false);
    let user_dir = registry::user_applications_dir();
    registry
        .generated
        .iter()
        .filter(|p| p.starts_with(target_dir) || p.starts_with(&user_dir))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// The id-based interface
// ---------------------------------------------------------------------------

/// Where to look and where to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Directory new entries are written to.
    pub target_dir: PathBuf,
    /// The user's extra catalog file.
    pub catalog_path: PathBuf,
    /// Treat NoDisplay/Hidden entries as missing.
    pub ignore_nodisplay: bool,
    /// Extra Wine prefixes, or directories to search for them.
    pub extra_prefixes: Vec<PathBuf>,
}

impl Default for Options {
    /// The same defaults the command line uses.
    fn default() -> Self {
        Options {
            target_dir: registry::user_applications_dir(),
            catalog_path: default_user_catalog_path(),
            ignore_nodisplay: false,
            extra_prefixes: Vec::new(),
        }
    }
}

/// One application that has no menu entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    /// Stable for the life of a [`Scan`]: `native:<bin>` or `wine:<file>`.
    pub id: String,
    pub name: String,
    /// What to group by: `programs`, or the owner of the Wine prefix.
    pub group: String,
    /// Where it lives: the program's path, or the prefix it is in.
    pub detail: String,
}

/// Counts for the header of a scan.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub catalog_templates: usize,
    pub installed: usize,
    pub in_menu: usize,
    pub windows_prefixes: usize,
    pub windows_apps: usize,
    pub windows_in_menu: usize,
    pub windows_no_launcher: usize,
    /// Prefixes nothing can safely be launched from.
    pub windows_left_alone: usize,
}

enum Pending {
    Native(Candidate),
    Windows(winapps::WinApp),
}

/// The result of looking at this machine, with enough kept back to create
/// entries for whichever items the caller picks.
pub struct Scan {
    pub items: Vec<Item>,
    pub summary: Summary,
    /// Parallel to `items`.
    pending: Vec<Pending>,
    tools: winapps::Tools,
    target_dir: PathBuf,
}

/// What became of one requested item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Created,
    WouldCreate,
    Exists,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallResult {
    pub id: String,
    pub name: String,
    pub status: Status,
    /// The file written (or that would be), when there is one.
    pub path: Option<String>,
    /// Why it failed.
    pub message: Option<String>,
}

impl Scan {
    /// Look for native programs and Windows apps that have no menu entry.
    pub fn run(opts: &Options) -> Result<Scan> {
        let catalog = Catalog::load(Some(&opts.catalog_path))?;
        let registry = Registry::build(Some(&opts.target_dir), opts.ignore_nodisplay);
        let (native, installed, covered) = find_candidates(&catalog, &registry);
        let wine = scan_wine(
            &opts.target_dir,
            opts.ignore_nodisplay,
            &opts.extra_prefixes,
        );

        let summary = Summary {
            catalog_templates: catalog.len(),
            installed,
            in_menu: covered,
            windows_prefixes: wine.prefixes,
            windows_apps: wine.found,
            windows_in_menu: wine.covered,
            windows_no_launcher: wine.no_launcher.len(),
            windows_left_alone: wine.unhandled.len(),
        };
        Ok(Scan::from_parts(
            native,
            wine.candidates,
            wine.tools,
            opts.target_dir.clone(),
            summary,
        ))
    }

    fn from_parts(
        native: Vec<Candidate>,
        windows: Vec<winapps::WinApp>,
        tools: winapps::Tools,
        target_dir: PathBuf,
        summary: Summary,
    ) -> Scan {
        let mut items = Vec::new();
        let mut pending = Vec::new();
        let mut seen = HashSet::new();

        for c in native {
            let id = format!("native:{}", c.template.bin);
            if !seen.insert(id.clone()) {
                continue;
            }
            items.push(Item {
                id,
                name: c.template.name.clone(),
                group: "programs".to_string(),
                detail: tilde(&c.bin_path),
            });
            pending.push(Pending::Native(c));
        }
        for app in windows {
            // The file name decides what gets written, so two apps that share
            // one are one entry: the second would only be skipped as existing.
            let id = format!("wine:{}", winapps::filename(&app));
            if !seen.insert(id.clone()) {
                continue;
            }
            items.push(Item {
                id,
                name: app.name.clone(),
                group: app.owner.as_str().to_string(),
                detail: tilde(&app.prefix),
            });
            pending.push(Pending::Windows(app));
        }

        Scan {
            items,
            summary,
            pending,
            tools,
            target_dir,
        }
    }

    /// Create entries for `ids`, in the order given, each at most once. An id
    /// this scan did not hand out fails; nothing else is ever written.
    pub fn install(&self, ids: &[String], force: bool, dry_run: bool) -> Vec<InstallResult> {
        let mut seen = HashSet::new();
        let mut results = Vec::new();
        let mut created = false;

        for id in ids {
            if !seen.insert(id.as_str()) {
                continue;
            }
            let Some(pos) = self.items.iter().position(|i| &i.id == id) else {
                results.push(InstallResult {
                    id: id.clone(),
                    name: id.clone(),
                    status: Status::Failed,
                    path: None,
                    message: Some("not part of the last scan; scan again".to_string()),
                });
                continue;
            };
            let item = &self.items[pos];

            let outcome = match &self.pending[pos] {
                Pending::Native(c) => {
                    write_entry(&c.template, &c.bin_path, &self.target_dir, force, dry_run)
                }
                Pending::Windows(app) => match winapps::render(app, &self.tools) {
                    Some(contents) => write_file(
                        &self.target_dir,
                        &winapps::filename(app),
                        &contents,
                        force,
                        dry_run,
                    ),
                    None => Err(anyhow!("cannot build an entry for {}", app.name)),
                },
            };

            let (status, path, message) = match outcome {
                Ok(Outcome::Created(p)) => {
                    created = true;
                    (Status::Created, Some(p), None)
                }
                Ok(Outcome::Would(p)) => (Status::WouldCreate, Some(p), None),
                Ok(Outcome::Exists(p)) => (Status::Exists, Some(p), None),
                Err(e) => (Status::Failed, None, Some(format!("{e:#}"))),
            };
            results.push(InstallResult {
                id: item.id.clone(),
                name: item.name.clone(),
                status,
                path: path.map(|p| p.display().to_string()),
                message,
            });
        }

        if created && !dry_run {
            refresh_desktop_database(&self.target_dir);
        }
        results
    }
}

/// A Wine prefix and who manages it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrefixInfo {
    pub owner: String,
    pub path: String,
}

pub fn list_prefixes(extra: &[PathBuf]) -> Vec<PrefixInfo> {
    discover_prefixes(extra)
        .into_iter()
        .map(|p| PrefixInfo {
            owner: p.owner.as_str().to_string(),
            path: tilde(&p.path),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefixes::Owner;
    use crate::testutil::Scratch;
    use crate::winapps::{Launch, Tools, WinApp};

    #[test]
    fn sanitizes_filenames() {
        assert_eq!(sanitize_stem("htop"), "htop");
        assert_eq!(sanitize_stem("my app"), "my-app");
        assert_eq!(sanitize_stem("a/b"), "a-b");
        assert_eq!(sanitize_stem(""), "kappfinder-entry");
    }

    #[test]
    fn write_file_creates_then_skips_then_forces() {
        let s = Scratch::new();
        let dir = s.path().join("apps");

        let first = write_file(&dir, "a.desktop", "one", false, false).unwrap();
        assert!(matches!(first, Outcome::Created(_)));
        assert_eq!(fs::read_to_string(dir.join("a.desktop")).unwrap(), "one");

        let again = write_file(&dir, "a.desktop", "two", false, false).unwrap();
        assert!(matches!(again, Outcome::Exists(_)));
        assert_eq!(fs::read_to_string(dir.join("a.desktop")).unwrap(), "one");

        let forced = write_file(&dir, "a.desktop", "two", true, false).unwrap();
        assert!(matches!(forced, Outcome::Created(_)));
        assert_eq!(fs::read_to_string(dir.join("a.desktop")).unwrap(), "two");

        // No temporary file is left behind.
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("a.desktop")]);
    }

    #[test]
    fn write_file_dry_run_writes_nothing() {
        let s = Scratch::new();
        let dir = s.path().join("apps");
        let out = write_file(&dir, "a.desktop", "x", false, true).unwrap();
        assert!(matches!(out, Outcome::Would(_)));
        assert!(!dir.exists());
    }

    /// A scan with one native program and one Windows app, writing into a
    /// scratch directory.
    fn scan_in(s: &Scratch) -> Scan {
        let bin = s.file("bin/fakeapp", "#!/bin/sh\n");
        let native = vec![Candidate {
            template: AppTemplate::auto("fakeapp"),
            bin_path: bin,
        }];
        let windows = vec![WinApp {
            name: "My App".to_string(),
            owner: Owner::Wine,
            prefix: s.path().join("pfx"),
            launch: Launch::WineShortcut {
                shortcut: s.path().join("pfx/app.lnk"),
            },
        }];
        let tools = Tools {
            home: s.path().to_path_buf(),
            wine: Some(PathBuf::from("/usr/bin/wine")),
            ..Tools::default()
        };
        Scan::from_parts(
            native,
            windows,
            tools,
            s.path().join("apps"),
            Summary::default(),
        )
    }

    #[test]
    fn items_have_stable_ids_and_groups() {
        let s = Scratch::new();
        let scan = scan_in(&s);
        assert_eq!(scan.items.len(), 2);
        assert_eq!(scan.items[0].id, "native:fakeapp");
        assert_eq!(scan.items[0].group, "programs");
        assert_eq!(scan.items[1].id, "wine:windows-wine-pfx-my-app.desktop");
        assert_eq!(scan.items[1].group, "wine");
        assert_eq!(scan.items[1].name, "My App");
    }

    #[test]
    fn two_windows_apps_that_share_a_file_name_are_one_item() {
        let s = Scratch::new();
        let app = |shortcut: &str| WinApp {
            name: "My App".to_string(),
            owner: Owner::Wine,
            prefix: s.path().join("pfx"),
            launch: Launch::WineShortcut {
                shortcut: s.path().join(shortcut),
            },
        };
        let scan = Scan::from_parts(
            Vec::new(),
            vec![app("a.lnk"), app("b.lnk")],
            Tools::default(),
            s.path().join("apps"),
            Summary::default(),
        );
        assert_eq!(scan.items.len(), 1);
    }

    #[test]
    fn install_writes_only_the_chosen_ids_once() {
        let s = Scratch::new();
        let scan = scan_in(&s);
        let apps = s.path().join("apps");

        let ids = vec![
            "native:fakeapp".to_string(),
            "native:fakeapp".to_string(),
            "native:nope".to_string(),
        ];
        let results = scan.install(&ids, false, false);
        assert_eq!(results.len(), 2, "the repeated id is handled once");
        assert_eq!(results[0].status, Status::Created);
        assert_eq!(results[1].status, Status::Failed);
        assert!(apps.join("fakeapp.desktop").is_file());
        // The Windows app was not chosen, so it was not written.
        assert!(!apps.join("windows-wine-pfx-my-app.desktop").exists());

        let again = scan.install(&["native:fakeapp".to_string()], false, false);
        assert_eq!(again[0].status, Status::Exists);
        let forced = scan.install(&["native:fakeapp".to_string()], true, false);
        assert_eq!(forced[0].status, Status::Created);
    }

    #[test]
    fn install_dry_run_reports_without_writing() {
        let s = Scratch::new();
        let scan = scan_in(&s);
        let ids: Vec<String> = scan.items.iter().map(|i| i.id.clone()).collect();
        let results = scan.install(&ids, false, true);
        assert!(results.iter().all(|r| r.status == Status::WouldCreate));
        assert!(!s.path().join("apps").exists());
    }

    #[test]
    fn a_windows_item_is_written_with_its_launch_command() {
        let s = Scratch::new();
        let scan = scan_in(&s);
        let id = "wine:windows-wine-pfx-my-app.desktop".to_string();
        let results = scan.install(std::slice::from_ref(&id), false, false);
        assert_eq!(results[0].status, Status::Created);
        let text =
            fs::read_to_string(s.path().join("apps/windows-wine-pfx-my-app.desktop")).unwrap();
        assert!(text.contains("Name=My App"), "{text}");
        assert!(text.contains("/Unix"), "{text}");
        assert!(text.contains("X-NexusMenu-Generated=true"), "{text}");
    }

    #[test]
    fn missing_prefix_arguments_are_reported() {
        let s = Scratch::new();
        let real = s.path().to_path_buf();
        let gone = s.path().join("gone");
        let args = [real, gone.clone()];
        assert_eq!(missing_prefix_args(&args), vec![gone.as_path()]);
    }
}
