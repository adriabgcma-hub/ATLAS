# Hefesto — plan revisado de implementación

Estado: ejecución autorizada el 2026-10-27, por entregas verificables. **Sin commits,
push, despliegues, instalación de certificados ni cambios en el Mac/servidor real
sin permiso adicional.** La revisión de seguridad sustituye al borrador conversado.

## 1. Base comprobada antes de empezar

Árbol limpio en `main`, igual a `origin/main`, HEAD `49c8cf2`. Revisados los últimos
12 commits y los cambios recientes del relay/núcleo. Se preservan `446ecb4` (cursor),
`703b135` (Markdown y streaming), `c7a478d` (identidad al rehidratar), `93716eb`
(IPC/red fuera del hilo principal) y `d37ef88` (comandos). No se reabre ese trabajo.

## 2. Producto acordado

- Atlas conserva conversación, calendario, Telegram y las capacidades actuales.
- Hefesto es su especialista para el Mac, no otro chat: Atlas delega por herramienta
  con contexto mínimo; el especialista devuelve resultados estructurados y Atlas narra.
- Orbe violeta/índigo, identidad del especialista separada del estado de tarea y del
  canal (`local`/`telegram`). El estado procede del ejecutor, no del texto del LLM.
- Solo actúa con Atlas ejecutándose; ventana oculta sí, proceso terminado no.
  Mac no disponible: rechazar y avisar. Nunca ejecutar encargos viejos al reabrir.
- App y Telegram, incluidos consentimientos por Telegram ligados a una acción exacta.
- Operaciones triviales sin aprobación. Plan solo si incluye pasos no triviales;
  las acciones de confirmación individual siguen preguntando aunque aparezcan en él.
- Abortar también suspende, invalida consentimientos y exige reactivación manual.
  Mandos: app, atajo local y Hub Panel a través del relay.
- Accesibilidad requiere habilitación explícita persistida y permisos macOS.
- Raíces: Documents, Desktop, Downloads, Pictures, Music, Movies y proyectos dados
  de alta. La ventana de tablas/CSV no forma parte de esta entrega.

## 3. Límites y valores iniciales conservadores

Estos valores permiten construir la base sin resolver decisiones sensibles por el LLM:

- En esta primera entrega solo existe `hefesto.echo`: texto de prueba, sin acceder al
  Mac, sin Groq, Telegram, shell, archivos de usuario ni permisos TCC.
- Datos de archivos y capturas no se envían al proveedor ni a Telegram por defecto.
  Antes de las herramientas reales se implementará consentimiento de exportación
  delimitado por contenido y destino; compartir chat no comparte todos los datos.
- No `sudo`, llaveros, claves, `.env`, dotfiles ni LaunchAgents. Las prohibiciones no
  se levantan mediante un botón de confirmación. El terminal general no se habilitará
  hasta definir su aislamiento y declarar sus limitaciones reales.
- La automatización de interfaz se pausará al bloquear/dormir el Mac y no se reanudará
  automáticamente. Las operaciones sin interfaz tendrán una política explícita propia.
- 12 turnos, 5 min de trabajo, 30 s por comando y 64 KiB de salida son presupuestos
  iniciales a medir, no garantías aprobadas de rendimiento. La espera humana tiene
  caducidad separada. Un presupuesto engloba Atlas, delegaciones y reintentos.
- Un solo trabajo de Hefesto activo por dispositivo; Atlas debe seguir atendiendo
  conversación y cancelación mientras el especialista espera.

## 4. Fronteras de confianza

### Transporte e identidad

La API existente confía en la LAN y usa HTTP. **No puede lanzar Hefesto.** La base
introduce un servicio HTTPS con certificado de cliente obligatorio (mTLS), en otro
listener del mismo proceso del relay, sin helper ni servicio de autoarranque nuevo.
Certificados emitidos por una CA confiable no bastan: una tabla local de huellas
SHA-256 vincula cada certificado a un rol y un dispositivo. Roles: `device` (su
registro, latido, recogida, resultado y control propio) y `controller` (estado,
control y envío de pruebas exclusivamente al dispositivo vinculado).

No se crean ni instalan credenciales reales automáticamente. No se confía en campos
`source`, `risk`, `agent` o `deviceId` enviados por un modelo como prueba de autoridad.
No hay downgrade a HTTP, certificados sin verificar, redirects ni proxies heredados.
El frontend nunca recibe las credenciales. Las pruebas TLS usan certificados efímeros.

