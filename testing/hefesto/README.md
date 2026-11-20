# Consolas de prueba de Hefesto

Herramientas manuales para hablar con el listener mTLS de Hefesto
(`https://atlas-hub.local:8767`) sin pasar por la app. El código se versiona
aquí; las credenciales **no**.

## Credenciales

Los dos scripts leen los certificados de `$HEFESTO_HOME`:

- Por defecto: `~/Library/Application Support/atlas-hefesto-test/`
- Contenido: `ca.pem`, `device.pem`/`device.key`,
  `controller.pem`/`controller.key`, `server.pem`/`server.key` y `peers.json`
  (huellas SHA-256 de `device` y `controller`).
- Permisos: directorio `700`, claves `600`.

Los certificados actuales se generaron el 2026-10-28 y son los que `atlas-hub`
tiene registrados. Si se pierden hay que regenerarlos **y** actualizar el relay.

## Uso

```bash
./hefesto-echo "hola"        # registra el dispositivo, pide eco y devuelve el texto
./hefesto-echo --status      # estado del dispositivo (generación, lease, suspensión)
./hefesto-control status     # estado visto desde el certificado controller
./hefesto-control resume|abort
./hefesto-control submit "texto"
```

Variables:

- `HEFESTO_HOME`: directorio de credenciales.
- `HEFESTO_URL`: por defecto `https://atlas-hub.local:8767`. Los scripts
  resuelven IPv4 porque mDNS anuncia primero un IPv6 link-local donde el relay
  no escucha, pero conservan el nombre para SNI y verificación.

## Regenerar credenciales

```bash
./generate-certs.sh           # crea CA + device/controller/server en $HEFESTO_HOME
./generate-certs.sh --force   # sobrescribe (invalida lo que atlas-hub confía)
```

Después hay que desplegar en `atlas-hub` los nuevos `ca.pem`, `server.pem`,
`server.key` y `peers.json` (certificados en `/opt/atlas/hefesto`; el relay los
lee de `HEFESTO_TLS_CA`, `HEFESTO_TLS_CERT`, `HEFESTO_TLS_KEY` y
`HEFESTO_PEERS_FILE`) y reiniciar `atlas-relay`. Los pasos completos están en
`docs/hefesto-plan.md` §8.

Regenerar sin desplegar deja la app y estas consolas sin poder conectar. El
`server.ext` incluye la IP LAN `192.168.100.26`; si la del hub cambia,
actualízala antes de regenerar (el nombre sigue funcionando por DNS).
