# API del cerebro (relay Python)

Frontera HTTP entre el cerebro (`server/atlas_relay.py`) y sus clientes
(app Tauri vía núcleo Rust, frontal web en dev). Todo JSON, todo **solo en
la red local**: fuera de loopback o del mismo /24 responde `403
same_network_only`. Sin autenticación adicional: la red local ES el perímetro.

Base por defecto: `http://127.0.0.1:8765` (ver `ATLAS_RELAY_URL`).

## Eventos y tipos

Los eventos usan los mismos nombres y formas que `contracts.md`:

- `session.state_changed` → `{requestId, sessionId, timestamp, state, message?}`
- `conversation.message_appended` → `{requestId, sessionId, timestamp, role, content, source}`

En `/v1/events` cada evento lleva además `seq` (cursor monótono) y `kind`
(`message` | `state` | `confirmation`).

## Endpoints

### `GET /v1/health`

```json
{"ok": true, "telegram": "enabled|disabled", "llm": "groq|stub", "events": 5}
```

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

### `POST /v1/send`

Body: `{"content": "…", "source": "local"|"telegram"}` (defecto `local`).

```json
{"ok": true, "requestId": "req-…", "sessionId": "ses-…"}
```

Errores: `400 empty_message` (vacío), `400 bad_source`, `400 bad_body`.

Comandos de chat (respuesta directa sin Groq): `/clear` (sesión nueva y
vacía) y `/help` (ayuda). Valen por ambos canales.

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
  `RELAY_CORS_ORIGIN` (vacío = sin CORS; solo dev web)
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
