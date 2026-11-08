#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
helper="$root/scripts/beta-release.py"
version_request= sender=
plan=false online=false retry_send=false
usage() {
    cat <<'HELP'
Usage: scripts/release-clients.sh [options]
  --plan                 Print the client release plan without changing anything.
  --version X.Y.Z        Bump shared client/daemon metadata and Android code, then commit.
  --online               Permit registry downloads; Cargo is offline by default.
  --send-command PATH    Executable accepting one absolute path; Windows then Android.
  --retry-delivery       Retry a previously interrupted or uncertain sender invocation.

The command requires an already integrated clean tau2 branch. It builds and verifies
Windows first, then Android, pushes tau2, and prepares or performs client delivery.
It never builds, installs, stops or restarts the daemon.
HELP
}
while (($#)); do
    case "$1" in
        --version|--send-command)
            (($# >= 2)) || { usage >&2; exit 2; }
            case "$1" in --version) version_request=$2;; --send-command) sender=$2;; esac
            shift 2;;
        --plan) plan=true; shift;;
        --online) online=true; shift;;
        --retry-delivery) retry_send=true; shift;;
        -h|--help) usage; exit 0;;
        *) usage >&2; exit 2;;
    esac
done
say() { printf '[%(%H:%M:%S)T] %s\n' -1 "$*"; }
die() { echo "ERROR: $*" >&2; exit 1; }
read -r current protocol code < <(python3 "$helper" meta)
if "$plan"; then
    printf 'Current: %s / protocol %s; requested version: %s\n' "$current" "$protocol" "${version_request:-unchanged}"
    echo 'Order: validate integrated tau2 -> version commit -> Windows -> Android -> push -> delivery -> stop.'
    echo 'Daemon build and deployment are separate and always later.'
    exit 0
fi
[[ $(git branch --show-current) == tau2 ]] || die 'Run from the tau2 worktree.'
[[ -z $(git status --porcelain) ]] || die 'Commit or reconcile local changes first.'
[[ -z "$sender" || "$sender" == /* && -f "$sender" && -x "$sender" ]] || die '--send-command must be an absolute executable path.'
umask 077
mkdir -p "${XDG_CACHE_HOME:-$HOME/.cache}"
exec 9>"${XDG_CACHE_HOME:-$HOME/.cache}/tau-client-release.lock"
flock -n 9 || die 'Another Tau client release owns the release lock.'
export CARGO_NET_OFFLINE=true
"$online" && export CARGO_NET_OFFLINE=false
unset CARGO_TARGET_DIR

git fetch origin tau2
git merge-base --is-ancestor origin/tau2 HEAD || die 'Local tau2 does not contain origin/tau2; integrate it before release.'
if [[ -n "$version_request" ]]; then
    python3 "$helper" bump "$version_request"
    if ! git diff --quiet; then
        git add Cargo.lock crates/daemon/Cargo.toml crates/frontend/Cargo.toml crates/frontend/android/AndroidManifest.xml
        git commit -m "Release Tau 2 beta $version_request"
    fi
fi
read -r version protocol code < <(python3 "$helper" meta)
head=$(git rev-parse HEAD)
state="$root/dist/releases/$version"
win="$root/dist/Tau-Beta-$version-windows-x64.exe"
apk="$root/dist/Tau-Beta-$version-android-arm64-v8a.apk"
sums="$root/dist/Tau-Beta-$version-SHA256SUMS.txt"
delivery="$state/delivery.json"
if [[ ! -d "$state" && ( -e "$win" || -e "$apk" ) ]]; then
    die 'This version already has client artifacts without release receipts; use a new version.'
fi
mkdir -p "$state"
python3 "$helper" check-key
key_win=$(python3 "$helper" key windows)
key_android=$(python3 "$helper" key android)
release_key="$head:$key_win:$key_android"
complete=false
if [[ -f "$state/clients.json" ]]; then
    python3 "$helper" hit "$state/clients.json" "$release_key" || die 'Finished client release inputs or artifacts changed; use a new version.'
    complete=true
fi
step() {
    local name=$1 key=$2 start log
    shift 2
    local -a outputs=()
    while [[ ${1:-} != -- ]]; do outputs+=("$1"); shift; done
    shift
    if python3 "$helper" hit "$state/$name.json" "$key"; then
        say "$name: reuse verified result"
        return
    fi
    start=$SECONDS
    log="$state/$name.log"
    say "$name: started; log $log"
    if ! "$@" 2>&1 | tee "$log"; then
        return 1
    fi
    python3 "$helper" stamp "$state/$name.json" "$key" "${outputs[@]}"
    say "$name: complete in $((SECONDS-start))s"
}
if "$complete"; then
    say 'clients: reuse immutable verified release'
else
    step windows "$key_win" "$win" "$win.sha256" -- scripts/build-windows.sh
    step android "$key_android" "$apk" "$apk.sha256" -- env ANDROID_ABI=arm64-v8a scripts/build-android.sh
    [[ $(python3 "$helper" key windows) == "$key_win" && $(python3 "$helper" key android) == "$key_android" ]] || die 'Build inputs changed during release.'
    [[ -z $(git status --porcelain) && $(git rev-parse HEAD) == "$head" ]] || die 'Checkout changed during release.'
    (cd dist && sha256sum "$(basename "$win")" "$(basename "$apk")") > "$sums"
    python3 - "$delivery" "$version" "$protocol" "$head" "$win" "$apk" <<'PY'
import hashlib
import json
from pathlib import Path
import sys
path, version, protocol, commit, windows, android = sys.argv[1:]
def item(platform, value):
    source = Path(value)
    with source.open('rb') as body:
        digest = hashlib.file_digest(body, 'sha256').hexdigest()
    return {'platform': platform, 'path': str(source), 'sha256': digest}
Path(path).write_text(json.dumps({
    'version': version,
    'protocol': int(protocol),
    'commit': commit,
    'ready': [item('windows-x64', windows), item('android-arm64-v8a', android)],
}, indent=2) + '\n')
PY
    python3 "$helper" stamp "$state/clients.json" "$release_key" "$win" "$apk" "$sums" "$delivery"
fi
[[ -z $(git status --porcelain) && $(git rev-parse HEAD) == "$head" ]] || die 'Checkout changed before publication.'
git push origin HEAD:refs/heads/tau2
if [[ -n "$sender" ]]; then
    for file in "$win" "$apk"; do
        name=$(basename "$file")
        key=$(sha256sum "$file" | cut -d' ' -f1)
        if python3 "$helper" hit "$state/sent-$name.json" "$key"; then
            say "send: already delivered $name"
            continue
        fi
        if [[ -f "$state/sending-$name" ]] && ! "$retry_send"; then
            die "Uncertain delivery of $name; reconcile it before using --retry-delivery."
        fi
        touch "$state/sending-$name"
        "$sender" "$file"
        python3 "$helper" stamp "$state/sent-$name.json" "$key" "$file"
        rm "$state/sending-$name"
    done
else
    say "Ready for Tau delivery, Windows then Android: $delivery"
fi
say "Client release complete: $version / protocol $protocol / commit $head"
printf '%s\n%s\n' "$win" "$apk"