La huella se autoriza inmediatamente después del handshake y **antes** de leer o
escribir HTTP: un certificado válido de la CA pero no listado se cierra sin respuesta
(el cliente lo observa como fallo de transporte, no como un error de protocolo
interpretable). Además, cada conexión tiene un plazo absoluto (5 s por defecto)
vigilado por un hilo perezoso del propio listener: al vencer se cierra el socket
aunque el cliente gotee bytes. Los timeouts por operación (3 s) no acotan un goteo
dentro de un mismo registro TLS y, sin plazo absoluto, un cliente lento podía ocupar
las 32 plazas antes de autenticarse. El vigilante solo existe mientras hay
conexiones y se detiene y une al cerrar el servidor.

Antes de activar delegación real hay que autenticar también sus entradas y
confirmaciones: las rutas HTTP antiguas no pueden convertirse en una puerta indirecta.
El relay seguirá siendo una parte confiable para aprobar desde Telegram; la política
local limita su poder pero no detecta una aprobación inventada por un relay comprometido.

### Ejecutor

Rust valida herramienta y argumentos de forma independiente. `argv` evita que una
shell interprete metacaracteres, pero no aísla lo que haga un programa. Las consultas
triviales serán adaptadores de opciones cerradas, no una lista de nombres de binarios.
No se dará al webview acceso genérico a shell/fs.

Las raíces deben comprobar el objeto abierto y los enlaces, no solo prefijos de ruta.
Abrir aplicaciones del usuario no las convierte en hijos ejecutores de Hefesto;
pueden seguir abiertas al terminar, sin seguir controladas por el especialista.

### Consentimiento y privacidad

Precedencia: prohibición > autorización individual > plan > trivial.

| Operación | Política |
| --- | --- |
| Información del sistema, multimedia reversible, abrir apps | Trivial por adaptador |
| Cierre normal de app | Trivial, sin descartar cambios ni aceptar diálogos de guardado |
| Crear archivo en raíz autorizada | Trivial, creación exclusiva sin truncar destinos |
| Sobrescribir/mover/renombrar existente, también en zona propia | Plan exacto |
| Borrado, cierre forzado, matar procesos del usuario | Individual, identidad revalidada |
| Capturas, accesibilidad, comunicación externa | Individual y consentimiento de datos |
| Terminal de consulta | Adaptadores y opciones permitidos |
| Terminal general | Pendiente de aislamiento, nunca trivial por nombre del binario |
| Accesos excluidos o elevación | Rechazo |

Un consentimiento identifica tarea, invocación o versión de plan, argumentos exactos,
dispositivo, usuario autorizante, caducidad y uso único. Cambiar destino invalida el
permiso. Telegram usa botones/identificadores vinculados, no un «sí» libre para la
siguiente acción. El contenido leído es dato no confiable, nunca autorización.
Auditoría: identificadores, estados, duración y códigos; no texto de documentos,
secretos ni argumentos crudos indiscriminados.

## 5. Protocolo v2 (eco y lectura)

Separado del historial/eventos de Atlas. JSON camelCase y `protocolVersion: 2` en
solicitudes POST y respuestas. IDs opacos ASCII `[A-Za-z0-9_-]`, 1–80 caracteres.
Errores `{protocolVersion: 2, error: "codigo_estable"}`, nunca rutas/certificados.
Cuerpo máximo 128 KiB (para transportar lecturas de hasta 16 KiB con escape JSON);
strings del eco limitados a 256 bytes UTF-8 y `output.text` a 64 KiB. Campos
desconocidos, bool por entero, versiones o tipos incorrectos se rechazan.
Generaciones entre 0 y `2^63-1`: cero representa un dispositivo aún no registrado,
nunca una instancia viva. La expiración usa milisegundos Unix positivos hasta `2^63-1`.

Cada certificado está ligado a un solo dispositivo; **no se acepta deviceId en las
solicitudes**. El servidor deriva identidad y permisos de la conexión TLS.

