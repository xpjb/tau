#!/usr/bin/env bash
# Deploy the isolated Tau 2 daemon without touching stable Tau.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
[[ $# == 0 ]] || { echo "Usage: $0" >&2; exit 2; }
if grep -Fq '/tau2-beta.service' /proc/$$/cgroup; then
    echo 'Moving daemon build and deployment outside tau2-beta.service.'
    echo 'The current Tau connection will reset during restart; issue no further commands until reconnect.'
    exec systemd-run --quiet --scope --collect --same-dir --expand-environment=no "$root/scripts/deploy-daemon.sh"
fi
[[ $EUID == 0 ]] || { echo 'Daemon deployment requires root.' >&2; exit 1; }
[[ $(git branch --show-current) == tau2 ]] || { echo 'Run from the tau2 worktree.' >&2; exit 1; }
[[ -z $(git status --porcelain) ]] || { echo 'Commit or reconcile local changes first.' >&2; exit 1; }
head=$(git rev-parse HEAD)
remote=$(git ls-remote origin refs/heads/tau2 | cut -f1)
[[ "$head" == "$remote" ]] || { echo 'Deploy only a published origin/tau2 commit.' >&2; exit 1; }
read -r version protocol _ < <(python3 scripts/beta-release.py meta)
mkdir -p "${XDG_CACHE_HOME:-$HOME/.cache}" dist
exec 9>"${XDG_CACHE_HOME:-$HOME/.cache}/tau-daemon-deploy.lock"
flock -n 9 || { echo 'Another Tau daemon deployment owns the lock.' >&2; exit 1; }
export CARGO_BUILD_JOBS=1 RAYON_NUM_THREADS=1 CARGO_NET_OFFLINE=${CARGO_NET_OFFLINE:-true}
unset CARGO_TARGET_DIR
stable_before=$(systemctl show tau.service -p MainPID -p ActiveState -p ExecMainStartTimestampMonotonic; sha256sum /usr/local/bin/taud)
start=$SECONDS
printf '[daemon] build Tau %s / protocol %s\n' "$version" "$protocol"
/usr/local/bin/cargo build --release --locked -p taud
binary="$root/target/release/taud"
artifact="$root/dist/Tau-Beta-$version-taud"
install -m 0700 "$binary" "$artifact"
sha256sum "$artifact" > "$artifact.sha256"
printf '[daemon] build complete in %dm%02ds\n' "$(((SECONDS-start)/60))" "$(((SECONDS-start)%60))"

install -d -m 0700 /var/lib/tau2-beta /root/.local/share/tau2-beta/outbox /root/.local/share/tau2-beta/uploads
install -d -m 0755 /usr/local/lib/tau2-beta
if [[ ! -f /etc/tau2-beta.env ]]; then
    python3 - <<'PY'
import os, secrets, shlex
from pathlib import Path
source = Path('/etc/tau.env')
token = None
if source.is_file():
    for line in source.read_text().splitlines():
        if line.startswith('TAU_TOKEN='):
            values = shlex.split(line.split('=', 1)[1])
            token = values[0] if len(values) == 1 else None
if not token:
    token = secrets.token_hex(32)
assert len(token) >= 32 and not any(character.isspace() for character in token)
fd = os.open('/etc/tau2-beta.env', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, 'w') as output:
    output.write('TAU_TOKEN=' + shlex.quote(token) + '\n')
PY
fi
chmod 0600 /etc/tau2-beta.env
v4=$(tailscale ip -4)
v6=$(tailscale ip -6)
read -r bind4 bind6 < <(V4="$v4" V6="$v6" python3 - <<'PY'
import ipaddress, json, os, subprocess
v4 = str(ipaddress.IPv4Address(os.environ['V4']))
v6 = str(ipaddress.IPv6Address(os.environ['V6']))
local = {address['local'] for interface in json.loads(subprocess.check_output(['ip', '-j', 'address'])) for address in interface.get('addr_info', [])}
print(v4 if v4 in local else '127.0.0.1', v6 if v6 in local else '::1')
PY
)
install -d -m 0755 /etc/systemd/system/tau2-beta.service.d
printf '[Service]\nEnvironment=TAU_TRANSFER_BIND=%s:8792\nEnvironment=TAU_TRANSFER_BIND_V6=[%s]:8792\n' "$bind4" "$bind6" > /etc/systemd/system/tau2-beta.service.d/native-bind.conf.new
chmod 0644 /etc/systemd/system/tau2-beta.service.d/native-bind.conf.new
mv /etc/systemd/system/tau2-beta.service.d/native-bind.conf.new /etc/systemd/system/tau2-beta.service.d/native-bind.conf
systemctl stop tau2-beta.service
install -m 0755 "$artifact" /usr/local/lib/tau2-beta/taud.new
mv -f /usr/local/lib/tau2-beta/taud.new /usr/local/lib/tau2-beta/taud
install -m 0644 "$root/assets/tau2-beta.service" /etc/systemd/system/tau2-beta.service
systemctl daemon-reload
systemctl enable tau2-beta.service
systemctl restart tau2-beta.service
health=
for _ in $(seq 1 100); do
    if health=$(curl --fail --silent --max-time 2 http://127.0.0.1:8791/v1/health); then break; fi
    sleep 0.1
done
HEALTH="$health" VERSION="$version" PROTOCOL="$protocol" python3 - <<'PY'
import json, os
health = json.loads(os.environ['HEALTH'])
assert health['version'] == os.environ['VERSION']
assert health['protocolVersion'] == int(os.environ['PROTOCOL'])
PY
pid=$(systemctl show tau2-beta.service -p MainPID --value)
cmp -s "$artifact" "/proc/$pid/exe" || { echo 'Running daemon binary does not match the release artifact.' >&2; exit 1; }
tailscale serve --bg --yes --http=8789 http://127.0.0.1:8791 >/dev/null
stable_after=$(systemctl show tau.service -p MainPID -p ActiveState -p ExecMainStartTimestampMonotonic; sha256sum /usr/local/bin/taud)
[[ "$stable_after" == "$stable_before" ]] || { echo 'Stable Tau identity changed during deployment.' >&2; exit 1; }
systemctl --no-pager --full status tau2-beta.service | sed -n '1,12p'
echo "Tau daemon $version / protocol $protocol deployed at http://vibe:8789."
echo 'Stable Tau was not modified.'
