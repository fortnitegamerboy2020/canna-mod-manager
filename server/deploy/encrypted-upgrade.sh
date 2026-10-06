#!/usr/bin/env bash
set -euo pipefail
source_dir=/home/canna-admin/canna-server
test -x "$source_dir/target/release/canna-server"
trap 'systemctl start canna' EXIT
systemctl stop canna
install -m 755 "$source_dir/target/release/canna-server" /opt/canna/canna-server
/opt/canna/canna-server --encrypt-existing --check-storage
if test -f "$source_dir/staging/catalog-import.json"; then
  /opt/canna/canna-server --import-catalog "$source_dir/staging/catalog-import.json"
fi
chown canna:canna /var/lib/canna/accounts.sqlite*
chmod 600 /var/lib/canna/accounts.sqlite*
install -m 644 "$source_dir/deploy/canna.service" /etc/systemd/system/canna.service
systemctl daemon-reload
systemctl start canna
sleep 2
systemctl is-active --quiet canna
curl --fail --silent http://127.0.0.1:8787/health
