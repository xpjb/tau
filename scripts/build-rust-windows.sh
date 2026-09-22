#!/usr/bin/env bash
set -euo pipefail
# Native frontend ships through the normal per-user installer, in its beta channel.
root=$(cd "$(dirname "$0")/.." && pwd)
exec "$root/scripts/build-windows-sfx.sh" --beta
