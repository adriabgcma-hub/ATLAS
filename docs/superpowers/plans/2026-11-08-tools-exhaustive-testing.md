# Plan de testing exhaustivo de las herramientas de Atlas

> **Estado:** en ejecución desde 2026-11-08. Petición del usuario: probar de
> forma exhaustiva, en segundo plano, todas las tools; al terminar, presentar
> un plan con lo que falló o podría ser mejor (incluida la propuesta de quitar
> límites y confirmaciones innecesarios) para decidirlo juntos.

**Objetivo:** cubrir las 14 herramientas (2 de calendario en el relay + 12 de
Hefesto por protocolo v9) en cuatro capas de profundidad creciente, recopilar
evidencias (fallos, latencias, fricciones, tarjetas, redacciones de log) y
producir una propuesta de cambios con datos.

## 1. Inventario de herramientas

| Tool | Tarjeta | Riesgo | Efectos reales |
| --- | --- | --- | --- |
| `calendar_read` | no | read | iCloud (lectura) |
| `calendar_write` | sí | write | iCloud (escribe) |
| `hefesto.echo` | no | external | ninguno |
| `hefesto.list_dir` | sí | external | ninguno (lista) |
| `hefesto.read_file` | sí | external | ninguno (lee ≤16 KiB) |
| `hefesto.write_file` | sí | write | crea/sobrescribe fichero |
| `hefesto.rename` | sí | write | mueve fichero |
| `hefesto.apps` (open/close/list) | no | external | abre/cierra apps |
| `hefesto.keyboard` (type/combo) | sí | external | teclea en app activa |
| `hefesto.media` (8 acciones) | sí | external | volumen/brillo/reproducción |
| `hefesto.windows` (list/move/resize/focus) | list no, efectos sí | external | mueve ventanas |
| `hefesto.system` (os/disk/uptime/memory) | no | external | ninguno |
| `hefesto.terminal` | sí | destructive | ejecuta argv |
| `hefesto.capture` | sí | external | captura pantalla + OCR local |
| `hefesto.file_info` (interna, no expuesta) | n/a | n/a | ninguno |

Estado actual: relay v9+ (`ee71d55`, con `/cancel`) en atlas-hub sano
(`/v1/health` OK, calendario `standby`); árbol limpio; `Atlas.app` corriendo
(PID 33074) y **ya registrada como dispositivo Hefesto** por el fallback de
`~/Library/Application Support/com.atlas.next/hefesto/` (`mac-one`, online,
generación 1184, sin suspender). No hace falta relanzar la app: la capa 3 se
prueba contra la instancia viva.

## 2. Capas de testing

### Capa 1 — Estática (sin red, sin efectos; se lanza ya en segundo plano)

- `python3 -m unittest discover -s server -p "test_*.py"` — 256 tests en 5
  ficheros (calendar, hefesto, delegación, integración mTLS, log).
- `cargo test --workspace` (207 + 2 ignoradas), `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo fmt --all -- --check`,
  `cargo check --workspace`.
- `npm test`, `npm run build`, `npm run lint` (frontend + panel).

Salida esperada: registro por suite con nº de tests, fallos y tiempo.

### Capa 2 — HTTP del relay en vivo (atlas-hub, sin efectos locales)

- `GET /v1/health`, `/v1/session`, `/v1/events?since=N`, `/v1/logs?tail=N`,
  `/v1/stats`, `/v1/system` (contratos y límites).
- Comandos locales por `POST /v1/send`: `/help`, `/log`, `/clear` (no
  destructivo: borra el contexto de una sesión de prueba nueva), `/cancel`
  con una petición real en vuelo (hay que enviar algo lento y cancelar a
  mitad; verifica que el turno muere y no ejecuta herramientas).
- Flujo completo `send → thinking → confirmation → confirmation_resolved →
  executing → responding → completed` con una tool de cada riesgo.
- Calendario en vivo **solo con permiso explícito** (ver §4): lecturas con
  pausa entre intentos y ciclo create→update→delete sobre un calendario
  editable con limpieza al final.

### Capa 3 — Dispositivo Hefesto en vivo (contra la app ya en marcha, PID 33074)

Matriz por tool (todas las acciones), disparada por HTTP sin tocar la UI
(`POST /v1/send` + `POST /v1/confirm allow_once` + seguimiento por
`/v1/events`), como en las matrices en vivo anteriores:

