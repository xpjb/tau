#!/usr/bin/env bash
# Beta-only deployment. Never replace stable Tau's executable, unit, state or route.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
binary=${TAU_BETA_BINARY:-$root/target/release/taud}
[[ -x "$binary" ]] || { echo 'Build: cargo build --release --locked -p taud' >&2; exit 1; }
install -d -m 0700 /var/lib/tau2-beta /root/.local/share/tau2-beta/outbox /root/.local/share/tau2-beta/uploads
install -d -m 0755 /usr/local/lib/tau2-beta
if [[ ! -f /etc/tau2-beta.env ]]; then
    # Reuse the user's connection token, not the stable daemon's OAuth credentials.
    python3 - <<'PY'
import os, secrets, shlex
from pathlib import Path
source=Path('/etc/tau.env')
token=None
if source.is_file():
    for line in source.read_text().splitlines():
        if line.startswith('TAU_TOKEN='):
            values=shlex.split(line.split('=',1)[1]); token=values[0] if len(values)==1 else None
if not token: token=secrets.token_hex(32)
assert len(token)>=32 and not any(c.isspace() for c in token)
fd=os.open('/etc/tau2-beta.env',os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
with os.fdopen(fd,'w') as f: f.write('TAU_TOKEN='+shlex.quote(token)+'\n')
PY
fi
chmod 0600 /etc/tau2-beta.env
# Native data is direct UDP. Kernel-mode Tailnet addresses can be bound
# directly; userspace tailscaled forwards inbound UDP to loopback instead.
# Never expose public wildcard sockets in either mode.
v4=$(tailscale ip -4)
v6=$(tailscale ip -6)
read -r bind4 bind6 < <(V4="$v4" V6="$v6" python3 - <<'PYADDR'
import ipaddress, json, os, subprocess
v4=str(ipaddress.IPv4Address(os.environ['V4']))
v6=str(ipaddress.IPv6Address(os.environ['V6']))
local={a['local'] for iface in json.loads(subprocess.check_output(['ip','-j','address'])) for a in iface.get('addr_info',[])}
print(v4 if v4 in local else '127.0.0.1', v6 if v6 in local else '::1')
PYADDR
)
install -d -m 0755 /etc/systemd/system/tau2-beta.service.d
printf '[Service]\nEnvironment=TAU_TRANSFER_BIND=%s:8792\nEnvironment=TAU_TRANSFER_BIND_V6=[%s]:8792\n' "$bind4" "$bind6" > /etc/systemd/system/tau2-beta.service.d/native-bind.conf.new
chmod 0644 /etc/systemd/system/tau2-beta.service.d/native-bind.conf.new
mv /etc/systemd/system/tau2-beta.service.d/native-bind.conf.new /etc/systemd/system/tau2-beta.service.d/native-bind.conf
# Atomic executable replacement leaves an already-running beta mapped to its old inode.
install -m 0755 "$binary" /usr/local/lib/tau2-beta/taud.new
mv -f /usr/local/lib/tau2-beta/taud.new /usr/local/lib/tau2-beta/taud
install -m 0644 "$root/deploy/tau2-beta.service" /etc/systemd/system/tau2-beta.service
systemctl daemon-reload
systemctl enable tau2-beta.service
systemctl restart tau2-beta.service
for _ in $(seq 1 100); do
    if curl --fail --silent http://127.0.0.1:8791/v1/health >/dev/null; then break; fi
    sleep 0.1
done
curl --fail --silent http://127.0.0.1:8791/v1/health >/dev/null
tailscale serve --bg --yes --http=8789 http://127.0.0.1:8791 >/dev/null
systemctl --no-pager --full status tau2-beta.service | sed -n '1,12p'
echo 'Beta URL: http://vibe:8789 (Tailnet only). Token: /etc/tau2-beta.env.'
echo 'Stable Tau and its existing Tailnet routes were not modified.'
