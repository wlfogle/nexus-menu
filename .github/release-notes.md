Finds installed applications that have no menu entry, and creates one.
(Formerly `kappfinder-rs`.)

A maintained, desktop-agnostic reimplementation of KDE's
[KAppFinder](https://apps.kde.org/kappfinder/), which was marked unmaintained
and dropped from release. Like the original it is catalog-driven rather than
heuristic: it ships templates for programs that commonly arrive without a
`.desktop` file, checks which are installed, and offers to create entries only
for the ones missing from your menu.

See [CHANGELOG.md](CHANGELOG.md) for the full list of what's included.

## Install

The attached archive contains a **statically linked** binary. It has no glibc
version requirement and runs on any x86_64 Linux.

```sh
tar xzf nexus-menu-*-x86_64-unknown-linux-musl.tar.gz
cd nexus-menu-*-x86_64-unknown-linux-musl
install -Dm755 nexus-menu ~/.local/bin/nexus-menu
```

Verify the download first if you like:

```sh
sha256sum -c nexus-menu-*.tar.gz.sha256
```

Or build from source with `cargo build --release` (Rust 1.85+).

## Desktop app

The window version is a separate, smaller download:
`nexus-menu-gui-<version>-x86_64-linux`, with its `.minisig` signature beside
it. It needs `libwebkit2gtk-4.1-0` and glibc 2.35 or newer.

```sh
install -Dm755 nexus-menu-gui-*-x86_64-linux ~/.local/bin/nexus-menu-gui
```

It updates itself: a banner offers **Install and restart** when a newer signed
release exists, and nothing is installed without that click. The signing key
(ID `72B341C1C33CBF82`) is compiled into the app, and the signature covers the
version number as well as the file. See the README's "Desktop app" section.

## Quick start

```sh
nexus-menu                  # read-only: what is installed but unmenued
nexus-menu install          # create the missing entries, interactively
nexus-menu remove           # undo everything this tool created
```

`scan`, `orphans`, and any `--dry-run` write nothing. Existing files are never
overwritten without `--force`, and `remove` only deletes files carrying this
tool's own marker.

## Verification

Every release is built in CI, which runs rustfmt, clippy with `-D warnings`,
the test suite, and `desktop-file-validate` over an entry rendered from every
catalog template — failing on any validator output, including hints.
