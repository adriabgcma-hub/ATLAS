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
# Se pasa por stdin (occ lee la línea del prompt sin necesidad de TTY);
# así la contraseña no queda en argv ni en variables del proceso.
# occ pide la contraseña DOS veces (nueva + confirmación): dos líneas por stdin.
printf '%s\n%s\n' "$P1" "$P1" | \
sudo -u services env HOME=/opt/services XDG_RUNTIME_DIR=/run/user/997 \
  podman exec -i -u abc nextcloud php /app/www/public/occ user:resetpassword adria-admin
echo "contraseña restablecida"