1. `echo` — delegación completa con continuador violeta.
2. `list_dir` — raíz autorizada; ruta fuera de raíces rechazada sin tarjeta;
   enlace que escapa rechazado.
3. `read_file` — fichero de prueba; binario rechazado; >16 KiB rechazado.
4. `write_file` — create (contenido exacto, hash corto en resumen);
   overwrite con hash de la tarjeta; hash obsoleto rechazado sin pisar;
   create sobre existente falla.
5. `rename` — mover dentro de raíces; destino existente rechazado; fuera de
   raíces rechazado.
6. `apps` — open TextEdit (proceso real), list (≤30, en ejecución primero,
   filtro query), close suave (proceso desaparece), app no instalada →
   texto estable sin burbuja, close de app no abierta → «no está abierta».
7. `keyboard` — type en fichero de TextEdit abierto desde disco (verificar
   en disco, sin duplicados); combo cmd+s (el fichero guarda); texto con
   controles rechazado; sin permiso de Accesibilidad → texto estable (solo
   si TCC no está concedido).
8. `media` — volume_up 2 (volumen real verificado con osascript y
   **restaurado**), volume_down, mute (alterna), brightness_up/down
   (restaurar), play_pause/next/previous con Music (si no molesta, sin
   música abierta: solo verificar texto estable); count 0/11/«2» → bad_count
   (capa 4).
9. `windows` — list de TextEdit; move a (100,150) verificado con bounds
   reales; resize a 900×700 verificado; focus desde otra app (frontmost
   real); título ambiguo → error estable sin mover.
10. `system` — os/disk/uptime/memory: sin tarjeta, sin burbuja, salida
    coincide con los binarios reales, sin procesos vivos al terminar.
11. `terminal` — `/bin/echo hola` (tarjeta, salida al modelo), comando con
    salida larga acotada, timeout (sleep 30) con SIGKILL del grupo y sin
    huérfanos, intérprete denegado (/bin/sh, /usr/bin/python3) rechazado,
    ruta fuera de las seis raíces rechazada.
12. `capture` — tarjeta con riesgo `external`; si el TCC de Grabación de
    pantalla sigue sin conceder: verificar el texto estable de permiso; si
    está concedido: PNG en `~/Documents/Atlas/capturas` y OCR del texto.
13. `file_info` — interna: no expuesta al modelo y sin tarjeta.

Verificaciones transversales: órbita violeta por fase, burbuja `agent:
hefesto` solo donde toca (escrituras, mover, apps open/close), tarjeta
etiquetada HEFESTO, tiempos (TTL 15 s, espera síncrona del outcome),
redacción en el log del relay (sin texto de teclado ni títulos de ventanas
ni argumentos de terminal).

### Capa 4 — Matriz de validación mTLS (certificado `controller`, sin UI)

Por cada tool: `action` fuera de vocabulario → `bad_action`; clave extra o
tipo incorrecto → `bad_argument`; valores fuera de rango (count 0/11,
index 0, width 99, x 20001, texto >4096 B, path >1024 B, protocolVersion
8) → rechazo; **cero filas** creadas por entradas inválidas; tarjeta solo
para invocaciones válidas; journal con claves redactadas
(`textSha256`, `commandSha256`, sin títulos ni contenido).

## 3. Contención de efectos

- Ficheros de prueba solo en `~/Documents/Atlas-pruebas/` (carpeta nueva,
  borrada al final salvo que el usuario quiera inspeccionar algo).
- Teclado solo sobre un TextEdit abierto desde fichero de prueba, nunca en
  la app en primer plano del usuario; antes de teclear, traer TextEdit al
  frente (focus es parte de la prueba de windows).
- Volumen y brillo: registrar valor inicial con `osascript` y restaurarlo.
- Apps: solo TextEdit (+ Music si existe); nada del trabajo del usuario.
- Procesos: matar por PID propio, auditoría `ps`/`pgrep`/`launchctl` al
  terminar cada capa; sin dev servers, sin huérfanos.
- iCloud: pausa entre peticiones; nunca ráfagas.

## 4. Permisos que se piden al usuario

1. **iCloud:** autorizar las pruebas de calendario en vivo (lecturas
   espaciadas y ciclo create→update→delete en un calendario editable con
   limpieza), o limitarlas a solo lectura, o saltarlas.
2. ~~Atlas.app~~ (ya no aplica: la app viva está registrada como
   dispositivo Hefesto; no se relanza nada).

## 5. Fricciones candidatas a medir (entrada para la propuesta final)

