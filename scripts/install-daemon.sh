#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
binary="$root/target/release/taud"
if [[ ! -x "$binary" ]]; then
    echo "Build taud with: cargo build --release --package taud" >&2
    exit 1
fi

install -d -m 0700 /var/lib/tau /var/lib/tau/pi-sessions
install -m 0755 "$binary" /usr/local/bin/taud
install -m 0644 "$root/deploy/tau.service" /etc/systemd/system/tau.service

if [[ ! -f /etc/tau.env ]]; then
    token=$(openssl rand -hex 32)
    umask 077
    cat > /etc/tau.env <<EOF
TAU_BIND=127.0.0.1:8787
TAU_TOKEN=$token
EOF
fi
chmod 0600 /etc/tau.env

systemctl daemon-reload
systemctl enable tau.service
systemctl restart tau.service
for _ in $(seq 1 50); do
    if curl --fail --silent http://127.0.0.1:8787/v1/health >/dev/null; then
        break
    fi
    sleep 0.1
done
curl --fail --silent http://127.0.0.1:8787/v1/health >/dev/null
tailscale serve --bg --yes --http=8787 http://127.0.0.1:8787 >/dev/null
systemctl --no-pager --full status tau.service | sed -n '1,16p'
host_name=$(tailscale status --json | python -c 'import json,sys; print(json.load(sys.stdin)["Self"]["HostName"])')
echo
echo "Tau connection settings:"
echo "  URL: http://$host_name:8787"
echo "  Token: $(sed -n 's/^TAU_TOKEN=//p' /etc/tau.env)"
