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
  Telegram con respuesta directa) e IPC de eventos.
- **Planificado**: memoria y registro de herramientas. Streaming progresivo
  implementado (visual, en frontend, cadencia 18 ms).
- **No activar por defecto**: micrófono, escucha permanente, wake word, TTS y agentes autónomos.

