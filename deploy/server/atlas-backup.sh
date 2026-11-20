#!/usr/bin/env bash
# Backup de Atlas Hub a la unidad exFAT etiquetada ATLASBACKUP.
#
# Lo ejecuta atlas-backup.timer (root). También a mano:
#   sudo bash /opt/atlas/repo/deploy/server/atlas-backup.sh
#
# Copia al directorio <unidad>/<fecha>/:
#   - volúmenes de Podman: mariadb-data, nextcloud-config, nextcloud-data,
#     jellyfin-config (jellyfin-cache se omite: se regenera)
#   - /opt/services/secrets (credenciales de MariaDB y Nextcloud)
#   - /opt/services/media (contenido de Jellyfin; hoy vacío)
#   - /opt/atlas/repo/.env (entorno del relay)
# Conserva los 2 backups más recientes en la unidad (rotación de 2 unidades).
# Si la tarjeta no está puesta, sale sin error (el timer semanal no falla).
set -euo pipefail

MOUNT=/mnt/atlasbackup
LABEL=ATLASBACKUP
STAMP=$(date +%Y-%m-%d_%H%M%S)
KEEP=2
VOLUMES=(mariadb-data nextcloud-config nextcloud-data jellyfin-config)
SERVICES_UID=997

log() { echo "[atlas-backup $(date +%H:%M:%S)] $*"; }
# podman rootless hereda el cwd y falla si `services` no puede entrar en él
# ("cannot chdir to ..."); se ejecuta siempre desde /opt/services (su home).
as_services() { ( cd /opt/services && runuser -u services -- env HOME=/opt/services XDG_RUNTIME_DIR=/run/user/"$SERVICES_UID" "$@" ); }

[ "$(id -u)" -eq 0 ] || { echo "ejecuta como root" >&2; exit 1; }

# Localiza (o monta) la tarjeta por etiqueta. Solo puede haber UNA unidad
# con esta etiqueta conectada: con dos, blkid elegiría al azar y el backup
# podría ir a la tarjeta equivocada.
mapfile -t DEVS < <(blkid -t LABEL="$LABEL" -o device 2>/dev/null || true)
if [ "${#DEVS[@]}" -eq 0 ]; then
  log "unidad $LABEL no presente; nada que hacer"
  exit 0
fi
if [ "${#DEVS[@]}" -gt 1 ]; then
  log "ERROR: ${#DEVS[@]} unidades con etiqueta $LABEL (${DEVS[*]}); deja solo una"
  exit 1
fi
DEV="${DEVS[0]}"
# findmnt -L no existe en util-linux 2.41: se consulta el punto de montaje
# del dispositivo concreto.
if MP=$(findmnt -n -S "$DEV" -o TARGET) && [ -n "$MP" ]; then
  OWN_MOUNT=0
else
  mkdir -p "$MOUNT"
  mount "$DEV" "$MOUNT"
  MP="$MOUNT"
  OWN_MOUNT=1
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
# podman no puede abrir /dev/stdout como `services` ("open /dev/stdout:
# permission denied") y devolvería un tar vacío; se exporta a un fichero
# temporal suyo y se comprime después.
TMP_TAR=""
trap '[ -n "$TMP_TAR" ] && rm -f "$TMP_TAR"' EXIT
for vol in "${VOLUMES[@]}"; do
  if as_services podman volume exists "$vol" 2>/dev/null; then
    log "  - $vol"
    TMP_TAR="/opt/services/.atlas-backup-$vol.tar"
    rm -f "$TMP_TAR"
    as_services podman volume export -o "$TMP_TAR" "$vol"
    if [ ! -s "$TMP_TAR" ]; then
      log "ERROR: export vacío de $vol; aborto"
      exit 1
    fi
    gzip -c "$TMP_TAR" > "$DEST/$vol.tar.gz"
    rm -f "$TMP_TAR"
    TMP_TAR=""
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
