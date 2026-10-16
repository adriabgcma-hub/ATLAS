# Instrucciones de trabajo

- Lee en  `docs/index.md` y lee las documentaciones necesarias

## Servidores de desarrollo (mantener levantados mientras se trabaja)

- Frontend: `npm run dev` → http://localhost:5173 (recarga sola con HMR).
- Relay Python (desde la raíz del repo, con el `.env` real cargado):
  `set -a; . ./.env; set +a; RELAY_HOST=127.0.0.1 RELAY_PORT=8765 RELAY_CORS_ORIGIN=http://localhost:5173 python3 server/atlas_relay.py`
  → http://127.0.0.1:8765 (núcleo lo drena vía `ATLAS_RELAY_URL`; el frontal
  web en dev lo sondea directo con CORS solo para ese origen).
- El relay **no** recarga solo: reinícialo (`pkill -f "server/atlas_relay.py"` y
  arrancar de nuevo) cada vez que cambies algo en `server/`.
- Nunca commitees `.env` (ignorado por Git): lleva tokens y API keys reales.
- Tauri no lee `.env` solo: para `tauri dev`/`build` con claves,
  `set -a; . ./.env; set +a` antes (el relay y las pruebas Rust igual).
- iCloud CalDAV: prohibido martillear. Cada prueba contra iCloud (una sola
  petición de lectura o más) requiere permiso explícito del usuario, con pausa
  entre intentos. Apple limita (throttle 403) ante ráfagas.
