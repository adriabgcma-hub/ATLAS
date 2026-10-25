#!/usr/bin/env bash
# Despliega el relay en el servidor remoto (atlas-hub) desde el Mac.
#
# El relay vive en atlas-hub como servicio systemd de SISTEMA
# (`atlas-relay.service`, usuario de sistema `atlas`, repo en
# /opt/atlas/repo): arranca con el sistema, no hay linger que gestionar.
# Telegram y la app web hablan con ese relay, no con uno local.
#
# Se entra por SSH como `adria_admin` (solo clave, alias `atlas-hub`; sin
# `-i`: vale la clave predeterminada del Mac). Como `atlas` no tiene login, el
# pull y el reinicio usan el sudo sin contraseña mínimo de
# deploy/server/atlas-deploy.sudoers.
#
# Variables opcionales (entorno):
#   ATLAS_DEPLOY_HOST  host SSH destino       (defecto atlas-hub)
#   ATLAS_RELAY_URL    URL de salud del relay (defecto
#                      http://atlas-hub.local:8765/v1/health)
#
# Uso (desde la raíz del repo):
#   ./scripts/deploy-relay.sh          # aborta si hay cambios sin commitear, push, baja en atlas-hub y reinicia
#   ./scripts/deploy-relay.sh --push   # igual, sin commitear (ya subido a mano)
#   ./scripts/deploy-relay.sh --pull   # solo baja cambios del remoto y reinicia el relay (sin commitear)
set -euo pipefail

DEPLOY_HOST="${ATLAS_DEPLOY_HOST:-atlas-hub}"
REMOTE="adria_admin@${DEPLOY_HOST}"
RELAY_REPO="/opt/atlas/repo"
SERVICE="atlas-relay"
HEALTH_URL="${ATLAS_RELAY_URL:-http://atlas-hub.local:8765/v1/health}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$REPO_ROOT"

ssh_cmd() {
  ssh -o BatchMode=yes "$REMOTE" "$@"
}

# 1) Cambios locales pendientes
MODE="${1:-}"
if [ "$MODE" != "--pull" ]; then
  # `--push` sube lo ya commiteado sin exigir árbol limpio (habrá WIP de otros
  # cambios); el resto de modos sí abortan si hay algo sin commitear.
  if [ "$MODE" != "--push" ] && { ! git diff --quiet || ! git diff --cached --quiet; }; then
    echo "⚠️  Hay cambios sin commitear. Commitea manualmente antes de desplegar."
    echo "   Archivos modificados:"
    git status --short
    echo ""
    echo "   Usa 'git add -A && git commit -m \"...\"' para commitear."
    echo "   O usa --push para subir sin commitear (si ya están commiteados)."
    exit 1
  fi
  echo "→ Subiendo cambios a origin/main..."
  git push origin main
fi

# 2) Bajar cambios en el remoto (atlas no tiene login: pull vía sudo -u atlas)
echo "→ Actualizando repo en $REMOTE ($RELAY_REPO)..."
if ! ssh_cmd "sudo -u atlas git -C $RELAY_REPO pull --ff-only"; then
  echo "⚠️  Falló el git pull en $REMOTE. ¿Está instalado deploy/server/atlas-deploy.sudoers?"
  echo "   Reinstálalo o valida manualmente: ssh $REMOTE 'sudo -u atlas"
  echo "   git -C $RELAY_REPO pull --ff-only'"
  exit 1
fi

# 3) Reiniciar el relay (servicio systemd de sistema en atlas-hub)
echo "→ Reiniciando relay en $REMOTE..."
if ! ssh_cmd "sudo systemctl restart $SERVICE && sleep 2 && systemctl is-active $SERVICE"; then
  echo "⚠️  No se pudo reiniciar $SERVICE (¿unidad activada?) o quedó inactivo."
  echo "   Revisa: ssh $REMOTE \"journalctl -u $SERVICE -n 20\""
  exit 1
fi

# 4) Comprobar salud (el relay solo escucha en la IP LAN de atlas-hub, no en
#    loopback, así que hay que alcanzarlo por su nombre .local o IP LAN)
echo "→ Comprobando salud del relay ($HEALTH_URL)..."
HEALTH=$(curl -s --max-time 5 "$HEALTH_URL" || true)
if echo "$HEALTH" | grep -q '"ok": true'; then
  echo "✅ Relay desplegado y sano en $HEALTH_URL"
  echo "   $HEALTH"
else
  echo "⚠️  El relay no responde aún. Revisa el log:"
  echo "   ssh $REMOTE \"journalctl -u $SERVICE -n 20\""
  exit 1
fi
