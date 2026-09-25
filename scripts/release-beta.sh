#!/usr/bin/env bash
# Beta only. Resumable, sequential release; no model/API calls, no implicit tests.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
helper="$root/scripts/beta-release.py"
cargo=/usr/local/bin/cargo
merge= version= sender=
push=false deploy=false check=false test=false plan=false online=false retry_send=false
usage() {
    cat <<'HELP'
Usage: scripts/release-beta.sh [options]
  --plan                 Print the plan; no build, Git mutation or service action.
  --merge LOCAL_BRANCH   Merge that feature into tau2-integration (never master).
  --version X.Y.Z        Bump daemon/client/lockfile + Android code, and commit.
  --push                 Push tau2-integration to origin/tau2; feature too if merged.
  --deploy               Install beta only, after both packages verify; requires pushed HEAD.
  --check                Compiler check, cached by build inputs (not docs-only commits).
  --test                 Nextest, cached by build inputs. OFF by default. Never Clippy.
  --online               Permit registry downloads; otherwise Cargo is offline.
  --send-command PATH    Executable accepting one absolute file path; Windows then Android.
  --retry-delivery       Explicitly retry an interrupted/uncertain sender invocation.

Default: build + verify packages, produce checksums and delivery.json; no tests,
push, backup or deployment. Rerun the same command to resume verified stages.
Logs and receipts: dist/releases/VERSION/. Never overwrite a different finished release.
HELP
}
while (($#)); do
    case "$1" in
        --merge|--version|--send-command)
            (($# >= 2)) || { usage >&2; exit 2; }
            case "$1" in --merge) merge=$2;; --version) version=$2;; --send-command) sender=$2;; esac
            shift 2;;
        --push) push=true; shift;; --deploy) deploy=true; shift;;
        --check) check=true; shift;; --test) test=true; shift;;
        --plan) plan=true; shift;; --online) online=true; shift;;
        --retry-delivery) retry_send=true; shift;;
        -h|--help) usage; exit 0;; *) usage >&2; exit 2;;
    esac
done
say() { printf '[%(%H:%M:%S)T] %s\n' -1 "$*"; }
die() { echo "ERROR: $*" >&2; exit 1; }
read -r current protocol code < <(python3 "$helper" meta)
if "$plan"; then
    printf 'Current: %s / protocol %s; requested version: %s; merge: %s\n' "$current" "$protocol" "${version:-unchanged}" "${merge:-none}"
    printf 'Check=%s Nextest=%s Push=%s Deploy-beta=%s Sender=%s\n' "$check" "$test" "$push" "$deploy" "${sender:-delivery.json only}"
    echo 'Order: prepare Git/version -> optional cached checks -> daemon -> Windows -> Android -> verify -> push -> beta deploy -> delivery.'
    echo 'One job; registry-offline unless --online. No backups, stable actions or implicit tests.'
    exit 0
