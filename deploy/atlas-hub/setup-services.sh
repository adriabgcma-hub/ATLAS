#!/usr/bin/env bash
# Prepara y arranca mariadb, nextcloud y jellyfin como usuario rootless `services`.
# Idempotente. Ejecutar como root: sudo bash setup-services.sh
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "ejecuta con sudo" >&2; exit 1; }
trap 'chmod 644 /tmp/atlas-setup.log 2>/dev/null || true' EXIT
exec > >(tee /tmp/atlas-setup.log) 2>&1

R=/opt/atlas/repo
S=/opt/services
U=(sudo -u services env HOME="$S" XDG_RUNTIME_DIR=/run/user/997)

install -d -o services -g services "$S/.config/containers/systemd"
# install -d crea los padres como root; podman rootless exige que ~/.config sea suyo
chown -R services:services "$S/.config"

if ! grep -q '^services:' /etc/subuid; then
  usermod --add-subuids 200000-265535 --add-subgids 200000-265535 services
  echo "rangos subuid/subgid añadidos a services"
fi

bash "$R/deploy/atlas-hub/ufw.sh" >/dev/null
echo "reglas ufw aplicadas"

if [ ! -s "$S/secrets/mariadb.env" ]; then
  printf 'MARIADB_ROOT_PASSWORD=%s\nMARIADB_DATABASE=nextcloud\nMARIADB_USER=nextcloud\nMARIADB_PASSWORD=%s\n' \
    "$(openssl rand -hex 24)" "$(openssl rand -hex 24)" \
    | install -m 600 -o services -g services /dev/stdin "$S/secrets/mariadb.env"
  echo "mariadb.env generado"
fi
if [ ! -s "$S/secrets/nextcloud.env" ]; then
  printf 'PUID=1000\nPGID=1000\nTZ=Europe/Madrid\n' \
    | install -m 600 -o services -g services /dev/stdin "$S/secrets/nextcloud.env"
  echo "nextcloud.env generado"
fi

install -m 644 -o services -g services \
  "$R"/deploy/atlas-hub/quadlet/network/atlas-services.network \
  "$R"/deploy/atlas-hub/quadlet/*.container \
  "$S/.config/containers/systemd/"

echo "== bajando imágenes (puede tardar varios minutos) =="
"${U[@]}" podman pull docker.io/library/mariadb:11 docker.io/linuxserver/nextcloud:latest docker.io/jellyfin/jellyfin:latest

echo "== arrancando unidades =="
"${U[@]}" systemctl --user daemon-reload
"${U[@]}" systemctl --user start mariadb.service nextcloud.service jellyfin.service

echo "== esperando a que respondan =="
for i in $(seq 1 36); do
  nc_code=$(curl -s -o /dev/null -w '%{http_code}' -m 3 http://127.0.0.1:8080/ || true)
  jf_code=$(curl -s -o /dev/null -w '%{http_code}' -m 3 http://127.0.0.1:8096/health || true)
  echo "intento $i: nextcloud=$nc_code jellyfin=$jf_code"
  if [ "$nc_code" != "000" ] && [ "$jf_code" = "200" ]; then break; fi
  sleep 5
done

echo "== dominios de confianza de Nextcloud =="
occ() { "${U[@]}" podman exec -u abc nextcloud php /config/www/nextcloud/occ "$@"; }
occ config:system:set trusted_domains 1 --value=atlas-hub || true
occ config:system:set trusted_domains 2 --value=atlas-hub.local || true
occ config:system:set trusted_domains 3 --value=100.121.19.28 || true

echo "== comprobación de MariaDB =="
PW=$(sed -n 's/^MARIADB_PASSWORD=//p' "$S/secrets/mariadb.env")
"${U[@]}" podman exec mariadb mariadb -unextcloud -p"$PW" -e 'SELECT 1;'

echo "== resumen =="
"${U[@]}" systemctl --user --no-pager --lines=0 is-active mariadb nextcloud jellyfin 2>&1 || true
"${U[@]}" podman ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
echo "--- datos de BD para el asistente de Nextcloud ---"
grep -E 'MARIADB_(DATABASE|USER|PASSWORD)' "$S/secrets/mariadb.env"
