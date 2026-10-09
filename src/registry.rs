//! An index of the `.desktop` entries already present on the system.
//!
//! This is how the tool answers its central question: "is this installed
//! program already reachable from the application menu?"

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::desktop;

/// `$XDG_DATA_HOME`, defaulting to `~/.local/share`.
pub fn data_home() -> PathBuf {
    if let Some(v) = env::var_os("XDG_DATA_HOME") {
        let p = PathBuf::from(v);
        if p.is_absolute() {
            return p;
        }
    }
    home_dir().join(".local/share")
}

/// `$XDG_CONFIG_HOME`, defaulting to `~/.config`.
pub fn config_home() -> PathBuf {
    if let Some(v) = env::var_os("XDG_CONFIG_HOME") {
        let p = PathBuf::from(v);
        if p.is_absolute() {
            return p;
        }
    }
    home_dir().join(".config")
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `$XDG_DATA_DIRS`, defaulting to the spec's `/usr/local/share:/usr/share`.
fn data_dirs() -> Vec<PathBuf> {
    match env::var_os("XDG_DATA_DIRS") {
        Some(v) if !v.is_empty() => env::split_paths(&v).collect(),
        _ => vec![
            PathBuf::from("/usr/local/share"),
            PathBuf::from("/usr/share"),
        ],
    }
}

/// The user-writable directory new entries go into by default.
pub fn user_applications_dir() -> PathBuf {
    data_home().join("applications")
}

/// Every directory that may contain menu entries, most-specific first.
pub fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![user_applications_dir()];
    for base in data_dirs() {
        dirs.push(base.join("applications"));
    }
    // Flatpak and Snap exports are usually already on XDG_DATA_DIRS, but add
    // them explicitly so we do not offer duplicates when they are not.
    dirs.push(data_home().join("flatpak/exports/share/applications"));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));

    let mut seen = Vec::new();
    for dir in dirs {
        if !seen.contains(&dir) {
            seen.push(dir);
        }
    }
    seen
}

/// Which programs already have a menu entry.
#[derive(Debug, Default)]
pub struct Registry {
    /// program basename -> the entry that launches it
    by_program: HashMap<String, PathBuf>,
    /// desktop file stem -> path, as a fallback match
    by_stem: HashMap<String, PathBuf>,
    /// Entries carrying our own generated marker.
    pub generated: Vec<PathBuf>,
    /// Directories that actually existed and were read.
    pub scanned_dirs: Vec<PathBuf>,
    /// Total `.desktop` files parsed.
    pub total_entries: usize,
    /// Windows programs and shortcuts that some entry launches, resolved so
    /// `dosdevices/c:` and `drive_c` spellings of one file compare equal.
    referenced: HashSet<PathBuf>,
    /// The `Exec` of every entry, whole and split into arguments.
    execs: Vec<(String, Vec<String>)>,
}

impl Registry {
    /// Walk every application directory and build the index.
    ///
    /// When `ignore_nodisplay` is true, entries marked `NoDisplay=true` or
    /// `Hidden=true` are treated as absent, so the tool will offer to create
    /// a visible replacement for them.
    pub fn build(extra_dir: Option<&Path>, ignore_nodisplay: bool) -> Self {
        let mut registry = Registry::default();

        let mut dirs = application_dirs();
        if let Some(extra) = extra_dir {
            let extra = extra.to_path_buf();
            if !dirs.contains(&extra) {
                dirs.insert(0, extra);
            }
        }

        for dir in dirs {
            if !dir.is_dir() {
                continue;
            }
            registry.scanned_dirs.push(dir.clone());

            for entry in WalkDir::new(&dir)
                .follow_links(true)
                .max_depth(8)
                .into_iter()
                .filter_map(Result::ok)
            {
                let path = entry.path();
                if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(parsed) = desktop::parse(path) else {
                    continue;
                };
                registry.total_entries += 1;

                if parsed.generated {
                    registry.generated.push(path.to_path_buf());
                }

                // Only Application entries launch programs. An empty Type is
                // malformed but common enough in the wild to accept.
                if !parsed.entry_type.is_empty() && parsed.entry_type != "Application" {
                    continue;
                }
                if ignore_nodisplay && !parsed.visible() {
                    continue;
                }

                if let Some(exec) = &parsed.exec {
                    for path in desktop::referenced_paths(exec) {
                        registry.referenced.insert(canonical(&path));
                    }
                    registry
                        .execs
                        .push((exec.clone(), desktop::tokenize_exec(exec)));
                }
                if let Some(program) = parsed.program() {
                    registry
                        .by_program
                        .entry(program)
                        .or_insert_with(|| path.to_path_buf());
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    registry
                        .by_stem
                        .entry(stem.to_string())
                        .or_insert_with(|| path.to_path_buf());
                }
            }
        }

        registry
    }

