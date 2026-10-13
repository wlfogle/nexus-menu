//! Editing, hiding and restoring menu entries, including ones this tool did
//! not create.
//!
//! An entry is identified by its *desktop file ID*: its path inside an
//! `applications` directory with the directory prefix removed and `/` turned
//! into `-`. When several files share an ID the first directory in precedence
//! order wins, and the user's own directory comes first. So a file in the
//! user's directory with a system entry's name replaces that entry for this
//! user and leaves the system file alone. That is how an edit of a system
//! entry is made here, the same way KDE's menu editor does it.
//!
//! Everything is built to be undone, and to be hard to misuse:
//!
//! - Writes go only to the user's applications directory and to a backup
//!   directory beside it. System directories are never written.
//! - Callers name entries by ID only. An ID must look like a plain file name
//!   (no `/`, no leading `.`), and is looked up again on every call, so a path
//!   cannot be smuggled in.
//! - Before the first change to a file this tool did not make, the original is
//!   saved once as `<id>.orig`. That snapshot is what "restore" uses and what
//!   tells a system entry that changed since it was edited.
//! - A file is only ever *deleted* if it carries one of this tool's markers
//!   ([`GENERATED_KEY`] or [`OVERRIDE_KEY`]). The user's own hand-written
//!   files are moved to the backup directory, never deleted, and system files
//!   are never touched.
//! - Only the `[Desktop Entry]` group is edited, line by line. Comments,
//!   unknown keys and `[Desktop Action ...]` groups are kept as they were.
//!
//! Every function takes its directories as a [`Dirs`], so tests run against
//! scratch trees and never the real menu.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::desktop::{
    escape_value, unescape_value, GENERATED_KEY, LEGACY_GENERATED_KEY, OVERRIDE_KEY,
};
use crate::registry;

/// The directories everything here reads and writes.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// The user's applications directory: the only place entries are written.
    pub user: PathBuf,
    /// Where originals and removed files are kept.
    pub backups: PathBuf,
    /// Directories searched after `user`, in precedence order. Read only.
    pub system: Vec<PathBuf>,
}

impl Dirs {
    pub fn new(user: PathBuf, backups: PathBuf, system: Vec<PathBuf>) -> Dirs {
        Dirs {
            user,
            backups,
            system,
        }
    }

    /// The usual layout around `user`: backups in a `nexus-menu/backups` folder
    /// beside it, and every other application directory the environment names
    /// (system, Flatpak, Snap) as read-only sources.
    pub fn around(user: PathBuf) -> Dirs {
        let backups = match user.parent() {
            Some(parent) => parent.join("nexus-menu/backups"),
            None => registry::data_home().join("nexus-menu/backups"),
        };
        let system = registry::application_dirs()
            .into_iter()
            .filter(|d| *d != user)
            .collect();
        Dirs {
            user,
            backups,
            system,
        }
    }
}

// ---------------------------------------------------------------------------
// Public data
// ---------------------------------------------------------------------------

/// Where an entry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Written by nexus-menu (carries its marker).
    Created,
    /// A file in the user's own directory that nexus-menu did not write.
    Yours,
    /// Installed by a package; may have an edited copy in the user's directory.
    System,
}

/// One entry in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryInfo {
    pub id: String,
    pub name: String,
    pub origin: Origin,
    /// The winning file asks not to be shown in menus (`NoDisplay`/`Hidden`).
    pub hidden: bool,
    /// The user's directory holds a copy of this tool's making or editing.
    pub edited: bool,
    /// The user's own file was removed; it is kept in the backup directory.
    pub removed: bool,
    /// The system entry changed since it was edited, so the edited copy may be
    /// hiding an update.
    pub stale: bool,
    /// "Restore original" has something to restore.
    pub restorable: bool,
}

/// The fields that can be edited.
///
/// `categories` and `keywords` are `;`-separated lists as in the file. They
/// are normalised on read and on write, so `A; B;;` and `A;B;` are the same.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Fields {
    pub name: String,
    pub comment: String,
    pub icon: String,
    pub exec: String,
    pub terminal: bool,
    pub categories: String,
    pub keywords: String,
}

/// An entry opened for editing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryDetail {
    pub info: EntryInfo,
    pub fields: Fields,
    /// The file a save would write.
    pub writes_to: String,
    /// What a save will do beyond the fields themselves.
    pub notes: Vec<String>,
}

/// Why one field was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum SaveError {
    /// The values were refused; nothing was written.
    Invalid(Vec<FieldError>),
    /// The save could not be done; `detail` says why.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Saved {
    /// The file that now holds the entry.
    pub path: String,
    /// False when the values were already what the file said.
    pub changed: bool,
    /// A new copy of a system entry was made in the user's directory.
    pub created_override: bool,
}

/// What removing an entry did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Removal {
    /// The file was made by nexus-menu and has been deleted.
    Deleted,
    /// The user's own file was moved to the backup directory.
    MovedToBackup,
    /// A copy marked `NoDisplay=true` now hides it; nothing was deleted.
    Hidden,
    AlreadyHidden,
}

/// What restoring an entry did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Restored {
    /// The edited copy was deleted; the system entry is back.
    OverrideRemoved,
    /// The user's own file was put back as it was before the first edit.
    OriginalRestored,
    /// The user's own file that was removed was put back.
    RemovedRestored,
}

// ---------------------------------------------------------------------------
// Names and discovery
// ---------------------------------------------------------------------------

/// Whether `id` is something that can be looked up: a plain file name ending
/// in `.desktop`. Anything that could name another place (a path, a hidden
/// file, `..`, a backslash, a control character) is refused before it gets near
/// the file system. Everything else is allowed, because real entries have names
/// like `cxassoc-cxoffice-0:application_x-crossover-exe::run.desktop` and
/// `Battle․net.desktop`, and an entry that cannot be named cannot be edited or
/// hidden.
pub fn valid_id(id: &str) -> bool {
    id.len() > ".desktop".len()
        && id.len() <= 255
        && id.ends_with(".desktop")
        && !id.starts_with('.')
        && !id.contains("..")
        && !id.chars().any(|c| c.is_control() || c == '/' || c == '\\')
}

/// The desktop file ID of `path`, found inside `dir`.
fn id_of(dir: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(dir).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("-"))
}

/// Every manageable entry in `dir`, by ID. The first file for an ID wins.
fn scan(dir: &Path, into: &mut BTreeMap<String, PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    let mut files: Vec<PathBuf> = WalkDir::new(dir)
        .follow_links(true)
        .max_depth(8)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("desktop")
        })
        .map(|e| e.into_path())
        .collect();
    // Sorted, so that the choice between `a-b.desktop` and `a/b.desktop`
    // (undefined by the spec) is at least the same every time.
    files.sort();
    for path in files {
        if let Some(id) = id_of(dir, &path) {
            if valid_id(&id) {
                into.entry(id).or_insert(path);
            }
        }
    }
}

