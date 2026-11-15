# Atlas Hub: servidor de servicios y acceso remoto (diseño)

Estado: **diseño cerrado con el usuario (2026-11-14; acceso remoto revisado el
2026-11-15)**. Nada se implementa hasta que el plan de implementación esté
aprobado. Decisiones abiertas marcadas como ⏸️.

## 1. Objetivo

Convertir `atlas-hub` (hoy solo el relay Atlas) en un servidor de servicios
propio — Nextcloud + Jellyfin — y poder usarlo desde fuera de casa, con Hub
Panel como resumen y Cockpit como consola de administración. Criterios fijos:
**escalable, fiable y seguro a nivel ciberseguridad**.

## 2. Estado actual (escaneo del 2026-11-13)

- Hardware: MacBookPro12,1 (2015), i5-5257U (2 núcleos/4 hilos), **sin VT-x**,
  8 GB RAM + 5,8 GB swap, **SSD Apple de 128 GB** (106 GB `/` ext4, 97 GB
  libres), conexión por **WiFi** (`wlp3s0`, 192.168.100.26/24).
- Sistema: Debian 13.7, kernel 6.12, systemd, **cgroup v2**, `ufw` **activo**.
- En ejecución: `atlas-relay.service`, `avahi-daemon`, `ssh`, `cron`,
  `bluetooth`, `wpa_supplicant`, `lid-screen`, etc. Escuchan 22, 8765 (relay),
  8767 (hefesto).
- Red: **IP pública real** (no CGNAT). El usuario **no quiere tocar el panel
  del router** ni abrir puertos; UPnP no responde en su router.
- Repos Debian: `podman` 5.4.2, `cockpit` 337, `cockpit-podman` (no instalados).
  `tailscale` no está en repos (cliente desde upstream).

## 3. Decisiones

| # | Decisión | Justificación |
|---|---|---|
| 1 | **Cockpit + Hub Panel** (no Portainer) | La terminal y el control de systemd los aporta Cockpit (maduro, PAM, auditado); no construimos terminal propia. Hub Panel = resumen + enlace. |
| 2 | **Tailscale gestionado** (revisado 2026-11-15) | Cero puertos abiertos y cero tocar el router (requisito del usuario). Trade-off aceptado: la topología/metadatos los ve Tailscale; el tráfico va cifrado de extremo a extremo. |
| 3 | **Podman rootless + Quadlet** | Sin daemon root (un escape no da root), integra nativo con Cockpit (unidades systemd), cgroup v2 ya activo, en repos Debian. |
| 4 | **Almacenamiento: todo en el SSD** | Sin disco externo por ahora (limitación aceptada: ~90 GB útiles). |
| 5 | **DERP: automático (Tailscale)** | Lo gestiona Tailscale; sin decisión propia. |
| 6 | **Backup: ⏸️ pendiente** | Sin copias por ahora. Opciones anotadas: disco externo en atlas-hub (recomendada) o nube cifrada (Borg). |
| 7 | **Integración Atlas: solo calendario → Nextcloud** | `calendar_read`/`calendar_write` solo cambian de backend CalDAV; archivos fuera de v1. |
| 8 | **Hub Panel: resumen + enlace a Cockpit** | Sin poderes de control propios (el control y la terminal viven en Cockpit). |
| 9 | **MagicDNS de Tailscale** | Nombres automáticos (`atlas-hub.<tailnet>.ts.net`); no hace falta DuckDNS ni dominio propio. |
| 10 | **DB Nextcloud: MariaDB** | Evita la migración SQLite→MariaDB posterior; estándar y escalable. |
| 11 | **Acceso a servicios: HTTP sobre el túnel** | El túnel WireGuard ya cifra el transporte; sin TLS propio en v1 (upgrade futuro con Caddy). |
| 12 | **Calendario: migrar eventos de iCloud** | Exportar `.ics` de iCloud e importar en Nextcloud antes de cambiar Atlas. |

## 4. Arquitectura

