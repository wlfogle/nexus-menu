//! Finding Wine prefixes on disk.
//!
//! A Wine prefix is a directory that holds a `drive_c` directory. Every
//! prefix has one and nothing else is required. Prefixes are looked for in:
//!
//! 1. `$WINEPREFIX` and anything named with `--prefix`;
//! 2. folders directly inside `$HOME` (`~/.wine`, …), the standard place for
//!    a plain Wine prefix;
//! 3. the folders each launcher keeps its prefixes in (Faugus, PortProton,
//!    PlayOnLinux, Lutris, WineZGUI, Bottles);
//! 4. **everywhere else in your home directory**, hidden folders included;
//! 5. **every local disk**.
//!
//! The search stops at each prefix it finds and skips directories that hold
//! templates, runtimes, Steam's per-game prefixes, caches and backups.
//!
//! Each prefix records which tool manages it, because an app inside a
//! launcher's prefix has to be started by that launcher: running a different
//! Wine against it can upgrade or corrupt it.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Which tool manages a prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    /// A plain Wine prefix in a standard place or one the user named.
    Wine,
    PortProton,
    Faugus,
    Lutris,
    Bottles,
    WineZGUI,
    PlayOnLinux,
    /// Found on a disk with nothing to say who made it.
    Other,
}

impl Owner {
    pub fn as_str(self) -> &'static str {
        match self {
            Owner::Wine => "wine",
            Owner::PortProton => "portproton",
            Owner::Faugus => "faugus",
            Owner::Lutris => "lutris",
            Owner::Bottles => "bottles",
            Owner::WineZGUI => "winezgui",
            Owner::PlayOnLinux => "playonlinux",
            Owner::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefix {
    pub path: PathBuf,
    pub owner: Owner,
}

/// Where each launcher keeps its prefixes, relative to `$HOME`.
const LAUNCHER_ROOTS: &[(&str, Owner)] = &[
    ("Faugus", Owner::Faugus),
    ("PortProton", Owner::PortProton),
    (
        ".var/app/ru.linux_gaming.PortProton/data/prefixes",
        Owner::PortProton,
    ),
    (".PlayOnLinux/wineprefix", Owner::PlayOnLinux),
    ("Games", Owner::Lutris),
    (
        ".var/app/io.github.fastrizwaan.WineZGUI/data/winezgui/Prefixes",
        Owner::WineZGUI,
    ),
    (
        ".var/app/com.usebottles.bottles/data/bottles/bottles",
        Owner::Bottles,
    ),
    (".local/share/bottles/bottles", Owner::Bottles),
];

/// How far below a root the search descends.
const MAX_DEPTH: usize = 8;

/// Whether `dir` is the root of a Wine prefix: it holds a `drive_c` directory.
pub fn is_prefix(dir: &Path) -> bool {
    dir.join("drive_c").is_dir()
}

/// Directories not worth entering: caches and build output, Steam's library
/// (one throwaway prefix per game in `compatdata`), trash, Windows system
/// trees, runtimes and templates, and backups, which are copies rather than
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
            | "compatibilitytools.d"
            | "shadercache"
            | "steamapps"
            | "steamlibrary"
            | "templates"
            | "template"
            | "runners"
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

/// Proton and Wine runtimes ship a `default_pfx` that is copied for each new
/// prefix. It has a `drive_c`, but it is the template, not a prefix.
fn is_template_prefix(dir: &Path) -> bool {
    dir.file_name().is_some_and(|n| {
        n.to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("default_pfx")
    })
}

/// Search `root` for prefixes. Never follows symlinks (except `root` itself),
/// stops descending at a prefix, and skips [`is_noise`] directories.
fn walk(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut it = WalkDir::new(root)
        .follow_links(false)
        .max_depth(MAX_DEPTH)
        .sort_by_file_name()
        .into_iter();

    while let Some(entry) = it.next() {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_dir() {
            continue;
        }
        if is_prefix(entry.path()) {
            if !is_template_prefix(entry.path()) {
                found.push(entry.path().to_path_buf());
            }
            it.skip_current_dir();
            continue;
        }
        if entry.depth() > 0 && entry.file_name().to_str().is_some_and(is_noise) {
            it.skip_current_dir();
        }
    }
    found
}

/// Which launcher a path belongs to, going by the names in it.
fn owner_by_path(path: &Path) -> Option<Owner> {
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        if name.contains("portproton") {
            return Some(Owner::PortProton);
        }
        if name.contains("faugus") {
            return Some(Owner::Faugus);
        }
        if name.contains("winezgui") {
            return Some(Owner::WineZGUI);
        }
        if name.contains("bottles") {
            return Some(Owner::Bottles);
        }
        if name.contains("playonlinux") {
            return Some(Owner::PlayOnLinux);
        }
        if name.contains("lutris") {
            return Some(Owner::Lutris);
        }
    }
    None
}

/// Find every Wine prefix. When two sources name the same directory the
/// earlier one wins, so a prefix the user named stays what they called it.
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

