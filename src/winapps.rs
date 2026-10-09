//! Windows applications in Wine prefixes, and the command that starts each.
//!
//! An application counts when the user installed it. In a plain Wine prefix,
//! and in a PortProton or Faugus one, that means a shortcut in the Start Menu
//! or on a Desktop; Windows and Wine components (Accessories, uninstallers,
//! readmes, …) are filtered out and an app reachable from several places is
//! reported once. Lutris, Bottles and WineZGUI keep their own list of apps, so
//! theirs are read from there.
//!
//! Every launcher is started the way that launcher starts things itself, with
//! the command taken from entries it wrote or from its own source:
//!
//! - **Wine**: `wine start.exe /Unix <shortcut>`, as Wine's menu builder does.
//! - **PortProton**: `flatpak run ru.linux_gaming.PortProton "<exe>"`, or its
//!   `start.sh` when it is not a Flatpak.
//! - **Faugus**: its file handler, `faugus-launcher <exe>`. Faugus runs any
//!   file in `<default-prefix>/default`, so only that prefix is handled.
//! - **Lutris**: `lutris lutris:rungame/<slug>`, which Lutris looks up by slug.
//! - **Bottles**: `bottles-cli run -b <bottle> -p <program>`.
//! - **WineZGUI**: the `.desktop` file WineZGUI keeps in each prefix, as is.
//!
//! Running bare `wine` against a launcher's prefix could upgrade or corrupt
//! it, so prefixes of other owners are never launched that way.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::desktop::{exec_quote, push_kv, GENERATED_KEY, SOURCE_KEY};
use crate::lnk;
use crate::prefixes::{Owner, Prefix};
use crate::registry::Registry;
use crate::scanner;

const PORTPROTON_FLATPAK: &str = "ru.linux_gaming.PortProton";
const FAUGUS_FLATPAK: &str = "io.github.Faugus.faugus-launcher";
const BOTTLES_FLATPAK: &str = "com.usebottles.bottles";
/// The Windows path of Wine's own program for opening a shortcut.
const START_EXE: &str = r"C:\windows\command\start.exe";
const ICON: &str = "wine";

/// How deep a Start Menu or Desktop is searched for shortcuts.
const MAX_DEPTH: usize = 6;

/// Start Menu folders that hold Windows and Wine components, not applications
/// the user installed.
const SYSTEM_FOLDERS: &[&str] = &[
    "accessories",
    "administrative tools",
    "maintenance",
    "startup",
    "system tools",
    "windows accessories",
    "windows powershell",
    "windows system",
    "wine",
];

/// Words that mark a shortcut as housekeeping rather than the application.
const NOISE_WORDS: &[&str] = &[
    "changelog",
    "documentation",
    "eula",
    "faq",
    "help",
    "homepage",
    "license",
    "manual",
    "readme",
    "support",
    "website",
];

/// How an application is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// Through Wine itself: run the shortcut with `start.exe /Unix`.
    WineShortcut {
        shortcut: PathBuf,
    },
    PortProton {
        exe: PathBuf,
    },
    Faugus {
        exe: PathBuf,
    },
    Lutris {
        slug: String,
    },
    Bottles {
        bottle: String,
        program: String,
    },
    /// WineZGUI's own `.desktop` file, reused unchanged.
    WineZgui {
        desktop: PathBuf,
    },
}

/// A user-installed Windows application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinApp {
    pub name: String,
    pub owner: Owner,
    pub prefix: PathBuf,
    pub launch: Launch,
}

/// The programs on this machine that launches go through.
#[derive(Debug, Clone, Default)]
pub struct Tools {
    pub home: PathBuf,
    pub wine: Option<PathBuf>,
    pub flatpak: Option<PathBuf>,
    pub lutris: Option<PathBuf>,
    pub bottles_cli: Option<PathBuf>,
    pub faugus: Option<PathBuf>,
    /// PortProton's `start.sh`, for an install that is not a Flatpak.
    pub portproton_script: Option<PathBuf>,
}

impl Tools {
    pub fn detect(home: &Path) -> Tools {
        Tools {
            home: home.to_path_buf(),
            wine: scanner::lookup("wine"),
            flatpak: scanner::lookup("flatpak"),
            lutris: scanner::lookup("lutris"),
            bottles_cli: scanner::lookup("bottles-cli"),
            faugus: scanner::lookup("faugus-launcher"),
            portproton_script: Some(home.join("PortProton/data_from_portwine/scripts/start.sh"))
                .filter(|p| p.is_file()),
        }
    }

    /// `flatpak`, if the Flatpak app `id` has been run on this machine.
    fn flatpak_app(&self, id: &str) -> Option<&PathBuf> {
        if self.home.join(".var/app").join(id).is_dir() {
            self.flatpak.as_ref()
        } else {
            None
        }
    }
}

/// What a desktop entry for an app runs.
struct Command {
    exec: String,
    try_exec: PathBuf,
    /// Working directory, for launchers whose own entries set one.
    path: Option<PathBuf>,
}

fn arg(path: &Path) -> String {
    exec_quote(&path.to_string_lossy())
}

/// The command that starts `app`, or `None` if the program it goes through is
/// not installed. WineZGUI apps are not built here: their entry is copied.
fn command(app: &WinApp, tools: &Tools) -> Option<Command> {
    match &app.launch {
        Launch::WineShortcut { shortcut } => {
            let wine = tools.wine.as_ref()?;
            let exec = format!(
                "env {} {} {} /Unix {}",
                exec_quote(&format!("WINEPREFIX={}", app.prefix.display())),
                arg(wine),
                exec_quote(START_EXE),
                arg(shortcut)
            );
            Some(Command {
                exec,
                try_exec: wine.clone(),
                path: None,
            })
        }
        Launch::PortProton { exe } => {
            if let Some(flatpak) = tools.flatpak_app(PORTPROTON_FLATPAK) {
                let scripts = tools
                    .home
                    .join(".var/app")
                    .join(PORTPROTON_FLATPAK)
                    .join("data/scripts");
                Some(Command {
                    exec: format!("{} run {PORTPROTON_FLATPAK} {}", arg(flatpak), arg(exe)),
                    try_exec: flatpak.clone(),
                    path: scripts.is_dir().then_some(scripts),
                })
            } else {
                let script = tools.portproton_script.as_ref()?;
                Some(Command {
                    exec: format!("env {} {}", arg(script), arg(exe)),
                    try_exec: script.clone(),
                    path: script.parent().map(Path::to_path_buf),
                })
            }
        }
        Launch::Faugus { exe } => {
            if let Some(flatpak) = tools.flatpak_app(FAUGUS_FLATPAK) {
                Some(Command {
                    exec: format!(
                        "{} run --branch=stable --arch=x86_64 --command=faugus-launcher \
                         --file-forwarding {FAUGUS_FLATPAK} @@ {} @@",
                        arg(flatpak),
                        arg(exe)
                    ),
                    try_exec: flatpak.clone(),
                    path: None,
                })
            } else {
                let bin = tools.faugus.as_ref()?;
                Some(Command {
                    exec: format!("{} {}", arg(bin), arg(exe)),
                    try_exec: bin.clone(),
                    path: None,
                })
            }
        }
        Launch::Lutris { slug } => {
            let lutris = tools.lutris.as_ref()?;
            Some(Command {
                exec: format!(
                    "env LUTRIS_SKIP_INIT=1 {} {}",
                    arg(lutris),
                    exec_quote(&format!("lutris:rungame/{slug}"))
                ),
                try_exec: lutris.clone(),
                path: None,
            })
        }
        Launch::Bottles { bottle, program } => {
            let tail = format!("run -b {} -p {}", exec_quote(bottle), exec_quote(program));
            if let Some(flatpak) = tools.flatpak_app(BOTTLES_FLATPAK) {
                Some(Command {
                    exec: format!(
                        "{} run --command=bottles-cli {BOTTLES_FLATPAK} {tail}",
                        arg(flatpak)
                    ),
                    try_exec: flatpak.clone(),
                    path: None,
                })
            } else {
                let bin = tools.bottles_cli.as_ref()?;
                Some(Command {
                    exec: format!("{} {tail}", arg(bin)),
                    try_exec: bin.clone(),
                    path: None,
                })
            }
        }
        Launch::WineZgui { .. } => None,
    }
}

