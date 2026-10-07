# Contributing

Thanks for taking a look. Catalog additions are the most valuable
contribution, and the easiest to get right.

## Before you start

```fish
git clone https://github.com/wlfogle/nexus-menu.git
cd nexus-menu
make check
```

Rust 1.74 or newer. `desktop-file-utils` is worth installing locally so you
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

## Changing the launcher

[`desktop/nexus-menu.desktop`](desktop/nexus-menu.desktop) is
hand-written, and two things about it are load-bearing:

- **`make install-launcher` rewrites `Exec=` and `TryExec=`** to the absolute
  binary path with `sed`. CI asserts the rewrite matched, so keep every
  `Exec=` in the form `sh -c "nexus-menu …"` and `TryExec=nexus-menu`
  on its own line. If you add or remove a desktop action, update the expected
  count in the CI step.
- **It must never carry `X-NexusMenu-Generated`.** That marker is what
  `remove` deletes by, and the launcher is not the tool's own output.

Validate after any change:

```fish
make validate
make install-launcher DESTDIR=/tmp/stage PREFIX=/usr
desktop-file-validate /tmp/stage/usr/share/applications/nexus-menu.desktop
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
- **Never widen what `remove` deletes.** It only touches files carrying the
  `X-NexusMenu-Generated` marker (or the legacy `X-KAppFinder-Generated`
  one that releases before the rename wrote), inside directories the user
  owns.

## Reporting bugs

Useful reports include:

- the command you ran and its full output,
- your desktop environment and distribution,
- for wrong or missing detection, the relevant `.desktop` file and the output
  of `nexus-menu scan`.

## Licence

Contributions are accepted under [GPL-3.0-or-later](LICENSE), the project's
licence.
