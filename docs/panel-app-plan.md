# ATLAS Panel — app macOS independiente (plan v1)

Estado: **plan aprobado, sin implementar** (2026-10-25). Sustituye al panel
web servido por el relay como *herramienta de control*, sin retirar `/panel/`
(sigue disponible para otros clientes de la LAN).

Objetivo: una app de escritorio **independiente de la app principal de ATLAS**
que permita ver el relay y **recuperarlo** (reinicio) aunque esté colgado, y
viceversa: si esta app falla, la principal sigue funcionando.

## 1. Decisiones fijadas

| Tema | Decisión |
|---|---|
| Tecnología | **Tauri v2** (Rust + React ya existente) |
| Formato | **App de barra de menús** (tray) + ventana al pulsar |
| UI | **Reutiliza `apps/panel`** (un solo frontend) |
| Canal de reinicio | **SSH con clave restringida** (comando forzado) |
| Red de seguridad | **Watchdog systemd en `atlas-hub`** (independiente del Mac) |
| Distribución | Local, firma **Apple Development**, sin App Store |
| Nombre / bundle | `ATLAS Panel` · `com.atlas.panel` |
| Avisos | **Notificación macOS** al caer el relay |
| Autoarranque | Al login, **off por defecto** |

## 2. Independencia de la app principal

- Bundle, proceso, icono, versión e identifier propios: no comparte runtime ni
  ciclo de actualización con la app principal.
- **Empaqueta su propio build** del panel dentro del bundle → no depende de que
  el relay sirva `/panel/`.
- Si el relay no responde: la app muestra estado "sin conexión" y ofrece el
  botón de reinicio (el caso de uso principal).
- La app principal no depende de esta; ambas solo comparten el relay (API HTTP).

## 3. Estructura de carpetas

```
apps/panel/          UI compartida React + Vite (se mantiene)
apps/panel-app/      shell Tauri v2 (nueva)
  src-tauri/
    tauri.conf.json  identifier com.atlas.panel · tray · ventana
    Cargo.toml
    src/main.rs      comandos HTTP, reinicio SSH, tray
    icons/
  package.json       script que builda la UI con PANEL_BASE=./ → dist
```

- `apps/panel/vite.config.ts`: base parametrizable por entorno
  (`PANEL_BASE`; defecto `/panel/` para el relay, `./` para la app).
- HTTP: **comandos Rust** (`health`, `snapshot`, `logs`, `session`, `stats`)
  con `reqwest` → **sin CORS** y sin tocar `RELAY_CORS_ORIGIN` del servidor.
- `restart_relay`: ejecuta un **único comando fijo** por SSH; el backend no
  acepta comandos arbitrarios desde la UI.
- Doble confirmación en la UI para reiniciar (regla ATLAS: sin permiso
  implícito).

## 4. Clave SSH restringida (corazón del reinicio)

- Clave `ed25519` **exclusiva de la app**, sin passphrase, permisos `600`,
  fuera del repo:
  `~/Library/Application Support/com.atlas.panel/keys/panel_ed25519`
- Entrada en `~adria_admin/.ssh/authorized_keys` de `atlas-hub` con comando
  forzado y sin reenvíos:

```
command="sudo systemctl restart atlas-relay && sudo systemctl is-active atlas-relay",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding ssh-ed25519 AAAA... atlas-panel
```

- Esa clave **solo** puede reiniciar `atlas-relay` (y consultar su estado);
  el sudoers mínimo `atlas-deploy` ya permite esas dos operaciones sin
  contraseña.
- Rotación: eliminar la línea y regenerar la clave.

## 5. Watchdog en el servidor (independiente del Mac)

- `deploy/server/atlas-healthcheck.py` (solo stdlib) +
  `atlas-health.service` (oneshot, root) + `atlas-health.timer` (60 s).
- Comprueba `http://<IP LAN>:8765/v1/health`; tras **2 fallos seguidos**
  reinicia `atlas-relay`.
- Funciona con el Mac apagado: cubre el caso "no hay nadie mirando".

## 6. Comportamiento de la app

- **Tray**: icono con semáforo (ok / warn / down) y menú con estado, "Abrir
  panel", "Reiniciar relay…", "Salir".
- **Ventana**: mismas tarjetas del panel (servicios, puertos, Mac, relay,
  peticiones, logs) + banner de conexión.
- **Notificaciones**: una por caída (sin spam), con enlace a la ventana.
- **Autoarranque**: toggle con `tauri-plugin-autostart`, apagado por defecto.

## 7. Fases

| Fase | Contenido |
|---|---|
| F0 | Este plan + docs |
| F1 | Scaffold Tauri, build de UI con `PANEL_BASE=./`, comandos HTTP |
| F2 | Tray + polling de salud + ventana |
| F3 | Clave restringida + reinicio con doble confirmación (comandos de instalación para `atlas-hub`) |
| F4 | Notificaciones + autoarranque |
| F5 | Watchdog systemd + despliegue |
| F6 | `tauri build` (Apple Development), firma profunda y prueba de caída real |

## 8. Pruebas

- Dev: `npm run tauri dev`; verificación visual con el usuario.
- Producción: `npm run tauri build` y prueba de la app generada.
- **Simulación de fallo**: `sudo systemctl stop atlas-relay` en `atlas-hub` →
  la app debe mostrarlo y recuperarlo desde la UI.
- Independencia: cerrar/eliminar una app y comprobar que la otra sigue.
- Verificar que no se exponen secretos ni rutas privadas en logs.

## 9. Riesgos y mitigaciones

- App con capacidad de reiniciar un servicio remoto → tratarla como
  privilegiada: clave restringida, comando forzado, confirmación explícita.
- Doble bundle y doble flujo de actualización → aceptado por diseño
  (independencia); versionado separado.
- UI duplicada → se evita reutilizando `apps/panel` con un build distinto.
