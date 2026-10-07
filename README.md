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

```fish
git clone https://github.com/wlfogle/kappfinder-rs.git
cd kappfinder-rs
cargo build --release
install -Dm755 target/release/kappfinder-rs ~/.local/bin/kappfinder-rs
```

Make sure `~/.local/bin` is on your `$PATH`.

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
kappfinder-rs orphans --limit 50
kappfinder-rs orphans --no-filter    # include coreutils, libraries, etc.
```

Read-only and heuristic. Treat it as a starting point, not a worklist.

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

## Global options

| Flag | Meaning |
| --- | --- |
| `--dir DIR` | Write entries here instead of `$XDG_DATA_HOME/applications` |
| `--catalog FILE` | Use this user catalog instead of the default path |
| `--ignore-nodisplay` | Treat `NoDisplay`/`Hidden` entries as missing |

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
cargo test                                  # 37 unit tests
cargo clippy --all-targets -- -D warnings
```

Tests cover `Exec` tokenising and wrapper unwrapping, Desktop Entry
escape/unescape round-tripping, selection-expression parsing, filename
sanitising, and catalog invariants — exactly one main category per entry,
`Audio`/`Video` always paired with `AudioVideo`, and no duplicate categories.

CI additionally renders an entry for **every one of the 117 templates** — not
just the ones installed on the runner — and runs `desktop-file-validate` over
all of them. Since the validator exits 0 on hints and warnings, CI treats
*any* output as a failure. No template can land that produces a
non-conforming entry.

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

## Limitations

- Linux/Unix only.
- The `orphans` filter is heuristic and will have false positives and
  negatives. It is deliberately kept out of the catalog-driven path, which is
  exact.
- Icons are theme names, so an application with no themed icon falls back to a
  generic one.

## Contributing

Catalog additions are the most useful contribution. A good entry is for a
program that is **commonly installed without a `.desktop` file** — if a
distro already ships one, it doesn't belong here.

Before opening a PR:

```fish
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The catalog invariant tests will reject entries with the wrong category
structure, so run them.

## Licence

[GPL-3.0-or-later](LICENSE), continuing the original KAppFinder's GPL lineage.

## Acknowledgements

The original KAppFinder was written by Matthias Hölzer-Klüpfel for KDE. This
is an independent reimplementation of its behaviour, not a fork — no original
code is reused.
