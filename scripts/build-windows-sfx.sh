#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
case "${1:-}" in
    "") beta=false; features=(); channel=Tau; ;;
    --beta) beta=true; features=(--features beta); channel=Tau-Beta; ;;
    *) echo "Usage: $0 [--beta]" >&2; exit 1 ;;
esac
(( $# <= 1 )) || { echo "Usage: $0 [--beta]" >&2; exit 1; }
if $beta; then
    version=$(python3 -c 'import tomllib; print(tomllib.load(open("frontend/Cargo.toml", "rb"))["package"]["version"])')
else
    version=$(awk -F '"' '/^const val TauClientVersion = / { print $2 }' "$root/app/composeApp/src/commonMain/kotlin/app/tau/Platform.kt")
fi
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Invalid application version" >&2; exit 1; }
work="$root/target/windows-sfx-$channel-$version"
bundle="$work/bundle"
payload="$work/tau-windows-payload.tar.lzma"
windows_target=$(realpath -m "${CARGO_TARGET_DIR:-$root/windows/target}")
output="$root/dist/$channel-$version-windows-x64.exe"
mkdir -p "$root/dist"
rm -rf "$work"
mkdir -p "$bundle/app"

if $beta; then
    native_target=$(realpath -m "${CARGO_TARGET_DIR:-$root/target}")
    cargo xwin build --locked --release --target x86_64-pc-windows-msvc --target-dir "$native_target" -p tau-frontend --bin tau
    cp "$native_target/x86_64-pc-windows-msvc/release/tau.exe" "$bundle/app/Tau Beta.exe"
    cp frontend/assets/DejaVu-LICENSE.txt "$bundle/app/"
    mkdir -p dist/tau-beta-windows-x64
    cp "$bundle/app/"* dist/tau-beta-windows-x64/
    TAU_VERSION="$version" cargo xwin build --locked --manifest-path "$root/windows/Cargo.toml" --release \
        --target x86_64-pc-windows-msvc --target-dir "$windows_target" -p tau-launcher --features beta --bin tau-beta-launcher
    launcher="$windows_target/x86_64-pc-windows-msvc/release/tau-beta-launcher.exe"
else
    "$root/app/gradlew" -p "$root/app" -PtauNativeTarget=windows :composeApp:prepareWindowsApp
    cp -a "$root/app/composeApp/build/windows/app/lib" "$bundle/app/lib"
    TAU_VERSION="$version" cargo xwin build --locked --manifest-path "$root/windows/Cargo.toml" --release \
        --target x86_64-pc-windows-msvc --target-dir "$windows_target" -p tau-launcher --bin tau-launcher
    launcher="$windows_target/x86_64-pc-windows-msvc/release/tau-launcher.exe"
fi

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
    --locked \
    --target-dir "$windows_target" \
    --release \
    --target x86_64-pc-windows-msvc \
    -p tau-setup "${features[@]}"

cp "$windows_target/x86_64-pc-windows-msvc/release/tau-setup.exe" "$output"
sha256sum "$output" > "$output.sha256"
ls -lh "$output" "$output.sha256"
