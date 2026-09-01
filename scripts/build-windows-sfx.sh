#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
version=${TAU_VERSION:-0.3.2}
cache=${XDG_CACHE_HOME:-$HOME/.cache}/tau
work="$root/target/windows-sfx-$version"
bundle="$work/bundle"
payload="$work/tau-windows-payload.tar.lzma"
launcher="$root/windows/target/x86_64-pc-windows-msvc/release/tau-launcher.exe"
output="$root/dist/Tau-$version-windows-x64.exe"
jdk_archive="$cache/OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip"
jdk_sha256=f9d6e191ab098c0d416e7d588a24420a8621cd2f4720dab2459b8b7b2d2d8b4e
jdk_url='https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip'

mkdir -p "$cache" "$root/dist"
rm -rf "$work"
mkdir -p "$bundle"

"$root/app/gradlew" --no-daemon -p "$root/app" :composeApp:prepareWindowsApp
mkdir -p "$bundle/app"
cp -a "$root/app/composeApp/build/windows/app/lib" "$bundle/app/lib"

if [[ ! -s "$jdk_archive" ]]; then
    temporary="$jdk_archive.tmp"
    rm -f "$temporary"
    curl --fail --location --retry 3 "$jdk_url" --output "$temporary"
    echo "$jdk_sha256  $temporary" | sha256sum --check --status
    mv "$temporary" "$jdk_archive"
fi
echo "$jdk_sha256  $jdk_archive" | sha256sum --check --status

mkdir -p "$work/jdk"
unzip -q "$jdk_archive" -d "$work/jdk"
jmods=$(find "$work/jdk" -type d -name jmods -print -quit)
if [[ -z "$jmods" ]]; then
    echo "Windows JDK archive has no jmods directory" >&2
    exit 1
fi
java_home=${TAU_JAVA_HOME:-/usr/lib/jvm/java-21-openjdk}
jlink="$java_home/bin/jlink"
if [[ ! -x "$jlink" ]]; then
    echo "Tau's Windows build requires a JDK 21 jlink; set TAU_JAVA_HOME" >&2
    exit 1
fi
"$jlink" \
    --module-path "$jmods" \
    --add-modules java.desktop,java.instrument,java.management,jdk.unsupported \
    --strip-debug \
    --no-header-files \
    --no-man-pages \
    --compress=2 \
    --output "$bundle/runtime"
rm -f \
    "$bundle/runtime/bin/java.exe" \
    "$bundle/runtime/bin/keytool.exe" \
    "$bundle/runtime/lib/jawt.lib" \
    "$bundle/runtime/lib/jvm.lib"

cargo xwin build \
    --manifest-path "$root/windows/Cargo.toml" \
    --release \
    --target x86_64-pc-windows-msvc \
    -p tau-launcher

BUNDLE="$bundle" PAYLOAD="$payload" python - <<'PY'
import lzma
import os
import tarfile
from pathlib import Path

bundle = Path(os.environ["BUNDLE"])
payload = Path(os.environ["PAYLOAD"])
with lzma.open(
    payload,
    "wb",
    format=lzma.FORMAT_ALONE,
    preset=9 | lzma.PRESET_EXTREME,
) as compressed:
    with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
        for path in sorted(bundle.rglob("*")):
            if not path.is_file():
                continue
            relative = path.relative_to(bundle).as_posix()
            info = archive.gettarinfo(str(path), arcname=relative)
            info.uid = 0
            info.gid = 0
            info.uname = ""
            info.gname = ""
            info.mtime = 1_735_689_600
            info.mode = 0o600
            with path.open("rb") as source:
                archive.addfile(info, source)
PY

TAU_VERSION="$version" \
TAU_PAYLOAD_ARCHIVE="$payload" \
TAU_LAUNCHER_EXE="$launcher" \
cargo xwin build \
    --manifest-path "$root/windows/Cargo.toml" \
    --release \
    --target x86_64-pc-windows-msvc \
    -p tau-setup

cp "$root/windows/target/x86_64-pc-windows-msvc/release/tau-setup.exe" "$output"
sha256sum "$output" > "$output.sha256"
ls -lh "$output" "$output.sha256"