fi
[[ $(git branch --show-current) == tau2-integration ]] || die 'Run in the tau2-integration worktree, not stable/master.'
[[ -z $(git status --porcelain) ]] || die 'Commit/reconcile local changes first; nothing will be auto-stashed or discarded.'
[[ -x "$cargo" ]] || die 'Managed Cargo wrapper missing.'
[[ -z "$sender" || "$sender" == /* && -f "$sender" && -x "$sender" ]] || die '--send-command must be an absolute executable path (no shell expression).'
if "$deploy"; then [[ $EUID == 0 ]] || die 'Beta deployment requires root.'; fi
umask 077
mkdir -p "${XDG_CACHE_HOME:-$HOME/.cache}"
exec 9>"${XDG_CACHE_HOME:-$HOME/.cache}/tau2-beta-release.lock"
flock -n 9 || die 'Another beta rollout owns the release lock.'
export CARGO_BUILD_JOBS=1 RAYON_NUM_THREADS=1 ANDROID_ABI=arm64-v8a
export CARGO_NET_OFFLINE=true JAVA_TOOL_OPTIONS="${JAVA_TOOL_OPTIONS:+$JAVA_TOOL_OPTIONS }-XX:ActiveProcessorCount=1"
"$online" && export CARGO_NET_OFFLINE=false
# Keep final artifacts workspace-private; the managed wrapper still owns its shared cache.
unset CARGO_TARGET_DIR
export PATH=/usr/local/bin:$PATH
export TAU_BETA_RELEASE_RUN
TAU_BETA_RELEASE_RUN=$(python3 -c 'import uuid; print(uuid.uuid4())')
child= state= stable_before=
stable_state() {
    systemctl show tau.service -p MainPID -p ActiveState -p ExecMainStartTimestampMonotonic
    sha256sum /usr/local/bin/taud
}
finish() {
    local rc=$?
    trap - EXIT INT TERM
    if ((rc != 0)); then
        [[ -z "$child" ]] || kill "$child" 2>/dev/null || true
        python3 "$helper" stop-owned "$TAU_BETA_RELEASE_RUN" >/dev/null 2>&1 || true
        say "Stopped (exit $rc). Logs: ${state:-not started}. Resume with the same command; do not repeat successful stages."
    fi
    if "$deploy" && [[ -n "$stable_before" && $(stable_state) != "$stable_before" ]]; then
        echo 'ERROR: stable identity changed during rollout; investigate. No repair/restart of stable was attempted.' >&2
        rc=1
    fi
    exit "$rc"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
if "$deploy"; then stable_before=$(stable_state); fi
if "$push"; then
    git fetch origin tau2
    git merge --ff-only origin/tau2
fi
if [[ -n "$merge" ]]; then
    feature=$(git show-ref --verify --hash "refs/heads/$merge") || die 'Feature must be an existing local branch.'
    git merge --no-ff --no-edit "$feature"
fi
if [[ -n "$version" ]]; then
    python3 "$helper" bump "$version"
    if ! git diff --quiet; then
        git add Cargo.lock daemon/Cargo.toml frontend/Cargo.toml frontend/android/AndroidManifest.xml
        git commit -m "Release Tau 2 beta $version"
    fi
fi
read -r version protocol code < <(python3 "$helper" meta)
prepared_head=$(git rev-parse HEAD)
state="$root/dist/releases/$version"
daemon="$root/dist/Tau-Beta-$version-taud"
win="$root/dist/Tau-Beta-$version-windows-x64.exe"
apk="$root/dist/Tau-Beta-$version-android-arm64-v8a.apk"
if [[ ! -d "$state" && ( -e "$daemon" || -e "$win" || -e "$apk" ) ]]; then
    die 'This version already has artifacts without automation receipts. Choose a new --version; do not overwrite delivered files.'
fi
mkdir -p "$state"
python3 "$helper" check-key
key_daemon=$(python3 "$helper" key daemon)
key_win=$(python3 "$helper" key windows)
key_android=$(python3 "$helper" key android)
release_key="$key_daemon:$key_win:$key_android"
finished=false
if [[ -f "$state/complete.json" ]]; then
    python3 "$helper" hit "$state/complete.json" "$release_key" || die 'Finished release inputs/artifacts changed. Use a new version; no silent replacement.'
    finished=true
fi
step() {
    local name=$1 key=$2 log start rc
    shift 2
    local -a outputs=()
    while [[ $1 != -- ]]; do outputs+=("$1"); shift; done
    shift
    if python3 "$helper" hit "$state/$name.json" "$key"; then say "$name: reuse verified result"; return; fi
    log="$state/$name.log"; start=$SECONDS
    say "$name: started; log $log"
    "$@" >"$log" 2>&1 & child=$!
    while kill -0 "$child" 2>/dev/null; do
        sleep 1
        if (( (SECONDS-start) > 0 && (SECONDS-start) % 30 == 0 )); then say "$name: running $((SECONDS-start))s (no extra job started)"; fi
    done
    if wait "$child"; then rc=0; else rc=$?; fi
    child=
    if ((rc)); then tail -n 18 "$log" >&2; return "$rc"; fi
    python3 "$helper" stamp "$state/$name.json" "$key" "${outputs[@]}"
    say "$name: complete in $((SECONDS-start))s"
}
if "$check"; then step check "$(python3 "$helper" key check)" -- "$cargo" check --locked --workspace --all-targets; fi
if "$test"; then step test "$(python3 "$helper" key test)" -- "$cargo" nextest run --locked --workspace --no-fail-fast; fi
if "$finished"; then
    say 'Packages: reuse immutable verified release; no rebuild or re-verification'
else
build_daemon() { "$cargo" build --release --locked -p taud; install -m 0700 "$root/target/release/taud" "$daemon"; }
step daemon "$key_daemon" "$daemon" -- build_daemon
step windows "$key_win" "$win" "$root/target/x86_64-pc-windows-msvc/release/tau.exe" "$root/target/windows-sfx-Tau-Beta-$version/tau-windows-payload.tar.lzma" "$root/windows/target/x86_64-pc-windows-msvc/release/tau-launcher.exe" -- scripts/build-windows-sfx.sh
step android "$key_android" "$root/target/android/arm64-v8a/tau-frontend-arm64-v8a.apk" "$root/target/android/arm64-v8a/libtau_frontend.so" -- frontend/android/build.sh
# Verification changes need not rebuild unchanged binaries.
verifier=$(sha256sum "$helper" | cut -d' ' -f1)
step verify-windows "$key_win:$verifier" "$win" -- python3 "$helper" windows "$version"
step verify-android "$key_android:$verifier" "$root/target/android/arm64-v8a/tau-frontend-arm64-v8a.apk" -- python3 "$helper" android "$version" "$code"
[[ $(python3 "$helper" key daemon) == "$key_daemon" && $(python3 "$helper" key windows) == "$key_win" && $(python3 "$helper" key android) == "$key_android" ]] || die 'Build inputs changed during rollout; no deployment.'
[[ -z $(git status --porcelain) ]] || die 'Working tree changed during rollout; no deployment.'
install -m 0600 "$root/target/android/arm64-v8a/tau-frontend-arm64-v8a.apk" "$apk"
(cd dist; sha256sum "$(basename "$win")" "$(basename "$apk")") > "$root/dist/Tau-Beta-$version-SHA256SUMS.txt"
python3 "$helper" stamp "$state/complete.json" "$release_key" "$daemon" "$win" "$apk"
python3 - "$state/delivery.json" "$version" "$protocol" "$win" "$apk" <<'PY'
import json, pathlib, sys
path, version, protocol, windows, android = sys.argv[1:]
pathlib.Path(path).write_text(json.dumps({'version':version,'protocol':int(protocol),'ready':[windows,android]},indent=2)+'\n')
PY
fi
[[ -z $(git status --porcelain) && $(git rev-parse HEAD) == "$prepared_head" ]] || die 'Checkout changed during rollout; no publication/deployment.'
if "$push"; then
    refs=('HEAD:refs/heads/tau2')
    [[ -z "$merge" ]] || refs+=("refs/heads/$merge:refs/heads/$merge")
    git push --atomic origin "${refs[@]}"
fi
if "$deploy"; then
    remote=$(git ls-remote origin refs/heads/tau2 | cut -f1)
    [[ $(git rev-parse HEAD) == "$remote" ]] || die 'Deploy requires HEAD published to origin/tau2; use --push.'
    deploy_key=$(sha256sum "$daemon" deploy/tau2-beta.service scripts/install-daemon.sh | sha256sum | cut -d' ' -f1)
    pid=$(systemctl show tau2-beta.service -p MainPID --value)
    if [[ ${pid:-0} -gt 0 ]] && python3 "$helper" hit "$state/deploy.json" "$deploy_key" && cmp -s "$daemon" "/proc/$pid/exe" && python3 "$helper" health "$version" "$protocol"; then
        say 'deploy: matching beta already running; no restart'
    else
        # A stale receipt must never bypass reinstallation after a failed health/identity check.
        rm -f "$state/deploy.json"
        step deploy "$deploy_key" /usr/local/lib/tau2-beta/taud /etc/systemd/system/tau2-beta.service /etc/systemd/system/tau2-beta.service.d/native-bind.conf -- env TAU_BETA_BINARY="$daemon" scripts/install-daemon.sh
        python3 "$helper" health "$version" "$protocol"
        pid=$(systemctl show tau2-beta.service -p MainPID --value)
        cmp -s "$daemon" "/proc/$pid/exe" || die 'Running beta binary mismatch.'
    fi
fi
if [[ -n "$sender" ]]; then
    for file in "$win" "$apk"; do
        name=$(basename "$file"); key=$(sha256sum "$file" | cut -d' ' -f1)
        if python3 "$helper" hit "$state/sent-$name.json" "$key"; then say "send: already delivered $name"; continue; fi
        if [[ -f "$state/sending-$name" ]] && ! "$retry_send"; then die "Uncertain delivery of $name. Reconcile the sender before explicitly using --retry-delivery."; fi
        touch "$state/sending-$name"
        "$sender" "$file" & child=$!
        if wait "$child"; then child=; else rc=$?; child=; exit "$rc"; fi
        python3 "$helper" stamp "$state/sent-$name.json" "$key" "$file"
        rm "$state/sending-$name"
    done
else
    say "Ready for Tau attachment delivery, Windows then Android: $state/delivery.json"
fi
if "$deploy"; then [[ $(stable_state) == "$stable_before" ]] || die 'Stable identity changed during rollout.'; fi
say "Complete: $version / protocol $protocol; pushed=$push deployed=$deploy; checksums dist/Tau-Beta-$version-SHA256SUMS.txt"
printf '%s\n%s\n' "$win" "$apk"
