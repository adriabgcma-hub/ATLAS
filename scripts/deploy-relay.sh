#!/usr/bin/env bash
# Despliega el relay en el servidor remoto (homelab) desde el Mac.
#
# Uso (desde la raíz del repo):
#   ./scripts/deploy-relay.sh          # commitea cambios pendientes, sube, despliega y reinicia el relay
#   ./scripts/deploy-relay.sh --push   # solo sube cambios pendientes y despliega
#   ./scripts/deploy-relay.sh --pull   # solo baja cambios del remoto y reinicia el relay (sin commitear)
#
# El relay vive en el servidor remoto y no recarga solo: cada despliegue lo
# reinicia. Telegram y la app web hablan con ese relay, no con uno local.
set -euo pipefail

RELAY_HOST="${RELAY_SSH_HOST:-192.168.100.26}"
RELAY_USER="${RELAY_SSH_USER:-adria}"
RELAY_DIR="${RELAY_SSH_DIR:-ATLAS}"
SSH_KEY="$HOME/.ssh/atlas_linux"
REMOTE="$RELAY_USER@$RELAY_HOST"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$REPO_ROOT"

ssh_cmd() {
  ssh -i "$SSH_KEY" -o BatchMode=yes "$REMOTE" "$@"
}

# 1) Cambios locales pendientes
if [ "${1:-}" != "--pull" ]; then
  if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "→ Hay cambios sin commitear; creando commit..."
    git add -A
    git commit -m "deploy: sincronización $(date '+%Y-%m-%d %H:%M')"
  fi
  echo "→ Subiendo cambios a origin/main..."
  git push origin main
fi

# 2) Bajar cambios en el remoto
echo "→ Actualizando repo en $REMOTE..."
ssh_cmd "cd $RELAY_DIR && git pull --ff-only"

# 3) Reiniciar el relay
echo "→ Reiniciando relay en $REMOTE..."
ssh_cmd "cd $RELAY_DIR && \
  pkill -f 'server/atlas_relay.py' 2>/dev/null || true; \
  sleep 1; \
  mkdir -p temp/atlas-relay; \
  setsid nohup python3 server/atlas_relay.py > temp/atlas-relay/relay.log 2>&1 < /dev/null & \
  sleep 2; \
  echo 'relay arrancado'"

# 4) Comprobar salud
echo "→ Comprobando salud del relay..."
sleep 1
HEALTH=$(curl -s --max-time 5 "http://$RELAY_HOST:8765/v1/health" || true)
if echo "$HEALTH" | grep -q '"ok": true'; then
  echo "✅ Relay desplegado y sano en http://$RELAY_HOST:8765"
  echo "   $HEALTH"
else
  echo "⚠️  El relay no responde aún. Revisa el log:"
  echo "   ssh -i $SSH_KEY $REMOTE \"tail -20 $RELAY_DIR/temp/atlas-relay/relay.log\""
  exit 1
fi