struct Index {
    user: BTreeMap<String, PathBuf>,
    system: BTreeMap<String, PathBuf>,
}

fn index(dirs: &Dirs) -> Index {
    let mut user = BTreeMap::new();
    scan(&dirs.user, &mut user);
    let mut system = BTreeMap::new();
    for dir in &dirs.system {
        scan(dir, &mut system);
    }
    Index { user, system }
}

impl Index {
    /// The file that wins for `id`, and whether it is in the user's directory.
    fn winner(&self, id: &str) -> Option<(&PathBuf, bool)> {
        match self.user.get(id) {
            Some(p) => Some((p, true)),
            None => self.system.get(id).map(|p| (p, false)),
        }
    }
}

fn orig_path(dirs: &Dirs, id: &str) -> PathBuf {
    dirs.backups.join(format!("{id}.orig"))
}

fn removed_path(dirs: &Dirs, id: &str) -> PathBuf {
    dirs.backups.join(format!("{id}.removed"))
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    String::from_utf8(bytes).map_err(|_| format!("{} is not valid UTF-8", path.display()))
}

/// Write `text` to `path` through a temporary file in the same directory, so
/// an interruption cannot leave a half-written entry in the menu.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("{} has no directory", path.display()))?;
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?
        .to_string_lossy();
    let tmp = dir.join(format!(".{name}.nexus-menu-tmp"));
    fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("cannot install {}: {e}", path.display()));
    }
    Ok(())
}

/// Save `text` as the original of `id`, once. True if it was written now.
fn ensure_snapshot(dirs: &Dirs, id: &str, text: &str) -> Result<bool, String> {
    let path = orig_path(dirs, id);
    if path.is_file() {
        return Ok(false);
    }
    write_atomic(&path, text)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// A desktop file, edited line by line
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Doc {
    lines: Vec<String>,
}

fn is_group_header(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('[') && t.ends_with(']')
}

/// `key=value` of a line, if it is one.
fn key_value(line: &str) -> Option<(&str, &str)> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') || is_group_header(t) {
        return None;
    }
    let (k, v) = t.split_once('=')?;
    Some((k.trim(), v.trim()))
}

impl Doc {
    fn parse(text: &str) -> Doc {
        Doc {
            lines: text.lines().map(str::to_string).collect(),
        }
    }

    fn render(&self) -> String {
        let mut s = self.lines.join("\n");
        s.push('\n');
        s
    }

    fn header(&self) -> Option<usize> {
        self.lines
            .iter()
            .position(|l| l.trim() == "[Desktop Entry]")
    }

    /// The lines of the `[Desktop Entry]` group: (first, one past the last).
    fn range(&self) -> Option<(usize, usize)> {
        let h = self.header()?;
        let end = self.lines[h + 1..]
            .iter()
            .position(|l| is_group_header(l))
            .map(|p| h + 1 + p)
            .unwrap_or(self.lines.len());
        Some((h + 1, end))
    }

    fn matching(&self, test: impl Fn(&str) -> bool) -> Vec<usize> {
        let Some((start, end)) = self.range() else {
            return Vec::new();
        };
        (start..end)
            .filter(|&i| key_value(&self.lines[i]).is_some_and(|(k, _)| test(k)))
            .collect()
    }

    /// The raw (still escaped) value of `key`.
    fn get(&self, key: &str) -> Option<String> {
        let i = *self.matching(|k| k == key).first()?;
        key_value(&self.lines[i]).map(|(_, v)| v.to_string())
    }

    fn text(&self, key: &str) -> Option<String> {
        self.get(key).map(|v| unescape_value(&v))
    }

    fn flag(&self, key: &str) -> bool {
        self.get(key)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    /// Set `key` to the already-escaped `raw`: replace the first line that has
    /// it, drop any duplicates, or add it at the end of the group.
    fn set(&mut self, key: &str, raw: &str) {
        let line = format!("{key}={raw}");
        let hits = self.matching(|k| k == key);
        if let Some(&first) = hits.first() {
            self.lines[first] = line;
            for &i in hits[1..].iter().rev() {
                self.lines.remove(i);
            }
        } else if let Some((start, end)) = self.range() {
            let mut at = end;
            while at > start && self.lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            self.lines.insert(at, line);
        }
    }

    fn remove(&mut self, key: &str) {
        for i in self.matching(|k| k == key).into_iter().rev() {
            self.lines.remove(i);
        }
    }

    /// Drop `key[locale]=...` lines: translations of a value that changed.
    fn remove_localized(&mut self, key: &str) {
        let prefix = format!("{key}[");
        for i in self
            .matching(|k| k.starts_with(&prefix) && k.ends_with(']'))
            .into_iter()
            .rev()
        {
            self.lines.remove(i);
        }
    }

    fn locales(&self, key: &str) -> Vec<String> {
        let prefix = format!("{key}[");
        self.matching(|k| k.starts_with(&prefix) && k.ends_with(']'))
            .into_iter()
            .filter_map(|i| {
                let (k, _) = key_value(&self.lines[i])?;
                Some(k[prefix.len()..k.len() - 1].to_string())
            })
            .collect()
    }

    /// Put `key=true` right under the header.
    fn mark(&mut self, key: &str) {
        self.remove(key);
        if let Some(h) = self.header() {
            self.lines.insert(h + 1, format!("{key}=true"));
        }
    }

    fn is_application(&self) -> bool {
        self.text("Type").as_deref() == Some("Application")
    }

    fn is_generated(&self) -> bool {
        self.flag(GENERATED_KEY) || self.flag(LEGACY_GENERATED_KEY)
    }

    fn is_override(&self) -> bool {
        self.flag(OVERRIDE_KEY)
    }

    fn hidden(&self) -> bool {
        self.flag("NoDisplay") || self.flag("Hidden")
    }
}

// ---------------------------------------------------------------------------
// Fields: reading, validating, applying
// ---------------------------------------------------------------------------

/// Split a `;`-list into its non-empty, trimmed, de-duplicated items.
fn normalize_list(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in raw.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        if !out.iter().any(|o| o == item) {
            out.push(item.to_string());
        }
    }
    out
}

fn list_value(items: &[String]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        format!("{};", items.join(";"))
    }
}

fn read_fields(doc: &Doc) -> Fields {
    let list = |key: &str| list_value(&normalize_list(&doc.text(key).unwrap_or_default()));
    Fields {
        name: doc.text("Name").unwrap_or_default(),
        comment: doc.text("Comment").unwrap_or_default(),
        icon: doc.text("Icon").unwrap_or_default(),
        exec: doc.text("Exec").unwrap_or_default(),
        terminal: doc.flag("Terminal"),
        categories: list("Categories"),
        keywords: list("Keywords"),
    }
}

