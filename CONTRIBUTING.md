# Contributing

Thanks for taking a look. Catalog additions are the most valuable
contribution, and the easiest to get right.

## Before you start

```fish
git clone https://github.com/wlfogle/nexus-menu.git
cd nexus-menu
make check
```

Rust 1.85 or newer. `desktop-file-utils` is worth installing locally so you
can validate entries the same way CI does — `make validate` uses it.

## Adding a catalog entry

Entries live in [`data/catalog.toml`](data/catalog.toml).

### Does it belong?

The bar is: **the program is commonly installed without a `.desktop` file.**

- ✅ Legacy X11 utilities, TUI applications, programs usually built from
  source or shipped as a bare binary.
- ❌ Anything whose distro package already ships a menu entry. Adding it
  means the tool offers a duplicate nobody wants.

Check before proposing:

```fish
# Is a .desktop file already shipped for it?
grep -rl "Exec=.*\bTHEPROGRAM\b" /usr/share/applications ~/.local/share/applications
```

### Writing the entry

```toml
[[app]]
bin = "myapp"                          # bare executable name, no path
name = "My Application"
generic_name = "Widget Editor"         # what kind of program it is
comment = "Edits widgets"              # one short sentence, no trailing dot
icon = "applications-development"      # icon THEME NAME, never a file path
categories = ["Development"]           # see rules below
terminal = true                        # true for TUI/console programs
keywords = ["widget", "editor"]
exec_args = "%F"                       # only if it accepts files/URLs
mime_types = ["text/plain"]            # only if it is a sensible handler
```

### Category rules

These are enforced by tests, so getting them wrong fails CI:

1. **Exactly one main category**, and it must come first. Main categories are
   `AudioVideo`, `Development`, `Education`, `Game`, `Graphics`, `Network`,
   `Office`, `Science`, `Settings`, `System`, `Utility`.
2. **`Audio` and `Video` must be accompanied by `AudioVideo`**, which the
   menu specification requires. They do not count as a second main category.
3. **No duplicates.**
4. Additional categories have required partners — `RasterGraphics` wants
   `2DGraphics`, `FileManager` wants `FileTools`, `Monitor` wants `System` or
   `Network`. `desktop-file-validate` emits a hint when one is missing, and
   CI fails on hints.

