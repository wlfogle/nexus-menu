#!/usr/bin/env bash
# Check scripts/make-latest-json.sh against a throwaway key, using rsign, the
# same tool the release signs with.
#
#   bash scripts/test-latest-json.sh
#
# Needs `rsign` (cargo install rsign2) and python3. Touches nothing outside a
# temporary directory.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
make_json=$here/make-latest-json.sh
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

command -v rsign >/dev/null || fail "rsign is not installed (cargo install rsign2)"

rsign generate -W -f -p "$tmp/k.pub" -s "$tmp/k.key" >/dev/null 2>&1
public=$(sed -n 2p "$tmp/k.pub")
printf '\177ELFfake release binary' >"$tmp/bin"
sign() { # version
    rsign sign -s "$tmp/k.key" -W -t "nexus-menu-gui version=$1" \
        -x "$tmp/sig-$1" "$tmp/bin" >/dev/null
}
sign 1.2.3
url=https://github.com/o/r/releases/download/v1.2.3/nexus-menu-gui-1.2.3-x86_64-linux

# --- a good release produces a manifest the app can use -----------------------
"$make_json" 1.2.3 "$url" "$tmp/sig-1.2.3" "a note" >"$tmp/latest.json"
python3 - "$tmp/latest.json" "$url" <<'PY' || fail "the manifest has the wrong shape"
import json, sys
m = json.load(open(sys.argv[1], encoding="utf-8"))
assert sorted(m) == ["notes", "signature", "url", "version"], sorted(m)
assert m["version"] == "1.2.3" and m["url"] == sys.argv[2] and m["notes"] == "a note"
assert "trusted comment: nexus-menu-gui version=1.2.3" in m["signature"]
PY

# The signature inside the manifest is the real one: pull it back out and let
# rsign check it against the binary.
python3 -c 'import json,sys; sys.stdout.write(json.load(open(sys.argv[1]))["signature"])' \
    "$tmp/latest.json" >"$tmp/extracted.minisig"
rsign verify -P "$public" -x "$tmp/extracted.minisig" "$tmp/bin" >/dev/null 2>&1 ||
    fail "the signature in the manifest does not verify the binary"

# --- mistakes are refused, with a message that says why ----------------------------
refuses() { # description expected-message-fragment -- command...
    local what=$1 fragment=$2
    shift 3
    if out=$("$@" 2>&1 >/dev/null); then
        fail "accepted: $what"
    fi
    grep -qF -- "$fragment" <<<"$out" || fail "$what: wrong message: $out"
}

refuses "a signature made for another version" "does not carry the line" -- \
    "$make_json" 9.9.9 "$url" "$tmp/sig-1.2.3"
refuses "a plain-http URL" "must be https" -- \
    "$make_json" 1.2.3 "http://example.test/x" "$tmp/sig-1.2.3"
refuses "a non-version" "is not a version number" -- \
    "$make_json" latest "$url" "$tmp/sig-1.2.3"
refuses "a missing signature file" "no signature file" -- \
    "$make_json" 1.2.3 "$url" "$tmp/nonexistent"
refuses "too few arguments" "usage:" -- \
    "$make_json" 1.2.3

echo "OK: manifest is well-formed, its signature verifies, and 5 mistakes are refused"
