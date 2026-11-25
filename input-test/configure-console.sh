#!/bin/sh
set -eu
sudo install -d /etc/systemd/system/getty@tty1.service.d
printf '%s\n' '[Service]' 'ExecStart=' 'ExecStart=-/sbin/agetty --autologin debian --noclear %I $TERM' | sudo tee /etc/systemd/system/getty@tty1.service.d/vantage-autologin.conf >/dev/null
sudo systemctl daemon-reload
