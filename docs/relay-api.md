# API del cerebro (relay Python)

Frontera HTTP entre el cerebro (`server/atlas_relay.py`) y sus clientes
(app Tauri vía núcleo Rust, frontal web en dev). Todo JSON, todo **solo en
la red local**: fuera de loopback o del mismo /24 responde `403
same_network_only`. Sin autenticación adicional: la red local ES el perímetro.

Base por defecto en clientes: `http://atlas-hub.local:8765` (ver
`ATLAS_RELAY_URL`). El relay **no escucha en loopback**: se enlaza a la IP LAN
de `atlas-hub` (autodetectada) y el acceso se hace por su nombre `.local` o
por la IP LAN del momento. `127.0.0.1` solo aparece en ejemplos de caída local.

Transporte: **HTTP/1.1 con keep-alive** (el servidor cierra conexiones ociosas
tras 15 s). Todas las respuestas llevan `Content-Length`; el `204` de CORS no
tiene cuerpo. El panel sondea cada 2 s, así que reutiliza la conexión y evita
pagar un TCP nuevo (y el intento IPv6 del nombre `.local`) en cada petición.

## Eventos y tipos

Los eventos usan los mismos nombres y formas que `contracts.md`:

- `session.state_changed` → `{requestId, sessionId, timestamp, state, message?}`
- `conversation.message_appended` → `{requestId, sessionId, timestamp, role, content, source}`

En `/v1/events` cada evento lleva además `seq` (cursor monótono) y `kind`
(`message` | `state` | `confirmation`).

## Endpoints

### `GET /v1/health`

```json
{"ok": true, "telegram": "enabled|disabled", "llm": "groq|stub",
 "calendar": "ok|standby|disabled", "events": 5, "queue": 0, "inflight": 0,
 "bot": {"id": 123, "username": "atlas_bot", "first_name": "Atlas"}}
```

`queue` e `inflight` son peticiones en cola/en vuelo; `bot` solo aparece con
`TELEGRAM_BOT_TOKEN` configurado (identidad `getMe`, se captura al arrancar).

### `GET /v1/session`

```json
{"session": {"sessionId": "ses-…", "state": "completed", "messages": […] } | null,
 "cursor": 5}
```

`cursor` es el `seq` vivo: el cliente arranca ahí para no re-jugar historia.

### `GET /v1/events?since=N`

```json
{"events": [{"seq": 6, "kind": "message", …}]}
```

`since < 0` → `400 bad_since`.

### `GET /v1/logs?tail=N`

Log detallado del relay (respuestas, tokens, tools, interacciones, Telegram).

```json
{"lines": ["2026-… [PROCESS] start request=…", …], "total": 1234}
```

`tail` 1..10000 (defecto 100). Archivo: `temp/atlas-relay/relay.log`.

### `GET /v1/stats`

Consumo acumulado de tokens (Groq).

```json
{"token_usage": {"prompt": 1234, "completion": 567, "total": 1801,
  "calls": 5, "by_model": {"openai/gpt-oss-120b": {"prompt": …, "completion": …, "total": …, "calls": …}}}}
```

El log detallado se escribe en `temp/atlas-relay/relay.log` (categorías:
RECEIVE, PROCESS, LLM, TOOL, COMPLETE, CANCEL, CONFIRM, TELEGRAM, LOCAL_REPLY,
PANEL).

### `GET /v1/system`

Snapshot best-effort del host atlas-hub (con caché de 3 s), fuente del panel:

```json
{"generated_at": "…",
 "units": [{"unit": "ssh.service", "description": "OpenBSD Secure Shell server",
            "load": "loaded", "active": "active", "sub": "running",
            "since": "Fri 2026-…", "pid": 123, "mem_bytes": 143000000,
            "actionable": false},
           {"unit": "atlas-relay.service", …}],
 "ports": [{"port": 8765, "address": "192.168.100.26", "service": "atlas-relay"}],
 "resources": {"mem_used_pct": 37, "disk_root": {"total": 111654973440, "free": 94074470400},
               "uptime_s": 16271, "load_1": 2.7, "temp_c": 70.0,
               "battery": {"cap_pct": 42, "status": "Full"}, "ac_connected": true},
 "actions": {"verbs": ["start", "stop", "restart"],
             "system_units": ["avahi-daemon.service", …]}}
```

