#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
cargo=/usr/local/bin/cargo
export CARGO_BUILD_JOBS=1 RAYON_NUM_THREADS=1
[[ $# == 0 || $# == 1 && $1 == --beta ]] || { echo "Usage: $0 [--beta]" >&2; exit 1; }
channel=Tau-Beta
version=$(python3 -c 'import tomllib; print(tomllib.load(open("frontend/Cargo.toml", "rb"))["package"]["version"])')
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Invalid application version" >&2; exit 1; }
work="$root/target/windows-sfx-$channel-$version"
bundle="$work/bundle"
payload="$work/tau-windows-payload.tar.lzma"
windows_target=$(realpath -m "${CARGO_TARGET_DIR:-$root/windows/target}")
output="$root/dist/$channel-$version-windows-x64.exe"
mkdir -p "$root/dist"
rm -rf "$work"
mkdir -p "$bundle/app"

native_target=$(realpath -m "${CARGO_TARGET_DIR:-$root/target}")
"$cargo" xwin build --locked --release --target x86_64-pc-windows-msvc --target-dir "$native_target" -p tau-frontend --bin tau
cp "$native_target/x86_64-pc-windows-msvc/release/tau.exe" "$bundle/app/Tau Beta.exe"
TAU_VERSION="$version" "$cargo" xwin build --locked --manifest-path "$root/windows/Cargo.toml" --release \
    --target x86_64-pc-windows-msvc --target-dir "$windows_target" -p tau-launcher --bin tau-launcher
launcher="$windows_target/x86_64-pc-windows-msvc/release/tau-launcher.exe"

BUNDLE="$bundle" PAYLOAD="$payload" python3 -I - <<'PY'
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
    # Good compression without preset 9's large dictionary and extreme CPU cost.
    preset=int(os.environ.get("TAU_LZMA_PRESET", "6")),
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
"$cargo" xwin build \
    --manifest-path "$root/windows/Cargo.toml" \
    --locked \
    --target-dir "$windows_target" \
    --release \
    --target x86_64-pc-windows-msvc \
    -p tau-setup

cp "$windows_target/x86_64-pc-windows-msvc/release/tau-setup.exe" "$output"
sha256sum "$output" > "$output.sha256"
ls -lh "$output" "$output.sha256"
