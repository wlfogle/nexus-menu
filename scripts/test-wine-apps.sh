#!/usr/bin/env bash
# End-to-end check of `nexus-menu wine install`, run by CI and runnable by hand:
#
#   scripts/test-wine-apps.sh target/release/nexus-menu
#
# It builds a throwaway Wine prefix with a stub `wine`, then proves that the
# command creates a valid entry for the one real app in it, ignores the
# uninstaller and the duplicate desktop shortcut, does nothing the second time,
# and that `remove` takes its own entry away again. Nothing outside a temporary
# directory is touched.
set -euo pipefail

bin=$(realpath "${1:?usage: $0 PATH_TO_NEXUS_MENU}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

prefix=$work/prefix
entries=$work/entries
stubs=$work/stubs
start_menu="$prefix/drive_c/ProgramData/Microsoft/Windows/Start Menu/Programs"

mkdir -p "$start_menu/Some Game" "$prefix/drive_c/users/me/Desktop" \
  "$stubs" "$entries" "$work/home"

# The app, an uninstaller that must be ignored, and the same app again on a
# desktop that must be reported only once.
: > "$start_menu/Some Game/Some Game.lnk"
: > "$start_menu/Some Game/Uninstall Some Game.lnk"
: > "$prefix/drive_c/users/me/Desktop/Some Game.lnk"

printf '#!/bin/sh\nexit 0\n' > "$stubs/wine"
chmod +x "$stubs/wine"

# An empty home and a stub wine: the result must not depend on this machine.
export HOME="$work/home"
export PATH="$stubs:$PATH"
unset WINEPREFIX

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

"$bin" --dir "$entries" wine install --yes --prefix "$prefix"

count=$(find "$entries" -name 'windows-*.desktop' | wc -l)
[ "$count" -eq 1 ] || fail "expected exactly one entry, found $count"
entry=$(find "$entries" -name 'windows-*.desktop')

grep -q '^Name=Some Game$' "$entry" || fail "wrong Name in $entry"
grep -q '^X-NexusMenu-Generated=true$' "$entry" || fail "missing generated marker"
grep -q "/Unix" "$entry" || fail "does not launch through start.exe /Unix"
grep -q "WINEPREFIX=$prefix" "$entry" || fail "does not name its prefix"
grep -q "^TryExec=$stubs/wine$" "$entry" || fail "does not use the wine on PATH"

# desktop-file-validate exits 0 for warnings and hints, so any output at all
# is a failure.
output=$(desktop-file-validate "$entry" 2>&1 || true)
[ -z "$output" ] || fail "desktop-file-validate reported: $output"

# The entry just written must count as covering its own app.
second=$("$bin" --dir "$entries" wine install --yes --prefix "$prefix")
grep -q "Nothing to do" <<<"$second" || fail "second run was not a no-op: $second"

"$bin" --dir "$entries" remove --yes > /dev/null
left=$(find "$entries" -name 'windows-*.desktop' | wc -l)
[ "$left" -eq 0 ] || fail "remove left $left entries behind"

echo "wine install, validate, idempotency and remove: all good"
