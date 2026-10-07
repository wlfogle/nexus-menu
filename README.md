# kappfinder-rs

[![CI](https://github.com/wlfogle/kappfinder-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/wlfogle/kappfinder-rs/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.74%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Linux-lightgrey.svg)](#requirements)

**Finds installed applications that have no menu entry, and creates one.**

A maintained, desktop-agnostic reimplementation of KDE's
[KAppFinder](https://apps.kde.org/kappfinder/) — which KDE marked unmaintained
and stopped releasing, and which Gentoo removed as a dead package in 2015.
Nothing replaced the specific job it did, so this does.

---

**Getting started:** [Requirements](#requirements) ·
[Install](#install) · [Desktop launcher](#desktop-launcher) ·
[First run](#first-run) · [Usage](#usage)

**Reference:** [Global options](#global-options) ·
[Environment](#environment) · [Exit codes](#exit-codes) ·
[Catalog contents](#whats-in-the-catalog) ·
[Extending the catalog](#extending-the-catalog)

**Help:** [Troubleshooting](#troubleshooting) · [Safety](#safety) ·
[Limitations](#limitations) · [Contributing](#contributing)

---

## The problem

You install `btop` from a repo, drop a vendor binary in `/opt`, or pull in
`x11-apps`. The programs are on your `$PATH` and work fine from a shell — but
they never appear in your application menu, because nobody shipped a
`.desktop` file for them.

`kappfinder-rs` finds exactly those programs and writes correct, spec-valid
entries for them.

```console
$ kappfinder-rs
kappfinder-rs — application menu entry finder

  catalog templates     117
  installed on $PATH    37
  already in the menu   4
  missing an entry      33
  desktop files seen    577 across 6 directories

Installed but missing a menu entry:

    1  alsamixer    terminal  ALSA Mixer
    2  btop         terminal  btop++
    3  ncdu         terminal  ncdu
    4  nmtui        terminal  Network Manager TUI
    5  xclock       gui       X Clock
    …

Run `kappfinder-rs install` to create these entries.
```

## Why it's catalog-driven

KAppFinder was **not** a launcher, and it did **not** blindly sweep `$PATH`.
It shipped a tree of templates for known programs, checked which were
installed, and offered entries for the ones missing from the menu.

That design was correct, and this keeps it. On a typical desktop `$PATH`
holds ~5000 executables and ~4800 of them have no menu entry — almost all
`ls`, `awk`, `libfoo-helper`, and `x86_64-linux-gnu-gcc-12`. A blind sweep
produces a list nobody can use. The curated catalog is what makes the output
actionable.

The blind sweep still exists as `orphans`, but it is opt-in, read-only, and
never creates anything.

## Requirements

- Linux or another Unix (executable detection uses Unix permission bits)
- Rust 1.74+ to build
- `desktop-file-utils` — **optional**, only used to refresh the desktop
  database after writing

Works with any freedesktop-compliant desktop: KDE Plasma, GNOME, Xfce, MATE,
Cinnamon, LXQt, Budgie, and tiling WMs that read `.desktop` files.

## Install

### Prebuilt binary

Grab the archive from the
[latest release](https://github.com/wlfogle/kappfinder-rs/releases/latest).
It is statically linked, so it has no glibc version requirement and runs on
any x86_64 Linux.

```fish
tar xzf kappfinder-rs-*-x86_64-unknown-linux-musl.tar.gz
cd kappfinder-rs-*-x86_64-unknown-linux-musl
install -Dm755 kappfinder-rs ~/.local/bin/kappfinder-rs
```

Each release also ships a `.sha256` file:

```fish
sha256sum -c kappfinder-rs-*.tar.gz.sha256
```

### With cargo

```fish
cargo install --git https://github.com/wlfogle/kappfinder-rs
```

This puts the binary in `~/.cargo/bin`.

### From source

```fish
git clone https://github.com/wlfogle/kappfinder-rs.git
cd kappfinder-rs
cargo build --release
install -Dm755 target/release/kappfinder-rs ~/.local/bin/kappfinder-rs
```

Or let the `Makefile` place the binary, the launcher, the icon, and the shell
completions in one step — see [Desktop launcher](#desktop-launcher) and
[Shell completions](#shell-completions):

```fish
sudo make install                    # system-wide, under /usr/local
make install PREFIX=$HOME/.local     # or just for you
```

### Put it on your PATH

`~/.local/bin` is not on `$PATH` by default on every distribution.

```fish
# fish — persistent, no config editing
fish_add_path ~/.local/bin
```

```bash
# bash / zsh — add to ~/.bashrc or ~/.zshrc
export PATH="$HOME/.local/bin:$PATH"
```

Confirm it worked:

```fish
kappfinder-rs --version
```

### Uninstalling

Remove any menu entries the tool created **before** deleting the binary,
since `remove` is what knows which files are its own:

```fish
kappfinder-rs remove
rm ~/.local/bin/kappfinder-rs        # or: cargo uninstall kappfinder-rs
rm -rf ~/.config/kappfinder-rs       # only if you made a user catalog
```

If you installed with `make`, the matching target removes the binary, the
launcher, the icon, and the completions together:

```fish
kappfinder-rs remove                 # first, while the binary still exists
make uninstall PREFIX=$HOME/.local   # or: sudo make uninstall
```

`make uninstall` deliberately leaves generated menu entries alone — they are
yours, and only the binary can tell which files it wrote.

## Desktop launcher

The tool is a terminal program, but it does not have to be launched from a
terminal. `make install` adds a menu entry and icon for it:

```fish
sudo make install                    # system-wide, under /usr/local
make install PREFIX=$HOME/.local     # or just for you
```

That places these files:

| Path | What |
| --- | --- |
| `$PREFIX/bin/kappfinder-rs` | the binary |
| `$PREFIX/share/applications/kappfinder-rs.desktop` | the launcher |
| `$PREFIX/share/icons/hicolor/scalable/apps/kappfinder-rs.svg` | the icon |
| `$PREFIX/share/fish/vendor_completions.d/kappfinder-rs.fish` | fish completions |
| `$PREFIX/share/bash-completion/completions/kappfinder-rs` | bash completions |
| `$PREFIX/share/zsh/site-functions/_kappfinder-rs` | zsh completions |

Look for **Menu Entry Finder** in your application menu. It opens a terminal
window, runs `install`, and waits for Enter at the end so the summary is
still on screen when the tool exits.

Right-click the launcher for the other commands, exposed as desktop actions:

- **Scan without changing anything** — the read-only report
- **List every binary with no menu entry** — the `orphans` sweep
- **Remove the entries this tool created** — the undo

Already installed the binary another way? Add just the launcher, pointing it
at wherever the binary actually lives:

```fish
make install-launcher PREFIX=$HOME/.local
make install-launcher PREFIX=$HOME/.local BINDIR=$HOME/.cargo/bin
```

The installed copy's `Exec=` and `TryExec=` are rewritten to that absolute
path. This matters: a desktop session's `$PATH` is not your shell's, and
often omits `~/.local/bin` entirely, so a launcher that called the binary by
bare name would fail for exactly the people who need it most.

The launcher carries no `X-KAppFinder-Generated` marker, so
`kappfinder-rs remove` will never delete it.

## First run

The normal sequence is look, then decide, then apply.

```fish
# 1. See what is missing. This writes nothing.
kappfinder-rs

# 2. See exactly what would be written, still without writing it.
kappfinder-rs install --dry-run

# 3. Apply, choosing from a numbered list.
kappfinder-rs install
```

Step 3 prompts:

```console
Create which entries? [all / none / e.g. 1,3,5-7] (default: all):
```

Accepted answers:

| Answer | Effect |
| --- | --- |
| empty, `a`, `all` | every listed entry |
| `n`, `no`, `none`, `q` | nothing |
| `1,3,5-7` | those positions; ranges and lists can be mixed |

Changed your mind? `kappfinder-rs remove` deletes everything it created.

## Usage

### Scan — read-only, the default

```fish
kappfinder-rs
```

Reports how many catalogued applications are installed, how many already have
entries, and lists what's missing. Writes nothing.

### Install

Interactive by default: it prints a numbered list and waits.

```fish
kappfinder-rs install              # prompt: all / none / 1,3,5-7
kappfinder-rs install --dry-run    # show what would be written
kappfinder-rs install --yes        # accept everything
kappfinder-rs install --force      # overwrite existing files
```

Entries are written to `$XDG_DATA_HOME/applications` (normally
`~/.local/share/applications`). Override with `--dir`.

### Create specific entries

Works for catalogued and uncatalogued binaries alike; uncatalogued ones get
generated defaults.

```fish
kappfinder-rs create btop ncdu
kappfinder-rs create /opt/vendor/bin/thing
```

### List binaries with no entry

```fish
kappfinder-rs orphans                # filtered
kappfinder-rs orphans --limit 50     # cap the listing (0 means no limit)
kappfinder-rs orphans --no-filter    # include coreutils, libraries, etc.
```

Read-only and heuristic. Treat it as a starting point, not a worklist — on a
typical desktop it still returns several thousand entries.

The noise filter is never applied to a catalogued program: the catalog is the
ground truth for "this is a real application", so those always appear.

### Undo

Every generated file carries `X-KAppFinder-Generated=true`. `remove` deletes
only files carrying that marker, and only inside the target directory or your
own applications directory — hand-written and system entries are never
touched.

```fish
kappfinder-rs remove --dry-run
kappfinder-rs remove
```

Editing a generated file is safe. Deleting the marker line opts it out of
`remove` permanently.

### Inspect the catalog

```fish
kappfinder-rs catalog
kappfinder-rs catalog --missing      # templates whose binary isn't installed
```

### Shell completions

`completions <shell>` prints a completion script to stdout for `bash`, `zsh`,
`fish`, `elvish`, or `powershell`. It never reads the catalog, so it works
even if your user catalog is broken.

For fish, a user-level install is one line:

```fish
kappfinder-rs completions fish > ~/.config/fish/completions/kappfinder-rs.fish
```

`make install` does this for you under its `PREFIX`, but a per-user
`PREFIX=$HOME/.local` puts the fish script in
`~/.local/share/fish/vendor_completions.d`, which is not on fish's completion
search path, so it silently does nothing. Point the Makefile at a directory
fish really reads:

```fish
make install-completions FISHCOMPDIR=$HOME/.config/fish/completions
```

The `BASHCOMPDIR` and `ZSHCOMPDIR` variables work the same way. Check what
your fish searches with `printf '%s\n' $fish_complete_path`.

Subcommands and flags complete; the positional shell name after
`completions` does not, because the generator does not emit hints for
positional arguments in fish.

### Wine prefixes

```fish
kappfinder-rs wine prefixes
kappfinder-rs wine prefixes --prefix /media/games/PortProton/data/prefixes
```

Lists the Wine prefixes the tool can see, and which tool manages each. This
is the discovery step for Windows applications; it is read-only and
launches nothing. Prefixes come from four places and nowhere else:

- **Top-level hidden folders of your home directory** that are prefixes —
  `~/.wine`, `~/.insomniac`, and so on. Only the top level is looked at:
  a recursive search finds Proton `default_pfx` templates and runtime files
  that nobody installed applications into.
- **`$WINEPREFIX` and `--prefix DIR`** (repeatable). `DIR` may be a prefix
  or a directory of prefixes, so it is also how to reach prefixes outside
  your home, such as ones on another disk.
- **What each launcher declares**: Faugus's `~/Faugus/*`, PortProton's
  `~/PortProton/data/prefixes/*`, the `prefix:` of each Lutris game config,
  and the `winePrefix` of each Heroic game config.
- **Every local disk**: a search for `drive_c` on each mounted drive that is a
  real block device (`/media`, `/mnt`, `/run/media`, …), up to eight levels
  deep. It stops at each prefix it finds and skips caches, build output
  (`target`, `node_modules`), Steam libraries and `compatdata`, trash,
  Windows system folders, and any directory with `backup` in its name.
  Network mounts such as NFS are never searched.

A directory counts as a prefix only if it has a `drive_c` folder and a
`system.reg` or `user.reg` registry hive. When two sources name the same
directory it is listed once, as plain Wine: a Lutris game that points at
`~/.wine` does not turn it into a Lutris prefix.

The owner column matters because entries must launch an app through the tool
that manages its prefix; running a different Wine against a launcher's prefix
can upgrade or corrupt it.

Not looked for inside your home directory: Bottles, WineZGUI, PlayOnLinux and
Steam Proton `compatdata`. Name them with `--prefix` if you want them listed.
The disk search will also report copies of old home directories (for example
a `.wine` inside a backup of a project tree) if they are not named like a
backup, so check the list before trusting it.

## Global options

These work before or after the subcommand.

| Flag | Meaning |
| --- | --- |
| `--dir DIR` | Write entries here instead of `$XDG_DATA_HOME/applications` |
| `--catalog FILE` | Use this user catalog instead of the default path |
| `--ignore-nodisplay` | Treat `NoDisplay`/`Hidden` entries as missing |
| `-h`, `--help` | Full help; `kappfinder-rs help <command>` for one command |
| `-V`, `--version` | Print the version |

## Environment

Behaviour follows the XDG base directory specification:

| Variable | Used for | Default |
| --- | --- | --- |
| `PATH` | Where installed programs are looked up | — |
| `XDG_DATA_HOME` | Where entries are written | `~/.local/share` |
| `XDG_DATA_DIRS` | Searched for existing entries | `/usr/local/share:/usr/share` |
| `XDG_CONFIG_HOME` | Where the user catalog is read from | `~/.config` |

Flatpak and Snap export directories are also searched for existing entries,
even when they are absent from `XDG_DATA_DIRS`.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success, including "nothing to do" and declining at the prompt |
| `1` | Runtime error, e.g. a named binary is not on `$PATH` |
| `2` | Bad command line — unknown flag or missing argument |

## What's in the catalog

117 templates, covering the categories that habitually arrive without menu
entries:

- **Legacy X11** — `xclock`, `xeyes`, `xcalc`, `xman`, `xkill`, `xfontsel`,
  `xedit`, `xmag`, `bitmap`, `editres`, …
- **Terminal emulators** — `xterm`, `urxvt`, `st`
- **System monitors** — `htop`, `btop`, `glances`, `nvtop`, `iotop`, `iftop`,
  `powertop`, `s-tui`
- **File managers & disk** — `ranger`, `nnn`, `lf`, `mc`, `vifm`, `ncdu`, `duf`
- **Editors** — `vim`, `nvim`, `helix`, `kakoune`, `micro`, `nano`
- **Multiplexers** — `tmux`, `zellij`, `screen`
- **Audio/video** — `cmus`, `ncmpcpp`, `cava`, `alsamixer`, `mpv`
- **Network & comms** — `weechat`, `irssi`, `neomutt`, `aerc`, `newsboat`,
  `lynx`, `w3m`, `nmtui`
- **Development** — `tig`, `lazygit`, `gitui`, `lazydocker`, `k9s`, `gdb`,
  `lldb`, `sqlite3`
- **Documents & images** — `zathura`, `mupdf`, `xpdf`, `feh`, `sxiv`, `nsxiv`
- **Toys** — `cmatrix`, `cbonsai`, `asciiquarium`, `nyancat`

## Extending the catalog

Create `~/.config/kappfinder-rs/catalog.toml` using the same syntax as
[`data/catalog.toml`](data/catalog.toml). An entry with the same `bin` as a
built-in replaces it outright — no rebuild needed.

```toml
[[app]]
bin = "myapp"
name = "My Application"
generic_name = "Widget Editor"
comment = "Edits widgets"
icon = "applications-development"     # icon *theme name*, not a path
categories = ["Development"]          # first element must be a main category
terminal = false
keywords = ["widget", "editor"]
exec_args = "%F"                      # appended after the binary path
mime_types = ["text/plain"]
```

Confirm it loaded with `kappfinder-rs catalog`.

## How "already has an entry" is decided

Every `.desktop` file under the XDG application directories — including
Flatpak and Snap exports — is parsed and the program it launches recorded.
Matching is by, in order:

1. **`TryExec`**, which is unambiguous: a bare path, no arguments, no field codes.
2. **The program extracted from `Exec`**, seeing through `env VAR=x`,
   `sh -c "…"`, `nohup`, `pkexec`, `systemd-run` and similar wrappers.
3. **The desktop file stem**, as a fallback for `Exec` values too convoluted
   to parse.

## Safety

- Existing files are **never** overwritten without `--force`.
- Writes go through a temporary file and an atomic rename, so an interrupted
  run cannot leave a half-written entry in your menu.
- `remove` only touches files carrying this tool's marker, inside directories
  you own. System and hand-written entries are never at risk.
- `scan`, `orphans`, and every `--dry-run` write nothing at all.
- Piped (non-interactive) stdin never auto-confirms; `--yes` is required.

## Correctness

```fish
make check      # fmt + clippy -D warnings + the unit tests
make validate   # desktop-file-validate the shipped launcher
```

Tests cover `Exec` tokenising and wrapper unwrapping, Desktop Entry
escape/unescape round-tripping, selection-expression parsing, filename
sanitising, catalog invariants (exactly one main category per entry,
`Audio`/`Video` always paired with `AudioVideo`, no duplicate categories),
and consistency between the catalog and the orphan noise filter — no
catalogued application may be classified as noise.

CI additionally renders an entry for **every one of the 117 templates** — not
just the ones installed on the runner — and runs `desktop-file-validate` over
all of them. Since the validator exits 0 on hints and warnings, CI treats
*any* output as a failure. No template can land that produces a
non-conforming entry.

The hand-written launcher is held to the same bar, validated both as
committed and as `make install-launcher` rewrites it — the form users
actually receive.

To reproduce locally:

```fish
set T (mktemp -d)
kappfinder-rs --dir $T install --yes
desktop-file-validate $T/*.desktop    # zero errors, warnings, and hints
```

> **Note on `Version=1.0`** — that key names the *Desktop Entry spec revision*
> the file claims to follow, not the application version.
> `desktop-file-validate` only recognises `1.0` and `1.1`, so `1.0` is emitted
> for maximum compatibility.

## Architecture

| Module | Responsibility |
| --- | --- |
| `src/catalog.rs` | Template model; built-in catalog embedded via `include_str!`, overlaid by the user catalog |
| `src/desktop.rs` | Desktop Entry parsing, `Exec` tokenising/unwrapping, entry rendering and escaping |
| `src/registry.rs` | Index of existing entries across XDG, Flatpak, and Snap directories |
| `src/scanner.rs` | `$PATH` resolution, executable detection, orphan noise filter |
| `src/select.rs` | Interactive numbered-list selection and confirmation |
| `src/main.rs` | CLI and command implementations |

## Troubleshooting

### The entry was created but does not appear in my menu

Confirm it was actually written and is valid:

```fish
ls ~/.local/share/applications/
desktop-file-validate ~/.local/share/applications/thatapp.desktop
```

Then nudge the desktop. Most environments notice new files on their own, but
caches sometimes need a push:

```fish
update-desktop-database ~/.local/share/applications
kbuildsycoca6                      # KDE Plasma 6 (kbuildsycoca5 on Plasma 5)
```

If it still does not show up, log out and back in. A fresh session rebuilds
the menu unconditionally.

### The launcher does nothing, or a window flashes and vanishes

Nothing at all usually means no default terminal emulator is configured —
the launcher is `Terminal=true` and has nowhere to run. Set one in your
desktop settings.

A window that opens and closes instantly means the binary is not where the
launcher thinks it is. Check what it was pointed at, and compare:

```fish
grep ^TryExec= ~/.local/share/applications/kappfinder-rs.desktop
command -v kappfinder-rs
```

If they disagree, reinstall the launcher against the real location:

```fish
make install-launcher PREFIX=$HOME/.local BINDIR=(dirname (command -v kappfinder-rs))
```

### A terminal application opens and closes instantly

Entries for TUI programs carry `Terminal=true`, which asks the desktop to run
them inside your terminal emulator. If no default terminal is configured, the
launcher has nowhere to put them. Set a default terminal in your desktop
settings, or edit the entry's `Exec=` to invoke your terminal directly:

```ini
Exec=alacritty -e /usr/bin/htop
```

### The entry has a generic or missing icon

`Icon=` holds an icon *theme name*, not a file path, and not every theme has
an icon for every application. Point it at a file instead:

```ini
Icon=/usr/share/pixmaps/thatapp.png
```

Your edit survives: `remove` keys off the `X-KAppFinder-Generated` marker,
not the file contents.

### `scan` says a program is already in the menu, but I cannot find it

The existing entry is probably marked `NoDisplay=true` or `Hidden=true`,
which keeps it out of menus while still occupying the slot. To get a visible
entry anyway:

```fish
kappfinder-rs --ignore-nodisplay install
```

### I want an entry for something not in the catalog

```fish
kappfinder-rs create thatprogram
```

If you will want it on other machines too, add it to your user catalog at
`~/.config/kappfinder-rs/catalog.toml` instead — see
[Extending the catalog](#extending-the-catalog).

### I want to undo everything

```fish
kappfinder-rs remove --dry-run     # check first
kappfinder-rs remove
```

Only files this tool generated are touched.

## Limitations

- Linux/Unix only.
- The `orphans` filter is heuristic by nature and will have false positives
  and negatives. It is deliberately kept out of the catalog-driven path,
  which is exact.
- Flatpak entries launch via `Exec=flatpak run <app-id>`, which resolves to
  the program `flatpak` rather than the application itself. A Flatpak install
  of a program therefore does not suppress an entry for a native binary of
  the same name.
- Icons are theme names, so an application with no themed icon falls back to a
  generic one.

## Project documentation

- [CHANGELOG.md](CHANGELOG.md) — released and unreleased changes
- [CONTRIBUTING.md](CONTRIBUTING.md) — how to add catalog entries, including
  the category rules the tests enforce
- [data/catalog.toml](data/catalog.toml) — the catalog itself, documented
  inline
- [desktop/kappfinder-rs.desktop](desktop/kappfinder-rs.desktop) — the
  launcher `make install` installs, with its desktop actions
- [Makefile](Makefile) — build, install, uninstall, and the
  `desktop-file-validate` check

## Contributing

Catalog additions are the most useful contribution. A good entry is for a
program that is **commonly installed without a `.desktop` file** — if a
distro already ships one, it doesn't belong here.

Before opening a PR:

```fish
make check
make validate
```

The catalog invariant tests will reject entries with the wrong category
structure, so run them.

## Licence

[GPL-3.0-or-later](LICENSE), continuing the original KAppFinder's GPL lineage.

## Acknowledgements

The original KAppFinder was written by Matthias Hölzer-Klüpfel for KDE. This
is an independent reimplementation of its behaviour, not a fork — no original
code is reused.
