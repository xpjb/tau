#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
version=${TAU_VERSION:-0.4.6}
work="$root/target/windows-sfx-$version"
bundle="$work/bundle"
payload="$work/tau-windows-payload.tar.lzma"
launcher="$root/windows/target/x86_64-pc-windows-msvc/release/tau-launcher.exe"
output="$root/dist/Tau-$version-windows-x64.exe"

mkdir -p "$root/dist"
rm -rf "$work"
mkdir -p "$bundle"

"$root/app/gradlew" --no-daemon -p "$root/app" :composeApp:prepareWindowsApp
mkdir -p "$bundle/app"
cp -a "$root/app/composeApp/build/windows/app/lib" "$bundle/app/lib"

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