- `hefesto.media`: la política del plan dice «multimedia reversible:
  trivial por adaptador», pero hoy exige tarjeta en cada pulsación.
- `list_dir`/`read_file`: tarjeta en **cada** lectura; candidato a
  consentimiento por raíz o por sesión.
- TTL de invocación 15 s y caducidad de confirmación 5 min: medir si
  ahogan turnos reales.
- Topes: lectura/escritura 16 KiB, listado 100/30 entradas, teclado
  4096 B, terminal 32 args/4096 B, count 1–10, índice 1–99.
- Doble confirmación observada antes: el modelo pide permiso por texto y
  luego sale la tarjeta (preguntar una vez).
- `calendar_write`: ¿conviene permitir ahorrar la tarjeta para create en
  calendarios editables? (medir fricción en vivo).

## 6. Registro y entrega

- Registrar cada resultado en este plan (§7 Resultados) y, al cerrar, en
  `docs/implemented.md` / `docs/hefesto-plan.md` §8 con commit exacto
  (solo si el usuario autoriza commitear).
- Entrega final al usuario: resumen de fallos, tabla de latencias y
  fricciones, y **propuesta de cambios** (qué confirmaciones/límites
  quitar o relajar y por qué) para decidir juntos.

## 7. Resultados

### Capa 1 — Estática: 8/8 suites en verde (870 tests, 0 fallos)
Python 598 OK · `cargo test` 211 OK (2 ignoradas) · clippy `-D warnings`
limpio · `cargo fmt --check` limpio · `cargo check` OK · `npm test` 61 OK ·
`npm run build` OK · `npm run lint` 0 avisos. Sin procesos huérfanos.

### Capa 2 — HTTP en vivo (atlas-hub)
- Contratos de error: `bad_since`, `empty_message`, `bad_source`,
  `bad_decision`, `bad_days` → 400 correctos. **Matiz:** `{}` en `/v1/send`
  devuelve `empty_message` (no `bad_body`).
- Comandos: `/help`, `/log`, `/cancel` sin nada en curso («No hay nada en
  curso.») OK. `/cancel` **con** petición en vuelo (tarjeta pendiente):
  «Cancelada la petición en curso.» y la petición original pasó a
  `cancelled` sin ejecutar. `POST /v1/cancel {requestId}` idem.
- `/clear`: pendiente de decisión del usuario (resetea la sesión activa).
- **Hallazgo:** un mensaje de 4001 caracteres se acepta y se **trunca en
  silencio** a 4000 (`text = text[:MAX_CONTENT_LEN]`), contra el límite
  documentado.
- Calendario: `calendars` (7 listados), `upcoming` (~10,8 s, TZID
  correctos), `calendar_read` days=3 y `period=next_week` (lunes 9 → lunes
  16) con día semanal, zona y uid corto correctos; ciclo
  create→update→delete en «Deberes» con tarjeta en las tres (create sin
  uid, update/delete tras leer el uid), limpieza verificada en iCloud tras
  expirar la caché de 120 s. **Fricción:** update/delete exigen uid →
  flujo de 2 turnos (leer para sacar el uid). La caché de lecturas no se
  invalida tras escritura (riesgo de dato obsoleto ≤120 s).

### Capa 3 — Dispositivo en vivo (app del usuario ya registrada, PID 33074)
| Tool | Resultado |
| --- | --- |
| echo | delegación + continuador violeta `agent: hefesto` con el eco ✓ |
| system os/disk/uptime/memory | sin tarjeta; disk coincide con `df` (48 GB) ✓ |
| apps list | Finder, Qoder, Atlas, Safari, sin tarjeta ni burbuja ✓ |
| apps open TextEdit | burbuja «Abierta», proceso real ✓ |
| apps open inexistente | texto estable, sin burbuja ✓ |
| apps close no abierta | ««Numbers» no está abierta», sin burbuja ✓ |
| apps close Music (abierta) | burbuja «Cerrada», proceso desaparece ✓ |
| list_dir Documents | tarjeta «Listar …» → listado ✓ |
| list_dir/read_file /etc | rechazadas sin tarjeta por el modelo (descripción del catálogo) y por el dispositivo vía mTLS («fuera de las raíces autorizadas») ✓ |
| read_file leeme.txt | tarjeta con aviso de exportación → frase secreta al modelo ✓ |
| write_file create/overwrite | tarjetas con tamaño/hash corto, ficheros verificados en disco, create sobre existente falla sin tocar ✓ |
| rename / destino existente | movido con burbuja; destino existente rechazado sin tarjeta ✓ |
| media up/down/mute×2 | efectos reales verificados con `osascript` (13→25→13; mute true→false) ✓ |
| media brillo y play/pause | tarjetas y textos estables; play lanzó Music (comportamiento del sistema) y se cerró con la tool ✓ |
| media con pantalla bloqueada | guard de entrada segura: tarjeta sí, **ninguna tecla**, texto estable ✓ |
| terminal /bin/echo | tarjeta `destructive` → «hola-mundo» al modelo ✓ |
| terminal /bin/sleep 30 | timeout + SIGKILL del grupo, sin huérfanos ✓ |
| terminal /bin/sh | rechazado por el relay sin tarjeta ✓ |
| capture | tarjeta `external` → **PNG real + OCR correcto** (TCC de pantalla ya concedido); con pantalla bloqueada la captura sale sin texto ✓ |
| keyboard/windows | **parcial:** list/focus fallan («no se encontró ninguna ventana») — ver hallazgos; type/combo/move/resize **pendientes** (pantalla bloqueada) |

