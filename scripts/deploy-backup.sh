#!/usr/bin/env bash
# Instala (una vez) el backup semanal de Atlas Hub en atlas-hub.
#
# Copia las unidades a /tmp y las instala en /etc/systemd/system con sudo
# interactivo (pide la contraseña de adria_admin), recarga systemd y activa el
# timer. El script del backup NO se copia: vive versionado en el repo desplegado
# (/opt/atlas/repo/deploy/server/atlas-backup.sh) y se actualiza con el `git
# pull` de scripts/deploy-relay.sh. Despliega el relay ANTES de ejecutar esto.
#
# Requiere la tarjeta exFAT con etiqueta ATLASBACKUP para probar una pasada.
#
# Uso (desde la raíz del repo):
#   ./scripts/deploy-backup.sh
#   ATLAS_DEPLOY_HOST=atlas-hub ./scripts/deploy-backup.sh

set -euo pipefail

DEPLOY_HOST="${ATLAS_DEPLOY_HOST:-atlas-hub}"
REMOTE="adria_admin@${DEPLOY_HOST}"
RELAY_REPO="/opt/atlas/repo"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$REPO_ROOT"

for file in deploy/server/atlas-backup.service deploy/server/atlas-backup.timer; do
  [ -f "$file" ] || { echo "⚠️  Falta $file"; exit 1; }
done

if ! ssh -o BatchMode=yes "$REMOTE" "test -f $RELAY_REPO/deploy/server/atlas-backup.sh"; then
  echo "⚠️  El script de backup no está en $RELAY_REPO."
  echo "   Despliega primero el relay: ./scripts/deploy-relay.sh"
  exit 1
fi

echo "→ Copiando unidades a $REMOTE:/tmp…"
scp -q deploy/server/atlas-backup.service deploy/server/atlas-backup.timer "$REMOTE:/tmp/"

echo "→ Instalando unidades y activando el timer (pedirá la contraseña de adria_admin)…"
ssh -t "$REMOTE" "
  set -e
  sudo install -m 644 -o root -g root /tmp/atlas-backup.service /tmp/atlas-backup.timer /etc/systemd/system/
  rm -f /tmp/atlas-backup.service /tmp/atlas-backup.timer
  sudo systemctl daemon-reload
  sudo systemctl enable --now atlas-backup.timer
  echo '--- timer ---'
  systemctl list-timers atlas-backup.timer --no-pager
"

echo "✅ Backup instalado. Prueba una pasada:"
echo "   ssh -t $REMOTE 'sudo systemctl start atlas-backup.service'"
echo "   Logs: ssh $REMOTE 'journalctl -u atlas-backup -f'"
