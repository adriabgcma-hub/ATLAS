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
(`message` | `state`).

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

### `POST /v1/send`

Body: `{"content": "…", "source": "local"|"telegram"}` (defecto `local`).

```json
{"ok": true, "requestId": "req-…", "sessionId": "ses-…"}
```

Errores: `400 empty_message` (vacío), `400 bad_source`, `400 bad_body`.

### `POST /v1/cancel`

Body: `{"requestId": "req-…"}` → `{"ok": true, "cancelled": true|false}`.
Idempotente: una petición desconocida devuelve `ok` sin efectos.

## Variables de entorno (nombres; valores solo en `.env` local)

- `TELEGRAM_BOT_TOKEN`, `TELEGRAM_ALLOWED_USER_IDS`
- `GROQ_API_KEY_1..3`, `GROQ_MODEL` (defecto `openai/gpt-oss-120b`),
  `GROQ_TEMPERATURE` (defecto `0.7`, rango `0..2`)
- `RELAY_HOST` (defecto IP LAN), `RELAY_PORT` (defecto `8765`),
  `RELAY_DATA_DIR` (defecto `temp/atlas-relay`, dentro del repo),
  `RELAY_CORS_ORIGIN` (vacío = sin CORS; solo dev web)

## Límites y notas

- Contenido por mensaje: 1..4000 caracteres; cuerpo HTTP máx. 64 KB.
- Cola de eventos: 500 (lo viejo se descarta); historial por sesión: 100
  (a Groq viajan los últimos 6).
- Los eventos no sobreviven al reinicio; sesiones y offset de Telegram sí.
- Groq con 3 claves rotativas (401/403/429/5xx/transporte → siguiente).
  Incidencia conocida: Cloudflare rechaza el UA de urllib → UA propio.
- Telegram por polling saliente con backoff (5 s→60 s); respuestas por
  `sendMessage` directo, también con la app cerrada.
