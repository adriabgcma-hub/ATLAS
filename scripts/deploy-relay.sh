#!/usr/bin/env bash
# Despliega el relay en el servidor remoto (homelab) desde el Mac.
#
# El relay vive en `homelab` como servicio systemd de usuario
# (`atlas-relay.service`), con linger activo, así que arranca solo con el
# sistema y sobrevive a cierres de sesión. Telegram y la app web hablan con
# ese relay, no con uno local.
#
# Uso (desde la raíz del repo):
#   ./scripts/deploy-relay.sh          # commitea cambios pendientes, sube, baja en homelab y reinicia el relay
#   ./scripts/deploy-relay.sh --push   # igual, sin commitear (ya subido a mano)
#   ./scripts/deploy-relay.sh --pull   # solo baja cambios del remoto y reinicia el relay (sin commitear)
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

# 3) Reiniciar el relay (servicio systemd de usuario en homelab)
echo "→ Reiniciando relay en $REMOTE..."
ssh_cmd "systemctl --user restart atlas-relay && sleep 2 && systemctl --user is-active atlas-relay"

# 4) Comprobar salud
echo "→ Comprobando salud del relay..."
HEALTH=$(curl -s --max-time 5 "http://$RELAY_HOST:8765/v1/health" || true)
if echo "$HEALTH" | grep -q '"ok": true'; then
  echo "✅ Relay desplegado y sano en http://$RELAY_HOST:8765"
  echo "   $HEALTH"
else
  echo "⚠️  El relay no responde aún. Revisa el log:"
  echo "   ssh -i $SSH_KEY $REMOTE \"journalctl --user -u atlas-relay -n 20\""
  exit 1
fi
