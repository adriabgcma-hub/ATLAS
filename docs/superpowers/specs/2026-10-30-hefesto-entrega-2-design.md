# Especificación: cierre de la entrega 2 de Hefesto (atajo, Hub Panel y matriz de fallos)

- Fecha: 2026-10-30
- Estado: diseño aprobado por el usuario; pendiente de revisión y plan de implementación
- Fuente: `docs/hefesto-plan.md` §6 (entregas 2 pendientes) y decisiones de la sesión de diseño

## 1. Contexto y objetivo

Hefesto vive en la app de Atlas (indicador, kill switch, arranque por intención,
suspensión persistente) y el relay delega encargos por intención (entrega 3,
cerrada). La entrega 2 quedó con tres pendientes que este diseño cierra:

1. **Atajo local**: el atajo global `cmd+shift+a` existe pero no está acabado ni validado.
2. **Control desde Hub Panel**: el panel hoy solo vigila y reinicia el relay; no puede actuar sobre Hefesto.
3. **Matriz de fallos**: la puerta de validación (salida normal, crash, pérdida de red, suspensión, reinicios) que el plan exige antes de activar capacidades reales.

Objetivo: que el usuario pueda **suspender Hefesto desde el Hub Panel** (no solo
desde la app), que el **atajo** quede acabado y testeado, y que la **matriz de
fallos** se ejecute y quede registrada. Sin acciones reales sobre el Mac (sigue
siendo solo eco); la reactivación de Hefesto sigue siendo **manual y en la app**.

## 2. Requisitos

### 2.1 Atajo local

- `cmd+shift+a` trae Atlas al frente desde cualquier app; pulsado de nuevo, lo oculta.
- Funciona con la ventana oculta (la ventana principal persiste oculta, no se destruye) y no recrea el webview.
- La decisión mostrar/ocultar está extraída a una función Rust pura y testeable.
- El atajo entra en la matriz de fallos (sigue operativo tras crash y reinicio del relay).

### 2.2 Control desde Hub Panel

- El Hub Panel muestra el estado de Hefesto (Activo / Suspendido / Sin configurar).
- Un botón **Suspender** en el panel suspende Hefesto de verdad (el dispositivo del Mac deja de ejecutar).
- La suspensión remota deja el mismo rastro que la local: marcador
  `hefesto-suspended` en el directorio de datos de la app y dispositivo invalidado en el journal.
- La reactivación sigue siendo **manual y en la app** (no se reactiva desde el panel).
- La latencia de la suspensión remota está acotada (~5 s, el latido del dispositivo).

### 2.3 Matriz de fallos

Cinco escenarios, cada uno con criterio de aceptación, ejecutados en vivo con el
usuario y registrados en `docs/hefesto-plan.md` §8:

| Escenario | Comportamiento esperado |
| --- | --- |
| Salida normal (Cmd+Q) | sin procesos huérfanos ni encargos colgados; la app vuelve a arrancar limpia |
| Crash (kill -9) | al relanzar reconecta; encargos pendientes resueltos como caducados/`unknown` |
| Pérdida de red (relay abajo) | el dispositivo reintenta sin bloquearse; al volver, reconecta solo |
| Suspensión (app y Hub Panel) | marcador escrito, encargo en vuelo cancelado, nada se ejecuta |
| Reinicio (relay y Mac) | recuperación del journal sin reejecutar nada |

## 3. Diseño

### 3.1 Atajo local

- Mantener el plugin `tauri-plugin-global-shortcut` con `command+shift+a`.
- Extraer del handler la decisión a `fn next_visibility(visible: bool, focused: bool) -> VisibilityAction` (Show/Hide) en `src-tauri/src/lib.rs` (o módulo propio), con test unitario de las tres ramas (visible+foco → ocultar; visible sin foco → enfocar; oculto → mostrar).
- Sin cambio de contrato ni de UI.

### 3.2 Control desde Hub Panel

Cadena de control remoto (el panel no obtiene certificados nuevos; usa el relay):

1. **Journal (protocolo v1)**: nueva operación `status`, rol controller, lectura pura:
   `POST /v1/hefesto/status {protocolVersion: 1}` →
   `{protocolVersion: 1, generation: int, suspended: bool}`.
   - `_FIELDS` gana `"status": set()` (sin campos obligatorios).
   - RBAC: `status` se añade a la lista controller (`submit`, `outcome`, `status`).
     `control` queda como está (ya lo puede enviar cualquier peer autenticado, incluido el controller).
   - `_operate`: rama `status` que devuelve `generation` y `suspended` del dispositivo del peer, sin mutar.
2. **Relay**: nuevos endpoints HTTP (documentados en `docs/relay.md`):
   - `GET /v1/hefesto/status` → `{configured: bool, deviceId: str|null, suspended: bool|null}`.
   - `POST /v1/hefesto/suspend` → lee `status` del journal (para la `generation` actual),
     envía `control {action: "abort", expectedGeneration}` y devuelve
     `{ok: true, suspended: true}`. Si el listener Hefesto no está configurado,
     `{ok: false, error: "hefesto_disabled"}`. Ante `stale_generation`, reintenta una vez.
   - La suspensión solo invalida (no reactiva): no hay endpoint de `resume` remoto.
3. **Dispositivo (observación de suspensión remota)**: en el bucle `hefesto_loop`
   (`src-tauri/src/lib.rs`), cuando una respuesta de estado trae `suspended: true`
   y el dispositivo se cree activo (sin aborto pendiente), se autosuspende:
   escribe el marcador `hefesto-suspended` y ejecuta el aborto local (igual que
   `hefesto_abort`). El campo `suspended` ya viaja en `Status` (wire.rs:305); solo falta observarlo.
4. **Hub Panel**: nueva tarjeta/tarjeta ampliada "Hefesto" que consume
   `GET /v1/hefesto/status` (sondeo, misma cadencia que el resto del panel) y un
   botón **Suspender** que llama a `POST /v1/hefesto/suspend`. Sin reinicio automático ni `resume`.

Trade-off asumido: la suspensión remota es asíncrona (~5 s) y unifica local/Hub
Panel/remoto en el mecanismo del journal, sin certificados nuevos en el panel.

### 3.3 Matriz de fallos

- Activar/documentar las dos pruebas reales ignoradas del panel
  (`consulta_health_del_relay_real`, `reinicia_el_relay_real` en
  `apps/panel-app/src-tauri/src/main.rs`) y, para las que requieren permisos o
  reinicios reales, pedir permiso antes (AGENTS.md).
- Ejecutar los cinco escenarios en vivo con el usuario (como en la T10), cada uno
  con su checklist y registro en `docs/hefesto-plan.md` §8. Al terminar, auditoría
  de procesos (`ps`/`pgrep`/`launchctl list`) y reporte de qué queda vivo.

## 4. Contratos y documentación

- `docs/contracts.md`: sección Hefesto v1 gana la operación `status` (controller, lectura pura).
- `docs/hefesto-plan.md`: §5 (tabla de operaciones) y §8 (registro) actualizados.
- `docs/relay.md`: documentar `GET /v1/hefesto/status` y `POST /v1/hefesto/suspend`.
- Los contratos tipados Rust/TS (`agent`/`taskId`) **no cambian**: no hay mensajes ni eventos nuevos.

## 5. Fuera de alcance

- Reactivación remota de Hefesto (sigue manual y en la app).
- Acciones reales sobre el Mac (archivos, apps, terminal, accesibilidad) — entrega 4, con matriz de consentimiento.
- `/cancel` como comando y automatizaciones del servidor.
- Ventana de tablas (aparcada).
