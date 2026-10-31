# Especificación: control de apps de Hefesto (puerta 7, corte 1)

- Fecha: 2026-10-31
- Estado: diseño aprobado por el usuario en sesión de brainstorming; pendiente de revisión del spec y del plan de implementación
- Fuente: `docs/hefesto-plan.md` §4 (matriz de consentimiento), §6 (puertas 5 y 7) y las decisiones de esta sesión.

## 1. Contexto y objetivo

Hasta ahora Hefesto (la puerta 4) lee y escribe archivos dentro de raíces autorizadas.
Esta puerta abre el frente de **accesibilidad**: que Hefesto pueda actuar sobre las
**aplicaciones** del Mac del usuario. Es un subsistema nuevo (el runtime del dispositivo
hoy solo maneja archivos) y **greenfield**, no una modificación acotada.

Este primer corte se limita a **abrir, cerrar (suave) y listar apps**. Deja explícitamente
fuera el teclado sintético y la gestión de ventanas (mover/redimensionar/traer), que son
los siguientes slices de la puerta 7 y tienen su propio riesgo y spec.

Objetivo: que el LLM pueda lanzar apps instaladas del usuario, cerrarlas de forma elegante
y enumerar qué hay disponible o en ejecución, con eficiencia de rendimiento y de tokens,
sin abrir acceso a shell, sin ejecutar binarios sueltos y sin API privada que ponga en
riesgo la firma/notarización del bundle.

## 2. Decisiones de diseño (alcance acordado)

1. **Universo de apps:** solo apps **instaladas**, resueltas por nombre o `bundleId`.
   Nada de rutas arbitrarias del modelo ni de ejecutar binarios sueltos. Habrá una
   herramienta para **listar disponibles y en ejecución** para que el modelo acierte el nombre.
2. **Cierre:** en esta versión **solo cierre suave** (`terminate`), sin cierre forzado y
   sin descartar cambios ni aceptar diálogos de guardado. El forzado queda para un corte
   posterior con consentimiento individual e identidad revalidada.
3. **Consentimiento:** las tres acciones son **trivial por adaptador** (`requiresConfirmation:
   False`), sin tarjeta, tal y como las clasifica el plan para «multimedia reversible, abrir
   apps» y «cierre normal de app».
4. **Visibilidad:** tras un `open`/`close` con éxito el relay emite una **burbuja violeta
   HEFESTO** (`agent: hefesto`) con el resultado, reutilizando `_hefesto_note()`. `list` es
   lectura: devuelve el resultado síncrono al modelo **sin burbuja**, igual que las lecturas
   de archivos (lo redacta Atlas en su turno).
5. **Mecanismo:** opción **A** (NSWorkspace en proceso vía `objc2`), por rendimiento y
   porque no engendra subprocess. La política de procesos del repo es no sobrecargar el Mac
   ni dejar huérfanos; la opción A no lanza ningún proceso, así que queda de fábrica.

## 3. Requisitos

### 3.1 Protocolo y wire

1. Subir `protocolVersion` a **4** en los tres espejos (`docs/contracts.md`,
   `crates/atlas-core`, `src/types/contracts.ts`) y en fixtures wire.
2. Un `toolId` nuevo en el dispositivo: `hefesto.apps` con payload
   `AppsPayload { action: "open"|"close"|"list", name?: string, bundleId?: string,
   query?: string }`.
3. Salida tipada `AppsOutput`:
   - `list` → `{ apps: [{name, bundleId, running}] }` **cortada** (techo ≤30).
   - `open`/`close` → texto corto de estado (p. ej. nombre resuelto y resultado).
4. El relay expone **una sola** herramienta al modelo: `hefesto_apps`, con `action` enum +
   `name`/`bundleId`/`query` y `requiresConfirmation: False`. Una herramienta con `action`,
   en vez de tres, reduce tokens del catálogo que viajan en cada petición.

### 3.2 Resolución híbrida de apps (dispositivo)

5. **En ejecución:** fuente de verdad `NSWorkspace.runningApplications` (respaldada por
   LaunchServices), exacta y barata. Cubre la parte principal de `list` y localiza una app
   viva antes de `terminate()`.