| Ruta | Rol | Cuerpo además de protocolVersion |
| --- | --- | --- |
| GET `/v1/hefesto/status` | Ambos | Sin cuerpo |
| POST `/v1/hefesto/register` | device | `instanceId` |
| POST `/v1/hefesto/heartbeat` | device | `instanceId` |
| POST `/v1/hefesto/control` | Ambos | `action: resume/abort`, `expectedGeneration` |
| POST `/v1/hefesto/submit` | controller | `submissionId`, `requestId`, `sessionId`, `taskId`, `toolId: hefesto.echo\|hefesto.list_dir\|hefesto.read_file`, `arguments: {text}` (eco) o `{path}` (lectura/listado) |
| POST `/v1/hefesto/next` | device | `instanceId`, `generation` |
| POST `/v1/hefesto/result` | device | `instanceId`, `generation`, `invocationId`, `output: {text}` |
| POST `/v1/hefesto/outcome` | controller | `invocationId` |

Estado: `{protocolVersion, deviceId, instanceId: string|null, generation,
suspended, online, activeInvocationId: string|null}`. Estado inicial suspendido.
`register`, `heartbeat`, `control` devuelven estado; `submit` y `result` devuelven
`{protocolVersion, invocationId, state}`; `next` devuelve
`{protocolVersion, invocation: Invocation|null}`; `outcome` devuelve
`{protocolVersion, invocationId, state, outputText: string|null}` (lectura pura).

Invocation: `{protocolVersion, invocationId, requestId, sessionId, taskId,
deviceId, instanceId, generation, toolId, arguments, expiresAtUnixMs}`, donde
`arguments` es `{text}` (eco) o `{path}` (lectura/listado).
No se considera autorización ni se ejecuta por recibir un mensaje de chat.

`path` es absoluto (empieza por `/`), ≤1024 bytes UTF-8, sin NUL; el dispositivo
resuelve la ruta canónica y verifica que queda dentro de una raíz autorizada
(Documents, Desktop, Downloads, Pictures, Music, Movies), rechazando enlaces que
escapan. `read_file` devuelve hasta 16 KiB de texto; `list_dir` hasta 100 entradas;
los binarios se rechazan como «no legible». Los errores de lectura (fuera de raíz,
demasiado grande, binario, sin permisos) vuelven como `succeeded` con el mensaje en
`output.text` (el protocolo no tiene estado `failed` para resultados).

### Estado, duplicados y fallos

- Latido con lease de 15 s; reloj monótono para disponibilidad. Al registrar una
  instancia nueva se incrementa generación y queda suspendida. No sustituir una
  instancia que siga viva: evita carreras dev/release. Registrarse de nuevo con el
  mismo ID no reanuda nada ni sirve para revivir una instancia caducada.
- El registro local es en dos fases (`prepare_registration` → `acknowledge_registration`),
  como el `resume`. El lease se ancla al **primer** envío: si se pierde el ACK y se
  reintenta, el reintento no alarga el lease local más allá del registro original del
  relay (que no renueva un ID vivo). Así el Mac nunca cree estar vivo más tiempo que el
  servidor y no ejecuta con una instancia remota ya caducada.
- Tras caducidad, el ID anterior no vuelve a admitir trabajos: nuevo registro y
  reactivación manual. Caída del relay invalida todas las instancias en memoria.
- `resume`/`abort` exigen la generación actual para evitar reactivaciones antiguas.
  Cambiar control incrementa generación. Abortar cancela pendientes y marca una
  acción despachada como `unknown` si no se puede confirmar su efecto.
- Submit requiere dispositivo vivo, no suspendido y libre; TTL de invocación 15 s
  (eco). No hay cola para dispositivos ausentes, ni encargos para futuros arranques.
- `submissionId` es clave de idempotencia duradera por dispositivo: repetir el mismo
  contenido devuelve la misma invocación/estado; cambiarlo produce conflicto.
- Estados persistidos en SQLite: `pending`, `dispatched`, `succeeded`, `cancelled`,
  `expired`, `unknown`. Reiniciar cancela pendientes y convierte despachadas a
  `unknown`. Nunca reintentar automáticamente un efecto de resultado desconocido.
- `next` puede repetir la misma entrega mientras es válida; Rust conserva el resultado
  por ID y rechaza argumentos modificados. Una entrega repetida no vuelve a ejecutar.
- Resultados repetidos idénticos son idempotentes; diferentes, de otra instancia,
  generación, dispositivo o caducados se rechazan. Un éxito ya registrado permite
  repetir exclusivamente su ACK idéntico incluso tras caducar o reiniciar; compara
  identidad/generación contra la invocación original y nunca ejecuta ni revive nada.
  No se persiste texto arbitrario en logs.