When in doubt, consult the
[freedesktop menu specification category registry](https://specifications.freedesktop.org/menu-spec/latest/apas02.html).

### Field codes

`exec_args` is appended verbatim after the binary path:

| Code | Meaning |
| --- | --- |
| `%f` | a single file path |
| `%F` | multiple file paths |
| `%u` | a single URL |
| `%U` | multiple URLs |

Omit `exec_args` entirely for programs that take no file arguments. Do not
add `%F` to something that cannot open files — launchers will pass it paths.

### Verify

```fish
cargo test

# Confirm the entry loads and renders a valid file
cargo build --release
set T (mktemp -d)
./target/release/nexus-menu --dir $T create --force myapp
cat $T/myapp.desktop
desktop-file-validate $T/*.desktop     # must print nothing at all
```

`desktop-file-validate` exits 0 on hints and warnings, so judge it by whether
it printed anything, not by its exit status. CI does the same.

## Code changes

```fish
cargo fmt
make check      # fmt --check, clippy -D warnings, tests
make validate   # desktop-file-validate the shipped launcher
```

Both must be clean; CI enforces them.

CI also builds and tests on the exact Rust version declared as `rust-version`
in `Cargo.toml` (the `msrv` job), using the locked dependencies. If a
dependency bump makes that job fail, either choose a version that still builds
on the declared minimum, or raise `rust-version` and update every place that
repeats it in the same change: the README badge and Requirements section,
CONTRIBUTING, and `.github/release-notes.md`.

The Windows-app commands also have an end-to-end check,
`scripts/test-wine-apps.sh <binary>`, which CI runs against the release
build. It builds a fake Wine prefix with a stub `wine` and touches nothing
outside a temporary directory:

```fish
cargo build
bash scripts/test-wine-apps.sh target/debug/nexus-menu
```

## The desktop app

`gui/` is the desktop app. It is a Cargo workspace of its own on purpose: Tauri
brings several hundred dependencies, and they must not enter the root
`Cargo.lock`, which the static musl release and the `msrv` job build from. CI
fails if a Tauri crate shows up there. The app uses the command line's code
through the library (`nexus-menu = { path = ".." }`), so logic belongs in
`src/engine.rs` and `src/entries.rs`, not in the app.

Building it needs Tauri's development packages and a current stable Rust:

```fish
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
make check-gui      # fmt --check, clippy -D warnings, tests
make build-gui
```

Things that are easy to get wrong:

- **A new command needs three edits.** Add it to `generate_handler!` in
  `gui/src/main.rs`, to the list in `gui/build.rs`, and grant its
  `allow-<name>` permission in `gui/capabilities/default.json`. A command that
  is in Rust but not in the other two cannot be reached from the page, which
  is the point.
- **The page never sends a path or a URL.** It sends ids that the engine handed
  out, and the engine looks each one up. Keep it that way; the tests in
  `gui/src/service.rs`, `src/engine.rs` and `src/entries.rs` assert it. The
  entry commands (`list_entries`, `get_entry`, `save_entry`, `remove_entries`,
  `restore_entry`) take a desktop file ID and field values, never a path.
- **Page data goes in with `textContent`, never `innerHTML`.** Names come from
  files on disk.
- **The `update-test-hooks` feature is for rehearsals only.** It lets the update
  address and key come from the environment and allows plain http. It must
  never be enabled for a release; `scripts/release-gui.sh` refuses a binary
  built with it.

To rehearse an update on a real window without touching GitHub: build the app
with `--features update-test-hooks`, build a second copy of the tree with a
higher version, sign that with `rsign sign -t "nexus-menu-gui version=X.Y.Z"`
using a throwaway key (`rsign generate -W`), serve a `latest.json` made by
`scripts/make-latest-json.sh` from a local directory, and start the first copy
with `NEXUS_MENU_TEST_UPDATE_URL` and `NEXUS_MENU_TEST_UPDATE_KEY` set to that
server and the throwaway public key.

## Releasing the desktop app

A release publishes the command-line archive and the app together, from one
tag. The app's version is in `gui/Cargo.toml` and moves with the command
line's, so:

1. Set the new version in `Cargo.toml` and `gui/Cargo.toml`, update both
   lockfiles (`cargo check` in each), and move the changelog's *Unreleased*
   section under the new version.
2. Tag it `vX.Y.Z` and push the tag. The release workflow builds both, and
   fails if the tag disagrees with either `Cargo.toml`.
3. The `gui` job builds on Ubuntu 22.04, signs, and uploads
   `nexus-menu-gui-X.Y.Z-x86_64-linux`, its `.minisig` and `latest.json`.

Installed copies read `releases/latest/download/latest.json`, which resolves
to the newest release. If the `gui` job fails, that release has no manifest and
update checks quietly fail for everyone until it is fixed; re-run the workflow
with the tag (it has a manual trigger) once it is.

To see what the job will do before tagging:

```fish
make build-gui
set -x NEXUS_MENU_SIGNING_KEY_FILE ~/.config/nexus-menu-signing/nexus-menu.key
bash scripts/release-gui.sh vX.Y.Z /tmp/gui-release     # writes nothing else
```

It stops with a message if the version, the build or the key is wrong.

### The signing key

Every release is signed with one Minisign key. Its public half is compiled into
the app (`PUBLIC_KEY` in `gui/src/updater.rs`); the secret half is the
repository secret `NEXUS_MENU_SIGNING_KEY`, set to the contents of the secret
key file, and a copy on the maintainer's machine. It has no password.

- **Back the secret key up somewhere that is not this repository.** If it is
  lost, no release can ever again be accepted by an installed copy, and
  everyone has to reinstall by hand.
- **Never print it or commit it.** The workflow passes it through the
  environment to a file with mode 600 that is deleted at the end of the step.
- **To change the key**, release one version with the new `PUBLIC_KEY`
  compiled in, signed with the old key, so installed copies accept it and
  carry the new key from then on. Only then sign with the new one.

## Changing the launcher

[`desktop/nexus-menu.desktop`](desktop/nexus-menu.desktop) is
hand-written, and two things about it are load-bearing:

- **`make install-launcher` rewrites `Exec=` and `TryExec=`** to the absolute
  binary path with `sed`. CI asserts the rewrite matched, so keep every
  `Exec=` in the form `sh -c "nexus-menu …"` and `TryExec=nexus-menu`
  on its own line. If you add or remove a desktop action, update the expected
  count in the CI step.
- **It must never carry `X-NexusMenu-Generated` or `X-NexusMenu-Override`.**
  Those markers are what `remove` and the desktop app delete by, and the
  launcher is not the tool's own output.

The desktop app's launcher, [`desktop/nexus-menu-gui.desktop`](desktop/nexus-menu-gui.desktop),
follows the same rule: `make install-gui-launcher` rewrites `Exec=` and
`TryExec=` (both must stay as the bare `nexus-menu-gui` on their own lines, and
CI asserts the rewrite matched), and it must never carry
`X-NexusMenu-Generated` or `X-NexusMenu-Override`. Its `StartupWMClass` is the
window class the app really reports (`nexus-menu-gui`); change one only with
the other.

Validate after any change:

```fish
make validate
make install-launcher DESTDIR=/tmp/stage PREFIX=/usr
desktop-file-validate /tmp/stage/usr/share/applications/nexus-menu.desktop
make install-gui-launcher DESTDIR=/tmp/stage PREFIX=/usr
desktop-file-validate /tmp/stage/usr/share/applications/nexus-menu-gui.desktop
```

House rules, mostly inherited from the problem domain:

- **No stubs or dead code.** If it isn't reachable, don't commit it.
- **The heuristic stays out of the exact path.** `orphans` is allowed to
  guess. `scan`, `install`, and `create` are not — they are catalog-driven
  and must stay that way.
- **Prefer a test over a special case.** The `xclipboard`/`xload` bug was
  found by asserting an invariant between two modules, not by listing
  exceptions. If you find yourself adding a name to an allowlist, consider
  whether the rule itself is wrong.
- **Never widen what is deleted.** The command-line `remove` only touches
  files carrying the `X-NexusMenu-Generated` marker (or the legacy
  `X-KAppFinder-Generated` one that releases before the rename wrote), inside
  directories the user owns. The desktop app's Entries tab (`src/entries.rs`)
  may also delete a file carrying `X-NexusMenu-Override`, the marker on the
  copies it makes of system entries, and nothing else. A hand-written file is
  moved to the backup folder instead of deleted, and a system file is never
  written. The marker is checked on the file as it is read at that moment.
- **Writes stay in two places.** The app writes only to the user's applications
  folder and to `<parent>/nexus-menu/backups` beside it, so `--dir` moves both.
  Before the first change to a file nexus-menu did not make, the original is
  saved once as `<id>.orig`; a removed hand-written file is kept as
  `<id>.removed`. A new operation needs tests like the existing ones: a system
  file's bytes are identical afterwards, nothing is created when the input is
  refused, and an id that could name another path is rejected.

## Reporting bugs

Useful reports include:

- the command you ran and its full output,
- your desktop environment and distribution,
- for wrong or missing detection, the relevant `.desktop` file and the output
  of `nexus-menu scan`.

## Licence

Contributions are accepted under [GPL-3.0-or-later](LICENSE), the project's
licence.