```
atlas-hub (Debian 13 · 8 GB · SSD 128 GB · WiFi)
│
├─ atlas-relay.service   → relay Atlas (8765)      — sin cambios de fondo
├─ hefesto (listener)    → mTLS (8767)             — sin cambios de fondo
├─ tailscaled            → cliente VPN (Tailscale) — NUEVO, sin puertos abiertos
├─ cockpit               → consola web (9090)      — NUEVO, solo LAN + tailnet
├─ podman rootless (usuario «services», sin login)
│   ├─ nextcloud  (Quadlet)   → archivos + CalDAV  — NUEVO
│   ├─ mariadb    (Quadlet)   → BD de Nextcloud    — NUEVO (no publicado)
│   └─ jellyfin   (Quadlet)   → media (direct play)— NUEVO
└─ ufw (activo)

MacBook / iPhone / iPad
└─ cliente Tailscale → red privada 100.64.0.0/10 (misma cuenta)
```

El relay y Hefesto no cambian de arquitectura: solo se amplía su perímetro. Los
servicios nuevos viven en contenedores rootless aislados; Tailscale, Cockpit y
Podman son piezas independientes (si una falla no tumba el relay).

## 5. Red y perímetro

Regla de oro: **no se abre ningún puerto a internet**. Tailscale atraviesa el
NAT sin puertos abiertos ni tocar el router. Todo lo demás queda accesible solo
desde LAN y desde la red Tailscale.

1. **Tailscale:** cliente `tailscaled` en `atlas-hub`, Mac e iPhone, misma
   cuenta. Sin puertos abiertos. Mantener el cliente **parcheado** (regla ya
   añadida a `AGENTS.md`).
2. **Relay y Hefesto:** el filtro `same_network_only` se amplía de
   `192.168.100.0/24` a **`192.168.100.0/24` + `100.64.0.0/10`**, y escuchan en
   la IP LAN **y** en la interfaz de Tailscale (nada de `0.0.0.0` ciego).
3. **Cockpit:** bind solo LAN + Tailscale; `ufw` permite 9090 solo desde esas
   dos redes.
4. **Nextcloud / Jellyfin:** se llega por la red Tailscale (MagicDNS) o LAN;
   nunca desde internet. Puertos mapeados en el host: Nextcloud 8080, Jellyfin
   8096; MariaDB no se publica.
5. **WiFi:** desactivar el *power-save* para estabilidad 24/7; si da problemas,
   adaptador Ethernet USB.

## 6. Plataforma de servicios (Podman rootless + Quadlet)

- Usuario de sistema **`services`** (sin login, home `/opt/services`), con
  `loginctl enable-linger services` para que los contenedores arranquen al boot.
  El usuario `atlas` del relay no se toca.
- **Quadlet units** (cada servicio = una unidad systemd, visible y controlable
  desde Cockpit):
  - `nextcloud` → `linuxserver/nextcloud` (bien mantenida).
  - `mariadb` → base de datos de Nextcloud (solo red interna del pod).
  - `jellyfin` → imagen oficial, **direct play** (sin transcodificar: el CPU no
    da para transcodificar).
- Volúmenes en el SSD: `/opt/services/nextcloud/{config,data}`,
  `/opt/services/mariadb/data`, `/opt/services/jellyfin/{config,cache}` y
  `/opt/media` para la librería.
- `cockpit-podman` → los contenedores se ven y gestionan desde Cockpit.
- Acceso: HTTP sobre el túnel Tailscale con MagicDNS (`nextcloud.atlas-hub`,
  `jellyfin.atlas-hub`). El navegador marcará «http»; el transporte va cifrado
  por WireGuard.

## 7. Integración Atlas ↔ Nextcloud (calendario)

- El relay sigue siendo el **único** componente que habla CalDAV; la app y el
  frontend no cambian.
- Se cambian las variables `CALDAV_*` del `.env` para apuntar a Nextcloud con
  una **contraseña de aplicación** de Nextcloud (nunca la contraseña de la
  cuenta). `CALDAV_EDIT_CALENDARS` se actualiza con los calendarios de
  Nextcloud.
