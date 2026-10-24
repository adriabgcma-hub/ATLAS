# Instrucciones de trabajo

- Lee en  `docs/index.md` y lee las documentaciones necesarias

## Servidores de desarrollo (mantener levantados mientras se trabaja)

- Frontend: `npm run dev` → http://localhost:5173 (recarga sola con HMR).
- Relay Python: **corre en el servidor remoto** `atlas-hub` (`atlas-hub.local`,
  Debian 13; la IP puede cambiar: usa SIEMPRE el nombre, no la IP), no en
  local. Dirección: `http://atlas-hub.local:8765` (núcleo lo drena vía
  `ATLAS_RELAY_URL`; el frontal web en dev lo sondea directo con CORS solo
  para ese origen). El `.env` se carga automáticamente al iniciar.
- En `atlas-hub` el relay es el servicio systemd **de sistema**
  `atlas-relay.service` (`/etc/systemd/system/`, fuente versionada en
  `deploy/server/atlas-relay.service`): usuario de sistema `atlas` (home
  `/opt/atlas`, sin login ni sudo), repo en `/opt/atlas/repo`. Logs:
  `journalctl -u atlas-relay -f` (en `atlas-hub`).
- **Sincronizar y desplegar el relay** (desde la raíz del repo del Mac):
  `./scripts/deploy-relay.sh` → sube cambios commiteados, hace pull en
  `/opt/atlas/repo` como `atlas` (`sudo -u atlas`), reinicia el servicio
  (`sudo systemctl restart`) y comprueba `/v1/health`. El relay **no** recarga
  solo: cada despliegue lo reinicia el script.
  **Importante:** el script NO commitea automáticamente. Commitea manualmente
  antes de desplegar. Si hay cambios sin commitear, el script aborta.
- El relay solo escucha en la IP LAN de `atlas-hub` (8765, autodetectada, no
  en `127.0.0.1`); el firewall `ufw` de `atlas-hub` debe dejar pasar
  `8765/tcp` SOLO desde `192.168.100.0/24`.
- Acceso SSH al servidor: alias `atlas-hub` del Mac (usuario administrador
  `adria_admin`, solo clave, `~/.ssh/id_ed25519`). El usuario del relay
  (`atlas`) no tiene login: el deploy usa el sudo sin contraseña mínimo de
  `deploy/server/atlas-deploy.sudoers` (pull como `atlas` + restart/is-active/
  status del servicio; ningún otro comando).
- Nunca commitees `.env` (ignorado por Git): lleva tokens y API keys reales.
  Para pasarlo a `atlas-hub` (queda en `/opt/atlas/repo/.env`, dueño `atlas`,
  modo 600): `scp .env adria_admin@atlas-hub:/tmp/atlas-env && ssh -t
  adria_admin@atlas-hub 'sudo install -m 600 -o atlas -g atlas /tmp/atlas-env
  /opt/atlas/repo/.env && rm -f /tmp/atlas-env'`.
- Tauri no lee `.env` solo: para `tauri dev`/`build` con claves,
  `set -a; . ./.env; set +a` antes (las pruebas Rust igual).
- iCloud CalDAV: prohibido martillear. Cada prueba contra iCloud (una sola
  petición de lectura o más) requiere permiso explícito del usuario, con pausa
  entre intentos. Apple limita (throttle 403) ante ráfagas.

## Reglas de construcción y pruebas

### Antes de cambiar código
- Lee el código y la documentación directamente relacionados con el cambio.
- Ejecuta `git status --short` y conserva los cambios ajenos.
- Usa `rg` para localizar usos antes de retirar una API, comando IPC, dependencia o contrato.
- No recorras ni reestructures todo el repositorio si el cambio no lo requiere.

### Arquitectura
- React + Three.js → presentación, interacción, estados visuales.
- Rust + Tauri → capacidades nativas, IPC, permisos, persistencia, coordinación.
- Contratos tipados para eventos, comandos, errores y estados compartidos.
- Proveedores de IA detrás de interfaces reemplazables.
- Cada herramienta/integración en un adaptador aislado y desactivable.
- El frontend nunca conoce claves, tokens ni detalles internos de proveedores.

### Seguridad y permisos
- Nunca guardar secretos/tokens/contraseñas/claves en el repositorio.
- Validar entradas antes de ejecutar herramientas; normalizar respuestas externas.
- Confirmación explícita para escritura, comunicación externa, ejecución de comandos, cambios de configuración y borrado.
- La falta de respuesta nunca significa autorización.
- Cancelación, timeouts y estados de error en operaciones lentas o con efectos.
- No exponer secretos, rutas privadas, datos personales ni contenido sensible en logs/respuestas.

### TypeScript y Rust
- TS estricto; no rebajar configuración del compilador.
- Rust modular; errores estructurados vía IPC.
- Evitar `unwrap`/`expect` en rutas controladas por el usuario o dependientes del entorno.
- Versionar eventos públicos, payloads y transiciones de estado.
- Preferir interfaces pequeñas, funciones acotadas, dependencias explícitas.
- Evitar estado global mutable si una sesión o módulo puede ser su propietario.

### Pruebas
- Ejecuta, cuando estén disponibles: `npm run build`, `npm run lint`, `cargo check --workspace`.
- Valida comandos IPC con entradas válidas, inválidas, vacías y canceladas.
- Comprueba manualmente los estados visuales afectados por un cambio de frontend.
- Verifica cancelación, errores y permisos en cualquier flujo que ejecute acciones.
- Si cambia el bundle de macOS, regenera con la identidad Apple Development y verifica firma profunda.
- Simula proveedores de IA y herramientas externas; no dependas de APIs reales para pruebas unitarias.
- No consideres terminado un cambio solo porque compile: debe respetar contratos y documentación.
- **Testing con el usuario:** lanza `npm run dev` y prueba contra http://localhost:5173 junto con el usuario.
- **Verificación en producción:** después de validar en dev, ejecuta `npm run tauri build` y prueba la aplicación generada.
- **Si el cambio afecta a `server/`:** despliega el relay con `./scripts/deploy-relay.sh` y verifica `/v1/health`.
- **Registro obligatorio:** documenta todo lo que se hace (pruebas ejecutadas, resultados, despliegues) en los documentos pertinentes.

### Documentación
- Actualiza la documentación cuando cambien arquitectura, contratos, permisos o comportamiento visible.
- Añade implementaciones terminadas a `docs/implemented.md` con el nombre exacto del commit.
- Mantén `docs/implementation-plan.md` alineado con el estado real.
- **Registro de trabajo:** cada cambio realizado (pruebas, despliegues, decisiones) debe quedar documentado en los documentos pertinentes. Este registro es importante y debe mantenerse actualizado.

### Commits y publicación
- Crea un commit únicamente cuando el usuario dé permiso explícito.
- Cuando se conceda, publica inmediatamente en el remoto de GitHub configurado.
- No hagas push, reescribas el historial ni cambies de rama sin permiso explícito.
- Si el cambio afecta a `server/`, despliega con `./scripts/deploy-relay.sh`.