Nada de esto es de la conversación: es observación del host (subprocesos con
timeout; un fallo de fuente degrada a `null`/vacío sin matar el endpoint). La
IP LAN del ejemplo es ilustrativa (puede cambiar): en URLs y clientes se usa
siempre el nombre, `atlas-hub.local`.

### `POST /v1/system/asks`

Pide una **acción** y devuelve su op-id (doble confirmación del panel). Body:

```json
{"kind": "service", "unit": "avahi-daemon.service", "verb": "restart"}
```

- `kind: "service"`: unidad systemd de sistema en la lista blanca
  (`PANEL_ACTION_UNITS`, p. ej. `atlas-relay.service` algún día, cuando haya
  reglas polkit para `atlas` en atlas-hub); verb ∈ `start|stop|restart`.
  Fuera de lista → `403 action_not_allowed`.
- `kind: "cancel_all"`: cancela TODAS las peticiones en vuelo del cerebro.

Respuesta: `{"ok": true, "op": "op-…", "kind": "…", "summary": "systemctl restart docker.service"}`
El op-id caduca en 30 s y vale una sola vez.

### `POST /v1/system/resolve`

Body: `{"op": "op-…", "decision": "allow_once"|"deny"}`.

- `deny` → `{"ok": true, "executed": false, "denied": true}`.
- `allow_once` → `{"ok": true, "executed": true, "result": {"ok": true, "rc": 0, …}}`.
- op inexistente/caducada → `404 expired_or_unknown` (se consume al resolver).

Todo queda en `relay.log` con categoría `PANEL` (asks/deny/execute).

### `POST /v1/system/cancel_all`

Atajo sin doble confirmación (es lógica interna del cerebro):
`{"ok": true, "cancelled": N}`. El panel prefieres el flujo asks/resolve con
`kind: "cancel_all"` para que salga la tarjeta ámbar.

### `GET /panel/*`

Estáticos del panel (build Vite de `apps/panel/dist`; base `/panel/`).
`GET /panel/` sirve `index.html`. 404 JSON si no hay build. Solo LAN.

### `POST /v1/send`

Body: `{"content": "…", "source": "local"|"telegram"}` (defecto `local`).

```json
{"ok": true, "requestId": "req-…", "sessionId": "ses-…"}
```

Errores: `400 empty_message` (vacío), `400 bad_source`, `400 bad_body`.

Comandos de chat (respuesta local, sin Groq). Se reconoce el primer token sin
distinguir mayúsculas; cualquier texto que empiece por `/` se trata como
comando y **no** llega al modelo. Un comando desconocido responde con el aviso
y la ayuda.

| Comando | Alias | Respuesta |
| --- | --- | --- |
| `/clear` | `/limpiar` | Reinicia la conversación (sesión nueva y vacía). |
| `/help` | `/ayuda` | Lista de comandos y flujo de permisos. |
| `/log` | `/registro` | Últimas 5 interacciones y consumo de tokens. |

`/log` ordena las interacciones de la más reciente a la más antigua. Por cada
una muestra `requestId`, canal, hora local, mensaje del usuario, tokens
consumidos (`prompt`/`completion`/`total`/`llamadas`) y resultado
(`completada`, `cancelada`, `error (…)`, `local`). Al final, el consumo total,
el desglose por modelo, la sesión activa y el estado de cola/vuelo. El uso por
petición vive en memoria y se poda (no se persiste): tras reiniciar, las
interacciones siguen apareciendo desde `sessions.json`, pero sin desglose de
tokens. `/help` incluye el flujo de confirmaciones y la lista de calendarios
editables. Los comandos valen por ambos canales y no consumen cuota.

### `POST /v1/cancel`

Body: `{"requestId": "req-…"}` → `{"ok": true, "cancelled": true|false}`.
Idempotente: una petición desconocida devuelve `ok` sin efectos.
Interrumpe también una confirmación en espera (equivale a denegar).

### `POST /v1/confirm`

Body: `{"requestId": "req-…", "decision": "allow_once"|"deny"}` →
`{"ok": true, "accepted": true|false}` (`false` también si caducó o no existe).
`decision` inválida → `400 bad_decision`.

### Eventos de confirmación

En `/v1/events` con `kind: "confirmation"`:

```json
{"seq": 8, "kind": "confirmation", "type": "permission.confirmation_required",
 "requestId": "req-…", "sessionId": "ses-…", "timestamp": "…",
 "toolId": "calendar_write", "summary": "Crear «Dentista» en Deberes",
 "risk": "write", "expiresAt": "…"}
```