/// Whether every `"` in an `Exec` value is closed (backslash escapes the next
/// character inside quotes, as the spec's quoting rules say).
fn quotes_balanced(exec: &str) -> bool {
    let mut quoted = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                chars.next();
            }
            _ => {}
        }
    }
    !quoted
}

/// What the page calls a field, so every message reads as a sentence.
fn label(field: &str) -> &'static str {
    match field {
        "name" => "Name",
        "comment" => "Comment",
        "exec" => "Command",
        "icon" => "Icon",
        "categories" => "Categories",
        "keywords" => "Keywords",
        _ => "This field",
    }
}

fn single_line(
    field: &'static str,
    value: &str,
    max: usize,
    errors: &mut Vec<FieldError>,
) -> String {
    let v = value.trim().to_string();
    if v.chars().any(char::is_control) {
        errors.push(FieldError {
            field,
            message: format!(
                "{} must be one line, with no control characters",
                label(field)
            ),
        });
    } else if v.chars().count() > max {
        errors.push(FieldError {
            field,
            message: format!("{} is too long (at most {max} characters)", label(field)),
        });
    }
    v
}

fn valid_category(token: &str) -> bool {
    token
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Check and normalise `input`. Nothing is written unless this succeeds.
fn validate(input: &Fields) -> Result<Fields, Vec<FieldError>> {
    let mut errors = Vec::new();
    let need = |field: &'static str, value: &str, errors: &mut Vec<FieldError>| {
        if value.is_empty() {
            errors.push(FieldError {
                field,
                message: format!("{} is required", label(field)),
            });
        }
    };

    let name = single_line("name", &input.name, 200, &mut errors);
    need("name", &name, &mut errors);

    let comment = single_line("comment", &input.comment, 500, &mut errors);

    let exec = single_line("exec", &input.exec, 2000, &mut errors);
    need("exec", &exec, &mut errors);
    if !exec.is_empty() && !quotes_balanced(&exec) {
        errors.push(FieldError {
            field: "exec",
            message: "Command has an opening quote that is never closed".to_string(),
        });
    }

    let icon = single_line("icon", &input.icon, 500, &mut errors);
    if icon.contains('/') && !icon.starts_with('/') {
        errors.push(FieldError {
            field: "icon",
            message: "Icon must be an icon name, or an absolute path starting with /".to_string(),
        });
    }

    let categories_raw = single_line("categories", &input.categories, 500, &mut errors);
    let categories = normalize_list(&categories_raw);
    if let Some(bad) = categories.iter().find(|t| !valid_category(t)) {
        errors.push(FieldError {
            field: "categories",
            message: format!("`{bad}` is not a category name (letters, digits, `.`, `_` and `-`)"),
        });
    }

    let keywords_raw = single_line("keywords", &input.keywords, 500, &mut errors);
    let keywords = normalize_list(&keywords_raw);
    if let Some(long) = keywords.iter().find(|k| k.chars().count() > 100) {
        errors.push(FieldError {
            field: "keywords",
            message: format!("`{long}` is too long (at most 100 characters each)"),
        });
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Fields {
        name,
        comment,
        icon,
        exec,
        terminal: input.terminal,
        categories: list_value(&categories),
        keywords: list_value(&keywords),
    })
}

/// Set `key` to `new` if that differs from `current`. Returns whether it did.
fn set_text(doc: &mut Doc, key: &str, new: &str, current: &str, drop_translations: bool) -> bool {
    if new == current {
        return false;
    }
    if new.is_empty() {
        doc.remove(key);
    } else {
        doc.set(key, &escape_value(new));
    }
    if drop_translations {
        doc.remove_localized(key);
    }
    true
}

/// Apply `new` to `doc`, touching only what changed. Returns whether anything
/// did. Changing the command also removes `TryExec`, which would otherwise
/// still point at the old program and can hide the entry, and turns off
/// `DBusActivatable`, which would otherwise make launchers ignore `Exec`.
fn apply(doc: &mut Doc, new: &Fields) -> bool {
    let cur = read_fields(doc);
    let mut changed = false;

    changed |= set_text(doc, "Name", &new.name, &cur.name, true);
    changed |= set_text(doc, "Comment", &new.comment, &cur.comment, true);
    changed |= set_text(doc, "Icon", &new.icon, &cur.icon, false);
    changed |= set_text(doc, "Categories", &new.categories, &cur.categories, false);
    changed |= set_text(doc, "Keywords", &new.keywords, &cur.keywords, true);

    if set_text(doc, "Exec", &new.exec, &cur.exec, false) {
        changed = true;
        doc.remove("TryExec");
        if doc.flag("DBusActivatable") {
            doc.set("DBusActivatable", "false");
        }
    }

    if new.terminal != cur.terminal {
        changed = true;
        doc.set("Terminal", if new.terminal { "true" } else { "false" });
    }
    changed
}

// ---------------------------------------------------------------------------
// Looking entries up
// ---------------------------------------------------------------------------

struct Loaded {
    id: String,
    path: PathBuf,
    in_user: bool,
    text: String,
    doc: Doc,
}

fn load(idx: &Index, id: &str) -> Result<Loaded, String> {
    if !valid_id(id) {
        return Err(format!("`{id}` is not a valid entry id"));
    }
    let (path, in_user) = idx
        .winner(id)
        .ok_or_else(|| format!("there is no entry `{id}`"))?;
    let text = read_text(path)?;
    let doc = Doc::parse(&text);
    if !doc.is_application() {
        return Err(format!("`{id}` is not an application entry"));
    }
    Ok(Loaded {
        id: id.to_string(),
        path: path.clone(),
        in_user,
        text,
        doc,
    })
}

fn info(dirs: &Dirs, idx: &Index, l: &Loaded) -> EntryInfo {
    let orig = orig_path(dirs, &l.id);
    let has_orig = orig.is_file();
    let system = idx.system.get(&l.id);

    let (origin, edited) = if !l.in_user {
        (Origin::System, false)
    } else if l.doc.is_generated() {
        (Origin::Created, false)
    } else if l.doc.is_override() {
        // An edited copy whose system entry has since gone is just the user's.
        (
            if system.is_some() {
                Origin::System
            } else {
                Origin::Yours
            },
            true,
        )
    } else {
        (Origin::Yours, has_orig)
    };

    let stale = origin == Origin::System
        && edited
        && has_orig
        && match (system.map(fs::read), fs::read(&orig)) {
            (Some(Ok(now)), Ok(then)) => now != then,
            _ => false,
        };
    let restorable = (l.in_user && l.doc.is_override())
        || (l.in_user && !l.doc.is_generated() && has_orig)
        || removed_path(dirs, &l.id).is_file();

    EntryInfo {
        id: l.id.clone(),
        name: l.doc.text("Name").unwrap_or_else(|| l.id.clone()),
        origin,
        hidden: l.doc.hidden(),
        edited,
        removed: false,
        stale,
        restorable,
    }
}

