#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
[[ $# == 0 ]] || { echo "Usage: $0" >&2; exit 2; }
cargo=/usr/local/bin/cargo
unset CARGO_TARGET_DIR
version=$(python3 -c 'import tomllib; print(tomllib.load(open("crates/frontend/Cargo.toml", "rb"))["package"]["version"])')
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Invalid application version' >&2; exit 1; }
start=$SECONDS
work="$root/target/windows-sfx-Tau-Beta-$version"
bundle="$work/bundle"
payload="$work/tau-windows-payload.tar.lzma"
native_target="$root/target"
windows_target="$root/crates/windows/target"
output="$root/dist/Tau-Beta-$version-windows-x64.exe"
export CARGO_BUILD_JOBS=1 RAYON_NUM_THREADS=1
mkdir -p "$root/dist"
rm -rf "$work"
mkdir -p "$bundle/app"
printf '[windows] build Tau Beta %s\n' "$version"

"$cargo" xwin build --locked --release --target x86_64-pc-windows-msvc --target-dir "$native_target" -p tau-frontend --bin tau
cp "$native_target/x86_64-pc-windows-msvc/release/tau.exe" "$bundle/app/Tau Beta.exe"
TAU_VERSION="$version" "$cargo" xwin build --locked --manifest-path "$root/crates/windows/Cargo.toml" --release \
    --target x86_64-pc-windows-msvc --target-dir "$windows_target" -p tau-launcher --bin tau-launcher
launcher="$windows_target/x86_64-pc-windows-msvc/release/tau-launcher.exe"

BUNDLE="$bundle" PAYLOAD="$payload" python3 -I - <<'PY'
import lzma
import os
import tarfile
from pathlib import Path

bundle = Path(os.environ["BUNDLE"])
payload = Path(os.environ["PAYLOAD"])
with lzma.open(payload, "wb", format=lzma.FORMAT_ALONE, preset=int(os.environ.get("TAU_LZMA_PRESET", "6"))) as compressed:
    with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
        for path in sorted(bundle.rglob("*")):
            if not path.is_file():
                continue
            info = archive.gettarinfo(str(path), arcname=path.relative_to(bundle).as_posix())
            info.uid = 0
            info.gid = 0
            info.uname = ""
            info.gname = ""
            info.mtime = 1_735_689_600
            info.mode = 0o600
            with path.open("rb") as source:
                archive.addfile(info, source)
PY

TAU_VERSION="$version" TAU_PAYLOAD_ARCHIVE="$payload" TAU_LAUNCHER_EXE="$launcher" \
"$cargo" xwin build --locked --release --target x86_64-pc-windows-msvc \
    --manifest-path "$root/crates/windows/Cargo.toml" --target-dir "$windows_target" -p tau-setup
cp "$windows_target/x86_64-pc-windows-msvc/release/tau-setup.exe" "$output"
python3 scripts/beta-release.py windows "$version" >/dev/null
sha256sum "$output" > "$output.sha256"
printf '[windows] complete in %dm%02ds\n' "$(((SECONDS-start)/60))" "$(((SECONDS-start)%60))"
ls -lh "$output" "$output.sha256"
