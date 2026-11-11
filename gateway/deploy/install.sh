#!/usr/bin/env bash
# Installs the gateway binary and unit on a Linux host. Run as root after `cargo build --release`.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="${1:-$here/../target/release/codex-gateway}"
id codex >/dev/null 2>&1 || useradd --system --home /var/lib/codex-gateway --shell /usr/sbin/nologin codex
install -d -o codex -g codex /opt/codex/gateway /var/lib/codex-gateway
install -m 0755 "$bin" /opt/codex/gateway/codex-gateway
install -d -m 0750 /etc/codex-gateway
[ -f /etc/codex-gateway/env ] || { install -m 0600 "$here/env.example" /etc/codex-gateway/env; echo "Edit /etc/codex-gateway/env before starting."; }
install -m 0644 "$here/codex-gateway.service" /etc/systemd/system/codex-gateway.service
systemctl daemon-reload
systemctl enable codex-gateway
echo "Installed. Start with: systemctl start codex-gateway"
