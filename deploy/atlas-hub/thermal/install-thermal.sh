#!/usr/bin/env bash
# Instala y aplica la protección térmica de atlas-hub (fase 1).
# Idempotente. Uso: sudo bash install-thermal.sh
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ "$(id -u)" -ne 0 ]; then
  echo "Ejecuta como root: sudo bash $0" >&2
  exit 1
fi

missing=()
for pkg in lm-sensors mbpfan; do
  dpkg -s "$pkg" >/dev/null 2>&1 || missing+=("$pkg")
done
if [ "${#missing[@]}" -gt 0 ]; then
  echo "Instalando: ${missing[*]}"
  DEBIAN_FRONTEND=noninteractive apt-get update -qq
  DEBIAN_FRONTEND=noninteractive apt-get install -y "${missing[@]}"
fi

if [ -f /etc/mbpfan.conf ] && ! cmp -s "$HERE/mbpfan.conf" /etc/mbpfan.conf \
   && [ ! -f /etc/mbpfan.conf.pre-atlas ]; then
  cp -a /etc/mbpfan.conf /etc/mbpfan.conf.pre-atlas
  echo "Config original guardada en /etc/mbpfan.conf.pre-atlas"
fi

install -m 644 "$HERE/mbpfan.conf" /etc/mbpfan.conf
install -d /etc/systemd/system/mbpfan.service.d
install -m 644 "$HERE/mbpfan.service.d/atlas.conf" /etc/systemd/system/mbpfan.service.d/atlas.conf
install -m 755 "$HERE/atlas-cpu-power" /usr/local/sbin/atlas-cpu-power
install -m 644 "$HERE/atlas-cpu-power.service" /etc/systemd/system/atlas-cpu-power.service
install -m 755 "$HERE/thermal-report.sh" /usr/local/bin/atlas-thermal-report

systemctl daemon-reload
systemctl enable atlas-cpu-power.service
systemctl start atlas-cpu-power.service
systemctl enable mbpfan.service
systemctl restart mbpfan.service

sleep 3
echo
echo "== Estado =="
systemctl is-active atlas-cpu-power.service mbpfan.service || true
echo
/usr/local/bin/atlas-thermal-report