**Hallazgo de apps/ventanas:** TextEdit abierto por Hefesto (2 veces) nació
sin ventana visible/AX hasta que el usuario lo activó con un clic en el
Dock (el fichero sí llegó). System Events confirmó ventana tras el clic.
`hefesto.apps open` no activa la app → las tools de ventana fallan justo
después de abrir.

### Capa 4 — Matriz mTLS (controller), 47+ casos
- Transporte: sin certificado → `SSLError`; CA ajena → `URLError` (cierre
  sin respuesta); HTTP plano cortado. RBAC: device→submit y controller→
  register/heartbeat → `403 forbidden`.
- Versiones: 8/`"9"`/bool → `invalid_version`. Echo >256 B/vacío/sin text →
  `invalid_text`. Tool desconocido → `unsupported_tool`. Rutas relativas →
  `invalid_path`. Campos extra/tipos → `invalid_fields`/`bad_argument`.
  Rango de count 0/11/str/bool → `bad_count`. Windows inválidos →
  `bad_action` (**drift:** la matriz v7 daba `bad_argument`).
  Combo sin modificadores: pasa el relay (regex laxa) y lo rechaza el
  dispositivo («atajo no válido», sin tecla) — documentado.
- `device_busy` (409) con una invocación en vuelo — diseñado.
- **Hallazgo de seguridad:** `/usr/bin/python3 -c …` fue **aceptado por el
  relay y EJECUTADO por el dispositivo** (SyntaxError real). La lista
  `DENIED` (idéntica en relay y dispositivo) cubre shells/elevación pero
  no intérpretes (`python3`, `osascript`, `swift`, `node`, `ruby`, `perl`,
  `awk`, `sed`) ni binarios de riesgo (`open`, `curl`, `defaults`,
  `crontab`, `security`…). El plan promete «intérpretes denegados por
  nombre».
- `file_info` aceptable vía submit directo del controller (es el camino
  que usa el relay para el plan; rol de confianza) — anotado, no es bug.
- Log del relay: terminal con `args=` vacío y resultados `<N chars>`;
  teclado sin texto; confirmaciones sin resumen. Redacción correcta.
  En el uso real del usuario (15:17 y 15:26) hubo dos `device_offline`
  justo tras arrancar el tracker — ventana post-registro a investigar.

### Auditoría final
Sin procesos de la tanda (TextEdit de prueba cerrado por PID propio);
apps vivas: Finder, Qoder, Atlas, Safari; `launchctl` sin agentes nuevos;
volumen 13 sin mute (restaurado). Artefactos de prueba borrados
(`~/Documents/Atlas-pruebas/`, `/tmp/atlas-qa-*`, `/tmp/qa-*`). Quedan dos
PNG de la prueba de captura en `~/Documents/Atlas/capturas/` (salida de la
tool, por diseño).

### Tramo final (tras la reactivación del usuario, ~21:00)
- El usuario relanzó la app (PID 11980, instancia `app-11980-…`, gen 1217,
  auto-activada). El estado degradado de ventanas desapareció: TextEdit
  abierto por Hefesto ya crea ventana y toma el foco → el hallazgo de
  «apps sin ventana» fue **ambiental** (sesión degradada antes del
  reinicio/relanzamiento), no del código de open.
