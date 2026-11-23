#!/usr/bin/env bash
# Crea una contraseña de aplicación (token) de Nextcloud usando la contraseña
# del admin, la guarda como credencial CalDAV del relay (reutiliza
# set-nextcloud-env.sh) y recarga el relay.
#
# Si la API de tokens no responde, cae a la contraseña del admin (el
# comportamiento anterior): el calendario sigue funcionando, pero un futuro
# cambio de contraseña volverá a romper la credencial.
#
# Uso (en atlas-hub, como root):
#   sudo bash /opt/atlas/repo/deploy/atlas-hub/create-nextcloud-token.sh
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "ejecuta con sudo" >&2; exit 1; }

REPO=/opt/atlas/repo
NC_USER="${ATLAS_NC_USER:-adria-admin}"
NC_URL="http://atlas-hub:8080"

# /dev/tty para que el pipe posterior a set-nextcloud-env.sh no robe la entrada.
read -r -s -p "Contraseña de $NC_USER en Nextcloud: " PW < /dev/tty
echo
PW=$(printf '%s' "$PW" | tr -d '\r\n\033')
[ -n "$PW" ] || { echo "contraseña vacía" >&2; exit 1; }

echo "== creando contraseña de aplicación =="
resp=$(curl -sS -u "$NC_USER:$PW" -X POST -H "OCS-APIRequest: true" \
  "$NC_URL/ocs/v2.php/core/getapppassword" || true)
token=$(printf '%s' "$resp" | sed -n 's:.*<apppassword>\(.*\)</apppassword>.*:\1:p')
if [ -n "$token" ]; then
  echo "token creado"
else
  echo "aviso: la API de tokens no respondió; se usará la contraseña del admin"
  token="$PW"
fi

printf '%s\n%s\n' "$NC_USER" "$token" | \
  bash "$REPO/deploy/atlas-hub/set-nextcloud-env.sh"

echo "== recargando el relay =="
systemctl restart atlas-relay
sleep 2
systemctl is-active atlas-relay
