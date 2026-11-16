#!/usr/bin/env bash
# Restablece la contraseña de adria-admin en Nextcloud leyéndola de forma
# oculta y pasándola a occ por variable (sin ttys anidados que cierran SSH).
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "ejecuta con sudo" >&2; exit 1; }
read -r -s -p "Nueva contraseña de adria-admin: " P1; echo
read -r -s -p "Repite la contraseña: " P2; echo
[ -n "$P1" ] || { echo "contraseña vacía" >&2; exit 1; }
[ "$P1" = "$P2" ] || { echo "no coinciden" >&2; exit 1; }
# Un pegado con bracketed paste puede colar caracteres de control
P1=$(printf '%s' "$P1" | tr -d '\r\n\033')
P1=${P1#\[200~}; P1=${P1%\[201~}

cd /
sudo -u services env HOME=/opt/services XDG_RUNTIME_DIR=/run/user/997 OC_PASS="$P1" \
  podman exec -u abc nextcloud php /app/www/public/occ user:resetpassword --password-from-env adria-admin
echo "contraseña restablecida"
