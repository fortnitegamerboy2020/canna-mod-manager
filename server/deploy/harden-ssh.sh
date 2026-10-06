#!/usr/bin/env bash
set -euo pipefail
# Run only after independent canna-admin SSH + sudo access has been verified.
cat > /etc/ssh/sshd_config.d/00-canna.conf <<'EOF'
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
EOF
/usr/sbin/sshd -t
systemctl reload ssh