    // 1. $WINEPREFIX and --prefix: plain Wine unless the path says otherwise.
    for root in env_prefix
        .into_iter()
        .chain(extra.iter().map(PathBuf::as_path))
    {
        for prefix in walk(root) {
            let owner = owner_by_path(&prefix).unwrap_or(Owner::Wine);
            add(prefix, owner);
        }
    }

    // 2. Folders directly inside home that are prefixes: ~/.wine and kin.
    if let Ok(read) = fs::read_dir(home) {
        let mut direct: Vec<PathBuf> = read
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| is_prefix(p))
            .collect();
        direct.sort();
        for prefix in direct {
            add(prefix, Owner::Wine);
        }
    }

    // 3. Each launcher's own folder.
    for (relative, owner) in LAUNCHER_ROOTS {
        for prefix in walk(&home.join(relative)) {
            add(prefix, *owner);
        }
    }

    // 4. Everywhere else in home, hidden folders included. After the launcher
    // folders, so they keep ownership of their own prefixes; what is left has
    // no launcher in its path to say who manages it.
    for prefix in walk(home) {
        let owner = owner_by_path(&prefix).unwrap_or(Owner::Other);
        add(prefix, owner);
    }

    // 5. Every local disk.
    for drive in drives {
        for prefix in walk(drive) {
            let owner = owner_by_path(&prefix).unwrap_or(Owner::Other);
            add(prefix, owner);
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
/// shares (NFS, SMB, SSHFS: slow to crawl and not where prefixes live),
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

/// `$HOME`, falling back to `/` as the rest of the crate does.
pub fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    /// Home and the named locations only, with no disks searched.
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
    fn a_directory_with_drive_c_is_a_prefix_and_nothing_else_is_required() {
        let s = Scratch::new();
        assert!(is_prefix(&s.prefix("with_hive")));

        let bare = s.path().join("bare");
        fs::create_dir_all(bare.join("drive_c")).unwrap();
        assert!(is_prefix(&bare));

        let only_reg = s.path().join("only_reg");
        fs::create_dir_all(&only_reg).unwrap();
        fs::write(only_reg.join("system.reg"), "").unwrap();
        assert!(!is_prefix(&only_reg));

        // drive_c has to be a directory, not a file.
        let file = s.path().join("file");
        fs::create_dir_all(&file).unwrap();
        fs::write(file.join("drive_c"), "").unwrap();
        assert!(!is_prefix(&file));
    }

    #[test]
    fn folders_directly_inside_home_are_plain_wine_prefixes() {
        let s = Scratch::new();
        let a = s.prefix(".wine");
        let b = s.prefix(".insomniac");
        let c = s.prefix("mywine");
        let found = discover(s.path(), None, &[]);
        assert_eq!(paths(&found), vec![b.clone(), a.clone(), c.clone()]);
        for p in [&a, &b, &c] {
            assert_eq!(owner_of(&found, p), Owner::Wine);
        }
    }

    #[test]
    fn prefixes_anywhere_else_in_home_are_found_but_not_treated_as_plain_wine() {
        let s = Scratch::new();
        let hidden = s.prefix(".local/share/somewhere/pfx");
        let visible = s.prefix("Documents/stray");
        let found = discover(s.path(), None, &[]);
        assert_eq!(found.len(), 2, "{found:?}");
        // Nothing in the path says who manages them, so they are left alone.
        assert_eq!(owner_of(&found, &hidden), Owner::Other);
        assert_eq!(owner_of(&found, &visible), Owner::Other);
    }

    #[test]
    fn the_home_search_names_a_launcher_from_the_path_and_skips_junk() {
        let s = Scratch::new();
        let winezgui =
            s.prefix(".var/app/io.github.fastrizwaan.WineZGUI/data/winezgui/Prefixes/x-1");
        s.prefix(".var/app/io.github.fastrizwaan.WineZGUI/data/winezgui/Templates/WineZGUI-win64");
        s.prefix(".steam/debian-installation/compatibilitytools.d/GE/files/share/default_pfx");
        s.prefix(".local/share/lutris/runners/proton/GE/files/share/default_pfx");
        s.prefix(".var/app/ru.linux_gaming.PortProton/data/dist/GE/share/default_pfx");
        s.prefix(".local/share/Steam/steamapps/compatdata/1/pfx");
        s.prefix(".cache/x/pfx");
        let found = discover(s.path(), None, &[]);
        assert_eq!(paths(&found), vec![winezgui.clone()]);
        assert_eq!(owner_of(&found, &winezgui), Owner::WineZGUI);
    }

    #[test]
    fn each_launcher_folder_is_searched_and_owned_by_that_launcher() {
        let s = Scratch::new();
        let faugus = s.prefix("Faugus/default");
        let portproton = s.prefix("PortProton/data/prefixes/DEFAULT");
        let playonlinux = s.prefix(".PlayOnLinux/wineprefix/ntlite");
        let lutris = s.prefix("Games/gog/sacred-2");
        let winezgui =
            s.prefix(".var/app/io.github.fastrizwaan.WineZGUI/data/winezgui/Prefixes/sacred2-58");
        let bottles = s.prefix(".var/app/com.usebottles.bottles/data/bottles/bottles/Gaming");
        let native_bottles = s.prefix(".local/share/bottles/bottles/Other");

        let found = discover(s.path(), None, &[]);
        assert_eq!(found.len(), 7, "{found:?}");
        assert_eq!(owner_of(&found, &faugus), Owner::Faugus);
        assert_eq!(owner_of(&found, &portproton), Owner::PortProton);
        assert_eq!(owner_of(&found, &playonlinux), Owner::PlayOnLinux);
        assert_eq!(owner_of(&found, &lutris), Owner::Lutris);
        assert_eq!(owner_of(&found, &winezgui), Owner::WineZGUI);
        assert_eq!(owner_of(&found, &bottles), Owner::Bottles);
        assert_eq!(owner_of(&found, &native_bottles), Owner::Bottles);
    }

    #[test]
    fn templates_runtimes_and_throwaway_prefixes_are_not_prefixes() {
        let s = Scratch::new();
        let bottles = ".var/app/com.usebottles.bottles/data/bottles";
        s.prefix(&format!("{bottles}/templates/abc"));
        let keep = s.prefix(&format!("{bottles}/bottles/Gaming"));
        // A runtime's template prefix, wherever it sits.
        s.prefix("Games/runtime/files/share/default_pfx");
        s.prefix("Games/runtime/files/share/default_pfx_arm64");
        // Steam's per-game prefixes and backups.
        s.prefix("Games/steamapps/compatdata/1234/pfx");
        s.prefix("Games/wine-backup-pre-wine11/pfx");
        s.prefix("Games/runners/proton/GE/default_pfx");

        let found = discover(s.path(), None, &[]);
        assert_eq!(paths(&found), vec![keep]);
    }

    #[test]
    fn a_symlinked_launcher_folder_is_followed_without_duplicating() {
        let s = Scratch::new();
        // Mirrors ~/PortProton -> ~/.var/app/ru.linux_gaming.PortProton
        let real = s.prefix(".var/app/ru.linux_gaming.PortProton/data/prefixes/DEFAULT");
        std::os::unix::fs::symlink(
            s.path().join(".var/app/ru.linux_gaming.PortProton"),
            s.path().join("PortProton"),
        )
        .unwrap();
        let found = discover(s.path(), None, &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].owner, Owner::PortProton);
        assert_eq!(
            fs::canonicalize(&found[0].path).unwrap(),
            fs::canonicalize(&real).unwrap()
        );
    }

    #[test]
    fn the_search_never_descends_into_a_prefix_or_through_symlinks() {
        let s = Scratch::new();
        let outer = s.prefix("Games/outer");
        s.prefix("Games/outer/drive_c/users/u/inner");
        // Outside home, so the only way to reach it is through the symlink.
        let outside = Scratch::new();
        let real = outside.prefix("real");
        std::os::unix::fs::symlink(&real, s.path().join("Games/link")).unwrap();
        let found = discover(s.path(), None, &[]);
        assert_eq!(paths(&found), vec![outer]);
    }

    #[test]
    fn explicit_locations_may_be_a_prefix_or_hold_prefixes() {
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
    fn wineprefix_environment_prefix_is_included_and_repeats_collapse() {
        let home = Scratch::new();
        let elsewhere = Scratch::new();
        let p = elsewhere.prefix("custom");
        let found = super::discover(home.path(), Some(&p), &[p.clone(), p.clone()], &[]);
        assert_eq!(paths(&found), vec![p]);
    }

    #[test]
    fn a_disk_search_finds_prefixes_and_names_the_launcher_from_the_path() {
        let home = Scratch::new();
        let disk = Scratch::new();
        let portproton = disk.prefix("PortProton/data/prefixes/BATTLE_NET");
        let bare = disk.path().join("Stuff/bare");
        fs::create_dir_all(bare.join("drive_c")).unwrap();
        let loose = disk.prefix("prefixes/mygame");
        let found = discover_drives(home.path(), &[disk.path().to_path_buf()]);
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(owner_of(&found, &portproton), Owner::PortProton);
        assert_eq!(owner_of(&found, &loose), Owner::Other);
        assert_eq!(owner_of(&found, &bare), Owner::Other);
    }

    #[test]
    fn a_disk_search_prunes_backups_steam_trash_and_build_output() {
        let home = Scratch::new();
        let disk = Scratch::new();
        disk.prefix("wine-backup-pre-wine11/pfx");
        disk.prefix("SteamLibrary/steamapps/compatdata/1234/pfx");
        disk.prefix("$RECYCLE.BIN/x");
        disk.prefix(".Trash-1000/files/old");
        disk.prefix("Repos/project/target/debug/pfx");
        disk.prefix("System Volume Information/p");
        let keep = disk.prefix("Games/real");
        let found = discover_drives(home.path(), &[disk.path().to_path_buf()]);
        assert_eq!(paths(&found), vec![keep]);
    }

    #[test]
    fn a_disk_search_is_depth_limited() {
        let home = Scratch::new();
        let disk = Scratch::new();
        let ok = disk.prefix("a/b/c/d/e/f/g/h");
        disk.prefix("z/b/c/d/e/f/g/h/i");
        let found = discover_drives(home.path(), &[disk.path().to_path_buf()]);
        assert_eq!(paths(&found), vec![ok]);
    }

    #[test]
    fn the_first_source_to_name_a_prefix_decides_its_owner() {
        let home = Scratch::new();
        let disk = Scratch::new();
        // Named explicitly, so it is plain Wine even though a disk holds it.
        let p = disk.prefix("Games/mine");
        let found = super::discover(
            home.path(),
            None,
            std::slice::from_ref(&p),
            &[disk.path().to_path_buf()],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(owner_of(&found, &p), Owner::Wine);
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
    fn noise_rules() {
        for n in [
            ".cache",
            "Cache",
            ".git",
            "node_modules",
            "compatdata",
            "steamapps",
            "Templates",
            "runners",
            ".Trash-1000",
            "Trash",
            "wine-backup-pre-wine11",
            "Backup",
        ] {
            assert!(is_noise(n), "{n} should be noise");
        }
        for n in [
            ".wine", "Games", "prefixes", "pfx", "DEFAULT", "bottles", "Prefixes",
        ] {
            assert!(!is_noise(n), "{n} should not be noise");
        }
    }
}