- **Teclado en vivo verificado completo:** type «QA-TECLADO-42» en TextEdit
  (tarjeta con el texto, confirmada) → el texto apareció en la ventana
  (verificado por OCR de captura) y combo cmd+s (tarjeta) → fichero en
  disco `QA-TECLADO-42base-`. Sin burbuja, log sin texto.
- **Ventanas en vivo:** `list` devuelve bounds reales por wire
  (`qa-teclado.txt` en 141,64 656×422); `move` a 100,150 con tarjeta y
  resultado correcto. `resize` falló («no se encontró ninguna ventana»)
  porque la ventana desapareció entre turnos (posible cierre manual del
  usuario, que estaba usando el Mac): resize/focus/ambiguo quedan **sin
  re-verificar en vivo**; cubiertos por la matriz del 2026-11-01/02 y las
  suites.
- **Corrección de la matriz mTLS:** los códigos «drift» de windows eran
  mío: el protocolo wire usa `list_windows`/`move_window`/`resize_window`/
  `focus_window` (el catálogo del modelo usa `list`/`move`/`resize`/
  `focus`). Con el vocabulario wire, todos los inválidos devuelven
  `bad_argument` como en la matriz v7. Sin drift real.
- **Narración errónea del modelo (repetida):** ante un resultado de
  ventanas vacío, el modelo narró «la solicitud se canceló» (no hubo
  cancelación; el turno completó). Mismo patrón que el «truncado» del
  renombrado: propuesta de prompt para que el modelo repita el texto
  estable de la tool, sin reinterpretar.
- **`/clear` probado en vivo:** responde «Historial reiniciado…» y crea
  sesión nueva vacía (la anterior queda archivada en `sessions.json`).
- Cierre de TextEdit por la tool («Cerrada «TextEdit»», proceso
  desaparece) y auditoría final limpia (arriba).

## 8. Correcciones pendientes (propuesta para ejecutar)

Orden acordado con el usuario: primero C1–C4 (correcciones), después
R5–R7 (relajaciones, pendientes de decisión). Nada se ejecuta sin permiso
explícito; cada ítem termina con pruebas y registro.

### C1 · [Seguridad] Terminal: denegar intérpretes y binarios de riesgo
**Evidencia:** el 2026-11-08 `/usr/bin/python3 -c "echo x"` pasó el relay,
se despachó y se **ejecutó** (SyntaxError real del dispositivo). Las listas
`_TERMINAL_DENIED` (relay) y `DENIED` (dispositivo) son idénticas y solo
cubren shells/elevación/control de sistema.
**Cambio:**
- `server/atlas_relay.py` `_TERMINAL_DENIED` (~línea 459): añadir
  intérpretes y puentes (`python3`, `osascript`, `swift`, `node`, `ruby`,
  `perl`, `awk`, `sed`, …) y binarios de riesgo (`open`, `curl`, `defaults`,
  `crontab`, `at`, `security`, `login`, `passwd`, `diskutil`, `tmutil`, …).
  La lista exacta se cierra con el usuario antes de tocar código. El relay
  debe rechazar **antes** de abrir la tarjeta.
- `crates/atlas-mac/src/terminal.rs` `DENIED` (~línea 22): misma lista
  (defensa en profundidad; el dispositivo ya rechaza `sh`/`sudo`).
**Aceptación:** matriz mTLS con cada nombre denegado → `bad_argument` y
cero filas; tests unitarios en ambos lados; prueba en vivo con `python3`
y `osascript` rechazados sin tarjeta; suites completas en verde.

### C2 · [UX] `hefesto_apps open`: activar la app tras lanzarla
**Evidencia:** el incidente de «apps sin ventana» del 2026-11-08 resultó
ambiental (sesión degradada; tras el reinicio del usuario, open sí creó
ventana y foco), pero activar explícitamente evita depender del estado
del sistema y hace que `windows` funcione justo después de abrir.
**Cambio:** `src-tauri/src/hefesto_apps.rs` (`WorkspaceApps`): tras el
`openURL` con éxito, `NSRunningApplication.activateWithOptions`
(ActivateAllWindows) en el hilo principal, con timeout como el resto del
puente.
**Aceptación:** test unitario del puente; matriz en vivo open → list/focus
con ventana visible; sin burbuja extra (sigue siendo «Abierta «X»»).

