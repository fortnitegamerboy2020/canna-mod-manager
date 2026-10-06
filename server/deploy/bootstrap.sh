#!/usr/bin/env bash
set -euo pipefail
# Run as root on the existing Ubuntu Droplet. Never formats a disk.
test "$(id -u)" = 0
mountpoint -q /mnt/canna
systemctl enable mnt-canna.mount
id canna-admin >/dev/null 2>&1 || useradd -m -s /bin/bash canna-admin
usermod -aG sudo canna-admin
install -d -m 700 -o canna-admin -g canna-admin /home/canna-admin/.ssh
install -m 600 -o canna-admin -g canna-admin /root/.ssh/authorized_keys /home/canna-admin/.ssh/authorized_keys
printf 'canna-admin ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/canna-admin
chmod 440 /etc/sudoers.d/canna-admin
visudo -cf /etc/sudoers.d/canna-admin
id canna >/dev/null 2>&1 || useradd --system --home /var/lib/canna --shell /usr/sbin/nologin canna
install -d -m 750 -o canna -g canna /var/lib/canna /mnt/canna/uploads /mnt/canna/modpacks
install -d -m 755 /opt/canna /etc/canna
ufw default deny incoming
ufw default allow outgoing
ufw allow OpenSSH
ufw allow 80/tcp
ufw allow 443/tcp
ufw --force enable
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq debian-keyring debian-archive-keyring apt-transport-https curl gpg build-essential pkg-config libssl-dev
curl -1fsSL https://dl.cloudsmith.io/public/caddy/stable/gpg.key -o /tmp/canna-caddy-key.asc
gpg --batch --yes --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg /tmp/canna-caddy-key.asc
curl -1fsSL https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt -o /etc/apt/sources.list.d/caddy-stable.list
chmod 644 /usr/share/keyrings/caddy-stable-archive-keyring.gpg /etc/apt/sources.list.d/caddy-stable.list
apt-get update -qq
apt-get install -y -qq caddy
systemctl enable caddy
ufw status