// ---------------------------------------------------------------------------
// The operations
// ---------------------------------------------------------------------------

/// Every application entry that has an ID, plus the user's own entries that
/// were removed and are waiting in the backup directory, sorted by name.
pub fn list_entries(dirs: &Dirs) -> Vec<EntryInfo> {
    let idx = index(dirs);
    let ids: BTreeSet<&String> = idx.user.keys().chain(idx.system.keys()).collect();

    let mut out: Vec<EntryInfo> = ids
        .iter()
        .filter_map(|id| load(&idx, id).ok())
        .map(|l| info(dirs, &idx, &l))
        .collect();

    if let Ok(read) = fs::read_dir(&dirs.backups) {
        for file in read.flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".removed") else {
                continue;
            };
            if !valid_id(id) || idx.winner(id).is_some() {
                continue;
            }
            let Ok(text) = read_text(&file.path()) else {
                continue;
            };
            let doc = Doc::parse(&text);
            if doc.is_application() {
                out.push(EntryInfo {
                    id: id.to_string(),
                    name: doc.text("Name").unwrap_or_else(|| id.to_string()),
                    origin: Origin::Yours,
                    hidden: false,
                    edited: false,
                    removed: true,
                    stale: false,
                    restorable: true,
                });
            }
        }
    }

    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// Open `id` for editing.
pub fn get_entry(dirs: &Dirs, id: &str) -> Result<EntryDetail, String> {
    let idx = index(dirs);
    let l = load(&idx, id)?;
    let target = dirs.user.join(id);

    let mut notes = Vec::new();
    if !l.in_user {
        notes.push(format!(
            "Saving writes a copy to {} and leaves the system file {} unchanged.",
            target.display(),
            l.path.display()
        ));
    } else if l.doc.is_override() {
        notes.push(
            "This is your edited copy of a system entry. \"Restore original\" deletes it and \
             brings the system entry back."
                .to_string(),
        );
    } else if !l.doc.is_generated() {
        notes.push(
            "This is your own file. A copy of the original is kept the first time you change \
             it, so \"Restore original\" can put it back."
                .to_string(),
        );
    }
    for key in ["Name", "Comment", "Keywords"] {
        let locales = l.doc.locales(key);
        if !locales.is_empty() {
            notes.push(format!(
                "{key} has translations ({}); changing it removes them.",
                locales.join(", ")
            ));
        }
    }
    if l.doc.get("TryExec").is_some() {
        notes.push("Changing the command also removes TryExec.".to_string());
    }
    if l.doc.flag("DBusActivatable") {
        notes.push(
            "This entry starts over D-Bus; changing the command turns that off so the command \
             is used."
                .to_string(),
        );
    }

    Ok(EntryDetail {
        info: info(dirs, &idx, &l),
        fields: read_fields(&l.doc),
        writes_to: target.display().to_string(),
        notes,
    })
}

/// Apply `input` to entry `id`. The values are checked first and nothing is
/// written if any is refused. The file written is always in the user's
/// directory under the entry's own name.
pub fn save_entry(dirs: &Dirs, id: &str, input: &Fields) -> Result<Saved, SaveError> {
    let idx = index(dirs);
    let l = load(&idx, id).map_err(SaveError::Failed)?;
    let fields = validate(input).map_err(SaveError::Invalid)?;

    let mut doc = l.doc.clone();
    if !apply(&mut doc, &fields) {
        return Ok(Saved {
            path: l.path.display().to_string(),
            changed: false,
            created_override: false,
        });
    }

    let target = dirs.user.join(id);
    let ours = l.in_user && (l.doc.is_generated() || l.doc.is_override());
    let created_override = !l.in_user;
    if created_override {
        doc.mark(OVERRIDE_KEY);
    }

    let snapshot_made = if ours {
        false
    } else {
        ensure_snapshot(dirs, id, &l.text).map_err(SaveError::Failed)?
    };
    if let Err(e) = write_atomic(&target, &doc.render()) {
        if snapshot_made {
            let _ = fs::remove_file(orig_path(dirs, id));
        }
        return Err(SaveError::Failed(e));
    }
    Ok(Saved {
        path: target.display().to_string(),
        changed: true,
        created_override,
    })
}

/// Remove `id` from the menu, in the way that fits where it came from. Only a
/// file carrying this tool's marker is ever deleted.
pub fn remove_entry(dirs: &Dirs, id: &str) -> Result<Removal, String> {
    let idx = index(dirs);
    let l = load(&idx, id)?;
    let target = dirs.user.join(id);

    if l.in_user {
        if l.doc.is_generated() {
            fs::remove_file(&l.path)
                .map_err(|e| format!("cannot delete {}: {e}", l.path.display()))?;
            return Ok(Removal::Deleted);
        }
        if l.doc.is_override() {
            if l.doc.hidden() {
                return Ok(Removal::AlreadyHidden);
            }
            let mut doc = l.doc.clone();
            doc.set("NoDisplay", "true");
            write_atomic(&target, &doc.render())?;
            return Ok(Removal::Hidden);
        }
        // The user's own file: keep it, in the backup directory.
        write_atomic(&removed_path(dirs, id), &l.text)?;
        fs::remove_file(&l.path)
            .map_err(|e| format!("cannot move {} away: {e}", l.path.display()))?;
        return Ok(Removal::MovedToBackup);
    }

    // A system entry: hide it with a marked copy in the user's directory.
    if l.doc.hidden() {
        return Ok(Removal::AlreadyHidden);
    }
    let mut doc = l.doc.clone();
    doc.set("NoDisplay", "true");
    doc.mark(OVERRIDE_KEY);
    let snapshot_made = ensure_snapshot(dirs, id, &l.text)?;
    if let Err(e) = write_atomic(&target, &doc.render()) {
        if snapshot_made {
            let _ = fs::remove_file(orig_path(dirs, id));
        }
        return Err(e);
    }
    Ok(Removal::Hidden)
}

