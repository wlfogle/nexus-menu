//! Finding Wine prefixes on disk.
//!
//! A Wine prefix is a directory holding a `drive_c` tree plus the registry
//! hives `system.reg` / `user.reg`. Prefixes come from four places:
//!
//! 1. every **top-level hidden directory** of `$HOME` that is itself a prefix
//!    (`~/.wine`, `~/.insomniac`, …) — only the top level, because a recursive
//!    search of the home directory turns up Proton `default_pfx` templates,
//!    Bottles templates and launcher runtime files that no application was
//!    ever installed into;
//! 2. `$WINEPREFIX` and any location named with `--prefix`;
//! 3. whatever each **launcher declares**: Faugus's `~/Faugus/*`, PortProton's
//!    `~/PortProton/data/prefixes/*`, the `prefix:` of every Lutris game
//!    config, and the `winePrefix` of every Heroic game config;
//! 4. a bounded search for `drive_c` on **every local disk** mounted
//!    elsewhere (`/media`, `/mnt`, …), which is where prefixes kept off the
//!    system drive live. Network mounts are not searched.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Which tool manages a prefix. This decides how entries for apps inside it
/// must be launched: through the launcher, never through bare `wine`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    Wine,
    PortProton,
    Faugus,
    Lutris,
    Heroic,
}

impl Owner {
    pub fn as_str(self) -> &'static str {
        match self {
            Owner::Wine => "wine",
            Owner::PortProton => "portproton",
            Owner::Faugus => "faugus",
            Owner::Lutris => "lutris",
            Owner::Heroic => "heroic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefix {
    pub path: PathBuf,
    pub owner: Owner,
}

/// Whether `dir` is the root of a Wine prefix.
///
/// `drive_c` alone is not enough — plenty of unrelated trees contain one — so
/// a registry hive is required as well.
pub fn is_prefix(dir: &Path) -> bool {
    dir.join("drive_c").is_dir()
        && (dir.join("system.reg").is_file() || dir.join("user.reg").is_file())
}

/// Immediate subdirectories of `dir`, following symlinks (`~/PortProton` is
/// one), sorted for stable output.
fn child_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false))
        .collect();
    dirs.sort();
    dirs
}

/// Immediate children of `dir` that are prefixes.
fn child_prefixes(dir: &Path) -> Vec<PathBuf> {
    child_dirs(dir)
        .into_iter()
        .filter(|p| is_prefix(p))
        .collect()
}

/// `dir` itself if it is a prefix, otherwise its immediate children that are.
fn prefixes_at(dir: &Path) -> Vec<PathBuf> {
    if is_prefix(dir) {
        vec![dir.to_path_buf()]
    } else {
        child_prefixes(dir)
    }
}

/// The prefix a launcher config names. Proton-style layouts keep the real
/// prefix in a `pfx` subdirectory of the directory the launcher records.
fn declared_prefix(named: &Path) -> Option<PathBuf> {
    if is_prefix(named) {
        return Some(named.to_path_buf());
    }
    let nested = named.join("pfx");
    is_prefix(&nested).then_some(nested)
}

/// Find every Wine prefix, in priority order: when two sources name the same
/// directory the earlier one wins, so `~/.wine` stays a plain Wine prefix even
/// though a Lutris game also points at it.
pub fn discover(
    home: &Path,
    env_prefix: Option<&Path>,
    extra: &[PathBuf],
    drives: &[PathBuf],
) -> Vec<Prefix> {
    let mut found: Vec<Prefix> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut add = |path: PathBuf, owner: Owner| {
        let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.insert(key) {
            found.push(Prefix { path, owner });
        }
    };

    // 1. Top-level hidden directories that are prefixes.
    for dir in child_dirs(home) {
        let hidden = dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        if hidden && is_prefix(&dir) {
            add(dir, Owner::Wine);
        }
    }

    // 2. $WINEPREFIX and --prefix.
    for location in env_prefix
        .into_iter()
        .chain(extra.iter().map(PathBuf::as_path))
    {
        for prefix in prefixes_at(location) {
            let owner = owner_by_path(&prefix);
            add(prefix, owner);
        }
    }

    // 3. What each launcher declares.
    for prefix in child_prefixes(&home.join("Faugus")) {
        add(prefix, Owner::Faugus);
    }
    for prefix in child_prefixes(&home.join("PortProton/data/prefixes")) {
        add(prefix, Owner::PortProton);
    }
    for named in lutris_prefixes(home) {
        if let Some(prefix) = declared_prefix(&named) {
            add(prefix, Owner::Lutris);
        }
    }
    for named in heroic_prefixes(home) {
        if let Some(prefix) = declared_prefix(&named) {
            add(prefix, Owner::Heroic);
        }
    }

    // 4. Every other prefix on the local disks. Last, so a launcher that
    // declares a prefix keeps ownership of it.
    for drive in drives {
        for prefix in walk_drive(drive) {
            let owner = owner_by_path(&prefix);
            add(prefix, owner);
        }
    }

    found
}