/// Whether the program `app` is started through is installed.
pub fn launchable(app: &WinApp, tools: &Tools) -> bool {
    matches!(app.launch, Launch::WineZgui { .. }) || command(app, tools).is_some()
}

/// Whether some existing menu entry already starts `app`.
///
/// Arguments are compared whole, never as substrings: an entry for
/// `sacred-2-gold` must not make `sacred-2` look covered.
pub fn covered(app: &WinApp, registry: &Registry) -> bool {
    match &app.launch {
        Launch::WineShortcut { shortcut } => registry.references(shortcut),
        Launch::PortProton { exe } | Launch::Faugus { exe } => registry.references(exe),
        Launch::Lutris { slug } => {
            let url = format!("lutris:rungame/{slug}");
            registry.has_arguments(&[url.as_str()])
        }
        Launch::Bottles { bottle, program } => {
            registry.has_arguments(&[bottle.as_str(), program.as_str()])
        }
        // WineZGUI's entry runs a script inside the prefix directory. The
        // trailing slash stops `armoredcore6` matching `armoredcore6-1a2b`.
        Launch::WineZgui { desktop } => desktop.parent().is_some_and(|dir| {
            let inside = format!("{}/", dir.display());
            registry.exec_contains(&inside)
        }),
    }
}

/// Put the generated marker into a desktop file we are copying, right after
/// its `[Desktop Entry]` header. `None` if it has no such header.
fn mark_generated(text: &str, source: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len() + 80);
    let mut marked = false;
    for line in text.lines() {
        out.push_str(line);
        out.push('\n');
        if !marked && line.trim() == "[Desktop Entry]" {
            out.push_str(&format!("{GENERATED_KEY}=true\n{SOURCE_KEY}={source}\n"));
            marked = true;
        }
    }
    marked.then_some(out)
}

/// Render the `.desktop` file for `app`. `None` if the program it goes
/// through is missing, or WineZGUI's own file cannot be read.
pub fn render(app: &WinApp, tools: &Tools) -> Option<String> {
    if let Launch::WineZgui { desktop } = &app.launch {
        return mark_generated(&fs::read_to_string(desktop).ok()?, app.owner.as_str());
    }

    let cmd = command(app, tools)?;
    let owner = app.owner.as_str();

    let mut out = String::with_capacity(512);
    out.push_str("# Generated by nexus-menu.\n");
    out.push_str("# Edit freely; `nexus-menu remove` only deletes files that still\n");
    out.push_str("# carry the X-NexusMenu-Generated marker below.\n");
    out.push_str("[Desktop Entry]\n");
    out.push_str("Type=Application\n");
    out.push_str("Version=1.0\n");
    push_kv(&mut out, "Name", &app.name);
    push_kv(
        &mut out,
        "Comment",
        &format!(
            "Windows application in the {owner} prefix {}",
            app.prefix.display()
        ),
    );
    push_kv(&mut out, "Exec", &cmd.exec);
    push_kv(&mut out, "TryExec", &cmd.try_exec.to_string_lossy());
    if let Some(path) = &cmd.path {
        push_kv(&mut out, "Path", &path.to_string_lossy());
    }
    push_kv(&mut out, "Icon", ICON);
    out.push_str("Terminal=false\n");
    push_kv(&mut out, "Keywords", &format!("wine;windows;{owner};"));
    out.push_str("StartupNotify=true\n");
    out.push_str(GENERATED_KEY);
    out.push_str("=true\n");
    push_kv(&mut out, SOURCE_KEY, owner);
    Some(out)
}

/// Lowercase letters and digits separated by single dashes.
fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
            pending_dash = false;
        } else {
            pending_dash = true;
        }
    }
    if out.is_empty() {
        "app".to_string()
    } else {
        out
    }
}

/// The file name for `app`'s entry. The owner and prefix are part of it so
/// the same application in two prefixes does not collide.
pub fn filename(app: &WinApp) -> String {
    let owner = app.owner.as_str();
    let prefix_name = app
        .prefix
        .file_name()
        .map(|n| slug(&n.to_string_lossy()))
        .unwrap_or_default();

    let mut parts = vec!["windows".to_string(), owner.to_string()];
    if !prefix_name.is_empty() && prefix_name != owner {
        parts.push(prefix_name);
    }
    parts.push(slug(&app.name));
    format!("{}.desktop", parts.join("-"))
}

// ---------------------------------------------------------------------------
// Finding the apps
// ---------------------------------------------------------------------------

/// The child of `dir` named `name`, ignoring ASCII case. Windows software
/// does not agree with itself about `Start Menu` versus `start menu`.
fn child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

fn descend_ci(base: &Path, parts: &[&str]) -> Option<PathBuf> {
    let mut path = base.to_path_buf();
    for part in parts {
        path = child_ci(&path, part)?;
    }
    Some(path)
}

/// Directories that may hold shortcuts, in preference order: all-users Start
/// Menu, per-user Start Menus, then Desktops. When one application is
/// reachable from several places the earliest wins.
fn shortcut_roots(prefix: &Path) -> Vec<PathBuf> {
    let drive_c = prefix.join("drive_c");
    let mut roots = Vec::new();

    if let Some(p) = descend_ci(
        &drive_c,
        &[
            "ProgramData",
            "Microsoft",
            "Windows",
            "Start Menu",
            "Programs",
        ],
    ) {
        roots.push(p);
    }

    let mut users: Vec<PathBuf> = fs::read_dir(drive_c.join("users"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    users.sort();

    for user in &users {
        // Wine keeps it in the profile; Windows installers use AppData.
        for parts in [
            &["Start Menu", "Programs"][..],
            &[
                "AppData",
                "Roaming",
                "Microsoft",
                "Windows",
                "Start Menu",
                "Programs",
            ][..],
        ] {
            if let Some(p) = descend_ci(user, parts) {
                roots.push(p);
            }
        }
    }
    for user in &users {
        if let Some(p) = child_ci(user, "Desktop") {
            roots.push(p);
        }
    }
    roots
}

/// Every `.lnk` under `root`, with its path relative to `root`.
fn shortcuts_under(root: &Path) -> Vec<(PathBuf, PathBuf)> {
    WalkDir::new(root)
        .follow_links(false)
        .max_depth(MAX_DEPTH)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("lnk"))
        })
        .filter_map(|e| {
            let rel = e.path().strip_prefix(root).ok()?.to_path_buf();
            Some((e.path().to_path_buf(), rel))
        })
        .collect()
}

