#!/usr/bin/env bash
# Reglas de perímetro. Idempotente. Nada abierto a internet: solo LAN + tailnet.
set -euo pipefail
LAN="192.168.100.0/24"
TAILNET="100.64.0.0/10"
sudo ufw allow from "${LAN}" to any port 9090 proto tcp comment 'cockpit LAN'
sudo ufw allow from "${TAILNET}" to any port 9090 proto tcp comment 'cockpit tailnet'
sudo ufw allow from "${LAN}" to any port 8765 proto tcp comment 'relay LAN'
sudo ufw allow from "${TAILNET}" to any port 8765 proto tcp comment 'relay tailnet'
sudo ufw allow from "${LAN}" to any port 8767 proto tcp comment 'hefesto LAN'
sudo ufw allow from "${TAILNET}" to any port 8767 proto tcp comment 'hefesto tailnet'
sudo ufw allow from "${LAN}" to any port 8080 proto tcp comment 'nextcloud LAN'
sudo ufw allow from "${TAILNET}" to any port 8080 proto tcp comment 'nextcloud tailnet'
sudo ufw allow from "${LAN}" to any port 8096 proto tcp comment 'jellyfin LAN'
sudo ufw allow from "${TAILNET}" to any port 8096 proto tcp comment 'jellyfin tailnet'
sudo ufw status numbered
