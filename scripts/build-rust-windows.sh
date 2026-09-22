#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
cargo xwin build --locked --release --target x86_64-pc-windows-msvc -p tau-frontend --bin tau
mkdir -p dist/tau2-windows-x64
cp "${CARGO_TARGET_DIR:-target}/x86_64-pc-windows-msvc/release/tau.exe" dist/tau2-windows-x64/Tau.exe
cp frontend/assets/DejaVu-LICENSE.txt dist/tau2-windows-x64/
printf 'Native portable build: %s/dist/tau2-windows-x64/Tau.exe\n' "$root"
