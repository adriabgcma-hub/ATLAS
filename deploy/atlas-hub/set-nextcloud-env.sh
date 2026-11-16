#!/usr/bin/env bash
# Apunta el relay al calendario de Nextcloud y añade las sondas del panel.
# Idempotente. Ejecutar como root: sudo bash set-nextcloud-env.sh
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "ejecuta con sudo" >&2; exit 1; }
ENV=/opt/atlas/repo/.env
[ -f "$ENV" ] || { echo "no existe $ENV" >&2; exit 1; }

read -r -p "Usuario de Nextcloud [adria-admin]: " NC_USER
NC_USER=${NC_USER:-adria-admin}
read -r -s -p "Contraseña de aplicación de Nextcloud: " NC_PASS
echo
# Un pegado con bracketed paste puede colar caracteres de control
NC_USER=$(printf '%s' "$NC_USER" | tr -d '\r\n\033')
NC_PASS=$(printf '%s' "$NC_PASS" | tr -d '\r\n\033')
NC_USER=${NC_USER#\[200~}; NC_USER=${NC_USER%\[201~}
NC_PASS=${NC_PASS#\[200~}; NC_PASS=${NC_PASS%\[201~}
[ -n "$NC_PASS" ] || { echo "contraseña vacía" >&2; exit 1; }

echo "== comprobando credenciales contra Nextcloud =="
code=$(curl -s -o /dev/null -w '%{http_code}' -u "${NC_USER}:${NC_PASS}" \
  -X PROPFIND -H 'Depth: 0' \
  'http://atlas-hub:8080/remote.php/dav/' || true)
if [ "$code" != "207" ]; then
  echo "ERROR: Nextcloud respondió HTTP $code (se esperaba 207)." >&2
  echo "Revisa usuario/contraseña de aplicación; no se ha guardado nada." >&2
  exit 1
fi
echo "credenciales OK"

tmp=$(mktemp)
grep -vE '^(CALDAV_URL|CALDAV_USERNAME|CALDAV_APP_PASSWORD|CALDAV_EDIT_CALENDARS|PANEL_SERVICE_PROBES|PANEL_SYSTEM_UNITS)=' "$ENV" > "$tmp" || true
{
  echo "CALDAV_URL=http://atlas-hub:8080/remote.php/dav"
  echo "CALDAV_USERNAME=${NC_USER}"
  echo "CALDAV_APP_PASSWORD=${NC_PASS}"
  echo "CALDAV_EDIT_CALENDARS=Exámenes,Eventos,Horario,Deberes"
  echo "PANEL_SERVICE_PROBES=nextcloud=http://atlas-hub:8080,jellyfin=http://atlas-hub:8096"
  echo "PANEL_SYSTEM_UNITS=ssh.service,avahi-daemon.service,ufw.service,atlas-relay.service,tailscaled.service,cockpit.service"
} >> "$tmp"
install -m 600 -o atlas -g atlas "$tmp" "$ENV"
rm -f "$tmp"

systemctl restart atlas-relay
sleep 2
systemctl is-active atlas-relay
curl -s -m 5 http://127.0.0.1:8765/v1/health
echo