- Cada comienzo de tarea comprueba vida local, suspensión, generación, identidad,
  expiración y política. El presupuesto monótono local evita que un reloj atrasado
  amplíe el lease. Reiniciar el Mac genera otra instancia e invalida las entregas viejas.
- Abortar no revierte efectos anteriores; el estado distingue solicitado/confirmado/
  desconocido. Sin red no se promete que Hub Panel haya parado el Mac.

## 6. Entregas y puertas de validación

1. **Base segura, sin efectos (cerrada, 2026-10-28):** protocolo v1, mTLS/roles, registro durable,
   deduplicación, leases, control y ejecutor Rust de eco con pruebas cruzadas.
2. **Integración app/panel (primer corte, 2026-10-28/29):** bucle nativo ligado a Atlas,
   instancia única, indicador y kill switch local, arranque disponible por intención,
   suspensión persistente entre reinicios, aborto local independiente de red y estado
   recuperable, implementados y probados en dev. Verificado en vivo el ciclo
   activar → eco → suspender con el relay real y la recuperación tras reiniciar el Mac.
   Pendientes de esta entrega: atajo y control Hub Panel, y la matriz completa de
   salida normal/crash/pérdida de red/suspensión/reinicios antes de ejecutar.
3. **Delegación e identidad:** entradas autenticadas, continuaciones no bloqueantes,
   orbe/etiquetas, snapshots atómicos de tareas/confirmaciones/cursor; mantener Atlas
   operativo mientras Hefesto espera. Medir resultados, latencia y tokens.
4. **Lecturas y archivos (corte 1, solo lectura, cerrada 2026-10-30):** protocolo v2 con
   `hefesto.list_dir`/`hefesto.read_file`, raíces del usuario con contención canónica
   (enlaces fuera se rechazan), consentimiento de exportación por operación, resultado
   síncrono al modelo, topes de 16 KiB/100 entradas y binarios rechazados en vez de
   volcados. Pendiente para el siguiente corte: creación exclusiva, planes para cambios
   existentes, escritura atómica y auditoría minimizada.
5. **Apps y multimedia:** adaptadores, cierre sin pérdida, permisos macOS por destino.
6. **Terminal:** consultas por adaptadores, presupuestos y supervisión de procesos;
   terminal general solo tras acordar y verificar aislamiento/límites.
7. **Accesibilidad/capturas:** habilitación explícita, TCC, bloqueo de sesión, modelo
   de visión si hace falta y consentimiento de envío. No activar sensores por defecto.

Cada entrega es local y comprobable antes de activar la siguiente. La base no es un
agente funcional ni un anuncio de que Hefesto ya controle el Mac.

### Aceptación mínima

- Certificado ausente/no autorizado, rol equivocado y HTTP: ningún acceso a Hefesto.
- Cambio de argumentos tras entrega o aprobación: rechazo.
- Entrega/resultado/submit duplicados: no repetir ejecución ni generar otra tarea.
- Lease caducado, app cerrada, instancia distinta: no aceptar encargos viejos.
- Reinicio tras efecto sin ACK: `unknown`, sin reejecución.
- Aborto: suspensión e invalidación, reactivar no recupera trabajo anterior.
- Datos sensibles: ninguna transmisión implícita al proveedor/Telegram ni log crudo.
- Regresión: comandos, calendario, chat e identidad estable siguen pasando pruebas.
- Sin procesos huérfanos ni servicios nuevos al cerrar la prueba.

## 7. Operación y entrega

La configuración TLS futura se provisiona fuera del repo; las pruebas usan datos
ficticios. No tocar `.env`, certificados, llaveros, permisos TCC, firewall, systemd ni
`authorized_keys` reales en esta ronda. Firmar siempre con identidad estable; no asumir
que cada build pierde TCC ni que notarizar conserva permisos. Verificarlo en su fase.

`docs/implemented.md` mantiene implementaciones publicadas y sus commits exactos:
no añadir un commit ficticio para cambios todavía locales. Registrar aquí pruebas y
resultados y mantener `docs/implementation-plan.md` sincronizado. El deploy habitual
requiere commits; queda pendiente hasta autorización explícita de publicación.

## 8. Registro de ejecución

- 2026-10-27: revisados historial y árbol limpio; plan actualizado tras la revisión.
- Integración opcional del listener en el relay, con configuración completa obligatoria
  y cierre del hilo/socket/journal al terminar. Sin activación ni despliegue real.
