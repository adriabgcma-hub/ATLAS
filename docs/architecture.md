# Arquitectura de Atlas

## Objetivo

Atlas es una aplicación de escritorio local, inspirada en Jarvis, con una interfaz visual persistente y una capa de inteligencia artificial capaz de coordinar conversaciones, memoria y herramientas bajo autorización explícita del usuario.

La interfaz actual es deliberadamente visual: escena WebGL, orbe central y órbitas. La IA, el audio y las herramientas se incorporarán por capas, sin acoplarlas a la escena.

## Principios

1. **Local primero**: la ventana, el estado visual, la configuración y los permisos viven en el dispositivo.
2. **Capacidades explícitas**: la IA no puede ejecutar una herramienta que no esté registrada y autorizada.
3. **Efectos confirmables**: leer datos y modificar datos son operaciones distintas; las acciones sensibles requieren confirmación.
4. **Proveedores intercambiables**: el motor no depende de un único proveedor de modelos.
5. **UI desacoplada**: React renderiza la experiencia; Rust coordina persistencia, permisos, procesos y acceso nativo.
6. **Trazabilidad**: cada petición, llamada a herramienta y resultado debe tener un identificador correlacionable.

## Capas

```text
┌──────────────────────────────────────────────┐
│ React + Three.js                              │
│ escena, orbe, chat, panel de permisos         │
└──────────────────────┬───────────────────────┘
                       │ eventos / comandos IPC
┌──────────────────────▼───────────────────────┐
│ Rust / Tauri                                  │
│ sesión, router, permisos, persistencia, logs  │
└───────────────┬──────────────────┬───────────┘
                │                  │
┌───────────────▼──────────┐ ┌─────▼───────────┐
│ Runtime de IA             │ │ Registro tools  │
│ modelo, contexto, stream  │ │ adaptadores     │
└───────────────┬──────────┘ └─────┬───────────┘
                │                  │
        ┌───────▼──────────────────▼───────┐
        │ proveedores externos y OS         │
        │ APIs, archivos, calendario, etc.  │
        └───────────────────────────────────┘
```

### Frontend

El frontend contiene la escena y las vistas de interacción. Debe consumir eventos tipados y no conocer claves API, tokens ni detalles de proveedores. La escena puede reflejar estados como `idle`, `thinking`, `executing` y `error`, pero no decide qué herramienta ejecutar.

### Núcleo Rust

El núcleo será la autoridad para:

- ciclo de vida de una sesión;
- validación de comandos IPC;
- permisos y confirmaciones;
- selección de proveedor;
- persistencia local;
- registro de auditoría sin secretos;
- cancelación y límites de tiempo.

### Runtime de IA

El runtime recibe un contexto acotado y devuelve texto, eventos de razonamiento operativo o solicitudes de herramienta. No debe ejecutar efectos directamente: siempre pasa por el registro de herramientas y el motor de permisos.

### Herramientas

Cada herramienta declara identidad, versión, esquema de entrada, esquema de salida, nivel de riesgo y requisitos de permiso. Los adaptadores externos se mantienen aislados para poder sustituirlos o desactivarlos.

### Servidor Python: el cerebro (Fase 3)

El relay es el dueño único de sesiones, historial, Groq y Telegram, con la
app abierta o cerrada:

- Consulta el bot por polling saliente y responde por `sendMessage` directo
  (24/7 aunque la app esté cerrada). Sin webhooks: nunca se expone a internet.
- Llama a Groq con 3 API keys rotativas (solo entorno local, nunca repo ni UI).
- Un worker único procesa en FIFO estricto; sesiones persistidas en disco.
- Sirve eventos por HTTP solo en red local (`/v1/events`): la app drena y
  presenta cuando está abierta. El núcleo Rust es transporte (contratos + HTTP),
  sin estado propio.

### Hefesto: base aislada del especialista del Mac

La primera entrega es solo eco, no control del Mac. `server/hefesto.py` separa el
journal SQLite y el protocolo mTLS del chat/FIFO/eventos existentes. El listener es
opcional y comparte proceso con el relay; el HTTP de Atlas no le delega trabajo.
`crates/atlas-mac` contiene contratos estrictos, cliente HTTPS y ejecutor de eco con
política local, identidad de instancia, caducidad, deduplicación y suspensión.

La biblioteca todavía no está enlazada a Tauri: no hay bucle nativo, nuevo proceso,
control de apps/archivos/terminal ni cambios en UI. La integración del ciclo de vida,
el aborto local y los controles app/panel preceden a las herramientas reales. Después
se añadirá delegación no bloqueante; no se amplía la espera del worker FIFO actual.
Detalle y puertas de validación en [el plan de Hefesto](./hefesto-plan.md).

### Hub Panel: app macOS independiente

`apps/panel-app` es una app Tauri v2 **separada** de la principal (bundle
`Hub Panel`, identifier `com.atlas.panel`, versión propia). No comparte proceso
ni ciclo de vida: si una app falla, la otra sigue.

- **UI compartida**: reutiliza `apps/panel` con otro build (base `./`), de modo
  que `/panel/` y la app muestran las mismas tarjetas.
- **Barra de menús**: icono con semáforo que sondea `/v1/health` cada 5 s;
  menú con estado, abrir panel, reiniciar relay y autoarranque al login.
- **Ventana**: tarjetas del panel, **creada bajo demanda** al abrir o al pulsar
  el icono y **destruida** al cerrarla (Cmd+W oculta sin salir de la app). En
  reposo solo queda la bandeja (~16 MB de huella). Con la ventana visible la app
  es `Regular` (Dock, Cmd+Tab, menú) y al cerrarla pasa a `Accessory`.
- **Reinicio**: SSH con clave `ed25519` restringida a un comando forzado de
  `systemctl restart atlas-relay`; doble confirmación en la UI.
- **Aviso**: notificación macOS una vez por caída del relay.
- **Red de seguridad**: el watchdog `atlas-health.timer` del servidor reinicia
  el relay aunque no haya nadie delante del Mac.

Detalle de fases y operación en `panel-app-plan.md` y `relay.md` §8.

## Flujo de una petición

1. El usuario escribe o activa una entrada.
2. Atlas crea un `request_id` y registra una sesión mínima.
3. El router construye el contexto permitido.
4. El proveedor de IA responde con texto o una solicitud de herramienta.
5. El registro valida la herramienta y sus argumentos.
6. El motor decide si puede ejecutarse automáticamente o necesita confirmación.
7. Se ejecuta, cancela o rechaza la operación.
8. El resultado vuelve al modelo y a la interfaz.
9. La sesión se cierra con estado y auditoría.

## Estado de implementación

- **Activo**: ventana Tauri transparente, escena WebGL, orbe y órbitas, barra de
  input, panel de conversación, cerebro Python 24/7 (sesiones, Groq con rotación,
  Telegram con respuesta directa), IPC de eventos, tools de calendario con
  confirmaciones, Hub Panel (barra de menús, semáforo, reinicio por SSH,
  notificaciones, autoarranque) y watchdog del relay en `atlas-hub`.
- **Planificado**: memoria y nuevas herramientas. El registro de herramientas
  y `calendar_read`/`calendar_write` ya están implementados (`docs/tools.md`).
- **Streaming**: revelado progresivo implementado en el frontend (cadencia
  18 ms), no en el servidor.
- **No activar por defecto**: micrófono, escucha permanente, wake word, TTS y agentes autónomos.

