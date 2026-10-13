# nexus-menu

[![CI](https://github.com/wlfogle/nexus-menu/actions/workflows/ci.yml/badge.svg)](https://github.com/wlfogle/nexus-menu/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Linux-lightgrey.svg)](#requirements)

**Finds installed applications that have no menu entry, and creates one.**
*(Formerly `kappfinder-rs`.)*

A maintained, desktop-agnostic reimplementation of KDE's
[KAppFinder](https://apps.kde.org/kappfinder/) — which KDE marked unmaintained
and stopped releasing, and which Gentoo removed as a dead package in 2015.
Nothing replaced the specific job it did, so this does.

---

**Getting started:** [Requirements](#requirements) ·
[Install](#install) · [Desktop launcher](#desktop-launcher) ·
[Desktop app](#desktop-app) · [First run](#first-run) · [Usage](#usage)

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

`nexus-menu` finds exactly those programs and writes correct, spec-valid
entries for them.

```console
$ nexus-menu
nexus-menu — application menu entry finder

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

Run `nexus-menu install` to create these entries.
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
- Rust 1.85+ to build from source (the prebuilt binary needs no Rust)
- `desktop-file-utils` — **optional**, only used to refresh the desktop
  database after writing
- The [desktop app](#desktop-app), which is optional, additionally needs
  webkit2gtk 4.1 at run time (`libwebkit2gtk-4.1-0` on Debian and Ubuntu
  22.04 or newer). The command line needs nothing beyond the above.

Works with any freedesktop-compliant desktop: KDE Plasma, GNOME, Xfce, MATE,
Cinnamon, LXQt, Budgie, and tiling WMs that read `.desktop` files.

## Install

### Prebuilt binary

Grab the archive from the
[latest release](https://github.com/wlfogle/nexus-menu/releases/latest).
It is statically linked, so it has no glibc version requirement and runs on
any x86_64 Linux.

```fish
tar xzf nexus-menu-*-x86_64-unknown-linux-musl.tar.gz
cd nexus-menu-*-x86_64-unknown-linux-musl
install -Dm755 nexus-menu ~/.local/bin/nexus-menu
```

Each release also ships a `.sha256` file:

```fish
sha256sum -c nexus-menu-*.tar.gz.sha256
```

### With cargo

```fish
cargo install --git https://github.com/wlfogle/nexus-menu
```

This puts the binary in `~/.cargo/bin`.

### From source

```fish
git clone https://github.com/wlfogle/nexus-menu.git
cd nexus-menu
cargo build --release
install -Dm755 target/release/nexus-menu ~/.local/bin/nexus-menu
```

Or let the `Makefile` place the binary, the launcher, the icon, and the shell
completions in one step — see [Desktop launcher](#desktop-launcher) and
[Shell completions](#shell-completions):

```fish
sudo make install                    # system-wide, under /usr/local
make install PREFIX=$HOME/.local     # or just for you
```

### Desktop app

The window is a separate download from the command line. Take
`nexus-menu-gui-<version>-x86_64-linux` from the
[latest release](https://github.com/wlfogle/nexus-menu/releases/latest); a
`.minisig` signature sits beside it. It is about 8 MB and uses your system's
webkit2gtk, so it needs `libwebkit2gtk-4.1-0` installed. It is built on Ubuntu
22.04, so it needs glibc 2.35 or newer.

```fish
install -Dm755 nexus-menu-gui-*-x86_64-linux ~/.local/bin/nexus-menu-gui
```

To get it into the application menu, run this from the command-line archive
(or a checkout), which ships the launcher:

```fish
make install-gui-launcher PREFIX=$HOME/.local
```

To build it from source you need a current stable Rust and Tauri's development
packages:

```fish
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
make install-gui PREFIX=$HOME/.local     # binary, launcher and icon
```

See [Desktop app](#desktop-app) for what it does and how it updates itself.

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
nexus-menu --version
```

### Uninstalling

Remove any menu entries the tool created **before** deleting the binary,
since `remove` is what knows which files are its own:

```fish
nexus-menu remove
rm ~/.local/bin/nexus-menu        # or: cargo uninstall nexus-menu
rm ~/.local/bin/nexus-menu-gui    # the desktop app, if you installed it
rm -rf ~/.config/nexus-menu       # only if you made a user catalog
rm -rf ~/.local/share/nexus-menu  # the originals the desktop app saved; only if you will never restore anything
```

If you installed with `make`, the matching target removes the binary, the
launcher, the icon, and the completions together:

```fish
nexus-menu remove                 # first, while the binary still exists
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
| `$PREFIX/bin/nexus-menu` | the binary |
| `$PREFIX/share/applications/nexus-menu.desktop` | the launcher |
| `$PREFIX/share/icons/hicolor/scalable/apps/nexus-menu.svg` | the icon |
| `$PREFIX/share/fish/vendor_completions.d/nexus-menu.fish` | fish completions |
| `$PREFIX/share/bash-completion/completions/nexus-menu` | bash completions |
| `$PREFIX/share/zsh/site-functions/_nexus-menu` | zsh completions |

Look for **Menu Entry Finder (Terminal)** in your application menu. It opens a
terminal window, runs `install`, and waits for Enter at the end so the summary
is still on screen when the tool exits. (The desktop app has its own launcher,
**Menu Entry Finder**; see [Desktop app](#desktop-app).)

Right-click the launcher for the other commands, exposed as desktop actions:

- **Scan without changing anything** — the read-only report
- **Create entries for Windows apps** — `wine install`, for the apps in your
  Wine prefixes and game launchers (the default action covers programs on
  your `PATH` only)
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

The launcher carries no `X-NexusMenu-Generated` marker, so
`nexus-menu remove` will never delete it.

## Desktop app

`nexus-menu-gui` is a window for the same job as the command line. It runs the
same code, so an entry made in the window is byte-for-byte the one
`nexus-menu wine install` writes. [Install](#desktop-app) it separately; the
command line does not depend on it.

- **Missing** lists the applications that have no menu entry, grouped by where
  they come from: programs on your `PATH` first, then Windows apps grouped by
  the tool that owns their Wine prefix. Filter, tick, and press **Create
  selected**. **Dry run** reports what would be written and writes nothing;
  **Overwrite existing entries** is `--force`.
- **Entries** lists every application entry on the machine and lets you edit,
  remove or hide any of them, and put them back. See
  [Editing and removing entries](#editing-and-removing-entries).
- **Prefixes** lists the Wine prefixes it found and which tool manages each.

It takes the command line's `--dir`, `--catalog`, `--prefix` and
`--ignore-nodisplay`, plus `--no-update-check`.

### Editing and removing entries

The **Entries** tab lists every application entry on the machine. The buttons
above the list choose where they come from: **Created by nexus-menu** (the
default), **Yours** (files in your own applications folder that nexus-menu did
not write), **System** (installed by packages) or **All**. The box filters by
name or id.

**Edit** opens a form for the name, comment, command, icon, categories,
keywords and the run-in-terminal switch. Before you save, the form says what
the save will do. What happens to the file depends on where the entry came
from:

- **Made by nexus-menu:** the file is changed in place and keeps its marker.
- **Yours:** the file is changed in place, and only the lines you changed.
  Comments, keys the form does not know and `[Desktop Action]` groups stay as
  they were. A copy of the file is saved once, before the first change.
- **System:** the system file is never written. A copy goes into your own
  applications folder under the same name, marked `X-NexusMenu-Override=true`,
  and the desktop uses that instead (the same way KDE's own menu editor makes a
  change). The system file as it was is saved first.

Changing the name, comment or keywords removes that field's translations, which
would otherwise go on showing the old text in other languages; translations of
fields you did not change are kept. Changing the command also removes
`TryExec`, which would still point at the old program and can hide the entry,
and turns off `DBusActivatable`, which would make launchers ignore the command.
A problem with a value is shown under its field and nothing is written.

**Remove** (**Hide** for a system entry) asks for a second click:

- **Made by nexus-menu:** the file is deleted. The program is not touched.
- **Yours:** the file is moved to the backup folder, not deleted. It stays in
  the list, struck through, until you restore it.
- **System:** the copy in your folder is marked `NoDisplay=true`. The program
  stays installed, and it is still offered in "Open with" lists, because
  `NoDisplay` hides an entry from menus without removing it as a file handler.

**Restore original** undoes whichever of these was done: it deletes your edited
copy so the system entry is back, puts your file back as it was before the
first edit, or returns a file you removed. An entry edited and then removed
takes two restores, the second giving the original back.

The saved originals are in `~/.local/share/nexus-menu/backups`, beside the
applications folder (with `--dir`, in `nexus-menu/backups` beside that folder).
If a package updates a system entry after you edited it, the entry is flagged
**System entry changed since your edit**, because your copy is hiding the
update; restore it to see the new version.

The command line does not edit entries, and `nexus-menu remove` does not touch
these copies. It deletes only what it generated.

### Updating

An installed copy updates itself. On launch it checks, quietly in the
background, whether the latest release is newer. If it is, a banner offers
**Install and restart**; nothing is installed without that click. The
command-line tool is not updated this way.

Why that is safe to leave on:

- Releases are signed with a [Minisign](https://jedisct1.github.io/minisign/)
  key whose public half is compiled into the app (key ID `72B341C1C33CBF82`,
  `RWSCvzzDwUGzcj+r+gMxp0xOOsnu3D/HmB0vq5E1nGVqOqGl5+b6ymS2`). A download
  whose signature does not verify is discarded.
- The signature also covers the version number. Someone who can serve files
  to you still cannot pass an old, genuinely signed build off as a newer one.
- Only https is used, a download over 128 MiB is refused, and it must be an
  executable.
- The new file is written beside the installed one and renamed over it, so a
  failure at any step leaves the installed binary exactly as it was.

It will not update a copy it cannot safely replace. One in a directory you
cannot write to (for example `/usr/local/bin` after `sudo make install-gui`)
says so and points you at the releases page, and a build running from a cargo
`target/` directory never replaces itself. `--no-update-check` turns the check
off altogether.

To check a download by hand, use `rsign` (`cargo install rsign2`) or any other
Minisign tool:

```fish
rsign verify -P RWSCvzzDwUGzcj+r+gMxp0xOOsnu3D/HmB0vq5E1nGVqOqGl5+b6ymS2 \
  -x nexus-menu-gui-<version>-x86_64-linux.minisig \
  nexus-menu-gui-<version>-x86_64-linux
```

## First run

The normal sequence is look, then decide, then apply.

```fish
# 1. See what is missing. This writes nothing.
nexus-menu

# 2. See exactly what would be written, still without writing it.
nexus-menu install --dry-run

# 3. Apply, choosing from a numbered list.
nexus-menu install
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

Changed your mind? `nexus-menu remove` deletes everything it created.

## Usage

### Scan — read-only, the default

```fish
nexus-menu
```

Reports how many catalogued applications are installed, how many already have
entries, and lists what's missing. Writes nothing.

### Install

Interactive by default: it prints a numbered list and waits.

```fish
nexus-menu install              # prompt: all / none / 1,3,5-7
nexus-menu install --dry-run    # show what would be written
nexus-menu install --yes        # accept everything
nexus-menu install --force      # overwrite existing files
```

Entries are written to `$XDG_DATA_HOME/applications` (normally
`~/.local/share/applications`). Override with `--dir`.

### Create specific entries

Works for catalogued and uncatalogued binaries alike; uncatalogued ones get
generated defaults.

```fish
nexus-menu create btop ncdu
nexus-menu create /opt/vendor/bin/thing
```

### List binaries with no entry

```fish
nexus-menu orphans                # filtered
nexus-menu orphans --limit 50     # cap the listing (0 means no limit)
nexus-menu orphans --no-filter    # include coreutils, libraries, etc.
```

Read-only and heuristic. Treat it as a starting point, not a worklist — on a
typical desktop it still returns several thousand entries.

The noise filter is never applied to a catalogued program: the catalog is the
ground truth for "this is a real application", so those always appear.

### Undo

Every generated file carries `X-NexusMenu-Generated=true`. (Files made before
the rename to nexus-menu carry `X-KAppFinder-Generated=true`, which `remove`
still recognises.) `remove` deletes
only files carrying that marker, and only inside the target directory or your
own applications directory — hand-written and system entries are never
touched.

```fish
nexus-menu remove --dry-run
nexus-menu remove
```

Editing a generated file is safe. Deleting the marker line opts it out of
`remove` permanently.

This command only deletes what it generated. To edit, hide or restore other
entries, use the desktop app's **Entries** tab
([Editing and removing entries](#editing-and-removing-entries)).

### Inspect the catalog

```fish
nexus-menu catalog
nexus-menu catalog --missing      # templates whose binary isn't installed
```

### Shell completions

`completions <shell>` prints a completion script to stdout for `bash`, `zsh`,
`fish`, `elvish`, or `powershell`. It never reads the catalog, so it works
even if your user catalog is broken.

For fish, a user-level install is one line:

```fish
nexus-menu completions fish > ~/.config/fish/completions/nexus-menu.fish
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

### Windows apps

```fish
nexus-menu wine scan                 # what is installed but not in the menu
nexus-menu wine install --dry-run    # exactly what would be written
nexus-menu wine install              # choose from a numbered list
```

Finds the Windows applications **you installed** in your Wine prefixes and
creates a menu entry for each one that has none. Every app is started the way
its own launcher starts things. Pointing a different Wine at a launcher's
prefix can upgrade or corrupt it, so that is never done.

| Where the app lives | How it is found | What its entry runs |
| --- | --- | --- |
| Plain Wine (`~/.wine`, `$WINEPREFIX`, …) | Start Menu and Desktop shortcuts | `wine start.exe /Unix <shortcut>`, as Wine's own menu builder does |
| PortProton | the same, resolved to the program | `flatpak run ru.linux_gaming.PortProton "<exe>"`, or its `start.sh` if it is not a Flatpak |
| Faugus | the same, in its default prefix only | Faugus's file handler: `flatpak run … --command=faugus-launcher … @@ "<exe>" @@` |
| Lutris | each game's config in `~/.config/lutris/games` | `env LUTRIS_SKIP_INIT=1 lutris lutris:rungame/<slug>` |
| Bottles | the programs in each bottle's `bottle.yml` | `bottles-cli run -b <bottle> -p <program>` |
| WineZGUI | the `.desktop` file it keeps in each prefix | that file, copied unchanged |

Which shortcuts count:

- Windows and Wine components do not: the Accessories, System Tools and
  Startup folders, uninstallers, readmes, manuals, help and website links.
- For PortProton and Faugus the shortcut has to point at a real `.exe`,
  `.bat`, `.cmd` or `.com` inside the prefix. Web links (`.url`), documents
  and help files are skipped.
- An app on several desktops, or in the Start Menu and on a desktop, is listed
  once.

"Already in the menu" means some entry launches that app, whichever tool made
it: by shortcut or program path, by Lutris slug, by bottle and program, or by
WineZGUI prefix. Run `install` twice and the second run creates nothing.

Left alone, and reported as such: PlayOnLinux prefixes; prefixes found on a
disk with nothing in their path to say who manages them (`other`); and Faugus
prefixes other than its default one, because Faugus runs every file in
`<default-prefix>/default`. An app whose launcher is not installed is counted
as "launcher not installed" and skipped. Heroic is not supported.

Good to know:

- A shortcut's command-line arguments are not carried over to PortProton and
  Faugus entries, only the program.
- Lutris's own menu shortcuts use a numeric game id, which cannot be matched
  to a slug, so a game that already has a Lutris-made shortcut may be offered
  again.
- Entries use the generic `wine` icon.
- Everything written carries the generated marker, so `remove` takes it away.

### Wine prefixes

```fish
nexus-menu wine prefixes
nexus-menu wine prefixes --prefix /media/games/PortProton/data/prefixes
```

Lists every prefix the tool can see and who manages it. A directory is a
prefix if it has a `drive_c` folder; nothing else is required. Prefixes are
looked for in these places, and the first to name one decides its owner:

- **`$WINEPREFIX` and `--prefix DIR`** (repeatable). `DIR` may be a prefix or a
  folder of prefixes. Plain Wine unless the path names a launcher.
- **Folders directly inside your home directory**: `~/.wine`, `~/.insomniac`,
  and so on. Plain Wine.
- **Each launcher's own folder**: `~/Faugus`, `~/PortProton`,
  `~/.PlayOnLinux/wineprefix`, `~/Games` (Lutris), and WineZGUI's and Bottles'
  folders under `~/.var/app`.
- **Everywhere else in your home directory**, hidden folders included. A path
  that names a launcher belongs to it; anything else is listed as `other`.
- **Every local disk** (`/media`, `/mnt`, `/run/media`, …), searched the same
  way.

The search stops at each prefix and skips folders that cannot hold one worth
listing: templates and runtimes (`default_pfx`, Bottles' `templates`,
`runners`, `compatibilitytools.d`), Steam's per-game prefixes (`compatdata`,
`steamapps`), caches and build output, trash, Windows system folders, and any
folder with `backup` in its name. It never follows symlinks, goes at most
eight levels deep, and never searches network mounts such as NFS.

Old copies of home directories (for example a `.wine` inside a backup of a
project tree) are listed as `other` unless their folder is named like a
backup. They are never touched.

## Global options

These work before or after the subcommand.

| Flag | Meaning |
| --- | --- |
| `--dir DIR` | Write entries here instead of `$XDG_DATA_HOME/applications` |
| `--catalog FILE` | Use this user catalog instead of the default path |
| `--ignore-nodisplay` | Treat `NoDisplay`/`Hidden` entries as missing |
| `-h`, `--help` | Full help; `nexus-menu help <command>` for one command |
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

Create `~/.config/nexus-menu/catalog.toml` using the same syntax as
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

Confirm it loaded with `nexus-menu catalog`.

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
- The desktop app can edit and hide more than that, but it writes only inside
  your own applications folder and a backup folder beside it. System files are
  never written. Only a file carrying a nexus-menu marker is ever deleted; a
  file of yours that you remove is moved to the backup folder instead, and the
  original is saved before the first change to it. See
  [Editing and removing entries](#editing-and-removing-entries).
- `scan`, `orphans`, and every `--dry-run` write nothing at all.
- Piped (non-interactive) stdin never auto-confirms; `--yes` is required.

## Correctness

```fish
make check      # fmt + clippy -D warnings + the unit tests
make validate   # desktop-file-validate the shipped launcher
make check-gui  # the same for the desktop app (needs Tauri's packages)
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
nexus-menu --dir $T install --yes
desktop-file-validate $T/*.desktop    # zero errors, warnings, and hints
```

> **Note on `Version=1.0`** — that key names the *Desktop Entry spec revision*
> the file claims to follow, not the application version.
> `desktop-file-validate` only recognises `1.0` and `1.1`, so `1.0` is emitted
> for maximum compatibility.

## Architecture

The crate is a library plus a thin command-line binary, and the desktop app is
a second crate that uses the same library.

| Module | Responsibility |
| --- | --- |
| `src/lib.rs` | The library root; everything below except `main.rs` is in it |
| `src/engine.rs` | Scanning and creating entries, returning data instead of printing. The command line formats it; the desktop app shows it. Also the id-based interface the app uses to scan and install, so the page never hands over a path |
| `src/entries.rs` | Editing, hiding, removing and restoring any application entry, system ones included, by desktop file ID. A system file is never written: an edit of one is a marked copy in your own folder, and originals are saved to a backup folder first. Only a file with a nexus-menu marker is ever deleted |
| `gui/` | The desktop app: a separate Cargo workspace (Tauri), so its dependency tree stays out of the command line's lockfile. `src/service.rs` is its logic, `src/updater.rs` the self-update, `ui/` the page |
| `src/catalog.rs` | Template model; built-in catalog embedded via `include_str!`, overlaid by the user catalog |
| `src/desktop.rs` | Desktop Entry parsing, `Exec` tokenising/unwrapping, entry rendering and escaping |
| `src/registry.rs` | Index of existing entries across XDG, Flatpak, and Snap directories |
| `src/scanner.rs` | `$PATH` resolution, executable detection, orphan noise filter |
| `src/select.rs` | Interactive numbered-list selection and confirmation |
| `src/main.rs` | Argument parsing and printing for the command line |

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
grep ^TryExec= ~/.local/share/applications/nexus-menu.desktop
command -v nexus-menu
```

If they disagree, reinstall the launcher against the real location:

```fish
make install-launcher PREFIX=$HOME/.local BINDIR=(dirname (command -v nexus-menu))
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

Your edit survives: `remove` keys off the `X-NexusMenu-Generated` marker,
not the file contents.

### `scan` says a program is already in the menu, but I cannot find it

The existing entry is probably marked `NoDisplay=true` or `Hidden=true`,
which keeps it out of menus while still occupying the slot. To get a visible
entry anyway:

```fish
nexus-menu --ignore-nodisplay install
```

### I want an entry for something not in the catalog

```fish
nexus-menu create thatprogram
```

If you will want it on other machines too, add it to your user catalog at
`~/.config/nexus-menu/catalog.toml` instead — see
[Extending the catalog](#extending-the-catalog).

### I want to undo everything

```fish
nexus-menu remove --dry-run     # check first
nexus-menu remove
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
- The desktop app has been run on X11 (KDE Plasma) only; Wayland has not been
  tried. It is Linux x86_64 only, and the prebuilt one needs glibc 2.35 or
  newer.
- Editing or hiding an entry in the desktop app changes files, and the menu
  depends on your desktop picking the change up. The files were checked
  (checksums, `desktop-file-validate`) and the app was driven in a real
  window, but a change was not watched appearing in Plasma's own menu. If you
  customised the menu with KMenuEdit, an entry can stay where you put it
  whatever its Categories say. Entries can be edited in the app only; the
  command line has no `edit`.

## Project documentation

- [CHANGELOG.md](CHANGELOG.md) — released and unreleased changes
- [CONTRIBUTING.md](CONTRIBUTING.md) — how to add catalog entries, including
  the category rules the tests enforce
- [data/catalog.toml](data/catalog.toml) — the catalog itself, documented
  inline
- [desktop/nexus-menu.desktop](desktop/nexus-menu.desktop) — the
  launcher `make install` installs, with its desktop actions
- [desktop/nexus-menu-gui.desktop](desktop/nexus-menu-gui.desktop) — the
  desktop app's launcher, installed by `make install-gui`
- [Makefile](Makefile) — build, install, uninstall, and the
  `desktop-file-validate` check
- [scripts/](scripts) — the end-to-end check for Windows apps, and the
  release tooling for the desktop app (see CONTRIBUTING)

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
