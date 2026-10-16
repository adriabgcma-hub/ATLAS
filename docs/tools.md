# Catálogo de herramientas de Atlas

Las herramientas viven en el cerebro (`server/atlas_relay.py`) y el modelo las
descubre por function calling de Groq. Riesgo y confirmación según
`contracts.md`: leer es automático; escribir, externo o destructivo requiere
confirmación explícita en UI (la ausencia de respuesta nunca autoriza; caduca
en 5 minutos).

## calendar_read · v1

- **Riesgo:** `read` — ejecución automática, sin confirmación.
- **Entrada:** `{days?: 1..31 (defecto 7), calendar?: nombre (opcional)}`.
- **Salida:** texto compacto (una línea por evento con uid corto) o
  `sin eventos en ese rango` / `calendario no disponible (…)`.
- **Notas:** lee todos los calendarios iCloud visibles.

## calendar_write · v1

- **Riesgo:** `write` — siempre con confirmación (`permission.confirmation_required`
  con resumen en lenguaje natural; `allow_once` | `deny` por `/v1/confirm`).
- **Entrada:** `{action: create|update|delete, calendar, uid?, summary?, start?,
  end?, description?, location?}`. Fechas `YYYY-MM-DD` (todo el día) o
  `YYYY-MM-DDTHH:MM` (se interpretan en `RELAY_TIMEZONE`, se guardan en UTC).
- **Solo editables** (comparación insensible a mayúsculas/espacios;
  configurable por `CALDAV_EDIT_CALENDARS`, defecto
  `Exámenes,Celebraciones,Horario,Deberes`): cualquier otro calendario se
  rechaza en el servidor aunque el usuario haya permitido. Resto: solo lectura.
- **Salida:** texto de resultado (`creado …`, `modificado …`, `borrado …`) o el
  motivo (`calendario no editable`, `evento no encontrado`, `fecha inválida`,
  `calendario: …`).
- **Notas:** `update`/`delete` usan `uid` (lo devuelve `calendar_read`);
  `update` mezcla campos sobre el evento existente (UID intacto); textos con
  escape RFC 5545.