/// Undo what was done to `id`: delete this tool's edited copy of a system
/// entry, or put the user's own file back.
pub fn restore_entry(dirs: &Dirs, id: &str) -> Result<Restored, String> {
    if !valid_id(id) {
        return Err(format!("`{id}` is not a valid entry id"));
    }
    let user_file = dirs.user.join(id);
    let orig = orig_path(dirs, id);
    let removed = removed_path(dirs, id);

    if user_file.is_file() {
        let doc = Doc::parse(&read_text(&user_file)?);
        if doc.is_override() {
            fs::remove_file(&user_file)
                .map_err(|e| format!("cannot delete {}: {e}", user_file.display()))?;
            let _ = fs::remove_file(&orig);
            return Ok(Restored::OverrideRemoved);
        }
        if doc.is_generated() {
            return Err(
                "this entry was created by nexus-menu; there is no earlier version to restore"
                    .to_string(),
            );
        }
        if orig.is_file() {
            write_atomic(&user_file, &read_text(&orig)?)?;
            let _ = fs::remove_file(&orig);
            return Ok(Restored::OriginalRestored);
        }
        return Err("there is nothing to restore for this entry".to_string());
    }

    if removed.is_file() {
        write_atomic(&user_file, &read_text(&removed)?)?;
        let _ = fs::remove_file(&removed);
        return Ok(Restored::RemovedRestored);
    }
    Err("there is nothing to restore for this entry".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    /// What a real entry has: translations, TryExec, comments, an unknown key
    /// and an action group.
    const FOX: &str = "# Fixture entry\n\
[Desktop Entry]\n\
Type=Application\n\
Name=Fox\n\
Name[de]=Fuchs\n\
Comment=Browse\n\
Comment[de]=Surfen\n\
Exec=fox %u\n\
TryExec=fox\n\
Icon=fox\n\
Terminal=false\n\
Categories=Network;WebBrowser;\n\
Keywords=web;browser;\n\
Keywords[de]=netz;\n\
X-Custom=keep me\n\
Actions=new;\n\
\n\
[Desktop Action new]\n\
Name=New window\n\
Exec=fox --new\n";

    fn setup() -> (Scratch, Dirs) {
        let s = Scratch::new();
        let d = Dirs::new(
            s.path().join("home/applications"),
            s.path().join("home/nexus-menu/backups"),
            vec![
                s.path().join("sys1/applications"),
                s.path().join("sys2/applications"),
            ],
        );
        (s, d)
    }

    fn sys1(s: &Scratch, name: &str, text: &str) -> PathBuf {
        s.file(&format!("sys1/applications/{name}"), text)
    }

    fn user(s: &Scratch, name: &str, text: &str) -> PathBuf {
        s.file(&format!("home/applications/{name}"), text)
    }

    fn read(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    fn fields(d: &Dirs, id: &str) -> Fields {
        get_entry(d, id).unwrap().fields
    }

    fn entry(d: &Dirs, id: &str) -> EntryInfo {
        list_entries(d)
            .into_iter()
            .find(|e| e.id == id)
            .unwrap_or_else(|| panic!("{id} is not listed"))
    }

    fn ids(d: &Dirs) -> Vec<String> {
        list_entries(d).into_iter().map(|e| e.id).collect()
    }

    // --- naming, precedence, listing -----------------------------------------

    #[test]
    fn the_id_comes_from_the_path_with_slashes_turned_into_dashes() {
        let (s, d) = setup();
        sys1(&s, "kde4/viewer.desktop", FOX);
        assert_eq!(ids(&d), vec!["kde4-viewer.desktop"]);
    }

    #[test]
    fn your_directory_wins_and_earlier_system_directories_win_over_later_ones() {
        let (s, d) = setup();
        sys1(
            &s,
            "a.desktop",
            "[Desktop Entry]\nType=Application\nName=One\n",
        );
        s.file(
            "sys2/applications/a.desktop",
            "[Desktop Entry]\nType=Application\nName=Two\n",
        );
        s.file(
            "sys2/applications/b.desktop",
            "[Desktop Entry]\nType=Application\nName=Bee\n",
        );
        user(
            &s,
            "b.desktop",
            "[Desktop Entry]\nType=Application\nName=Mine\n",
        );

        let a = entry(&d, "a.desktop");
        assert_eq!((a.name.as_str(), a.origin), ("One", Origin::System));
        let b = entry(&d, "b.desktop");
        assert_eq!((b.name.as_str(), b.origin), ("Mine", Origin::Yours));
    }

    #[test]
    fn only_manageable_application_entries_are_listed() {
        let (s, d) = setup();
        sys1(&s, "good.desktop", FOX);
        sys1(&s, "a..b.desktop", FOX);
        sys1(
            &s,
            "link.desktop",
            "[Desktop Entry]\nType=Link\nName=L\nURL=x\n",
        );
        sys1(
            &s,
            "dir.desktop",
            "[Desktop Entry]\nType=Directory\nName=D\n",
        );
        sys1(&s, "noheader.desktop", "Name=Nothing\n");
        sys1(&s, ".hidden.desktop", FOX);
        assert_eq!(ids(&d), vec!["good.desktop"]);
    }

    #[test]
    fn origin_hidden_and_order_are_reported() {
        let (s, d) = setup();
        user(
            &s,
            "made.desktop",
            "[Desktop Entry]\nType=Application\nName=made\nX-NexusMenu-Generated=true\n",
        );
        user(
            &s,
            "legacy.desktop",
            "[Desktop Entry]\nType=Application\nName=Legacy\nX-KAppFinder-Generated=true\n",
        );
        user(
            &s,
            "mine.desktop",
            "[Desktop Entry]\nType=Application\nName=Zed\n",
        );
        sys1(
            &s,
            "quiet.desktop",
            "[Desktop Entry]\nType=Application\nName=Quiet\nNoDisplay=true\n",
        );

        let list = list_entries(&d);
        let order: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(order, vec!["Legacy", "made", "Quiet", "Zed"]);
        assert_eq!(entry(&d, "made.desktop").origin, Origin::Created);
        assert_eq!(entry(&d, "legacy.desktop").origin, Origin::Created);
        assert_eq!(entry(&d, "mine.desktop").origin, Origin::Yours);
        let q = entry(&d, "quiet.desktop");
        assert_eq!(q.origin, Origin::System);
        assert!(q.hidden);
        assert!(!entry(&d, "mine.desktop").hidden);
    }

    #[test]
    fn real_world_file_names_are_accepted() {
        let (s, d) = setup();
        for name in [
            "Battle․net.desktop",
            "cxassoc-cxoffice-0:application_x-crossover-exe::run.desktop",
            "== Some Releases @ site ==.desktop",
            "io.github.Foo_Bar.desktop",
        ] {
            assert!(valid_id(name), "{name:?}");
            sys1(&s, name, FOX);
        }
        let listed = ids(&d);
        assert_eq!(listed.len(), 4, "{listed:?}");
        // And such an entry can really be edited and put back.
        let id = "cxassoc-cxoffice-0:application_x-crossover-exe::run.desktop";
        let mut f = fields(&d, id);
        f.name = "Run".into();
        assert!(save_entry(&d, id, &f).unwrap().created_override);
        assert_eq!(restore_entry(&d, id).unwrap(), Restored::OverrideRemoved);
    }

    #[test]
    fn get_entry_reads_the_fields_and_says_what_saving_will_do() {
        let (s, d) = setup();
        sys1(&s, "fox.desktop", FOX);
        let detail = get_entry(&d, "fox.desktop").unwrap();
        assert_eq!(
            detail.fields,
            Fields {
                name: "Fox".into(),
                comment: "Browse".into(),
                icon: "fox".into(),
                exec: "fox %u".into(),
                terminal: false,
                categories: "Network;WebBrowser;".into(),
                keywords: "web;browser;".into(),
            }
        );
        assert_eq!(
            detail.writes_to,
            d.user.join("fox.desktop").display().to_string()
        );
        let notes = detail.notes.join("\n");
        assert!(notes.contains("Saving writes a copy"), "{notes}");
        assert!(notes.contains("Name has translations (de)"), "{notes}");
        assert!(notes.contains("Keywords has translations (de)"), "{notes}");
        assert!(notes.contains("removes TryExec"), "{notes}");
    }

    // --- saving ----------------------------------------------------------------

    #[test]
    fn saving_a_system_entry_writes_an_override_and_never_touches_the_system_file() {
        let (s, d) = setup();
        let system = sys1(&s, "fox.desktop", FOX);
        let before = read(&system);

        let mut f = fields(&d, "fox.desktop");
        f.name = "Renamed".into();
        f.icon = "other".into();
        let saved = save_entry(&d, "fox.desktop", &f).unwrap();
        assert!(saved.changed && saved.created_override);

        assert_eq!(read(&system), before, "the system file changed");
        let out = read(&d.user.join("fox.desktop"));
        assert!(out.contains("Name=Renamed"), "{out}");
        assert!(out.contains("Icon=other"), "{out}");
        assert!(out.contains("X-NexusMenu-Override=true"), "{out}");
        // Everything else survives.
        for kept in [
            "# Fixture entry",
            "X-Custom=keep me",
            "Comment=Browse",
            "Comment[de]=Surfen",
            "Keywords[de]=netz;",
            "Actions=new;",
            "[Desktop Action new]",
            "Exec=fox --new",
            "TryExec=fox",
        ] {
            assert!(out.contains(kept), "lost `{kept}`:\n{out}");
        }
        // The translation of the value that changed is gone.
        assert!(!out.contains("Name[de]"), "{out}");
        // The snapshot is the system file as it was.
        assert_eq!(read(&orig_path(&d, "fox.desktop")), before);

        let e = entry(&d, "fox.desktop");
        assert_eq!(e.name, "Renamed");
        assert!(e.edited && e.restorable && !e.stale);
        assert_eq!(e.origin, Origin::System);
    }

    #[test]
    fn saving_again_edits_the_override_in_place_and_keeps_the_first_snapshot() {
        let (s, d) = setup();
        let system = sys1(&s, "fox.desktop", FOX);
        let original = read(&system);

        let mut f = fields(&d, "fox.desktop");
        f.comment = "First".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        f.comment = "Second".into();
        let again = save_entry(&d, "fox.desktop", &f).unwrap();
        assert!(again.changed && !again.created_override);

        assert!(read(&d.user.join("fox.desktop")).contains("Comment=Second"));
        assert_eq!(read(&orig_path(&d, "fox.desktop")), original);
    }

    #[test]
    fn saving_your_own_entry_edits_it_in_place_after_a_snapshot_and_restore_puts_it_back() {
        let (s, d) = setup();
        let path = user(&s, "mine.desktop", FOX);
        let original = read(&path);

        let mut f = fields(&d, "mine.desktop");
        f.exec = "fox --private %u".into();
        let saved = save_entry(&d, "mine.desktop", &f).unwrap();
        assert!(saved.changed && !saved.created_override);

        let out = read(&path);
        assert!(out.contains("Exec=fox --private %u"), "{out}");
        assert!(
            !out.contains("X-NexusMenu-Override"),
            "a file of yours was marked"
        );
        assert_eq!(read(&orig_path(&d, "mine.desktop")), original);
        let e = entry(&d, "mine.desktop");
        assert_eq!(e.origin, Origin::Yours);
        assert!(e.edited && e.restorable);

        assert_eq!(
            restore_entry(&d, "mine.desktop").unwrap(),
            Restored::OriginalRestored
        );
        assert_eq!(read(&path), original);
        assert!(!orig_path(&d, "mine.desktop").exists());
    }

    #[test]
    fn saving_an_entry_nexus_menu_made_edits_it_in_place_and_keeps_its_marker() {
        let (s, d) = setup();
        let path = user(
            &s,
            "made.desktop",
            "[Desktop Entry]\nType=Application\nName=Made\nExec=made\nX-NexusMenu-Generated=true\n",
        );
        let mut f = fields(&d, "made.desktop");
        f.name = "Made again".into();
        save_entry(&d, "made.desktop", &f).unwrap();

        let out = read(&path);
        assert!(out.contains("Name=Made again"), "{out}");
        assert!(out.contains("X-NexusMenu-Generated=true"), "{out}");
        assert!(!d.backups.exists(), "a snapshot was made of our own output");
        assert_eq!(entry(&d, "made.desktop").origin, Origin::Created);
    }

    #[test]
    fn a_save_that_changes_nothing_writes_nothing() {
        let (s, d) = setup();
        sys1(&s, "fox.desktop", FOX);
        let f = fields(&d, "fox.desktop");
        let saved = save_entry(&d, "fox.desktop", &f).unwrap();
        assert!(!saved.changed);
        assert!(!d.user.join("fox.desktop").exists());
        assert!(!d.backups.exists());
    }

    #[test]
    fn translations_are_kept_for_values_that_did_not_change() {
        let (s, d) = setup();
        sys1(&s, "fox.desktop", FOX);
        let mut f = fields(&d, "fox.desktop");
        f.keywords = "web;browser;fast;".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        let out = read(&d.user.join("fox.desktop"));
        assert!(out.contains("Name[de]=Fuchs"), "{out}");
        assert!(out.contains("Comment[de]=Surfen"), "{out}");
        assert!(!out.contains("Keywords[de]"), "{out}");
        assert!(out.contains("Keywords=web;browser;fast;"), "{out}");
    }

    #[test]
    fn changing_the_command_drops_tryexec_and_turns_dbus_activation_off() {
        let (s, d) = setup();
        let dbus = FOX.replace("Terminal=false\n", "Terminal=false\nDBusActivatable=true\n");
        sys1(&s, "fox.desktop", &dbus);

        // Changing something else leaves both alone.
        let mut f = fields(&d, "fox.desktop");
        f.comment = "Other".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        let kept = read(&d.user.join("fox.desktop"));
        assert!(kept.contains("TryExec=fox") && kept.contains("DBusActivatable=true"));

        f.exec = "fox --safe %u".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        let out = read(&d.user.join("fox.desktop"));
        assert!(!out.contains("TryExec"), "{out}");
        assert!(out.contains("DBusActivatable=false"), "{out}");
        assert!(out.contains("Exec=fox --safe %u"), "{out}");
    }

    #[test]
    fn terminal_categories_and_keywords_are_written_normalised() {
        let (s, d) = setup();
        sys1(&s, "fox.desktop", FOX);
        let mut f = fields(&d, "fox.desktop");
        f.terminal = true;
        f.categories = " Utility ; Network;;Utility ".into();
        f.keywords = "".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        let out = read(&d.user.join("fox.desktop"));
        assert!(out.contains("Terminal=true"), "{out}");
        assert!(out.contains("Categories=Utility;Network;"), "{out}");
        assert!(!out.contains("Keywords"), "{out}");
        let back = fields(&d, "fox.desktop");
        assert!(back.terminal);
        assert_eq!(back.categories, "Utility;Network;");
        assert_eq!(back.keywords, "");
    }

    #[test]
    fn values_are_escaped_and_come_back_unchanged() {
        let (s, d) = setup();
        sys1(&s, "fox.desktop", FOX);
        let mut f = fields(&d, "fox.desktop");
        f.exec = r"fox --dir C:\games\fox %u".into();
        f.comment = r"slash \ and more".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        let out = read(&d.user.join("fox.desktop"));
        assert!(out.contains(r"Exec=fox --dir C:\\games\\fox %u"), "{out}");
        let back = fields(&d, "fox.desktop");
        assert_eq!(back.exec, r"fox --dir C:\games\fox %u");
        assert_eq!(back.comment, r"slash \ and more");
    }

    #[test]
    fn refused_values_are_refused_before_anything_is_written() {
        let (s, d) = setup();
        let system = sys1(&s, "fox.desktop", FOX);
        let before = read(&system);
        let good = fields(&d, "fox.desktop");

        let cases: Vec<(&str, Fields)> = vec![
            (
                "name",
                Fields {
                    name: "x\nExec=evil".into(),
                    ..good.clone()
                },
            ),
            (
                "name",
                Fields {
                    name: "  ".into(),
                    ..good.clone()
                },
            ),
            (
                "exec",
                Fields {
                    exec: "".into(),
                    ..good.clone()
                },
            ),
            (
                "exec",
                Fields {
                    exec: "fox \"unclosed".into(),
                    ..good.clone()
                },
            ),
            (
                "exec",
                Fields {
                    exec: "fox\r\n[Desktop Entry]".into(),
                    ..good.clone()
                },
            ),
            (
                "icon",
                Fields {
                    icon: "dir/icon".into(),
                    ..good.clone()
                },
            ),
            (
                "comment",
                Fields {
                    comment: "c".repeat(501),
                    ..good.clone()
                },
            ),
            (
                "categories",
                Fields {
                    categories: "Net work;".into(),
                    ..good.clone()
                },
            ),
            (
                "keywords",
                Fields {
                    keywords: "a\tb".into(),
                    ..good.clone()
                },
            ),
        ];
        for (field, input) in cases {
            match save_entry(&d, "fox.desktop", &input) {
                Err(SaveError::Invalid(errs)) => {
                    assert!(
                        errs.iter().any(|e| e.field == field),
                        "{field}: got {errs:?}"
                    );
                }
                other => panic!("{field}: expected Invalid, got {other:?}"),
            }
        }
        assert_eq!(read(&system), before);
        assert!(
            !d.user.join("fox.desktop").exists(),
            "an override was written"
        );
        assert!(!d.backups.exists(), "a snapshot was written");
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let (s, d) = setup();
        sys1(&s, "fox.desktop", FOX);
        let input = Fields {
            name: "".into(),
            exec: "".into(),
            ..fields(&d, "fox.desktop")
        };
        let Err(SaveError::Invalid(errs)) = save_entry(&d, "fox.desktop", &input) else {
            panic!("expected Invalid");
        };
        let named: Vec<&str> = errs.iter().map(|e| e.field).collect();
        assert_eq!(named, vec!["name", "exec"]);
        // Each message is a sentence of its own, ready to show under the field.
        let said: Vec<&str> = errs.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(said, vec!["Name is required", "Command is required"]);
    }

    // --- remove and restore ----------------------------------------------------

    #[test]
    fn removing_a_system_entry_hides_it_with_a_marked_copy_and_restore_brings_it_back() {
        let (s, d) = setup();
        let system = sys1(&s, "fox.desktop", FOX);
        let before = read(&system);

        assert_eq!(remove_entry(&d, "fox.desktop").unwrap(), Removal::Hidden);
        let out = read(&d.user.join("fox.desktop"));
        assert!(out.contains("NoDisplay=true"), "{out}");
        assert!(out.contains("X-NexusMenu-Override=true"), "{out}");
        assert_eq!(read(&system), before, "the system file changed");
        let e = entry(&d, "fox.desktop");
        assert!(e.hidden && e.edited && e.restorable);
        assert_eq!(
            remove_entry(&d, "fox.desktop").unwrap(),
            Removal::AlreadyHidden
        );

        assert_eq!(
            restore_entry(&d, "fox.desktop").unwrap(),
            Restored::OverrideRemoved
        );
        assert!(!d.user.join("fox.desktop").exists());
        assert!(!orig_path(&d, "fox.desktop").exists());
        let e = entry(&d, "fox.desktop");
        assert!(!e.hidden && !e.edited);
        assert_eq!(read(&system), before);
    }

    #[test]
    fn removing_an_entry_nexus_menu_made_deletes_it_because_of_its_marker() {
        let (s, d) = setup();
        let path = user(
            &s,
            "made.desktop",
            "[Desktop Entry]\nType=Application\nName=Made\nExec=made\nX-NexusMenu-Generated=true\n",
        );
        assert_eq!(remove_entry(&d, "made.desktop").unwrap(), Removal::Deleted);
        assert!(!path.exists());
    }

    #[test]
    fn removing_your_own_entry_moves_it_to_the_backup_folder_and_restore_returns_it() {
        let (s, d) = setup();
        let path = user(&s, "mine.desktop", FOX);
        let original = read(&path);

        assert_eq!(
            remove_entry(&d, "mine.desktop").unwrap(),
            Removal::MovedToBackup
        );
        assert!(!path.exists());
        assert_eq!(read(&removed_path(&d, "mine.desktop")), original);

        let e = entry(&d, "mine.desktop");
        assert!(e.removed && e.restorable);
        assert_eq!(e.origin, Origin::Yours);
        assert_eq!(e.name, "Fox");

        assert_eq!(
            restore_entry(&d, "mine.desktop").unwrap(),
            Restored::RemovedRestored
        );
        assert_eq!(read(&path), original);
        assert!(!removed_path(&d, "mine.desktop").exists());
        assert!(!entry(&d, "mine.desktop").removed);
    }

    #[test]
    fn restoring_after_edit_then_remove_takes_two_steps_back() {
        let (s, d) = setup();
        let path = user(&s, "mine.desktop", FOX);
        let original = read(&path);
        let mut f = fields(&d, "mine.desktop");
        f.name = "Edited".into();
        save_entry(&d, "mine.desktop", &f).unwrap();
        remove_entry(&d, "mine.desktop").unwrap();

        // First what was removed (the edited file), then the original.
        assert_eq!(
            restore_entry(&d, "mine.desktop").unwrap(),
            Restored::RemovedRestored
        );
        assert!(read(&path).contains("Name=Edited"));
        assert_eq!(
            restore_entry(&d, "mine.desktop").unwrap(),
            Restored::OriginalRestored
        );
        assert_eq!(read(&path), original);
    }

    #[test]
    fn restore_refuses_when_there_is_nothing_to_restore_and_never_deletes_a_plain_file() {
        let (s, d) = setup();
        let plain = user(&s, "mine.desktop", FOX);
        let made = user(
            &s,
            "made.desktop",
            "[Desktop Entry]\nType=Application\nName=Made\nX-NexusMenu-Generated=true\n",
        );
        sys1(&s, "sys.desktop", FOX);

        assert!(restore_entry(&d, "mine.desktop").is_err());
        assert!(plain.exists());
        assert!(restore_entry(&d, "made.desktop").is_err());
        assert!(made.exists());
        assert!(restore_entry(&d, "sys.desktop").is_err());
        assert!(restore_entry(&d, "unknown.desktop").is_err());
    }

    #[test]
    fn a_stale_snapshot_never_turns_a_system_file_into_one_of_yours() {
        // An override that was deleted by hand leaves its snapshot behind; a
        // later restore must not copy that system text into the user's folder.
        let (s, d) = setup();
        sys1(&s, "fox.desktop", FOX);
        let mut f = fields(&d, "fox.desktop");
        f.name = "Edited".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        fs::remove_file(d.user.join("fox.desktop")).unwrap();

        assert!(restore_entry(&d, "fox.desktop").is_err());
        assert!(!d.user.join("fox.desktop").exists());
    }

    #[test]
    fn ids_that_could_name_other_paths_are_refused_everywhere() {
        let (s, d) = setup();
        sys1(&s, "fox.desktop", FOX);
        let outside = s.file("outside.desktop", FOX);
        let before = read(&outside);
        let good = fields(&d, "fox.desktop");

        for id in [
            "../outside.desktop",
            "../../etc/passwd",
            "a/b.desktop",
            "/etc/passwd",
            "outside",
            ".hidden.desktop",
            ".desktop",
            "",
            "a..b.desktop",
            "nul\u{0}.desktop",
            "new\nline.desktop",
        ] {
            assert!(!valid_id(id), "{id:?} was accepted");
            assert!(get_entry(&d, id).is_err(), "get {id:?}");
            assert!(
                matches!(save_entry(&d, id, &good), Err(SaveError::Failed(_))),
                "save {id:?}"
            );
            assert!(remove_entry(&d, id).is_err(), "remove {id:?}");
            assert!(restore_entry(&d, id).is_err(), "restore {id:?}");
        }
        assert_eq!(read(&outside), before);
        assert!(
            !d.user.exists() && !d.backups.exists(),
            "something was created"
        );
    }

    #[test]
    fn an_id_that_is_not_an_application_entry_is_refused() {
        let (s, d) = setup();
        sys1(
            &s,
            "link.desktop",
            "[Desktop Entry]\nType=Link\nName=L\nURL=x\n",
        );
        assert!(get_entry(&d, "link.desktop").is_err());
        assert!(remove_entry(&d, "link.desktop").is_err());
        assert!(matches!(
            save_entry(&d, "link.desktop", &Fields::default()),
            Err(SaveError::Failed(_))
        ));
        assert!(!d.user.exists());
    }

    // --- state flags -------------------------------------------------------------

    #[test]
    fn a_system_entry_that_changed_after_you_edited_it_is_flagged() {
        let (s, d) = setup();
        let system = sys1(&s, "fox.desktop", FOX);
        let mut f = fields(&d, "fox.desktop");
        f.name = "Mine".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        assert!(!entry(&d, "fox.desktop").stale);

        fs::write(
            &system,
            FOX.replace("Exec=fox %u", "Exec=fox --new-flag %u"),
        )
        .unwrap();
        assert!(entry(&d, "fox.desktop").stale);
    }

    #[test]
    fn an_edited_copy_whose_system_entry_is_gone_counts_as_yours() {
        let (s, d) = setup();
        let system = sys1(&s, "fox.desktop", FOX);
        let mut f = fields(&d, "fox.desktop");
        f.name = "Mine".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        fs::remove_file(&system).unwrap();
        let e = entry(&d, "fox.desktop");
        assert_eq!(e.origin, Origin::Yours);
        assert!(e.edited && e.restorable && !e.stale);
    }

    #[test]
    fn no_temporary_files_are_left_behind() {
        let (s, d) = setup();
        sys1(&s, "fox.desktop", FOX);
        user(&s, "mine.desktop", FOX);
        let mut f = fields(&d, "fox.desktop");
        f.name = "A".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        remove_entry(&d, "mine.desktop").unwrap();
        restore_entry(&d, "mine.desktop").unwrap();

        for dir in [&d.user, &d.backups] {
            for e in fs::read_dir(dir).unwrap().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                assert!(!name.contains("tmp"), "left {name} in {}", dir.display());
            }
        }
    }

    #[test]
    fn files_are_written_with_a_trailing_newline_and_still_parse_as_before() {
        let (s, d) = setup();
        sys1(&s, "fox.desktop", FOX.trim_end());
        let mut f = fields(&d, "fox.desktop");
        f.comment = "x".into();
        save_entry(&d, "fox.desktop", &f).unwrap();
        let out = read(&d.user.join("fox.desktop"));
        assert!(out.ends_with('\n'));
        assert!(crate::desktop::parse(&d.user.join("fox.desktop")).is_some());
    }

    #[test]
    fn dirs_around_puts_the_backups_beside_the_user_directory() {
        let d = Dirs::around(PathBuf::from("/x/y/applications"));
        assert_eq!(d.user, PathBuf::from("/x/y/applications"));
        assert_eq!(d.backups, PathBuf::from("/x/y/nexus-menu/backups"));
        assert!(!d.system.contains(&d.user));
    }
}