    /// The entry that already covers `bin`, if any.
    ///
    /// Matching is by launched program first, then by desktop-file stem, which
    /// catches entries whose `Exec` is too convoluted to parse.
    pub fn covers(&self, bin: &str) -> Option<&Path> {
        if let Some(p) = self.by_program.get(bin) {
            return Some(p.as_path());
        }
        self.by_stem.get(bin).map(|p| p.as_path())
    }

    /// Whether some entry already launches this Windows program or shortcut,
    /// by Wine's `start.exe /Unix` or as a launcher's executable argument.
    pub fn references(&self, path: &Path) -> bool {
        self.referenced.contains(&canonical(path))
    }

    /// Whether one entry's command line has every one of `wanted` as a whole
    /// argument. Whole, not substring: `sacred-2` is not `sacred-2-gold`.
    pub fn has_arguments(&self, wanted: &[&str]) -> bool {
        self.execs
            .iter()
            .any(|(_, args)| wanted.iter().all(|w| args.iter().any(|a| a == w)))
    }

    /// Whether some entry's command line contains `text` anywhere.
    pub fn exec_contains(&self, text: &str) -> bool {
        self.execs.iter().any(|(exec, _)| exec.contains(text))
    }
}

/// `path` with symlinks resolved, or unchanged if it does not exist.
fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    #[test]
    fn application_dirs_include_user_dir() {
        let dirs = application_dirs();
        assert!(dirs.contains(&user_applications_dir()));
    }

    #[test]
    fn application_dirs_are_unique() {
        let dirs = application_dirs();
        let mut sorted = dirs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), dirs.len(), "duplicate application dirs");
    }

    #[test]
    fn xdg_paths_are_absolute() {
        assert!(data_home().is_absolute());
        assert!(config_home().is_absolute());
    }

    #[test]
    fn indexes_windows_programs_that_existing_entries_launch() {
        let s = Scratch::new();
        s.file(
            "applications/wine-app.desktop",
            r#"[Desktop Entry]
Type=Application
Name=Wine App
Exec=env WINEPREFIX="/p/.wine" wine C:\\windows\\command\\start.exe /Unix /p/.wine/Start\\ Menu/My\\ App.lnk
"#,
        );
        s.file(
            "applications/launcher-game.desktop",
            "[Desktop Entry]\nType=Application\nName=Game\nExec=flatpak run some.Launcher \"/g/Prefixes/X/drive_c/Game/game.exe\"\n",
        );

        let reg = Registry::build(Some(&s.path().join("applications")), false);
        assert!(reg.references(Path::new("/p/.wine/Start Menu/My App.lnk")));
        assert!(reg.references(Path::new("/g/Prefixes/X/drive_c/Game/game.exe")));
        assert!(!reg.references(Path::new("/p/.wine/Other.lnk")));
    }

    #[test]
    fn hidden_entries_do_not_count_as_covering_a_windows_app_when_asked_to_ignore_them() {
        let s = Scratch::new();
        s.file(
            "applications/hidden.desktop",
            "[Desktop Entry]\nType=Application\nName=H\nNoDisplay=true\nExec=wine start.exe /Unix /p/Hidden.lnk\n",
        );
        let dir = s.path().join("applications");
        assert!(Registry::build(Some(&dir), false).references(Path::new("/p/Hidden.lnk")));
        assert!(!Registry::build(Some(&dir), true).references(Path::new("/p/Hidden.lnk")));
    }

    #[test]
    fn a_symlinked_spelling_of_a_shortcut_path_is_recognised() {
        let s = Scratch::new();
        let real = s.file("real/drive_c/Desktop/App.lnk", "");
        // Mirrors dosdevices/c: -> ../drive_c inside a prefix.
        std::fs::create_dir_all(s.path().join("real/dosdevices")).unwrap();
        std::os::unix::fs::symlink(
            s.path().join("real/drive_c"),
            s.path().join("real/dosdevices/c:"),
        )
        .unwrap();
        let via_link = s.path().join("real/dosdevices/c:/Desktop/App.lnk");
        s.file(
            "applications/e.desktop",
            &format!(
                "[Desktop Entry]\nType=Application\nName=A\nExec=wine start.exe /Unix {}\n",
                via_link.display()
            ),
        );
        let reg = Registry::build(Some(&s.path().join("applications")), false);
        assert!(reg.references(&real));
    }

    #[test]
    fn building_the_registry_does_not_panic() {
        // The real system index; content varies by machine, so only assert
        // that it completes and reports the directories it read.
        let reg = Registry::build(None, false);
        assert!(reg.total_entries >= reg.generated.len());
    }
}