En `/v1/events` con `kind: "confirmation_resolved"` (cuando el usuario
confirma o deniega, por botón o por texto):
```json
{"seq": 9, "kind": "confirmation_resolved", "type": "permission.resolved",
  "requestId": "req-…", "sessionId": "ses-…", "timestamp": "…",
  "decision": "allow_once"|"deny"}
```

## Herramientas (function calling)

El cerebro ofrece el catálogo de `docs/tools.md` al modelo en cada turno
(máx. 4 turnos). Flujo: `tool_calls` → ejecutar lo legible / pedir
confirmación lo sensible → respuestas `role: tool` → turno final. El tiempo
de la confirmación no consume el plazo de la operación (30 s + espera).

### `GET /v1/calendar/calendars`

```json
{"calendars": [{"id": "/…/", "name": "Casa"}]}
```

Sin credenciales → `501 calendar_not_configured`; fallo iCloud → `502 calendar_upstream`.

### `GET /v1/calendar/upcoming?days=N`

```json
{"events": [{"uid": "…", "summary": "Dentista", "start": "…", "end": "…",
             "status": "", "location": "", "calendar": "Casa"}]}
```

`days` 1..31 (defecto 7); fuera de rango → `400 bad_days`. Solo iCloud.
Nota: el cliente CalDAV usa PROPFIND + GET (no REPORT) para evitar el
throttle de Apple. Las lecturas son rápidas (~10 s para todos los
calendarios) y se cachean 120 s en el relay.

## Variables de entorno (nombres; valores solo en `.env` local)

- `TELEGRAM_BOT_TOKEN`, `TELEGRAM_ALLOWED_USER_IDS`
- `GROQ_API_KEY_1..3`, `GROQ_MODEL` (defecto `openai/gpt-oss-120b`),
  `GROQ_TEMPERATURE` (defecto `0.7`, rango `0..2`)
- `RELAY_HOST` (defecto IP LAN), `RELAY_PORT` (defecto `8765`),
  `RELAY_DATA_DIR` (defecto `temp/atlas-relay`, dentro del repo),
  `RELAY_CORS_ORIGIN` (vacío = sin CORS; en dev admite lista de orígenes
  separada por comas: sólo se devuelve el Origin que pide si está en la lista)
- `PANEL_*` (panel, ver `server/system_monitor.py`): `PANEL_SYSTEM_UNITS`,
  `PANEL_ACTION_UNITS` (lista blanca de acciones systemd; vacía por defecto
  en atlas-hub hasta instalar reglas polkit para `atlas`), `PANEL_DIST_DIR`
  (build del panel `apps/panel/dist`)
- `CALDAV_USERNAME` (tu Apple ID), `CALDAV_APP_PASSWORD` (contraseña de app)
- `RELAY_TIMEZONE` (defecto `Europe/Madrid`; la hora del contexto sale de aquí,
  nunca de APIs del Mac, para poder correr el servidor donde sea)

## Sistema y contexto (Groq)

Cada llamada lleva dos mensajes `system` + historial (últimos 6):

1. **Fijo** (estable entre llamadas → aprovecha caché de prefijo): identidad de
   Atlas, idioma del usuario, tono cercano sin tutear, concisión, brevedad en
   Telegram. Sin system prompt no hay nada de esto: pedirlo fue decisión explícita.
2. **Variable** (separado para no romper la caché): fecha/hora desde
   `RELAY_TIMEZONE` + canal actual (`Telegram` | `aplicación`).

La agenda solo entra vía tools (decisión explícita): el calendario está
conectado al relay, pero el LLM no lo ve hasta que haya herramientas.

## Límites y notas

- Contenido por mensaje: 1..4000 caracteres; cuerpo HTTP máx. 64 KB.
- Cola de eventos: 500 (lo viejo se descarta); historial por sesión: 100
  (a Groq viajan los últimos 6).
- Los eventos no sobreviven al reinicio; sesiones y offset de Telegram sí.
- Groq con 3 claves rotativas (401/403/429/5xx/transporte → siguiente).
  Incidencia conocida: Cloudflare rechaza el UA de urllib → UA propio.
- Telegram por polling saliente con backoff (5 s→60 s); respuestas por
  `sendMessage` directo, también con la app cerrada.