- Regresión de frontend: `npm test` (31 pruebas) y `npm run build`, correctos.
  `npm run lint` mantiene tres fallos preexistentes del panel: `api.ts:149` (refs en
  render), `Cards.tsx:19,26` (exports para Fast Refresh). No se modifican esos archivos.
- Sintaxis de integración y pruebas Python comprobada con `py_compile`.
- `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`: 60 pruebas
  correctas (48 unitarias + 12 de integración). Incluyen mTLS efímero, roles, entradas
  estrictas, carreras de registro, idempotencia, relojes, aborto, reinicio, cierre de
  recursos, goteo lento y regresión de comandos.
- `cargo test -p atlas-mac`: 45 pruebas correctas, incluida la integración Rust–Python
  por mTLS con certificados temporales. Se comprueban eco Unicode, entrega/resultado
  duplicados, mutación, rechazo de roles/CA/huellas, aborto, reinicio y ausencia de replay.
  El proceso Python y sus certificados temporales se eliminan al finalizar la prueba.
- Corregida la discrepancia de generación inicial: cero solo sin instancia; ambos
  extremos rechazan enteros fuera del rango definido. Añadida limpieza ante interrupción
  de `Thread.start()`, además del cierre normal y por error dentro del contexto.
- `cargo test --workspace`: 56 pruebas correctas; las dos pruebas reales del panel
  permanecen ignoradas (no se consulta ni reinicia el servidor). `cargo check --workspace`,
  `cargo clippy -p atlas-mac --all-targets -- -D warnings`, `cargo fmt -p atlas-mac -- --check`
  y `git diff --check`, correctos. Cargo reutiliza dependencias existentes, sin cambiar versiones.
- Auditoría de procesos: sin Atlas/Hefesto/dev activos (0 MiB), puertos 5173/5174 libres.
  `launchctl list` conserva Hub Panel registrado sin PID; no se ha modificado ni detenido.
- No se cambia UI ni se enlaza la biblioteca a Tauri, por lo que no se reconstruye ni
  instala un bundle. Las pruebas reales de salida/crash/bloqueo de Atlas corresponden
  a la entrega 2; esta validación cubre protocolo, biblioteca y fixtures aislados.
- 2026-10-28, revisión final independiente con dos hallazgos MEDIUM corregidos:
  1. **Lease local tras ACK perdido (Rust):** el lease local se anclaba al ACK, pero el
     relay lo ancla al registro original y no renueva un ID vivo; perder el ACK y
     reintentar alargaba la vida que el Mac creía tener. Ahora el registro es en dos
     fases (`prepare_registration` fija el primer envío; `acknowledge_registration`
     valida contra él) y hay prueba de regresión: un reintento no permite ejecutar más
     allá de los 15 s del primer envío.
  2. **Trabajo lento antes de autenticar (Python):** el listener leía HTTP antes de
     comprobar la huella y no tenía plazo absoluto; un cliente lento podía ocupar las 32
     plazas sin haberse autenticado. Ahora la huella se autoriza al terminar el handshake
     y antes de cualquier byte HTTP (no listado ⇒ cierre sin respuesta, visto por el
     cliente como fallo de transporte), y un vigilante perezoso cierra la conexión a los
     5 s aunque gotee bytes; el vigilante se detiene y une en `server_close`.
  Pruebas nuevas: goteo lento cortado antes del timeout de inactividad, certificado no
  listado cerrado sin tocar el journal y reintento de registro sin extensión de lease.
  Revalidado: Python 60 (48 + 12), `cargo test -p atlas-mac` 45, `cargo test --workspace`
  56, clippy `-D warnings`, `cargo fmt -p atlas-mac -- --check` y `git diff --check`.
- Auditoría tras la revisión: sin procesos Atlas/Hefesto/Vite/Tauri ni pruebas Python
  vivos; puertos 5173/5174/8765 sin listener. `launchctl list` conserva Hub Panel
  registrado sin PID (no se ha tocado).
- 2026-10-28, prueba en vivo en loopback con el cableado real del relay: listener
  arrancado por `hefesto_listener` (sin Telegram/Groq ni `.env`), certificados efímeros
  y journal temporal, manejado con `curl` como cliente mTLS externo. 13 comprobaciones
  correctas: estado inicial, registro, resume, submit, next, result, submit repetido
  idempotente, abort y estado final; huella no listada (curl 56, sin respuesta HTTP),
  CA ajena (curl 60) y HTTP plano (curl 56, sin servicio). El arnés vive en `/tmp`, no
  en el repo; al terminar no quedan procesos, listeners ni temporales.
