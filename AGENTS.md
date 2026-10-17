# Instrucciones de trabajo

- Lee en  `docs/index.md` y lee las documentaciones necesarias

## Servidores de desarrollo (mantener levantados mientras se trabaja)

- Frontend: `npm run dev` → http://localhost:5173 (recarga sola con HMR).
- Relay Python: **corre en el servidor remoto** `homelab` (`adria@192.168.100.26`,
  Debian 13), no en local. Dirección: `http://192.168.100.26:8765` (núcleo lo
  drena vía `ATLAS_RELAY_URL`; el frontal web en dev lo sondea directo con CORS
  solo para ese origen). El `.env` se carga automáticamente al iniciar.
- En `homelab` el relay es el servicio systemd de usuario `atlas-relay.service`
  (con linger activo: arranca con el sistema y sobrevive a cierres de sesión).
  Logs: `journalctl --user -u atlas-relay -f` (desde `homelab`).
- **Sincronizar y desplegar el relay** (desde la raíz del repo del Mac):
  `./scripts/deploy-relay.sh` → commitea cambios pendientes, hace `git push`,
  baja los cambios en `homelab`, reinicia el servicio y comprueba `/v1/health`.
  El relay **no** recarga solo: cada despliegue lo reinicia el script.
- El relay solo escucha en la IP LAN de `homelab` (`192.168.100.26:8765`, no en
  `127.0.0.1`); el firewall `ufw` de Debian debe dejar pasar el 8765/tcp.
- Acceso SSH al servidor: clave `~/.ssh/atlas_linux` del Mac (usuario `adria`).
- Nunca commitees `.env` (ignorado por Git): lleva tokens y API keys reales.
  Para pasarlo a `homelab`: `scp -i ~/.ssh/atlas_linux .env adria@192.168.100.26:ATLAS/.env`.
- Tauri no lee `.env` solo: para `tauri dev`/`build` con claves,
  `set -a; . ./.env; set +a` antes (las pruebas Rust igual).
- iCloud CalDAV: prohibido martillear. Cada prueba contra iCloud (una sola
  petición de lectura o más) requiere permiso explícito del usuario, con pausa
  entre intentos. Apple limita (throttle 403) ante ráfagas.