### C3 · [Límite] Mensajes >4000 caracteres: rechazar, no truncar
**Evidencia:** un mensaje de 4001 chars fue aceptado y truncado en
silencio (`text = text[:MAX_CONTENT_LEN]`, `server/atlas_relay.py`
receive ~línea 1113); `docs/relay-api.md` documenta el límite como 1..4000.
**Cambio:** devolver `400 content_too_long` (o el código que se acuerde)
en vez de truncar; actualizar `docs/relay-api.md`.
**Aceptación:** test de 4001 chars → 400; 4000 → aceptado; mensaje normal
intacto.

### C4 · [Pulido] Contratos y narración
- **Echo vacío:** `""` pasa hoy (≤256 B). Rechazar explícitamente en el
  validador del relay y del dispositivo (`invalid_text`).
- **`{}` en `/v1/send`:** devuelve `empty_message`; el contrato documenta
  `bad_body` para cuerpo inválido. Alinear (o documentar el actual).
- **Narración del modelo (×2):** tras un renombrado correcto narró un
  «truncado» inventado; tras un listado de ventanas vacío narró «la
  solicitud se canceló» sin cancelación. Añadir al prompt fijo del relay
  la instrucción de repetir el texto estable de la tool sin
  reinterpretarlo.
- **Nomenclatura de ventanas:** no hay drift real (ver §7, tramo final);
  opcional aclarar en `docs/contracts.md` la doble nomenclatura
  catálogo (`list`) vs wire (`list_windows`).
**Aceptación:** tests por caso; prueba en vivo de una narración con texto
estable.

### R5 · [Relajación, decisión pendiente] `hefesto_media` sin tarjeta
La política del plan (hefesto-plan §4) ya declara la multimedia reversible
como «trivial por adaptador», pero el relay exige tarjeta en cada
pulsación (8 acciones). Opciones: (a) quitar la tarjeta a las 8 acciones,
(b) solo volumen/brillo (reversibles), mantener mute/play. Evidencia: las
5 pruebas en vivo necesitaron tarjeta para efectos 100 % reversibles y
verificados. Cambio en `HEFESTO_MEDIA_TOOL` (`requiresConfirmation`) +
docs/contratos. Sin cambio de journal ni de dispositivo.

### R6 · [Relajación, decisión pendiente] Lecturas con consentimiento por raíz
Hoy `hefesto_list_dir`/`hefesto_read_file` piden tarjeta en **cada**
operación (3 lecturas de la misma raíz = 3 tarjetas idénticas). Opción:
consentimiento por raíz válido para la sesión (una tarjeta por raíz; las
lecturas posteriores dentro de esa raíz no preguntan). Afecta a relay
(catálogo + flujo), contratos y frontend (tarjeta y estado). Requiere
diseño previo del token de sesión por raíz antes de tocar código.

### R7 · [Relajación, decisión pendiente] `calendar_write` update/delete por título
Hoy update/delete exigen `uid` → flujo de 2 turnos (leer para sacar el uid
→ escribir). Opción: resolver por `summary`+`calendar` cuando hay una
única coincidencia (el relay hace la búsqueda); si hay varias o ninguna,
error estable como hoy. Cambio en el relay; decisión de UX del usuario.

### V1 · [Verificación pendiente] Ventanas: resize/focus/ambiguo en vivo
El 2026-11-08 quedaron sin re-verificar porque la ventana de prueba
desapareció entre turnos (uso activo del Mac). Cubiertos por la matriz
2026-11-01/02 y las suites; repetir cuando haya una ventana estable de
TextEdit (abierta desde el Dock por el usuario).

## 9. Estado de ejecución de las correcciones

- **C1 — implementado, commiteado y desplegado el 2026-11-08 (TDD):** terminal
  de consulta con lista blanca de 22 binarios en relay (`_TERMINAL_ALLOWED`),
  journal (`hefesto.py`), dispositivo (`terminal.rs`) y puente (revalida la
  ruta canónica); raíces reducidas a `/usr/bin`, `/bin`, `/usr/sbin`,
  `/sbin`. Commit `239a0e0`, relay desplegado (`/v1/health` OK). Verificado
  en vivo tras el despliegue: `/usr/bin/python3` rechazado **sin tarjeta** y
  `/bin/echo` con tarjeta y salida. Tests: Python 264, workspace Rust 214
  (2 ignoradas), clippy y fmt limpios; los RED se vieron fallar antes
  (el puente devolvía `1\n` de python3). **Pendiente:** bundle nuevo para la
  defensa del dispositivo (el relay ya rechaza primero).
- C2–C4, R5–R7 y V1: pendientes.

