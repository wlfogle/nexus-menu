Finds installed applications that have no menu entry, and creates one.

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

Or build from source with `cargo build --release` (Rust 1.74+).

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
