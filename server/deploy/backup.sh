#!/usr/bin/env bash
set -euo pipefail
umask 077
exec 9>/run/lock/canna-backup.lock
flock -n 9 || exit 0
backup=/var/backups/canna-staging
previous=/var/backups/canna-current
test ! -e /var/backups/canna-previous || { echo "Prior interrupted backup needs recovery"; exit 1; }
rm -rf /var/backups/canna-staging
mountpoint -q /mnt/canna
install -d -m 700 "$backup" "$backup/state" "$backup/volume" "$backup/keys" "$backup/config"
needed=$(du -sb /var/lib/canna /mnt/canna /etc/canna/keys | awk '{s+=$1} END {print s}')
already=0
if test -d "$previous"; then already=$(du -sb "$previous" | awk '{print $1}'); fi
binary=$(stat -c %s /opt/canna/canna-server)
test "$((needed + already + 2 * binary))" -le 10737418240 || { echo 'Backup staging plus current copy exceeds the 10 GiB budget; existing recovery copy preserved'; exit 1; }
free=$(df --output=avail -B1 /var/backups | tail -1)
test "$free" -gt "$((needed + binary + 5368709120))" || { echo 'Insufficient backup space: preserve 5 GiB free'; exit 1; }
was_running=0
if systemctl is-active --quiet canna; then was_running=1; fi
resume() { if test "$was_running" = 1; then systemctl start canna; fi; }
trap resume EXIT
systemctl stop canna
rsync -a --link-dest="$previous/state" /var/lib/canna/ "$backup/state/"
rsync -a --link-dest="$previous/volume" /mnt/canna/ "$backup/volume/"
rsync -a --link-dest="$previous/keys" /etc/canna/keys/ "$backup/keys/"
cp -p /etc/caddy/Caddyfile /etc/systemd/system/canna.service "$backup/config/"
cp -p /opt/canna/canna-server "$backup/config/"
resume
was_running=0
restore_test=$(mktemp -d /var/backups/canna-restore-check.XXXXXX)
trap 'rm -rf "$restore_test"' EXIT
cp -a "$backup/state/." "$restore_test/"
CANNA_STATE="$restore_test" CANNA_UPLOADS="$backup/volume/uploads" CREDENTIALS_DIRECTORY="$backup/keys" "$backup/config/canna-server" --check-storage
CANNA_STATE="$restore_test" CANNA_UPLOADS="$backup/volume/uploads" CREDENTIALS_DIRECTORY="$backup/keys" "$backup/config/canna-server" --audit-catalog
date -u +%FT%TZ > "$backup/last-verified"
if test -d "$previous"; then mv "$previous" /var/backups/canna-previous; fi
mv "$backup" "$previous"
rm -rf /var/backups/canna-previous
echo 'Primary-disk recovery copy and isolated storage/catalog restore check passed'