/// Whether a shortcut's name marks it as housekeeping (an uninstaller, a
/// readme, a link to a website) instead of the application itself.
fn is_noise_name(stem: &str) -> bool {
    let lower = stem.to_lowercase();
    if lower.contains("read me") || lower.contains("release notes") {
        return true;
    }
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|w| w.starts_with("uninst") || w.starts_with("unins0") || NOISE_WORDS.contains(&w))
}

/// Whether a shortcut, given its path inside a Start Menu or Desktop, belongs
/// to Windows or Wine rather than to something the user installed.
fn is_system_shortcut(rel: &Path) -> bool {
    let in_system_folder = rel.parent().is_some_and(|dirs| {
        dirs.components().any(|c| {
            let name = c.as_os_str().to_string_lossy().to_lowercase();
            SYSTEM_FOLDERS.contains(&name.as_str())
        })
    });
    let noise = rel
        .file_stem()
        .is_some_and(|stem| is_noise_name(&stem.to_string_lossy()));
    in_system_folder || noise
}

/// A name reduced to lowercase letters and digits, so `Wasteland 3` and
/// `wasteland-3` count as the same application.
fn normalise(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// A user-installed application's shortcut: the name to show and the `.lnk`.
struct Shortcut {
    name: String,
    path: PathBuf,
}

/// The user-installed applications' shortcuts in one prefix, each once.
fn shortcuts_in_prefix(prefix: &Path) -> Vec<Shortcut> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut found = Vec::new();

    for root in shortcut_roots(prefix) {
        for (path, rel) in shortcuts_under(&root) {
            if is_system_shortcut(&rel) {
                continue;
            }
            let Some(name) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            let key = normalise(&name);
            if key.is_empty() || !seen.insert(key) {
                continue;
            }
            found.push(Shortcut { name, path });
        }
    }
    found.sort_by_key(|s| s.name.to_lowercase());
    found
}

/// Turn the Windows path a shortcut points at into the real file inside
/// `prefix`, matching each name ignoring case as Windows does. `None` if it
/// is not a file there.
fn resolve_windows_path(prefix: &Path, windows: &str) -> Option<PathBuf> {
    let (drive, rest) = windows.split_once(':')?;
    if drive.len() != 1 {
        return None;
    }
    let mut path = match drive.to_ascii_lowercase().as_str() {
        "c" => prefix.join("drive_c"),
        "z" => PathBuf::from("/"),
        letter => prefix.join("dosdevices").join(format!("{letter}:")),
    };
    for part in rest.split(['\\', '/']).filter(|p| !p.is_empty()) {
        path = child_ci(&path, part)?;
    }
    path.is_file().then_some(path)
}

/// The program a shortcut inside `prefix` launches, as a real file.
fn exe_of(prefix: &Path, shortcut: &Path) -> Option<PathBuf> {
    let bytes = fs::read(shortcut).ok()?;
    let target = resolve_windows_path(prefix, &lnk::target(&bytes)?)?;
    is_program(&target).then_some(target)
}

/// Whether a shortcut's target is something a launcher can run. Shortcuts
/// also point at web links (`.url`), documents and help files.
fn is_program(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "exe" | "bat" | "cmd" | "com"
        )
    })
}

/// Apps found through shortcuts whose target a launcher is handed directly.
fn exe_apps(prefix: &Prefix, launch: impl Fn(PathBuf) -> Launch) -> Vec<WinApp> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for shortcut in shortcuts_in_prefix(&prefix.path) {
        let Some(exe) = exe_of(&prefix.path, &shortcut.path) else {
            continue;
        };
        // "Game" and "Play Game" often point at one program.
        if !seen.insert(exe.clone()) {
            continue;
        }
        apps.push(WinApp {
            name: shortcut.name,
            owner: prefix.owner,
            prefix: prefix.path.clone(),
            launch: launch(exe),
        });
    }
    apps
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_string()
}

/// The value of `key` in simple `key: value` YAML, at exactly `indent`.
fn yaml_field(text: &str, indent: &str, key: &str) -> Option<String> {
    let wanted = format!("{indent}{key}: ");
    text.lines()
        .find_map(|line| line.strip_prefix(wanted.as_str()))
        .map(unquote)
}

/// A Bottles bottle: its name, and each program added to it.
fn bottle_apps(prefix: &Prefix) -> Vec<WinApp> {
    let Ok(text) = fs::read_to_string(prefix.path.join("bottle.yml")) else {
        return Vec::new();
    };
    let Some(bottle) = yaml_field(&text, "", "Name") else {
        return Vec::new();
    };

    let mut programs = Vec::new();
    let mut in_programs = false;
    for line in text.lines() {
        if line == "External_Programs:" {
            in_programs = true;
        } else if in_programs && !line.starts_with(' ') {
            break;
        } else if in_programs {
            if let Some(name) = line.strip_prefix("        name: ") {
                programs.push(unquote(name));
            }
        }
    }

    programs
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|program| WinApp {
            name: program.clone(),
            owner: prefix.owner,
            prefix: prefix.path.clone(),
            launch: Launch::Bottles {
                bottle: bottle.clone(),
                program,
            },
        })
        .collect()
}

/// A WineZGUI prefix: it holds one `.desktop` file for the app inside it.
fn winezgui_apps(prefix: &Prefix) -> Vec<WinApp> {
    let mut files: Vec<PathBuf> = fs::read_dir(&prefix.path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "desktop"))
        .collect();
    files.sort();

    files
        .into_iter()
        .filter_map(|desktop| {
            let text = fs::read_to_string(&desktop).ok()?;
            if !text.lines().any(|l| l.starts_with("Exec=")) {
                return None;
            }
            let name = text
                .lines()
                .find_map(|l| l.strip_prefix("Name="))?
                .trim()
                .to_string();
            Some(WinApp {
                name,
                owner: prefix.owner,
                prefix: prefix.path.clone(),
                launch: Launch::WineZgui { desktop },
            })
        })
        .collect()
}