- El relay alcanza Nextcloud por localhost (mismo servidor), sin salir a
  internet.
- **Migración:** exportar los eventos de iCloud (`.ics`) e importarlos en
  Nextcloud **antes** de cambiar Atlas. No borrar los datos de iCloud hasta
  verificar que la migración es correcta.
- Fuera del repo (opcional del usuario): apuntar el iPhone a Nextcloud CalDAV
  para unificar calendarios.

## 8. Hub Panel

- Tarjeta nueva **«Servicios»**: estado (activo/inactivo) de Nextcloud, Jellyfin
  y Tailscale, junto al relay. Fuente: el relay (que ya expone las unidades
  systemd en `/v1/system`; se añaden las nuevas).
- Botón **«Abrir Cockpit»** → `https://atlas-hub:9090`. Cockpit trae certificado
  autofirmado (aviso la primera vez; mejora futura con Let's Encrypt).
- Hub Panel **no** gana poderes de control: encender/apagar y terminal siguen
  en Cockpit.

## 9. Observabilidad y fiabilidad

- **Verlo todo:** Cockpit (métricas del host, logs del journal, terminal), las
  tarjetas del panel y el log del relay (ya existente).
- **Fiabilidad:** watchdog del relay (`atlas-health.timer`, ya existe), `linger`
  para los contenedores, WiFi con power-save desactivado.

## 10. Seguridad

- **No se abre ningún puerto a internet** (Tailscale atraviesa el NAT).
- Secretos fuera del repo: credenciales CalDAV y contraseña de aplicación de
  Nextcloud en `/opt/atlas/repo/.env` (dueño `atlas`, 600).
- Hub Panel sin control privilegiado; terminal solo vía Cockpit (PAM).
- El frontend nunca ve credenciales de proveedores ni de CalDAV (invariante ya
  existente).
- Tailscale: mantener `tailscaled` **parcheado y revisado** en cada
  mantenimiento (recogido en `AGENTS.md`).

## 11. Pendientes explícitos

- **Backup:** sin copias por ahora. Decidir destino (disco externo en
  atlas-hub recomendado, o nube cifrada) cuando se retome.
- **HTTPS para Nextcloud/Jellyfin:** Caddy + Let's Encrypt sobre la tailnet
  (upgrade posterior).
- **Media en disco externo:** cuando la librería crezca más allá del SSD.
- **Certificado de Cockpit:** sustituir el autofirmado por Let's Encrypt.

## 12. Fuera de alcance (v1)

- Acceso de Atlas a archivos de Nextcloud (leer/escribir).
- Transcodificación en Jellyfin.
- Otros servicios (fotos, RSS, gestor de contraseñas…): la plataforma los
  permitirá añadir después.
- Integración de Telegram con los servicios.

## 13. Riesgos y mitigaciones

| Riesgo | Mitigación |
|---|---|
| WiFi inestable en un servidor 24/7 | power-save off; Ethernet USB si persiste |
| Tailscale ve topología/metadatos | trade-off aceptado por el usuario; tráfico cifrado |
| SSD único sin backup | riesgo asumido y pendiente; decidir backup pronto |
| RAM (MariaDB+Nextcloud+Jellyfin ≈ 1 GB) | cabe en 8 GB; monitorizar en Cockpit |
| Migración de calendario | importar en Nextcloud y verificar antes de borrar iCloud |
| `tailscaled` fuera de repos Debian | instalar desde upstream y mantener parcheado |

## 14. Verificación (criterios de éxito)

1. Desde el iPhone fuera de casa (datos móviles): llega por Tailscale a
   Nextcloud/Jellyfin.
2. Nextcloud: subir/descargar archivo y crear un evento de calendario.
3. Jellyfin: reproducir un vídeo (direct play).
4. Atlas: `calendar_read` y `calendar_write` contra Nextcloud.
5. Hub Panel: tarjeta Servicios con los estados correctos y botón Cockpit.
6. Cockpit: terminal, gestión de unidades y estado de contenedores.
7. Relay: `/v1/health` ok tras todos los cambios.