- 2026-10-28, publicación y despliegue de prueba (autorizados): commit `e22157f`
  subido a `origin/main` y relay desplegado con `./scripts/deploy-relay.sh`;
  `/v1/health` correcto (Telegram y Groq activos). PKI de prueba (CA, servidor y
  clientes `device`/`controller`, `peers.json`) generada fuera del repo en un
  directorio del usuario con permisos 700/600, y validada localmente contra un
  listener real con la consola `hefesto-echo` (eco, reutilización de instancia,
  Unicode y límite de 256 bytes).
- 2026-10-28, activación del listener en atlas-hub completada por el usuario con
  el bundle de `/tmp/hefesto-setup`: `.env` con las cinco variables, certificados
  en `/opt/atlas/hefesto` (700) y `ufw` limitado a la LAN. Verificado desde el
  Mac: `atlas-relay` activo (PID 5584, ~22 MB), listener en
  `192.168.100.26:8767`, `/v1/health` correcto, estado mTLS 200, envío con el
  certificado de dispositivo 403 `forbidden` (RBAC) y HTTP plano cortado antes
  de cualquier respuesta. El bundle de activación (con `server.key`) se eliminó
  del servidor.
- 2026-10-28, corregida la consola de prueba `hefesto-echo` (fuera del repo):
  `atlas-hub.local` anuncia primero un IPv6 link-local donde el relay no escucha
  y `urllib` gastaba el timeout de 10 s en cada petición, agotando la lease de
  15 s (`device_offline`). Ahora resuelve IPv4 conservando el nombre para SNI y
  verificación de certificado, y hace `heartbeat` justo después de registrar
  porque registrar un ID vivo no renueva la lease. Eco completo correcto dos
  veces seguidas (Unicode incluido, instancia reutilizada, generación estable) y
  `--status` mostrando la instancia viva.
- 2026-10-29, integración en la app (local, sin commit): módulo
  `crates/atlas-mac/src/runtime.rs` (latido 5 s, sondeo 1 s, reconexión 15 s,
  fases, snapshot camelCase y aborto local con reintento remoto) y cableado
  Tauri (`hefesto_status`/`hefesto_resume`/`hefesto_abort`, evento
  `hefesto-state-changed`, hilo único y credenciales solo por entorno
  `ATLAS_HEFESTO_CA/CERT/KEY/DEVICE_ID`; el webview nunca las ve). Badge
  violeta con Activar/Suspender y último eco. Pruebas: `cargo test -p
  atlas-mac` 53 (11 de runtime), `cargo test -p atlas-next` 1, `cargo check
  --workspace`, clippy sin avisos, `npm run build` y `npm test` 31.
- Prueba en vivo en dev con el relay real (el Mac se reinició entre medias y se
  relanzó `tauri dev`): la app registra `app-2810-1793228549937-1` (generación
  40, suspendida); el usuario pulsa Activar → generación 41; el eco
  `6275aef5…` se ejecuta en menos de 1 s (sin agotar el TTL de 15 s) y el
  usuario lo ve en el chip; el usuario pulsa Suspender → generación 42 con
  `suspended: true`. Ciclo completo confirmado.
- Borde de diseño registrado: un `resume` iniciado por el controlador no
  habilita la ejecución del dispositivo; el ejecutor solo adopta la activación
  si la app la inicia (ACK con la generación exacta siguiente) y rechaza una
  respuesta "sin cambios" (`crates/atlas-mac/tests/executor.rs`). En la prueba
  previa el eco quedó pendiente hasta caducar; no se relaja el contrato.
- Al cerrar: `tauri dev` parado por tarea/PID, puerto 5173 libre, sin procesos
  Atlas/Vite/Tauri ni agentes nuevos en `launchctl`. Siguen en `atlas-hub` el
  relay y el listener 8767; nada queda vivo en el Mac.
- Pendiente: atajo local, control desde Hub Panel, matriz completa de fallos y
  commit (requiere permiso explícito).