/// Lutris keeps a config per game. A Windows game has a Wine prefix; one that
/// is not installed has no executable on disk and is skipped.
pub fn lutris_apps(home: &Path) -> Vec<WinApp> {
    let dirs = [
        home.join(".config/lutris/games"),
        home.join(".var/app/net.lutris.Lutris/config/lutris/games"),
    ];

    let mut apps = Vec::new();
    for dir in dirs {
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "yml"))
            .collect();
        files.sort();

        for file in files {
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let (Some(name), Some(slug), Some(exe), Some(prefix)) = (
                yaml_field(&text, "", "name"),
                yaml_field(&text, "", "game_slug"),
                yaml_field(&text, "  ", "exe"),
                yaml_field(&text, "  ", "prefix"),
            ) else {
                continue;
            };
            if !Path::new(&exe).is_file() || !prefix.starts_with('/') {
                continue;
            }
            apps.push(WinApp {
                name,
                owner: Owner::Lutris,
                prefix: PathBuf::from(prefix),
                launch: Launch::Lutris { slug },
            });
        }
    }
    apps
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// The value of a string key in a flat JSON object, without a JSON parser.
fn json_string(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\""))?;
    let rest = text[at + key.len() + 2..].trim_start().strip_prefix(':')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// Faugus opens any file in `<default-prefix>/default`, so that is the only
/// one of its prefixes an app can be launched into.
fn is_faugus_default(prefix: &Path, home: &Path) -> bool {
    let configs = [
        home.join(".var/app/io.github.Faugus.faugus-launcher/config/faugus-launcher/config.json"),
        home.join(".config/faugus-launcher/config.json"),
    ];
    let Some(value) = configs
        .iter()
        .find_map(|c| fs::read_to_string(c).ok())
        .and_then(|text| json_string(&text, "default-prefix"))
    else {
        return false;
    };
    let default = match value.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(value),
    };
    same_file(prefix, &default.join("default"))
}

/// Whether apps in this prefix can be started safely.
fn is_handled(prefix: &Prefix, tools: &Tools) -> bool {
    match prefix.owner {
        Owner::Wine | Owner::PortProton | Owner::Bottles | Owner::WineZGUI | Owner::Lutris => true,
        Owner::Faugus => is_faugus_default(&prefix.path, &tools.home),
        Owner::PlayOnLinux | Owner::Other => false,
    }
}

fn apps_in_prefix(prefix: &Prefix) -> Vec<WinApp> {
    match prefix.owner {
        Owner::Wine => shortcuts_in_prefix(&prefix.path)
            .into_iter()
            .map(|s| WinApp {
                name: s.name,
                owner: prefix.owner,
                prefix: prefix.path.clone(),
                launch: Launch::WineShortcut { shortcut: s.path },
            })
            .collect(),
        Owner::PortProton => exe_apps(prefix, |exe| Launch::PortProton { exe }),
        Owner::Faugus => exe_apps(prefix, |exe| Launch::Faugus { exe }),
        Owner::Bottles => bottle_apps(prefix),
        Owner::WineZGUI => winezgui_apps(prefix),
        // Lutris apps come from its game configs, not from the prefix.
        Owner::Lutris | Owner::PlayOnLinux | Owner::Other => Vec::new(),
    }
}

/// Every app found, and the prefixes nothing can safely be launched from.
pub struct Found {
    pub apps: Vec<WinApp>,
    pub unhandled: Vec<Prefix>,
}

