#!/usr/bin/env bash
# Backup de Atlas Hub a la tarjeta exFAT etiquetada ATLASBACKUP.
#
# Lo ejecuta atlas-backup.timer (root). También a mano:
#   sudo bash /opt/atlas/repo/deploy/server/atlas-backup.sh
#
# Copia al directorio <tarjeta>/<fecha>/:
#   - volúmenes de Podman: mariadb-data, nextcloud-config, nextcloud-data,
#     jellyfin-config (jellyfin-cache se omite: se regenera)
#   - /opt/services/secrets (credenciales de MariaDB y Nextcloud)
#   - /opt/services/media (contenido de Jellyfin; hoy vacío)
#   - /opt/atlas/repo/.env (entorno del relay)
# Conserva los 2 backups más recientes en la tarjeta (rotación de 2 tarjetas).
# Si la tarjeta no está puesta, sale sin error (el timer semanal no falla).
set -euo pipefail

MOUNT=/mnt/atlasbackup
LABEL=ATLASBACKUP
STAMP=$(date +%Y-%m-%d_%H%M%S)
KEEP=2
VOLUMES=(mariadb-data nextcloud-config nextcloud-data jellyfin-config)
SERVICES_UID=997

log() { echo "[atlas-backup $(date +%H:%M:%S)] $*"; }
as_services() { runuser -u services -- env HOME=/opt/services XDG_RUNTIME_DIR=/run/user/"$SERVICES_UID" "$@"; }

[ "$(id -u)" -eq 0 ] || { echo "ejecuta como root" >&2; exit 1; }

# Localiza (o monta) la tarjeta por etiqueta.
if findmnt -n -L "$LABEL" >/dev/null 2>&1; then
  MP=$(findmnt -n -L "$LABEL" -o TARGET)
  OWN_MOUNT=0
elif DEV=$(blkid -L "$LABEL" 2>/dev/null) && [ -n "$DEV" ]; then
  mkdir -p "$MOUNT"
  mount "$DEV" "$MOUNT"
  MP="$MOUNT"
  OWN_MOUNT=1
else
  log "tarjeta $LABEL no presente; nada que hacer"
  exit 0
fi

# Seguridad: nunca escribir en el disco interno.
SRC=$(findmnt -n -o SOURCE --target "$MP" || true)
case "$SRC" in
  /dev/sda*)
    log "ERROR: $LABEL resuelve al disco interno ($SRC); aborto"
    [ "$OWN_MOUNT" = 1 ] && umount "$MP"
    exit 1
    ;;
esac

DEST="$MP/$STAMP"
mkdir -p "$DEST"
log "destino: $DEST"

log "exportando volúmenes de Podman…"
for vol in "${VOLUMES[@]}"; do
  if as_services podman volume exists "$vol" 2>/dev/null; then
    log "  - $vol"
    as_services podman volume export -o - "$vol" | gzip -c > "$DEST/$vol.tar.gz"
  else
    log "  - $vol (no existe; se omite)"
  fi
done

log "copiando secretos, media y .env…"
tar -czf "$DEST/secrets.tar.gz" -C /opt/services secrets
if [ -d /opt/services/media ]; then
  tar -czf "$DEST/media.tar.gz" -C /opt/services media
fi
if [ -f /opt/atlas/repo/.env ]; then
  tar -czf "$DEST/relay-env.tar.gz" -C /opt/atlas/repo .env
fi

( cd "$DEST" && sha256sum -- *.tar.gz > SHA256SUMS )
log "tamaño del backup: $(du -sh "$DEST" | cut -f1)"

sync

# Retención: deja en la tarjeta los KEEP backups más recientes.
mapfile -t all < <(find "$MP" -maxdepth 1 -mindepth 1 -type d -name '20*_*' -printf '%f\n' | sort)
if [ "${#all[@]}" -gt "$KEEP" ]; then
  for old in "${all[@]:0:${#all[@]}-$KEEP}"; do
    log "borrando backup antiguo: $old"
    rm -rf -- "$MP/$old"
  done
fi

[ "$OWN_MOUNT" = 1 ] && umount "$MP"
log "backup completado"
