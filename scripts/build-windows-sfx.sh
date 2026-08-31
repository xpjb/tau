#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
version=${TAU_VERSION:-0.1.0}
cache=${XDG_CACHE_HOME:-$HOME/.cache}/tau
work="$root/target/windows-sfx-$version"
bundle="$work/bundle"
payload="$work/tau-windows-payload.zip"
launcher="$root/windows/target/x86_64-pc-windows-msvc/release/tau-launcher.exe"
output="$root/dist/Tau-$version-windows-x64.exe"
jre_archive="$cache/OpenJDK17U-jre_x64_windows_hotspot_17.0.20.1_1.zip"
jre_sha256=bc21a93923103cdaac93ee337b0ae4365e739fde36df823dd456bc67c8a9d352
jre_url='https://github.com/adoptium/temurin17-binaries/releases/download/jdk-17.0.20.1%2B1/OpenJDK17U-jre_x64_windows_hotspot_17.0.20.1_1.zip'

mkdir -p "$cache" "$root/dist"
rm -rf "$work"
mkdir -p "$bundle"

"$root/app/gradlew" --no-daemon -p "$root/app" :composeApp:prepareWindowsApp
mkdir -p "$bundle/app"
cp -a "$root/app/composeApp/build/windows/app/lib" "$bundle/app/lib"

if [[ ! -s "$jre_archive" ]]; then
    temporary="$jre_archive.tmp"
    rm -f "$temporary"
    curl --fail --location --retry 3 "$jre_url" --output "$temporary"
    echo "$jre_sha256  $temporary" | sha256sum --check --status
    mv "$temporary" "$jre_archive"
fi
echo "$jre_sha256  $jre_archive" | sha256sum --check --status

mkdir -p "$work/jre"
unzip -q "$jre_archive" -d "$work/jre"
javaw=$(find "$work/jre" -type f -path '*/bin/javaw.exe' -print -quit)
if [[ -z "$javaw" ]]; then
    echo "Windows JRE archive has no javaw.exe" >&2
    exit 1
fi
runtime_root=$(dirname "$(dirname "$javaw")")
mkdir -p "$bundle/runtime"
cp -a "$runtime_root"/. "$bundle/runtime"/

cargo xwin build \
    --manifest-path "$root/windows/Cargo.toml" \
    --release \
    --target x86_64-pc-windows-msvc \
    -p tau-launcher

BUNDLE="$bundle" PAYLOAD="$payload" python - <<'PY'
import os
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

bundle = Path(os.environ["BUNDLE"])
payload = Path(os.environ["PAYLOAD"])
with ZipFile(payload, "w", ZIP_DEFLATED, compresslevel=9) as archive:
    for path in sorted(bundle.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(bundle).as_posix()
        info = ZipInfo(relative, (2025, 1, 1, 0, 0, 0))
        info.compress_type = ZIP_DEFLATED
        info.external_attr = 0o100600 << 16
        with path.open("rb") as source, archive.open(info, "w") as target:
            while chunk := source.read(1024 * 1024):
                target.write(chunk)
PY

TAU_VERSION="$version" \
TAU_PAYLOAD_ZIP="$payload" \
TAU_LAUNCHER_EXE="$launcher" \
cargo xwin build \
    --manifest-path "$root/windows/Cargo.toml" \
    --release \
    --target x86_64-pc-windows-msvc \
    -p tau-setup

cp "$root/windows/target/x86_64-pc-windows-msvc/release/tau-setup.exe" "$output"
sha256sum "$output" > "$output.sha256"
ls -lh "$output" "$output.sha256"