pub fn collect(prefixes: &[Prefix], tools: &Tools) -> Found {
    let mut apps = Vec::new();
    let mut unhandled = Vec::new();
    for prefix in prefixes {
        if is_handled(prefix, tools) {
            apps.extend(apps_in_prefix(prefix));
        } else {
            unhandled.push(prefix.clone());
        }
    }
    apps.extend(lutris_apps(&tools.home));
    Found { apps, unhandled }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop;
    use crate::testutil::Scratch;

    const START_MENU: &str = "drive_c/ProgramData/Microsoft/Windows/Start Menu/Programs";

    fn names(apps: &[WinApp]) -> Vec<&str> {
        apps.iter().map(|a| a.name.as_str()).collect()
    }

    fn prefix(path: PathBuf, owner: Owner) -> Prefix {
        Prefix { path, owner }
    }

    fn wine_apps(path: &Path) -> Vec<WinApp> {
        apps_in_prefix(&prefix(path.to_path_buf(), Owner::Wine))
    }

    // --- plain Wine shortcuts ------------------------------------------------

    #[test]
    fn finds_start_menu_and_desktop_shortcuts_and_reports_each_app_once() {
        let s = Scratch::new();
        let p = s.prefix("wine");
        let start = s.file(&format!("wine/{START_MENU}/Epubor/Epubor Ultimate.lnk"), "");
        s.file("wine/drive_c/users/me/Desktop/Wasteland 3.lnk", "");
        // The same application again on the Public desktop: Start Menu wins.
        s.file("wine/drive_c/users/Public/Desktop/Epubor Ultimate.lnk", "");

        let apps = wine_apps(&p);
        assert_eq!(names(&apps), vec!["Epubor Ultimate", "Wasteland 3"]);
        assert_eq!(apps[0].launch, Launch::WineShortcut { shortcut: start });
        assert_eq!(apps[0].prefix, p);
    }

    #[test]
    fn identical_desktops_of_several_users_collapse_to_one_app() {
        let s = Scratch::new();
        let p = s.prefix("wine");
        for user in ["loufogle", "steamuser", "Public"] {
            s.file(
                &format!("wine/drive_c/users/{user}/Desktop/WizTree.lnk"),
                "",
            );
        }
        assert_eq!(names(&wine_apps(&p)), vec!["WizTree"]);
    }

    #[test]
    fn per_user_start_menus_are_searched_in_both_layouts() {
        let s = Scratch::new();
        let p = s.prefix("wine");
        s.file(
            "wine/drive_c/users/me/Start Menu/Programs/Wasteland 3 Editor.lnk",
            "",
        );
        s.file(
            "wine/drive_c/users/me/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Ubisoft Connect.lnk",
            "",
        );
        assert_eq!(
            names(&wine_apps(&p)),
            vec!["Ubisoft Connect", "Wasteland 3 Editor"]
        );
    }

    #[test]
    fn windows_and_wine_components_are_not_user_apps() {
        let s = Scratch::new();
        let p = s.prefix("wine");
        s.file(
            "wine/drive_c/users/me/Start Menu/Programs/Accessories/Notepad.lnk",
            "",
        );
        s.file(
            "wine/drive_c/users/me/Start Menu/Programs/System Tools/Registry.lnk",
            "",
        );
        s.file(
            "wine/drive_c/users/me/Start Menu/Programs/Startup/Updater.lnk",
            "",
        );
        s.file(
            &format!("wine/{START_MENU}/Wine/Wine Configuration.lnk"),
            "",
        );
        assert!(wine_apps(&p).is_empty());
    }

    #[test]
    fn uninstallers_and_documentation_links_are_filtered() {
        let s = Scratch::new();
        let p = s.prefix("wine");
        let dir = format!("wine/{START_MENU}/Some Game");
        for name in [
            "Some Game",
            "Uninstall Some Game",
            "Some Game Uninstaller",
            "unins000",
            "Readme",
            "Read Me First",
            "Release Notes",
            "User Manual",
            "Help",
            "Visit the Website",
            "License Agreement",
        ] {
            s.file(&format!("{dir}/{name}.lnk"), "");
        }
        assert_eq!(names(&wine_apps(&p)), vec!["Some Game"]);
    }

    #[test]
    fn noise_words_do_not_match_inside_other_words() {
        for ok in [
            "HelpDesk",
            "Manualist",
            "Supported Hardware Tool",
            "Wasteland 3",
        ] {
            assert!(!is_noise_name(ok), "{ok} is an application");
        }
        for bad in [
            "Uninstall_Foo",
            "Foo-Uninstall",
            "unins001",
            "README",
            "Help",
        ] {
            assert!(is_noise_name(bad), "{bad} is housekeeping");
        }
    }

    #[test]
    fn directory_and_extension_case_is_ignored() {
        let s = Scratch::new();
        let p = s.prefix("wine");
        s.file(
            "wine/drive_c/programdata/microsoft/windows/start menu/programs/App.LNK",
            "",
        );
        s.file("wine/drive_c/users/me/Desktop/Link.url", "");
        s.file("wine/drive_c/users/me/Desktop/notes.txt", "");
        assert_eq!(names(&wine_apps(&p)), vec!["App"]);
    }

    // --- PortProton and Faugus: shortcuts resolved to an executable ----------

    /// A prefix with NTLite installed and a Start Menu shortcut to it.
    fn prefix_with_ntlite(s: &Scratch, rel: &str, target: &str) -> (PathBuf, PathBuf) {
        let p = s.prefix(rel);
        let exe = s.file(
            &format!("{rel}/drive_c/Program Files/NTLite/NTLite.exe"),
            "",
        );
        s.file_bytes(
            &format!("{rel}/{START_MENU}/NTLite/NTLite.lnk"),
            &lnk::fixture(target),
        );
        (p, exe)
    }

    #[test]
    fn portproton_apps_are_launched_by_their_executable() {
        let s = Scratch::new();
        let (p, exe) = prefix_with_ntlite(
            &s,
            "PortProton/data/prefixes/DEFAULT",
            r"C:\Program Files\NTLite\NTLite.exe",
        );
        let apps = apps_in_prefix(&prefix(p, Owner::PortProton));
        assert_eq!(names(&apps), vec!["NTLite"]);
        assert_eq!(apps[0].launch, Launch::PortProton { exe });
        assert_eq!(apps[0].owner, Owner::PortProton);
    }

    #[test]
    fn a_shortcut_target_is_matched_ignoring_case() {
        let s = Scratch::new();
        let (p, exe) =
            prefix_with_ntlite(&s, "Faugus/default", r"c:\PROGRAM FILES\ntlite\NTLITE.EXE");
        let apps = apps_in_prefix(&prefix(p, Owner::Faugus));
        assert_eq!(apps[0].launch, Launch::Faugus { exe });
    }

    #[test]
    fn shortcuts_to_missing_or_unreadable_targets_are_skipped() {
        let s = Scratch::new();
        let p = s.prefix("PortProton/data/prefixes/DEFAULT");
        // Target does not exist.
        s.file_bytes(
            &format!("PortProton/data/prefixes/DEFAULT/{START_MENU}/Gone.lnk"),
            &lnk::fixture(r"C:\Nope\Gone.exe"),
        );
        // Not a shortcut at all.
        s.file(
            &format!("PortProton/data/prefixes/DEFAULT/{START_MENU}/Junk.lnk"),
            "not a link",
        );
        assert!(apps_in_prefix(&prefix(p, Owner::PortProton)).is_empty());
    }

    #[test]
    fn shortcuts_to_web_links_and_documents_are_not_apps() {
        let s = Scratch::new();
        let rel = "PortProton/data/prefixes/DEFAULT";
        let p = s.prefix(rel);
        for file in ["Website.url", "Manual.pdf", "Help.chm"] {
            s.file(&format!("{rel}/drive_c/Program Files/NTLite/{file}"), "");
            s.file_bytes(
                &format!("{rel}/{START_MENU}/NTLite/NTLite on {file}.lnk"),
                &lnk::fixture(&format!(r"C:\Program Files\NTLite\{file}")),
            );
        }
        assert!(apps_in_prefix(&prefix(p.clone(), Owner::PortProton)).is_empty());

        // A batch file is a program a launcher can run.
        let bat = s.file(&format!("{rel}/drive_c/Games/Run.BAT"), "");
        s.file_bytes(
            &format!("{rel}/{START_MENU}/Run Game.lnk"),
            &lnk::fixture(r"C:\Games\Run.BAT"),
        );
        let apps = apps_in_prefix(&prefix(p, Owner::PortProton));
        assert_eq!(names(&apps), vec!["Run Game"]);
        assert_eq!(apps[0].launch, Launch::PortProton { exe: bat });
    }

    #[test]
    fn two_shortcuts_to_one_program_are_one_app() {
        let s = Scratch::new();
        let (p, _) = prefix_with_ntlite(
            &s,
            "PortProton/data/prefixes/DEFAULT",
            r"C:\Program Files\NTLite\NTLite.exe",
        );
        s.file_bytes(
            "PortProton/data/prefixes/DEFAULT/drive_c/users/Public/Desktop/Play NTLite.lnk",
            &lnk::fixture(r"C:\Program Files\NTLite\NTLite.exe"),
        );
        assert_eq!(apps_in_prefix(&prefix(p, Owner::PortProton)).len(), 1);
    }

    #[test]
    fn only_the_default_faugus_prefix_is_handled() {
        let s = Scratch::new();
        let home = s.path();
        let default = s.prefix("Faugus/default");
        let other = s.prefix("Faugus/other");
        s.file(
            ".var/app/io.github.Faugus.faugus-launcher/config/faugus-launcher/config.json",
            &format!(
                "{{\n    \"default-prefix\": \"{}\",\n    \"mangohud\": \"False\"\n}}",
                home.join("Faugus").display()
            ),
        );
        let tools = Tools {
            home: home.to_path_buf(),
            ..Tools::default()
        };
        assert!(is_handled(&prefix(default, Owner::Faugus), &tools));
        assert!(!is_handled(&prefix(other, Owner::Faugus), &tools));
    }

    #[test]
    fn faugus_without_a_config_is_not_handled() {
        let s = Scratch::new();
        let p = s.prefix("Faugus/default");
        let tools = Tools {
            home: s.path().to_path_buf(),
            ..Tools::default()
        };
        assert!(!is_handled(&prefix(p, Owner::Faugus), &tools));
    }

    #[test]
    fn other_prefixes_are_never_handled() {
        let tools = Tools::default();
        for owner in [Owner::PlayOnLinux, Owner::Other] {
            assert!(!is_handled(&prefix(PathBuf::from("/x"), owner), &tools));
        }
        for owner in [
            Owner::Wine,
            Owner::PortProton,
            Owner::Bottles,
            Owner::WineZGUI,
            Owner::Lutris,
        ] {
            assert!(is_handled(&prefix(PathBuf::from("/x"), owner), &tools));
        }
    }

    // --- Bottles ---------------------------------------------------------------

    const BOTTLE_YML: &str = "\
Environment: Gaming
External_Programs:
    5595f488-e764-4318-9942-3855195b0cd2:
        arguments: WINEDLLOVERRIDES=\"locationapi=d\" WINE_SIMULATE_WRITECOPY=1 %command%
        executable: Battle.net.exe
        id: 5595f488-e764-4318-9942-3855195b0cd2
        name: Battle.net
        path: C:\\Program Files (x86)\\Battle.net\\Battle.net.exe
    cd7c90ce-f35e-4d86-a7d2-4a3d8fc07324:
        auto_discovered: true
        executable: Amazon Games.exe
        folder: /home/u/Amazon
            Games/App
        id: cd7c90ce-f35e-4d86-a7d2-4a3d8fc07324
        name: Amazon Games
Inherited_Environment_Variables: []
Name: Gaming
Path: Gaming
";

    #[test]
    fn bottles_programs_are_read_from_the_bottle_config() {
        let s = Scratch::new();
        let p = s.prefix("bottles/Gaming");
        s.file("bottles/Gaming/bottle.yml", BOTTLE_YML);
        let apps = apps_in_prefix(&prefix(p, Owner::Bottles));
        assert_eq!(names(&apps), vec!["Battle.net", "Amazon Games"]);
        assert_eq!(
            apps[0].launch,
            Launch::Bottles {
                bottle: "Gaming".into(),
                program: "Battle.net".into()
            }
        );
    }

    #[test]
    fn a_bottle_with_no_programs_or_no_config_has_no_apps() {
        let s = Scratch::new();
        let none = s.prefix("bottles/Empty");
        s.file(
            "bottles/Empty/bottle.yml",
            "External_Programs: {}\nName: Empty\n",
        );
        let missing = s.prefix("bottles/Sketchup");
        assert!(apps_in_prefix(&prefix(none, Owner::Bottles)).is_empty());
        assert!(apps_in_prefix(&prefix(missing, Owner::Bottles)).is_empty());
    }

    // --- WineZGUI --------------------------------------------------------------

    const WINEZGUI_DESKTOP: &str = "\
[Desktop Entry]
Name=ARMORED CORE VI
Type=Application
Exec=bash -c \"'/prefixes/armoredcore6/armoredcore6.sh'\"
Icon=/prefixes/armoredcore6/armoredcore6.png
Categories=Game;
";

    #[test]
    fn winezgui_reuses_the_desktop_file_it_keeps_in_the_prefix() {
        let s = Scratch::new();
        let p = s.prefix("Prefixes/armoredcore6");
        let desktop = s.file(
            "Prefixes/armoredcore6/armoredcore6.desktop",
            WINEZGUI_DESKTOP,
        );
        s.file("Prefixes/armoredcore6/readme.txt", "");
        s.file(
            "Prefixes/armoredcore6/no-exec.desktop",
            "[Desktop Entry]\nName=X\n",
        );

        let apps = apps_in_prefix(&prefix(p, Owner::WineZGUI));
        assert_eq!(names(&apps), vec!["ARMORED CORE VI"]);
        assert_eq!(apps[0].launch, Launch::WineZgui { desktop });

        let text = render(&apps[0], &Tools::default()).expect("renders");
        // WineZGUI's own lines are untouched; only the marker is added.
        for line in WINEZGUI_DESKTOP.lines() {
            assert!(text.lines().any(|l| l == line), "lost line {line}");
        }
        assert!(text.contains("X-NexusMenu-Generated=true"));
        assert!(text.contains("X-NexusMenu-Source=winezgui"));
        assert!(text.starts_with("[Desktop Entry]\nX-NexusMenu-Generated"));
    }

    #[test]
    fn a_desktop_file_without_a_header_cannot_be_copied() {
        assert_eq!(mark_generated("Name=X\n", "winezgui"), None);
    }

    // --- Lutris ----------------------------------------------------------------

    fn lutris_yml(exe: &str, prefix: &str) -> String {
        format!(
            "game:\n  args: -x\n  exe: {exe}\n  prefix: {prefix}\ngame_slug: sacred-2-gold\nname: Sacred 2 Gold\nscript:\n  game:\n    exe: drive_c/x.exe\n    prefix: $GAMEDIR\nslug: sacred-2-gold-gog\n"
        )
    }

    #[test]
    fn lutris_games_are_read_from_their_configs() {
        let s = Scratch::new();
        let exe = s.file("Games/sacred/drive_c/sacred2.exe", "");
        s.file(
            ".config/lutris/games/sacred-1.yml",
            &lutris_yml(
                &exe.to_string_lossy(),
                &s.path().join("Games/sacred").to_string_lossy(),
            ),
        );
        let apps = lutris_apps(s.path());
        assert_eq!(names(&apps), vec!["Sacred 2 Gold"]);
        assert_eq!(
            apps[0].launch,
            Launch::Lutris {
                slug: "sacred-2-gold".into()
            }
        );
    }

    #[test]
    fn lutris_games_that_are_not_installed_or_not_windows_are_skipped() {
        let s = Scratch::new();
        // exe missing on disk: not installed.
        s.file(
            ".config/lutris/games/gone.yml",
            &lutris_yml("/nope/game.exe", "/nope"),
        );
        // A native game: no Wine prefix.
        let native = s.file("Games/native/run.sh", "");
        s.file(
            ".config/lutris/games/native.yml",
            &format!(
                "game:\n  exe: {}\ngame_slug: native\nname: Native\n",
                native.display()
            ),
        );
        assert!(lutris_apps(s.path()).is_empty());
    }

    // --- launch commands -------------------------------------------------------

    fn tools(home: &Path) -> Tools {
        Tools {
            home: home.to_path_buf(),
            wine: Some(PathBuf::from("/usr/bin/wine")),
            flatpak: Some(PathBuf::from("/usr/bin/flatpak")),
            lutris: Some(PathBuf::from("/usr/games/lutris")),
            bottles_cli: Some(PathBuf::from("/usr/bin/bottles-cli")),
            faugus: Some(PathBuf::from("/usr/bin/faugus-launcher")),
            portproton_script: Some(PathBuf::from(
                "/home/u/PortProton/data_from_portwine/scripts/start.sh",
            )),
        }
    }

    fn app(owner: Owner, launch: Launch) -> WinApp {
        WinApp {
            name: "Some App".into(),
            owner,
            prefix: PathBuf::from("/p/My Prefix"),
            launch,
        }
    }

    /// The words of the `Exec=` an app renders to, as a desktop would run them.
    fn argv(app: &WinApp, tools: &Tools) -> Vec<String> {
        let s = Scratch::new();
        let text = render(app, tools).expect("renders");
        let entry = desktop::parse(&s.file("e.desktop", &text)).expect("parses");
        desktop::tokenize_exec(&entry.exec.expect("has Exec"))
    }

    #[test]
    fn wine_runs_the_shortcut_through_start_exe() {
        let t = tools(Path::new("/home/u"));
        let a = app(
            Owner::Wine,
            Launch::WineShortcut {
                shortcut: PathBuf::from("/p/My Prefix/Desktop/Assassin's \"Creed\" (X).lnk"),
            },
        );
        assert_eq!(
            argv(&a, &t),
            vec![
                "env",
                "WINEPREFIX=/p/My Prefix",
                "/usr/bin/wine",
                r"C:\windows\command\start.exe",
                "/Unix",
                "/p/My Prefix/Desktop/Assassin's \"Creed\" (X).lnk",
            ]
        );
    }

    #[test]
    fn portproton_flatpak_runs_the_exe_through_the_flatpak() {
        let s = Scratch::new();
        s.file(".var/app/ru.linux_gaming.PortProton/data/scripts/.keep", "");
        let t = tools(s.path());
        let a = app(
            Owner::PortProton,
            Launch::PortProton {
                exe: PathBuf::from("/p/drive_c/Program Files/NTLite/NTLite.exe"),
            },
        );
        assert_eq!(
            argv(&a, &t),
            vec![
                "/usr/bin/flatpak",
                "run",
                "ru.linux_gaming.PortProton",
                "/p/drive_c/Program Files/NTLite/NTLite.exe",
            ]
        );
        // PortProton's own entries run from its scripts directory.
        let text = render(&a, &t).unwrap();
        let scripts = s
            .path()
            .join(".var/app/ru.linux_gaming.PortProton/data/scripts");
        assert!(text.contains(&format!("Path={}\n", scripts.display())));
    }

    #[test]
    fn portproton_without_a_flatpak_runs_its_start_script() {
        let s = Scratch::new();
        let t = tools(s.path());
        let a = app(
            Owner::PortProton,
            Launch::PortProton {
                exe: PathBuf::from("/p/game.exe"),
            },
        );
        assert_eq!(
            argv(&a, &t),
            vec![
                "env",
                "/home/u/PortProton/data_from_portwine/scripts/start.sh",
                "/p/game.exe",
            ]
        );
    }

    #[test]
    fn faugus_flatpak_passes_the_exe_the_way_its_own_file_handler_does() {
        let s = Scratch::new();
        s.file(".var/app/io.github.Faugus.faugus-launcher/.keep", "");
        let t = tools(s.path());
        let a = app(
            Owner::Faugus,
            Launch::Faugus {
                exe: PathBuf::from("/p/Game Dir/game.exe"),
            },
        );
        assert_eq!(
            argv(&a, &t),
            vec![
                "/usr/bin/flatpak",
                "run",
                "--branch=stable",
                "--arch=x86_64",
                "--command=faugus-launcher",
                "--file-forwarding",
                "io.github.Faugus.faugus-launcher",
                "@@",
                "/p/Game Dir/game.exe",
                "@@",
            ]
        );
    }

    #[test]
    fn faugus_installed_natively_runs_the_binary() {
        let s = Scratch::new();
        let t = tools(s.path());
        let a = app(
            Owner::Faugus,
            Launch::Faugus {
                exe: PathBuf::from("/p/game.exe"),
            },
        );
        assert_eq!(
            argv(&a, &t),
            vec!["/usr/bin/faugus-launcher", "/p/game.exe"]
        );
    }

    #[test]
    fn lutris_runs_a_game_by_slug_the_way_its_own_shortcuts_do() {
        let t = tools(Path::new("/home/u"));
        let a = app(
            Owner::Lutris,
            Launch::Lutris {
                slug: "sacred-2-gold".into(),
            },
        );
        assert_eq!(
            argv(&a, &t),
            vec![
                "env",
                "LUTRIS_SKIP_INIT=1",
                "/usr/games/lutris",
                "lutris:rungame/sacred-2-gold",
            ]
        );
    }

    #[test]
    fn bottles_runs_a_program_by_bottle_and_name() {
        let s = Scratch::new();
        let a = app(
            Owner::Bottles,
            Launch::Bottles {
                bottle: "Gaming".into(),
                program: "Battle.net".into(),
            },
        );

        // Native install.
        assert_eq!(
            argv(&a, &tools(s.path())),
            vec![
                "/usr/bin/bottles-cli",
                "run",
                "-b",
                "Gaming",
                "-p",
                "Battle.net"
            ]
        );

        // Flatpak install.
        s.file(".var/app/com.usebottles.bottles/.keep", "");
        assert_eq!(
            argv(&a, &tools(s.path())),
            vec![
                "/usr/bin/flatpak",
                "run",
                "--command=bottles-cli",
                "com.usebottles.bottles",
                "run",
                "-b",
                "Gaming",
                "-p",
                "Battle.net",
            ]
        );
    }

    #[test]
    fn names_with_spaces_stay_one_argument() {
        let s = Scratch::new();
        let a = app(
            Owner::Bottles,
            Launch::Bottles {
                bottle: "My Bottle".into(),
                program: "Amazon Games".into(),
            },
        );
        let words = argv(&a, &tools(s.path()));
        assert!(words.contains(&"My Bottle".to_string()));
        assert!(words.contains(&"Amazon Games".to_string()));
    }

    #[test]
    fn an_app_whose_launcher_is_not_installed_cannot_be_launched() {
        let none = Tools {
            home: PathBuf::from("/nowhere"),
            ..Tools::default()
        };
        let apps = [
            app(
                Owner::Wine,
                Launch::WineShortcut {
                    shortcut: PathBuf::from("/x.lnk"),
                },
            ),
            app(
                Owner::PortProton,
                Launch::PortProton {
                    exe: PathBuf::from("/x.exe"),
                },
            ),
            app(
                Owner::Faugus,
                Launch::Faugus {
                    exe: PathBuf::from("/x.exe"),
                },
            ),
            app(Owner::Lutris, Launch::Lutris { slug: "x".into() }),
            app(
                Owner::Bottles,
                Launch::Bottles {
                    bottle: "b".into(),
                    program: "p".into(),
                },
            ),
        ];
        for a in &apps {
            assert!(!launchable(a, &none), "{:?}", a.launch);
            assert_eq!(render(a, &none), None);
        }
    }

    // --- rendered entries ------------------------------------------------------

    #[test]
    fn rendered_entries_are_marked_generated_and_never_use_the_legacy_marker() {
        let s = Scratch::new();
        let t = tools(Path::new("/home/u"));
        let a = app(Owner::Lutris, Launch::Lutris { slug: "x".into() });
        let text = render(&a, &t).unwrap();
        assert!(!text.contains("X-KAppFinder"));
        assert!(text.contains("X-NexusMenu-Source=lutris"));
        let entry = desktop::parse(&s.file("e.desktop", &text)).unwrap();
        assert!(entry.generated);
        assert_eq!(entry.entry_type, "Application");
        assert_eq!(entry.try_exec.as_deref(), Some("/usr/games/lutris"));
    }

    // --- coverage by existing entries -----------------------------------------

    fn registry_with(entries: &[(&str, &str)]) -> (Scratch, Registry) {
        let s = Scratch::new();
        for (file, exec) in entries {
            s.file(
                &format!("applications/{file}.desktop"),
                &format!("[Desktop Entry]\nType=Application\nName=X\nExec={exec}\n"),
            );
        }
        let reg = Registry::build(Some(&s.path().join("applications")), false);
        (s, reg)
    }

    #[test]
    fn an_app_is_covered_when_an_entry_already_launches_it() {
        let (_s, reg) = registry_with(&[
            (
                "wine",
                r"env WINEPREFIX=/p wine C:\\windows\\command\\start.exe /Unix /p/Start\\ Menu/My\\ App.lnk",
            ),
            (
                "pp",
                "flatpak run ru.linux_gaming.PortProton \"/p/Game One/game.exe\"",
            ),
            (
                "fg",
                "/usr/bin/flatpak run --command=faugus-launcher x @@ \"/f/other.exe\" @@",
            ),
            (
                "lutris",
                "env LUTRIS_SKIP_INIT=1 lutris lutris:rungame/sacred-2-gold",
            ),
            ("bottle", "bottles-cli run -b Gaming -p \"Battle.net\""),
            (
                "zgui",
                "bash -c \"'/zgui/Prefixes/armoredcore6-1a2b/armoredcore6.sh'\"",
            ),
        ]);

        let wine = |lnk: &str| {
            app(
                Owner::Wine,
                Launch::WineShortcut {
                    shortcut: PathBuf::from(lnk),
                },
            )
        };
        assert!(covered(&wine("/p/Start Menu/My App.lnk"), &reg));
        assert!(!covered(&wine("/p/Start Menu/Other.lnk"), &reg));

        let pp = |exe: &str| {
            app(
                Owner::PortProton,
                Launch::PortProton {
                    exe: PathBuf::from(exe),
                },
            )
        };
        assert!(covered(&pp("/p/Game One/game.exe"), &reg));
        assert!(!covered(&pp("/p/Game Two/game.exe"), &reg));

        let fg = |exe: &str| {
            app(
                Owner::Faugus,
                Launch::Faugus {
                    exe: PathBuf::from(exe),
                },
            )
        };
        assert!(covered(&fg("/f/other.exe"), &reg));
        assert!(!covered(&fg("/f/new.exe"), &reg));

        let lutris = |slug: &str| app(Owner::Lutris, Launch::Lutris { slug: slug.into() });
        assert!(covered(&lutris("sacred-2-gold"), &reg));
        // A shorter slug is a different game, not covered by the longer one.
        assert!(!covered(&lutris("sacred-2"), &reg));
        assert!(!covered(&lutris("the-ascent"), &reg));

        let bottle = |b: &str, p: &str| {
            app(
                Owner::Bottles,
                Launch::Bottles {
                    bottle: b.into(),
                    program: p.into(),
                },
            )
        };
        assert!(covered(&bottle("Gaming", "Battle.net"), &reg));
        assert!(!covered(&bottle("Gaming", "Battle"), &reg));
        assert!(!covered(&bottle("Gaming", "Amazon Games"), &reg));
        assert!(!covered(&bottle("Other", "Battle.net"), &reg));

        let zgui = |dir: &str| {
            app(
                Owner::WineZGUI,
                Launch::WineZgui {
                    desktop: PathBuf::from(dir).join("x.desktop"),
                },
            )
        };
        assert!(covered(&zgui("/zgui/Prefixes/armoredcore6-1a2b"), &reg));
        // The same name without its suffix is another prefix.
        assert!(!covered(&zgui("/zgui/Prefixes/armoredcore6"), &reg));
        assert!(!covered(&zgui("/zgui/Prefixes/sacred2"), &reg));
    }

    #[test]
    fn a_rendered_entry_covers_its_own_app() {
        let s = Scratch::new();
        let t = tools(Path::new("/home/u"));
        let apps = [
            app(
                Owner::Wine,
                Launch::WineShortcut {
                    shortcut: PathBuf::from("/p/My Prefix/Desktop/A B.lnk"),
                },
            ),
            app(
                Owner::PortProton,
                Launch::PortProton {
                    exe: PathBuf::from("/p/My Prefix/drive_c/a b.exe"),
                },
            ),
            app(
                Owner::Faugus,
                Launch::Faugus {
                    exe: PathBuf::from("/p/My Prefix/drive_c/c d.exe"),
                },
            ),
            app(
                Owner::Lutris,
                Launch::Lutris {
                    slug: "sacred-2-gold".into(),
                },
            ),
            app(
                Owner::Bottles,
                Launch::Bottles {
                    bottle: "Gaming".into(),
                    program: "Battle.net".into(),
                },
            ),
        ];
        for (i, a) in apps.iter().enumerate() {
            s.file(
                &format!("applications/{i}.desktop"),
                &render(a, &t).expect("renders"),
            );
        }
        let reg = Registry::build(Some(&s.path().join("applications")), false);
        for a in &apps {
            assert!(
                covered(a, &reg),
                "{:?} is not covered by its own entry",
                a.launch
            );
        }
    }

    // --- file names ------------------------------------------------------------

    #[test]
    fn file_names_embed_owner_prefix_and_a_safe_slug() {
        let mut a = app(
            Owner::PortProton,
            Launch::PortProton {
                exe: PathBuf::from("/x.exe"),
            },
        );
        a.prefix = PathBuf::from("/h/PortProton/data/prefixes/GOG");
        a.name = "Assassin's Creed Mirage".into();
        assert_eq!(
            filename(&a),
            "windows-portproton-gog-assassin-s-creed-mirage.desktop"
        );

        // A prefix named like its owner does not repeat it.
        let mut wine = app(
            Owner::Wine,
            Launch::WineShortcut {
                shortcut: PathBuf::from("/x.lnk"),
            },
        );
        wine.prefix = PathBuf::from("/home/u/.wine");
        wine.name = "***".into();
        assert_eq!(filename(&wine), "windows-wine-app.desktop");
    }

    #[test]
    fn the_same_app_in_two_prefixes_gets_two_file_names() {
        let mk = |prefix: &str| {
            let mut a = app(
                Owner::Wine,
                Launch::WineShortcut {
                    shortcut: PathBuf::from("/x.lnk"),
                },
            );
            a.prefix = PathBuf::from(prefix);
            a
        };
        assert_ne!(
            filename(&mk("/home/u/.wine")),
            filename(&mk("/home/u/.insomniac"))
        );
    }

    // --- collecting ------------------------------------------------------------

    #[test]
    fn collect_reads_each_owner_and_reports_the_prefixes_it_cannot_handle() {
        let s = Scratch::new();
        let wine = s.prefix(".wine");
        s.file(&format!(".wine/{START_MENU}/WizTree.lnk"), "");
        let pol = s.prefix(".PlayOnLinux/wineprefix/ntlite");
        let other = s.prefix("Documents/stray");
        let t = tools(s.path());

        let prefixes = [
            prefix(wine, Owner::Wine),
            prefix(pol.clone(), Owner::PlayOnLinux),
            prefix(other.clone(), Owner::Other),
        ];
        let found = collect(&prefixes, &t);
        assert_eq!(names(&found.apps), vec!["WizTree"]);
        assert_eq!(
            found
                .unhandled
                .iter()
                .map(|p| p.path.clone())
                .collect::<Vec<_>>(),
            vec![pol, other]
        );
    }
}
