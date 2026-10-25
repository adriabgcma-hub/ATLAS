#!/usr/bin/env bash
# Instala (una vez) el watchdog del relay en atlas-hub.
#
# Copia las unidades a /tmp, las instala en /etc/systemd/system con sudo
# interactivo (hace falta la contraseña de adria_admin: el sudoers mínimo del
# deploy no cubre esta instalación), recarga systemd, activa el timer y lanza
# una primera comprobación.
#
# El script del chequeo NO se copia: vive versionado en el repo desplegado
# (/opt/atlas/repo/deploy/server/atlas-healthcheck.py) y se actualiza con el
# `git pull` de `scripts/deploy-relay.sh`. Despliega el relay ANTES de ejecutar
# esto la primera vez.
#
# Uso (desde la raíz del repo):
#   ./scripts/deploy-healthcheck.sh
#   ATLAS_DEPLOY_HOST=atlas-hub ./scripts/deploy-healthcheck.sh

set -euo pipefail

DEPLOY_HOST="${ATLAS_DEPLOY_HOST:-atlas-hub}"
REMOTE="adria_admin@${DEPLOY_HOST}"
RELAY_REPO="/opt/atlas/repo"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$REPO_ROOT"

for file in deploy/server/atlas-health.service deploy/server/atlas-health.timer; do
  [ -f "$file" ] || { echo "⚠️  Falta $file"; exit 1; }
done

if ! ssh -o BatchMode=yes "$REMOTE" "test -f $RELAY_REPO/deploy/server/atlas-healthcheck.py"; then
  echo "⚠️  El script del watchdog no está en $RELAY_REPO."
  echo "   Despliega primero el relay: ./scripts/deploy-relay.sh"
  exit 1
fi

echo "→ Copiando unidades a $REMOTE:/tmp..."
scp -q deploy/server/atlas-health.service deploy/server/atlas-health.timer "$REMOTE:/tmp/"

echo "→ Instalando unidades y activando el timer (pedirá la contraseña de adria_admin)..."
ssh -t "$REMOTE" "
  set -e
  sudo install -m 644 -o root -g root /tmp/atlas-health.service /tmp/atlas-health.timer /etc/systemd/system/
  rm -f /tmp/atlas-health.service /tmp/atlas-health.timer
  sudo systemctl daemon-reload
  sudo systemctl enable --now atlas-health.timer
  sudo systemctl start atlas-health.service
  echo '--- timer ---'
  systemctl list-timers atlas-health.timer --no-pager
  echo '--- última pasada ---'
  systemctl status atlas-health.service --no-pager --lines=8 || true
"

echo "✅ Watchdog instalado. Logs: ssh $REMOTE \"journalctl -u atlas-health -f\""
