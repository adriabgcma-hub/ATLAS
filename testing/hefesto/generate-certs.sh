#!/usr/bin/env bash
# Genera las credenciales de prueba mTLS de Hefesto fuera del repo.
# Directorio de salida: $HEFESTO_HOME (por defecto:
# ~/Library/Application Support/atlas-hefesto-test), con permisos 700/600.
set -euo pipefail

CONF_DIR="$(cd "$(dirname "$0")/certs.d" && pwd)"
OUT_DIR="${HEFESTO_HOME:-$HOME/Library/Application Support/atlas-hefesto-test}"
DEVICE_ID="${HEFESTO_DEVICE_ID:-mac-one}"
FORCE=0

if [ "${1:-}" = "--force" ]; then
  FORCE=1
fi

if [ -e "$OUT_DIR/ca.key" ] && [ "$FORCE" -ne 1 ]; then
  echo "error: ya hay credenciales en $OUT_DIR" >&2
  echo "       usa --force solo si vas a desplegar las nuevas en atlas-hub" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
chmod 700 "$OUT_DIR"
cd "$OUT_DIR"
umask 077

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout ca.key -out ca.pem -config "$CONF_DIR/ca.cnf" >/dev/null

for role in device controller server; do
  openssl req -new -newkey rsa:2048 -nodes \
    -keyout "$role.key" -out "$role.csr" -config "$CONF_DIR/$role.cnf" >/dev/null
  openssl x509 -req -in "$role.csr" \
    -CA ca.pem -CAkey ca.key -CAcreateserial \
    -days 365 -extfile "$CONF_DIR/$role.ext" -out "$role.pem" >/dev/null
done

rm -f ./*.csr ./*.srl
chmod 600 ./*.key
chmod 644 ./*.pem

fingerprint() {
  openssl x509 -in "$1.pem" -noout -fingerprint -sha256 \
    | sed 's/^.*=//' | tr -d ':' | tr 'A-F' 'a-f'
}

cat > peers.json <<EOF
{
  "$(fingerprint device)": {"role": "device", "deviceId": "$DEVICE_ID"},
  "$(fingerprint controller)": {"role": "controller", "deviceId": "$DEVICE_ID"}
}
EOF
chmod 644 peers.json

echo "credenciales generadas en $OUT_DIR"
echo "huella device: $(fingerprint device)"
echo "recuerda desplegar ca.pem, server.pem, server.key y peers.json en atlas-hub"