/// How far below a drive's root the search descends. PortProton keeps a prefix
/// four levels down (`PortProton/data/prefixes/NAME`) and Heroic five; eight
/// leaves room without crawling whole disks.
const DRIVE_MAX_DEPTH: usize = 8;

/// Directories not worth entering while searching a disk: caches and build
/// output, Steam's library (one throwaway prefix per game in `compatdata`),
/// trash, Windows system trees, and backups, which are copies rather than
/// prefixes anyone would add menu entries for.
fn is_noise(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "cache"
            | ".cache"
            | ".git"
            | "node_modules"
            | "target"
            | "venv"
            | ".venv"
            | "__pycache__"
            | "compatdata"
            | "shadercache"
            | "steamapps"
            | "steamlibrary"
            | "trash"
            | "$recycle.bin"
            | "system volume information"
            | "lost+found"
            | "windows"
            | "program files"
            | "program files (x86)"
            | "programdata"
    ) || n.starts_with(".trash")
        || n.contains("backup")
}

/// Search one disk for prefixes: never follows symlinks, stops descending the
/// moment it finds a prefix (everything inside is the prefix's own content),
/// and skips [`is_noise`] directories.
fn walk_drive(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut it = WalkDir::new(root)
        .follow_links(false)
        .max_depth(DRIVE_MAX_DEPTH)
        .into_iter();

    while let Some(entry) = it.next() {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_dir() {
            continue;
        }
        if is_prefix(entry.path()) {
            found.push(entry.path().to_path_buf());
            it.skip_current_dir();
            continue;
        }
        if entry.depth() > 0 && entry.file_name().to_str().is_some_and(is_noise) {
            it.skip_current_dir();
        }
    }
    found
}

/// Mount points of every local disk worth searching, read from the kernel.
pub fn drive_roots(home: &Path) -> Vec<PathBuf> {
    fs::read_to_string("/proc/mounts")
        .map(|text| parse_mounts(&text, home))
        .unwrap_or_default()
}

/// Pick the searchable disks out of `/proc/mounts` text.
///
/// A disk is a mount whose source is a block device. That excludes network
/// shares (NFS, SMB, SSHFS — slow to crawl and not where prefixes live),
/// pseudo filesystems, and loop devices (snaps). The root filesystem and the
/// system mount points are skipped, as is anything at or under `/home` or
/// containing the home directory, since home is searched by its own rules.
pub fn parse_mounts(text: &str, home: &Path) -> Vec<PathBuf> {
    const SYSTEM: &[&str] = &[
        "/boot",
        "/recovery",
        "/snap",
        "/var",
        "/usr",
        "/etc",
        "/proc",
        "/sys",
        "/dev",
        "/tmp",
        "/run",
    ];

    let mut roots: Vec<PathBuf> = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(source), Some(mount)) = (fields.next(), fields.next()) else {
            continue;
        };
        if !source.starts_with("/dev/") || source.starts_with("/dev/loop") {
            continue;
        }

        let mount = PathBuf::from(unescape_mount(mount));
        // Removable media is mounted under /run/media on many desktops.
        let under_system =
            SYSTEM.iter().any(|s| mount.starts_with(s)) && !mount.starts_with("/run/media");
        if mount == Path::new("/")
            || under_system
            || mount.starts_with("/home")
            || home.starts_with(&mount)
        {
            continue;
        }
        if !roots.contains(&mount) {
            roots.push(mount);
        }
    }
    roots
}

/// `/proc/mounts` writes spaces, tabs, newlines and backslashes in a mount
/// point as three-digit octal escapes.
fn unescape_mount(s: &str) -> String {
    s.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

/// Owner of an explicitly named location, judged by its path: a prefix inside
/// a PortProton or Faugus tree belongs to that launcher.
fn owner_by_path(prefix: &Path) -> Owner {
    for component in prefix.components() {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        if name.contains("portproton") {
            return Owner::PortProton;
        }
        if name.contains("faugus") {
            return Owner::Faugus;
        }
    }
    Owner::Wine
}

fn files_with_extension(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(ext))
        .collect();
    files.sort();
    files
}

