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