- 2026-10-29, aclarado el papel del interruptor tras preguntarlo el usuario:
  la detección por intención (entrega 3) decide *cuándo* Atlas delega; el
  interruptor decide *si este Mac acepta ejecutar*. Decisión del usuario para
  el arranque: **disponible por intención** (activo) salvo suspensión
  pendiente, en vez de exigir Activar en cada apertura. Implementado en la
  capa Tauri con una compuerta (`hefesto_gate`): un registro nuevo reactiva
  solo si no hay marcador; una suspensión desde `ready` (Hub Panel/Telegram)
  o local escribe `hefesto-suspended` en el directorio de datos de la app
  (`~/Library/Application Support/com.atlas.next/`) y exige Activar a mano,
  también tras reiniciar; Activar adjunta la reactivación y borra el
  marcador. Lectura conservadora: si el marcador no se puede consultar, se
  asume suspendido. Pruebas: 4/4 en `cargo test -p atlas-next`, clippy sin
  avisos; `cargo fmt --check` solo conserva el desorden de imports previo.
- 2026-10-29, prueba en vivo del nuevo arranque: tras lanzar dev sin marcador
  la app registró `app-4995-…` (gen 44), se auto-activó (gen 45) sin clic y
  ejecutó el eco `935386e3…`; un abort del controlador llevó a gen 46,
  `suspended: true` y marcador creado; al reiniciar registró `app-5427-…`
  (gen 48) y siguió suspendida durante 12 s (sin reactivarse sola); el
  usuario pulsó Activar (gen 49, marcador limpio). Un eco posterior quedó
  pendiente porque la app se cerró limpiamente durante la prueba (sin
  informes de fallo): al morir el proceso no hay sondeo y la invocación
  caduca a los 15 s, como está diseñado. Relanzada sin marcador, se
  auto-activó (gen 52) y ejecutó `596ddef4…`; el usuario pulsó Suspender
  (gen 53) y el marcador quedó escrito. Al cerrar: `tauri dev` parado por
  tarea/PID, 5173 libre, sin procesos ni agentes Atlas en el Mac; el
  marcador de suspensión queda a propósito como último estado intencionado.
- 2026-10-29, entrega 3 (delegación por intención) implementada y desplegada:
  operación `outcome` de lectura (rol controller), el listener entrega el
  journal y deriva el dispositivo, herramienta `hefesto_echo` con catálogo
  dinámico, continuador persistente con cancelación, hilo de seguimiento con
  recuperación tras reinicio, contratos `agent`/`taskId` (Rust y TS) y
  etiqueta violeta HEFESTO. Commits `10f3ae1`…`616e16d` (baseline `855e035`,
  valla de docs `313ac0e`). Pruebas: `python3 -m unittest discover -s server
  -p 'test_hefesto*.py' -q` 87, `npm test` 37, `npm run build` y
  `cargo check --workspace`.
- 2026-10-29, prueba en vivo con el relay real desplegado: delegación local
  con continuador violeta, delegación por Telegram con seguimiento
  «Hefesto: …» al chat de origen, cancelación a mitad de turno y suspensión
  con encargo en vuelo («El encargo a Hefesto se canceló.»), y reinicio del
  relay con encargo en vuelo (recuperación → «estado desconocido»). El
  tracker arranca con `device=mac-one` y el listener escucha en 8767.
- 2026-10-29, dos defectos encontrados en la prueba en vivo y corregidos:
  1) la cancelación a mitad de turno no cortaba la ejecución de herramientas
  (el único control estaba en `_complete`); ahora `_handle_tool_call` y
  `_delegate_echo` revalidan la cancelación (`0f7877b`). 2) el botón de
  cancelar quedaba deshabilitado durante una petición en curso (InputBar
  deshabilitaba input y botón justo cuando debían cancelar); corregido con
  test de regresión (`b43a68d`). Bundle de Atlas reconstruido con Apple
  Development, firma profunda válida, instalado en `/Applications/Atlas.app`.