/// The prefix of every Lutris game config, native or Flatpak.
fn lutris_prefixes(home: &Path) -> Vec<PathBuf> {
    [
        home.join(".config/lutris/games"),
        home.join(".var/app/net.lutris.Lutris/config/lutris/games"),
    ]
    .iter()
    .flat_map(|dir| files_with_extension(dir, "yml"))
    .filter_map(|path| fs::read_to_string(path).ok())
    .filter_map(|text| parse_lutris_prefix(&text))
    .collect()
}

/// The `winePrefix` of every Heroic game config, native or Flatpak.
fn heroic_prefixes(home: &Path) -> Vec<PathBuf> {
    [
        home.join(".config/heroic/GamesConfig"),
        home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic/GamesConfig"),
    ]
    .iter()
    .flat_map(|dir| files_with_extension(dir, "json"))
    .filter_map(|path| fs::read_to_string(path).ok())
    .flat_map(|text| parse_heroic_prefixes(&text))
    .collect()
}

/// The `game.prefix` of a Lutris game config.
///
/// Lutris YAML repeats `prefix:` inside installer scripts at deeper
/// indentation, often as `$GAMEDIR`. Only the two-space-indented absolute one
/// is the game's own, so that is the only one read — no YAML parser needed.
pub fn parse_lutris_prefix(text: &str) -> Option<PathBuf> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("  prefix: ") {
            let value = rest.trim().trim_matches(|c| c == '"' || c == '\'');
            if value.starts_with('/') {
                return Some(PathBuf::from(value));
            }
        }
    }
    None
}

