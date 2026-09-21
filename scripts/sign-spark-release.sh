#!/usr/bin/env bash
# Sign only the assembled release. Passwords reach Minisign through stdin, never argv.
set +x
set -euo pipefail
test "$#" -eq 1
test -n "${SPARKPLANE_MINISIGN_SECRET_KEY:-}"
test -n "${MINISIGN_PASSWORD:-}"
release_dir=$(realpath -- "$1")
(cd "$release_dir/appliance" && sha256sum --check --strict SHA256SUMS)
keydir=$(mktemp -d)
trap 'rm -f -- "$keydir/release.key"; rmdir -- "$keydir"' EXIT
umask 077
printf '%s' "$SPARKPLANE_MINISIGN_SECRET_KEY" > "$keydir/release.key"
sign_file() {
    printf '%s\n' "$MINISIGN_PASSWORD" | minisign -Sm "$1" -s "$keydir/release.key"
}
printf '%s\n' "$MINISIGN_PASSWORD" | minisign -R -s "$keydir/release.key" -p "$release_dir/minisign.pub"
sign_file "$release_dir/appliance/SHA256SUMS"
tar -C "$release_dir/appliance" -czf "$release_dir/sparkplane-appliance-aarch64.tar.gz" .
cd "$release_dir"
sha256sum sparkplane-x86_64-unknown-linux-gnu sparkplane-aarch64-unknown-linux-gnu \
    sparkplane-appliance-aarch64.tar.gz minisign.pub > SHA256SUMS
sign_file SHA256SUMS
minisign -V -m SHA256SUMS -p minisign.pub
