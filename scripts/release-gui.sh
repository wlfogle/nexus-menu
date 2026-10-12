#!/usr/bin/env bash
# Turn a freshly built desktop app into the three files a release publishes.
#
#   release-gui.sh TAG OUTDIR
#
# Needs
#   - the release binary at gui/target/release/nexus-menu-gui, built by the caller
#   - `rsign` on PATH (cargo install rsign2)
#   - NEXUS_MENU_SIGNING_KEY_FILE: path to the secret signing key (no password)
#   - optionally GITHUB_REPOSITORY (default wlfogle/nexus-menu), for the URLs
#
# Writes to OUTDIR
#   nexus-menu-gui-VERSION-x86_64-linux            the binary
#   nexus-menu-gui-VERSION-x86_64-linux.minisig    its signature
#   latest.json                                    what installed copies read
#
# This is a script, not workflow steps, so the whole release path can be run
# and checked on a developer machine, not first discovered on release day.
#
# It refuses to produce anything it can tell would be unusable: a version that
# disagrees with the tag, a binary built with the update test hooks, or a
# signing key that is not the one compiled into the app. That last one matters
# most: such a release would publish without complaint and then be rejected by
# every copy already installed.
set -euo pipefail

die() {
    echo "release-gui: $*" >&2
    exit 1
}

[ $# -eq 2 ] || die "usage: $0 TAG OUTDIR"
tag=$1
out=$2
version=${tag#v}
repo=${GITHUB_REPOSITORY:-wlfogle/nexus-menu}
key=${NEXUS_MENU_SIGNING_KEY_FILE:-}
[ -n "$key" ] && [ -f "$key" ] || die "NEXUS_MENU_SIGNING_KEY_FILE must name the secret key file"

root=$(cd "$(dirname "$0")/.." && pwd)
bin=$root/gui/target/release/nexus-menu-gui
[ -x "$bin" ] || die "no built binary at $bin"

# 1. The tag, the app's own version and the binary all agree.
manifest_version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/gui/Cargo.toml" | head -1)
[ "$manifest_version" = "$version" ] ||
    die "the tag says $version but gui/Cargo.toml says $manifest_version"
reported=$("$bin" --version)
[ "$reported" = "nexus-menu-gui $version" ] ||
    die "the binary says '$reported', expected 'nexus-menu-gui $version'"

# 2. A real, stripped executable that was not built with the test hooks, which
#    let the update address and key come from the environment.
head -c 4 "$bin" | grep -aq 'ELF' || die "$bin is not an ELF executable"
if grep -aq 'NEXUS_MENU_TEST_UPDATE_URL' "$bin"; then
    die "the binary was built with the update-test-hooks feature; rebuild without it"
fi

# 3. Stage and sign. The signed trusted comment is what the app checks the
#    version against, so it is set here and nowhere else.
asset=nexus-menu-gui-$version-x86_64-linux
mkdir -p "$out"
cp "$bin" "$out/$asset"
rsign sign -s "$key" -W -t "nexus-menu-gui version=$version" \
    -x "$out/$asset.minisig" "$out/$asset" >/dev/null

# 4. The signature has to verify with the public key compiled into the app.
public=$(sed -n 's/^pub const PUBLIC_KEY: &str = "\(.*\)";/\1/p' "$root/gui/src/updater.rs")
[ -n "$public" ] || die "could not read PUBLIC_KEY from gui/src/updater.rs"
rsign verify -P "$public" -x "$out/$asset.minisig" "$out/$asset" >/dev/null 2>&1 ||
    die "the signature does not verify with the public key compiled into the app: the secret key is not the matching one, and no installed copy would accept this release"

# 5. The manifest.
"$root/scripts/make-latest-json.sh" "$version" \
    "https://github.com/$repo/releases/download/$tag/$asset" \
    "$out/$asset.minisig" \
    "nexus-menu-gui $version" >"$out/latest.json"

echo "release-gui: staged in $out"
ls -l "$out"