6. **Instaladas y nombre→app:** un **mapa cacheado** `nombre/bundleId → URL` construido con
   un escaneo único de `/Applications`, `/System/Applications` y `~/Applications`,
   parseando `Info.plist` (CFBundleName/CFBundleDisplayName/CFBundleIdentifier). Es el
   lookup público que LaunchServices no ofrece; se rellena de forma perezosa y se cachea
   en el dispositivo.
7. **Abrir:** resolver `name` contra el mapa (o usar `bundleId` directo) y lanzar con
   `NSWorkspace.openApplicationAtURL` (que usa LaunchServices internamente para lanzar).
8. **Sin API privada** (`_LSCopyAllApplicationURLs`, `LSApplicationWorkspace`) y **sin lista
   de binarios**: se mantiene la definición de «adaptador de opciones cerradas».

### 3.3 Concurrencia y límites

9. AppKit exige hilo principal para `openApplicationAtURL` y `terminate()`. El `executor`
   (que corre en el hilo del runtime) **no** llama la capa `unsafe` directamente: la
   reenvía al hilo principal y espera el resultado por un canal con timeout. El ejecutor no
   bloquea y AppKit corre en su hilo. Como `crates/atlas-mac` no conoce el `AppHandle` de
   Tauri, el reenvío se hace vía una **interfaz de inyección** (un callback/canal que el
   cableado de `src-tauri` conecta a `run_on_main_thread`), no llamando Tauri desde el
   ejecutor. Cómo se cablea ese puente lo decide el plan.
10. `list` prioriza en‑ejecución + coincidencias de `query` y corta a ≤30 entradas, para no
    volcar al modelo las ~300 apps instaladas ni gastar tokens. Ninguna ruta de usuario al
    proveedor salvo el nombre resuelto.

### 3.4 Flujo de datos

11. El modelo emite `hefesto_apps {action:"open", name:"Spotify"}` → relay, sin tarjeta,
    **delega** `hefesto.apps` al dispositivo con `invocationId` y `taskId` nuevos (igual que
    escritura). El turno del modelo espera el `outcome` síncrono.
12. Dispositivo: enruta por `toolId`, reenvía al hilo principal, resuelve y ejecuta, y
    devuelve `outcome` con `AppsOutput`.
13. Relay: para `open`/`close` con éxito → `_hefesto_note()` emite la burbuja violeta
    («Abierta Spotify» / «Cerrada Mail»); el texto del `outcome` vuelve al modelo y Atlas
    redacta la respuesta normal (etiqueta ATLAS). Para `list` → solo devuelve al modelo,
    sin burbuja.

### 3.5 Manejo de errores (texto estable en el turno, sin lanzar nada)

14. **App no encontrada / sin resolver:** «no encuentro «X» entre las apps instaladas». No abre.
15. **Fallo al abrir:** «no se pudo abrir «X»».
16. **Cierre elegante no confirmado:** tras `terminate()` se vigilan unos segundos; si sigue
    viva → ««X» sigue abierta (¿tiene cambios sin guardar?); no la fuerzo». Si desaparece →
    burbuja «Cerrada X».
17. **Timeout / dispositivo sin respuesta:** caducidad → «Hefesto no respondió a tiempo». Si
    el dispositivo se reinició con el encargo en vuelo → `unknown`, **sin reintentar la
    acción** (idempotencia por `invocationId`, como `file_info`).
18. **Args malformados:** `action` fuera de `open|close|list`, o `open`/`close` sin `name` ni
    `bundleId` → error de validación en el relay **antes** de delegar (no se submit).
19. **Cancelación a mitad de turno:** revalidar el estado cancelado antes de delegar (patrón
    ya corregido en escritura); si canceló, no se ejecuta la acción.

### 3.6 Persistencia y privacidad

20. **Journal** (`hefesto.py`): `hefesto.apps` es operación **sin contenido sensible**; no
    persiste datos de usuario, solo `toolId`, estado e identificadores (mismo patrón que
    `file_info`). No hay contenido que anonimizar.

## 4. Fuera de alcance (cortes posteriores de la puerta 7)

