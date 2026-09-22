#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
cargo xwin build --locked --release --target x86_64-pc-windows-msvc -p tau-frontend --bin tau
mkdir -p dist/tau-beta-windows-x64
cp "${CARGO_TARGET_DIR:-target}/x86_64-pc-windows-msvc/release/tau.exe" "dist/tau-beta-windows-x64/Tau Beta.exe"
cp frontend/assets/DejaVu-LICENSE.txt dist/tau-beta-windows-x64/
printf 'Native portable build: %s/dist/tau-beta-windows-x64/Tau Beta.exe\n' "$root"
