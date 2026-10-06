#!/usr/bin/env bash
set -euo pipefail
test "$(id -u)" = 0
python3 - <<'PY'
import sqlite3
c=sqlite3.connect('/var/lib/canna/accounts.sqlite')
print({t: c.execute('SELECT COUNT(*) FROM '+t).fetchone()[0] for t in ('users','mods','packs')})
PY
systemctl stop canna
install -d -m 700 /etc/canna/keys
python3 - <<'PY'
import os, secrets
for name in ('database.key', 'uploads.key'):
    path='/etc/canna/keys/'+name
    if not os.path.exists(path):
        fd=os.open(path, os.O_WRONLY|os.O_CREAT|os.O_EXCL, 0o400)
        with os.fdopen(fd,'w') as f: f.write(secrets.token_hex(32))
PY
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq nmap nikto libssl-dev pkg-config
