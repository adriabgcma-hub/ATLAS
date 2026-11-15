#!/usr/bin/env bash
# Instala el cliente tailscale desde el repo oficial (Debian) y lo arranca.
set -euo pipefail
sudo apt-get update
sudo apt-get install -y curl
curl -fsSL https://tailscale.com/install.sh | sudo sh
sudo systemctl enable --now tailscaled