- Comandos de teclado (texto/atajos sintéticos vía CGEvent): slice propio, riesgo alto.
- Mover / redimensionar / traer ventanas (AXUIElement): slice propio.
- Cierre forzado (`forceTerminate`) y matar procesos del usuario: consentimiento individual
  con identidad revalidada, nunca trivial.
- Capturas de pantalla y modelo de visión.
- Concesión interactiva del permiso TCC de Accesibilidad (hace falta para teclado/ventanas;
  este corte de apps no lo requiere).

## 5. Pruebas

5.1 **Dispositivo (Rust, `crates/atlas-mac`)**
- Unitarias **puras** de la lógica de `apps.rs` (sin AppKit): resolver `name`→entrada del
  mapa (exacto, insensible a mayúsculas, variantes de acentos), fallo estable cuando no está,
  y filtrado/techo del `list` (≤30, prioridad en‑ejecución + `query`). El mapa se inyecta
  como datos de prueba; no se escanea el disco en unit tests.
- **Wire:** `AppsPayload`/`AppsOutput` (serialización, `action` inválido, `bundleId` vs `name`,
  `query` opcional), espejo de las pruebas v3.
- **Capa `unsafe` NSWorkspace:** fuera de unit tests (necesita main‑thread/entorno gráfico);
  se ejercita en prueba en vivo.
- **Despacho:** el `executor` enruta `hefesto.apps` por `toolId` sin tocar eco/lectura/escritura.

5.2 **Relay (Python, `test_hefesto_*`)**
- Catálogo expone `hefesto_apps` con `requiresConfirmation: False`.
- `open`/`close` con éxito → **una** burbuja `agent:hefesto` (`_hefesto_note`) con el texto del
  `outcome`; `list` → resultado síncrono **sin** burbuja.
- Validación: `action` fuera de vocabulario, o `open`/`close` sin `name`/`bundleId` → error de
  validación antes de delegar (no se submit).
- Idempotencia por `invocationId`; sin reejecución tras reinicio (`unknown`); cancelación a
  mitad no delega.
- Journal: `hefesto.apps` no persiste contenido (solo toolId/estado/ids), como `file_info`.

5.3 **Frontend (vitest)**
- `phaseOrbits`/`HEFESTO_PHASES`: órbita violeta para `hefesto_apps` (etiqueta tipo
  «Hefesto · Gestiona apps»).
- La burbuja HEFESTO queda cubierta por `ConversationPanel.test.tsx` (no hace falta test nuevo
  si el `agent:hefesto` ya se renderiza violeta).

5.4 **Contratos**
- Bump a v4 en los tres espejos con `hefesto.apps` + `AppsOutput`, y pruebas de fixture wire.

5.5 **Prueba en vivo (con el usuario, tras desplegar)**
- «abre <app inocua>» → burbuja «Abierta …»; «lista las apps en ejecución» → salida cortada sin
  burbuja; «cierra <app>» → burbuja «Cerrada …» o «sigue abierta (cambios), no la fuerzo»;
  `name`/ruta inexistente → error estable sin abrir; comprobar que no quedan procesos ni RAM
  añadida al cerrar (regla del Mac).

5.6 **Umbral de aceptación**
- `cargo test --workspace`, clippy `-D warnings` y `cargo fmt`,
  `python3 -m unittest discover` (nuevos casos), `npm test` + `npm run build`, y la matriz en
  vivo verde.

## 6. Riesgos y puntos de vigilancia

- **Hilo principal de AppKit:** el puente «reenviar al hilo principal + esperar por canal»
  (con `atlas-mac` desacoplado de Tauri, §3.3.9) es el punto más delicado; una mala
  interacción puede colgar el encargo. Se valida explícito en vivo.
- **Cache del mapa de instaladas:** obsolescencia si se instalan/desinstalan apps durante la
  sesión. Estrategia: caducidad perezosa o invalidación a demanda; se documenta en el plan.
- **Detección de cierre elegante:** `terminate()` es asincrónico; el «¿sigue viva?» necesita
  una pequeña espera acotada para no reportar mal un cierre pendiente de diálogo.
- **Firma/TCC:** este corte no pide Accesibilidad, pero sí mantiene la regla de no usar API
  privada; la revisión debe confirmar que solo se emplean llamadas públicas de
  NSWorkspace/LaunchServices/`Info.plist`.
