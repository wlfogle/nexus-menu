#!/usr/bin/env bash
# Write the update manifest the desktop app reads (latest.json) to stdout.
#
#   make-latest-json.sh VERSION URL SIGNATURE_FILE [NOTES] > latest.json
#
# VERSION         the release, e.g. 0.2.0 (no leading "v")
# URL             where the binary can be downloaded; must be https
# SIGNATURE_FILE  the binary's .minisig, as written by `rsign sign`
# NOTES           optional one-line description
#
# The signature goes into the manifest as text. Before writing anything this
# checks that the signature's trusted comment names VERSION, because that is
# the version the app will insist on. A release signed under the wrong version
# would publish fine and then be refused by every installed copy, so it is
# caught here, at release time, not by users.
set -euo pipefail

die() {
    echo "make-latest-json: $*" >&2
    exit 1
}

[ $# -ge 3 ] && [ $# -le 4 ] || die "usage: $0 VERSION URL SIGNATURE_FILE [NOTES]"
version=$1
url=$2
sigfile=$3
notes=${4:-}

[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] ||
    die "'$version' is not a version number like 1.2.3"
[[ $url == https://* ]] || die "the download URL must be https, got '$url'"
[ -f "$sigfile" ] || die "no signature file at '$sigfile'"

want="trusted comment: nexus-menu-gui version=$version"
grep -qxF "$want" "$sigfile" ||
    die "the signature does not carry the line '$want'; sign with: rsign sign -t \"nexus-menu-gui version=$version\" ..."

python3 - "$version" "$url" "$sigfile" "$notes" <<'PY'
import json
import sys

version, url, sigfile, notes = sys.argv[1:5]
with open(sigfile, encoding="utf-8") as f:
    signature = f.read()
print(json.dumps(
    {"version": version, "url": url, "signature": signature, "notes": notes},
    indent=2,
))
PY
