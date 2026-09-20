#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
binary=${1:?usage: package-spark-release.sh ARM64_BINARY OUTPUT_DIR}
output=${2:?usage: package-spark-release.sh ARM64_BINARY OUTPUT_DIR}
install -d "$output/configs/sparkplane/engines"
install -m 0555 "$binary" "$output/sparkplane-aarch64"
install -m 0644 "$repo/configs/sparkplane/models.toml" "$output/configs/sparkplane/models.toml"
shopt -s nullglob
engines=("$repo/configs/sparkplane/engines/"*.toml)
if ((${#engines[@]} == 0)); then
    echo "engine inventory is empty" >&2
    exit 1
fi
for engine in "${engines[@]}"; do
    install -m 0644 "$engine" "$output/configs/sparkplane/engines/$(basename "$engine")"
done
(
    cd "$output"
    release_engines=(configs/sparkplane/engines/*.toml)
    sha256sum sparkplane-aarch64 configs/sparkplane/models.toml "${release_engines[@]}" > SHA256SUMS
)