/// Every `winePrefix` named in a Heroic `GamesConfig/*.json` file, which is a
/// map of app name to settings.
pub fn parse_heroic_prefixes(text: &str) -> Vec<PathBuf> {
    let Ok(serde_json::Value::Object(games)) = serde_json::from_str::<serde_json::Value>(text)
    else {
        return Vec::new();
    };
    games
        .values()
        .filter_map(|settings| settings.get("winePrefix")?.as_str())
        .filter(|p| p.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

/// `$HOME`, falling back to `/` as the rest of the crate does.
pub fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scratch directory removed on drop, so tests need no extra crate.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = env::temp_dir().join(format!(
                "kappfinder-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        /// Create a valid prefix at `rel`.
        fn prefix(&self, rel: &str) -> PathBuf {
            let dir = self.0.join(rel);
            fs::create_dir_all(dir.join("drive_c")).unwrap();
            fs::write(dir.join("system.reg"), "WINE REGISTRY Version 2\n").unwrap();
            dir
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The home-directory sources only, with no disks searched.
    fn discover(home: &Path, env_prefix: Option<&Path>, extra: &[PathBuf]) -> Vec<Prefix> {
        super::discover(home, env_prefix, extra, &[])
    }

    /// Search the given scratch directories as if they were disks.
    fn discover_drives(home: &Path, drives: &[PathBuf]) -> Vec<Prefix> {
        super::discover(home, None, &[], drives)
    }

    fn paths(found: &[Prefix]) -> Vec<PathBuf> {
        found.iter().map(|p| p.path.clone()).collect()
    }

    fn owner_of(found: &[Prefix], path: &Path) -> Owner {
        found
            .iter()
            .find(|p| p.path == path)
            .unwrap_or_else(|| panic!("missing {}", path.display()))
            .owner
    }

    #[test]
    fn a_prefix_needs_drive_c_and_a_registry_hive() {
        let s = Scratch::new();
        assert!(is_prefix(&s.prefix("good")));

        let only_drive_c = s.path().join("only_drive_c");
        fs::create_dir_all(only_drive_c.join("drive_c")).unwrap();
        assert!(!is_prefix(&only_drive_c));

        let only_reg = s.path().join("only_reg");
        fs::create_dir_all(&only_reg).unwrap();
        fs::write(only_reg.join("user.reg"), "").unwrap();
        assert!(!is_prefix(&only_reg));

        let user_hive = s.path().join("user_hive");
        fs::create_dir_all(user_hive.join("drive_c")).unwrap();
        fs::write(user_hive.join("user.reg"), "").unwrap();
        assert!(is_prefix(&user_hive), "user.reg alone is a valid hive");
    }

    #[test]
    fn finds_top_level_hidden_directories_that_are_prefixes() {
        let s = Scratch::new();
        let a = s.prefix(".wine");
        let b = s.prefix(".insomniac");
        let found = discover(s.path(), None, &[]);
        // Sorted by name, so the output is stable between runs.
        assert_eq!(paths(&found), vec![b.clone(), a.clone()]);
        assert_eq!(owner_of(&found, &a), Owner::Wine);
        assert_eq!(owner_of(&found, &b), Owner::Wine);
    }

    #[test]
    fn does_not_search_below_the_top_level() {
        let s = Scratch::new();
        // These are exactly the junk a recursive walk turns up on a real
        // machine: templates and runtime files, not installed-app prefixes.
        s.prefix(".local/share/somewhere/pfx");
        s.prefix(".steam/compatibilitytools.d/GE-Proton/files/share/default_pfx");
        s.prefix(".var/app/com.usebottles.bottles/data/bottles/templates/abc");
        s.prefix(".PlayOnLinux/wineprefix/ntlite");
        assert!(discover(s.path(), None, &[]).is_empty());
    }

    #[test]
    fn ignores_visible_directories_that_are_not_launcher_roots() {
        let s = Scratch::new();
        s.prefix("Documents/stray");
        s.prefix("Downloads");
        assert!(discover(s.path(), None, &[]).is_empty());
    }

    #[test]
    fn a_hidden_directory_without_a_registry_hive_is_not_a_prefix() {
        let s = Scratch::new();
        fs::create_dir_all(s.path().join(".cache/drive_c")).unwrap();
        fs::create_dir_all(s.path().join(".config")).unwrap();
        assert!(discover(s.path(), None, &[]).is_empty());
    }

    #[test]
    fn a_hidden_file_is_ignored() {
        let s = Scratch::new();
        fs::write(s.path().join(".bashrc"), "").unwrap();
        assert!(discover(s.path(), None, &[]).is_empty());
    }

    #[test]
    fn faugus_prefixes_are_the_children_of_the_faugus_directory() {
        let s = Scratch::new();
        let a = s.prefix("Faugus/default");
        let b = s.prefix("Faugus/other");
        fs::create_dir_all(s.path().join("Faugus/not-a-prefix")).unwrap();
        let found = discover(s.path(), None, &[]);
        assert_eq!(paths(&found), vec![a.clone(), b]);
        assert_eq!(owner_of(&found, &a), Owner::Faugus);
    }

    #[test]
    fn portproton_prefixes_follow_a_symlinked_install_without_duplicating() {
        let s = Scratch::new();
        // Mirrors ~/PortProton -> ~/.var/app/ru.linux_gaming.PortProton
        let real = s.prefix(".var/app/ru.linux_gaming.PortProton/data/prefixes/DEFAULT");
        s.prefix(".var/app/ru.linux_gaming.PortProton/data/prefixes/GOG");
        fs::create_dir_all(
            s.path()
                .join(".var/app/ru.linux_gaming.PortProton/data/prefixes/DOTNET"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            s.path().join(".var/app/ru.linux_gaming.PortProton"),
            s.path().join("PortProton"),
        )
        .unwrap();

        let found = discover(s.path(), None, &[]);
        assert_eq!(found.len(), 2, "{found:?}");
        let default = found
            .iter()
            .find(|p| p.path.ends_with("DEFAULT"))
            .expect("DEFAULT prefix");
        assert_eq!(default.owner, Owner::PortProton);
        assert_eq!(
            fs::canonicalize(&default.path).unwrap(),
            fs::canonicalize(&real).unwrap()
        );
    }

    #[test]
    fn lutris_config_prefixes_are_found_wherever_they_live() {
        let s = Scratch::new();
        let prefix = s.prefix("Games/gog/sacred-2-remastered");
        let dir = s.path().join(".config/lutris/games");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("sacred-1.yml"),
            format!(
                "game:\n  exe: x.exe\n  prefix: {}\nscript:\n  game:\n    prefix: $GAMEDIR\n",
                prefix.display()
            ),
        )
        .unwrap();
        let found = discover(s.path(), None, &[]);
        assert_eq!(paths(&found), vec![prefix.clone()]);
        assert_eq!(owner_of(&found, &prefix), Owner::Lutris);
    }

    #[test]
    fn heroic_proton_prefixes_resolve_to_the_pfx_subdirectory() {
        let s = Scratch::new();
        let pfx = s.prefix("Games/Heroic/Prefixes/default/Game/pfx");
        let dir = s
            .path()
            .join(".var/app/com.heroicgameslauncher.hgl/config/heroic/GamesConfig");
        fs::create_dir_all(&dir).unwrap();
        // Heroic records the directory above `pfx`.
        fs::write(
            dir.join("abc.json"),
            format!(
                "{{\"abc\": {{\"winePrefix\": \"{}\"}}}}",
                pfx.parent().unwrap().display()
            ),
        )
        .unwrap();
        let found = discover(s.path(), None, &[]);
        assert_eq!(paths(&found), vec![pfx.clone()]);
        assert_eq!(owner_of(&found, &pfx), Owner::Heroic);
    }

    #[test]
    fn a_launcher_config_naming_a_missing_prefix_is_skipped() {
        let s = Scratch::new();
        let dir = s.path().join(".config/lutris/games");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("gone.yml"), "game:\n  prefix: /does/not/exist\n").unwrap();
        assert!(discover(s.path(), None, &[]).is_empty());
    }

    #[test]
    fn a_prefix_named_by_several_sources_is_reported_once_as_plain_wine() {
        let s = Scratch::new();
        // ~/.wine is a plain prefix, and a Lutris game also points at it.
        let wine = s.prefix(".wine");
        let dir = s.path().join(".config/lutris/games");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("ascent.yml"),
            format!("game:\n  prefix: {}\n", wine.display()),
        )
        .unwrap();
        let found = discover(s.path(), None, &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].owner, Owner::Wine);
    }

    #[test]
    fn explicit_location_may_be_a_prefix_or_a_directory_of_prefixes() {
        let home = Scratch::new();
        let outside = Scratch::new();
        let single = outside.prefix("custom");
        let a = outside.prefix("PortProton/data/prefixes/BATTLE_NET");
        let b = outside.prefix("PortProton/data/prefixes/DEFAULT");

        let found = discover(
            home.path(),
            None,
            &[
                single.clone(),
                outside.path().join("PortProton/data/prefixes"),
            ],
        );
        assert_eq!(paths(&found), vec![single.clone(), a.clone(), b]);
        assert_eq!(owner_of(&found, &single), Owner::Wine);
        assert_eq!(owner_of(&found, &a), Owner::PortProton);
    }

    #[test]
    fn wineprefix_environment_prefix_is_included() {
        let home = Scratch::new();
        let elsewhere = Scratch::new();
        let p = elsewhere.prefix("custom");
        assert_eq!(paths(&discover(home.path(), Some(&p), &[])), vec![p]);
    }

    #[test]
    fn repeated_locations_report_a_prefix_once() {
        let s = Scratch::new();
        let p = s.prefix(".wine");
        let found = discover(s.path(), None, &[p.clone(), p.clone()]);
        assert_eq!(paths(&found), vec![p]);
    }

    #[test]
    fn lutris_prefix_ignores_installer_script_values() {
        let text = "game:\n  prefix: /home/u/Games/x\nscript:\n  installer:\n  - task:\n      prefix: /other\n    prefix: $GAMEDIR\n";
        assert_eq!(
            parse_lutris_prefix(text),
            Some(PathBuf::from("/home/u/Games/x"))
        );
        assert_eq!(parse_lutris_prefix("game:\n  prefix: $GAMEDIR\n"), None);
        assert_eq!(parse_lutris_prefix(""), None);
    }

    #[test]
    fn a_drive_search_finds_prefixes_at_the_depths_launchers_use() {
        let home = Scratch::new();
        let disk = Scratch::new();
        let portproton = disk.prefix("PortProton/data/prefixes/BATTLE_NET");
        let heroic = disk.prefix("Heroic/Prefixes/default/Game/pfx");
        let loose = disk.prefix("prefixes/mygame");
        let found = discover_drives(home.path(), &[disk.path().to_path_buf()]);
        for p in [&portproton, &heroic, &loose] {
            assert!(paths(&found).contains(p), "missing {}", p.display());
        }
        assert_eq!(found.len(), 3);
        assert_eq!(owner_of(&found, &portproton), Owner::PortProton);
        assert_eq!(owner_of(&found, &loose), Owner::Wine);
    }

    #[test]
    fn a_drive_search_prunes_backups_steam_trash_and_build_output() {
        let home = Scratch::new();
        let disk = Scratch::new();
        disk.prefix("wine-backup-pre-wine11/pfx");
        disk.prefix("SteamLibrary/steamapps/compatdata/1234/pfx");
        disk.prefix("steamapps/compatdata/9/pfx");
        disk.prefix("$RECYCLE.BIN/x");
        disk.prefix(".Trash-1000/files/old");
        disk.prefix("Repos/project/target/debug/pfx");
        disk.prefix("System Volume Information/p");
        let keep = disk.prefix("Games/real");
        let found = discover_drives(home.path(), &[disk.path().to_path_buf()]);
        assert_eq!(paths(&found), vec![keep]);
    }

    #[test]
    fn a_drive_search_never_descends_into_a_prefix() {
        let home = Scratch::new();
        let disk = Scratch::new();
        let outer = disk.prefix("Games/outer");
        disk.prefix("Games/outer/drive_c/users/u/inner");
        let found = discover_drives(home.path(), &[disk.path().to_path_buf()]);
        assert_eq!(paths(&found), vec![outer]);
    }

    #[test]
    fn a_drive_search_is_depth_limited_and_ignores_symlinks() {
        let home = Scratch::new();
        let disk = Scratch::new();
        let ok = disk.prefix("a/b/c/d/e/f/g/h");
        disk.prefix("z/b/c/d/e/f/g/h/i");
        let real = disk.prefix("elsewhere/real");
        fs::create_dir_all(disk.path().join("links")).unwrap();
        std::os::unix::fs::symlink(&real, disk.path().join("links/to-real")).unwrap();
        let found = discover_drives(home.path(), &[disk.path().to_path_buf()]);
        assert!(paths(&found).contains(&ok));
        assert!(paths(&found).contains(&real));
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn a_launcher_that_declares_a_prefix_keeps_ownership_over_the_drive_search() {
        let home = Scratch::new();
        let disk = Scratch::new();
        let prefix = disk.prefix("Games/gog/sacred");
        let dir = home.path().join(".config/lutris/games");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("sacred.yml"),
            format!("game:\n  prefix: {}\n", prefix.display()),
        )
        .unwrap();
        let found = discover_drives(home.path(), &[disk.path().to_path_buf()]);
        assert_eq!(found.len(), 1);
        assert_eq!(owner_of(&found, &prefix), Owner::Lutris);
    }

    #[test]
    fn mounts_keep_local_disks_and_drop_network_system_and_loop_mounts() {
        let text = "\
/dev/nvme0n1p3 / ext4 rw 0 0
/dev/nvme0n1p1 /boot/efi vfat rw 0 0
/dev/nvme0n1p2 /recovery vfat rw 0 0
/dev/sda1 /media/usb0 ext4 rw 0 0
192.168.12.242:/mnt/hdd/media /mnt/tiamat-media nfs4 rw 0 0
/dev/nvme1n1p1 /media/loufogle/Games btrfs rw 0 0
/dev/nvme1n1p3 /media/loufogle/My\\040Drive btrfs rw 0 0
/dev/loop3 /snap/core/1 squashfs ro 0 0
tmpfs /run/user/1000 tmpfs rw 0 0
/dev/sdc1 /run/media/u/stick ext4 rw 0 0
/dev/sdd1 /home ext4 rw 0 0
/dev/sde1 /var/lib/foo ext4 rw 0 0
/dev/sda1 /media/usb0 ext4 rw 0 0
";
        let got = parse_mounts(text, Path::new("/home/u"));
        assert_eq!(
            got,
            vec![
                PathBuf::from("/media/usb0"),
                PathBuf::from("/media/loufogle/Games"),
                PathBuf::from("/media/loufogle/My Drive"),
                PathBuf::from("/run/media/u/stick"),
            ]
        );
    }

    #[test]
    fn a_mount_that_contains_home_is_not_searched() {
        let text = "/dev/sdb1 /data ext4 rw 0 0\n/dev/sdc1 /media/other ext4 rw 0 0\n";
        let got = parse_mounts(text, Path::new("/data/people/u"));
        assert_eq!(got, vec![PathBuf::from("/media/other")]);
    }

    #[test]
    fn heroic_parsing_tolerates_junk() {
        assert!(parse_heroic_prefixes("not json").is_empty());
        assert!(parse_heroic_prefixes("[1,2]").is_empty());
        assert!(parse_heroic_prefixes("{\"a\": {\"winePrefix\": \"relative\"}}").is_empty());
        assert_eq!(
            parse_heroic_prefixes("{\"a\": {\"winePrefix\": \"/p/q\"}, \"b\": {}}"),
            vec![PathBuf::from("/p/q")]
        );
    }
}