- 2026-10-30, cierre de la entrega 2 (tres frentes) implementado y desplegado:
  1. **Atajo local** `cmd+shift+a` para alternar la ventana de chat: extraída
     la lógica a una función pura `next_visibility(visible, focused)` en
     `src-tauri/src/lib.rs` con su test (4 combinaciones); oculta solo si la
     ventana está visible y con foco, y si no la muestra, la desminimiza y le
     da foco (`0166925`).
  2. **Control de suspensión desde Hub Panel:** el relay expone
     `GET /v1/hefesto/status` (estado `{configured, deviceId, suspended,
     generation}`) y `POST /v1/hefesto/suspend` (kill switch: lee la
     generación, envía `control abort`, reintenta una vez ante
     `stale_generation`); ambos detrás de `_lan_only()`. El panel añade la
     tarjeta Hefesto (badge con `deviceId`, cadena Consultando/Sin
     configurar/Suspendido/Activo y botón Suspender de un clic, activo solo
     con `configured && !suspended`) que sondea cada 2 s (`cc36b8b` +
     `e9f1895`, build del panel `cc92ca1`).
  3. **Matriz de fallos en vivo** (con el usuario, TODO verde): salida normal
     sin huérfanos; crash (`kill -9`) con relanzamiento limpio, reconexión y
     suspensión persistida; pérdida de red (reinicio del relay) con reconexión
     del dispositivo; suspensión remota desde el panel (marcador escrito,
     gen 123→124) y local desde la app (gen→128); reinicio con encargo en
     vuelo (tracker recupera `delegations=1`, continuación `state=unknown` sin
     reejecución). Atajo verificado en vivo.
  Commits de la entrega: spec `1ade92e`, plan `724ba0f`, relay `cc36b8b`,
  panel `e9f1895`, atajo `0166925`, build `cc92ca1`. Pruebas: Python 93
  (88 + 5 nuevas de control remoto), panel `npm run build` y
  `cargo test -p atlas-next` (atajo). Desplegado con `./scripts/deploy-relay.sh`
  (`/v1/health` OK) y bundle de Atlas reconstruido con Apple Development
  (firma profunda válida), instalado en `/Applications/Atlas.app`.
- Auditoría al cerrar la entrega 2: solo queda abierta la app del usuario
  (`/Applications/Atlas.app`, PID 83026, ~40 MB); sin procesos
  Hefesto/Vite/Tauri ni pruebas Python, puertos 5173/5174/8765 libres y
  `launchctl` sin agentes nuevos (Hub Panel registrado sin PID). El marcador
  de suspensión queda como último estado intencionado del usuario.
- 2026-10-30, lectura segura de archivos (puerta 4, corte 1), implementada,
  desplegada y probada en vivo con el usuario:
  1. **Protocolo v2**: `protocolVersion: 2`, `toolId` admitidos
     `hefesto.echo`/`hefesto.list_dir`/`hefesto.read_file`, `arguments` por
     herramienta (`{text}` ≤256 B; `{path}` absoluta ≤1024 B, sin NUL) y
     `output: {text}` ≤64 KiB (cuerpo máximo 128 KiB en transporte).
  2. **Dispositivo**: `atlas-mac::tools` con `Roots` (Documents, Desktop,
     Downloads, Pictures, Music, Movies; canónica y contención por
     componentes, un enlace que sale se rechaza), `list_dir` (≤100 entradas,
     ordenadas) y `read_file` (≤16 KiB, tamaño en el mensaje, NUL/UTF-8
     inválido → «binario/no legible»); el despachador del ejecutor enruta por
     `toolId` sin cambiar el eco.
  3. **Relay**: catálogo con `hefesto_list_dir`/`hefesto_read_file`
     (`requiresConfirmation: True`), tarjeta «Leer … — el contenido saldrá del
     Mac hacia el modelo», espera síncrona de `outcome` (0,5 s, 15 s) y el
     resultado vuelve al LLM en el mismo turno; la respuesta la redacta
     Atlas (las lecturas no generan mensaje «Hefesto:» aparte).
  4. **Registro IPv4**: el cliente Rust del dispositivo resuelve el nombre y
     ordena IPv4 primero (`resolve_to_addrs`); el link-local IPv6 de mDNS
     agotaba el timeout y el dispositivo no llegaba a registrarse. La
     resolución reintenta hasta tres veces y TLS/resolución se construyen en
     el hilo de Hefesto, fuera del arranque de la ventana (`e0eb90c`).
  5. **Prueba en vivo**: tarjeta mostrada, respuesta con `LUNA-42` leída de
     `~/Documents/atlas-prueba-lectura.txt`, y `/etc/hosts` rechazado por
     estar fuera de las raíces (sin confirmación).
  Commits: spec `3a4fd1a`, plan `f8ce127`, herramientas `1dd3914`, journal v2
  `6932ef7`, despachador `6658691`, relay `6d315ed`, docs `94e5ca1`, fix IPv4
  `295da62`. Pruebas: Python 147, `cargo test --workspace`, clippy
  `-D warnings` y `cargo fmt --check`; relay desplegado (`/v1/health` OK) y
  bundle Apple Development con firma profunda válida instalado.
