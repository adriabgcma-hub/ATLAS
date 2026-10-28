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

## 5. Protocolo de base v1 (solo eco)

Separado del historial/eventos de Atlas. JSON camelCase y `protocolVersion: 1` en
solicitudes POST y respuestas. IDs opacos ASCII `[A-Za-z0-9_-]`, 1–80 caracteres.
Errores `{protocolVersion: 1, error: "codigo_estable"}`, nunca rutas/certificados.
Cuerpo máximo 8 KiB; strings del eco limitados a 256 bytes UTF-8. Campos desconocidos,
bool por entero, versiones o tipos incorrectos se rechazan. Generaciones entre 0 y
`2^63-1`: cero representa un dispositivo aún no registrado, nunca una instancia viva.
La expiración usa milisegundos Unix positivos hasta `2^63-1`.

Cada certificado está ligado a un solo dispositivo; **no se acepta deviceId en las
solicitudes**. El servidor deriva identidad y permisos de la conexión TLS.

| Ruta | Rol | Cuerpo además de protocolVersion |
| --- | --- | --- |
| GET `/v1/hefesto/status` | Ambos | Sin cuerpo |
| POST `/v1/hefesto/register` | device | `instanceId` |
| POST `/v1/hefesto/heartbeat` | device | `instanceId` |
| POST `/v1/hefesto/control` | Ambos | `action: resume/abort`, `expectedGeneration` |
| POST `/v1/hefesto/submit` | controller | `submissionId`, `requestId`, `sessionId`, `taskId`, `toolId: hefesto.echo`, `arguments: {text}` |
| POST `/v1/hefesto/next` | device | `instanceId`, `generation` |
| POST `/v1/hefesto/result` | device | `instanceId`, `generation`, `invocationId`, `output: {text}` |

Estado: `{protocolVersion, deviceId, instanceId: string|null, generation,
suspended, online, activeInvocationId: string|null}`. Estado inicial suspendido.
`register`, `heartbeat`, `control` devuelven estado; `submit` devuelve
`{protocolVersion, invocationId, state}`; `next` devuelve
`{protocolVersion, invocation: Invocation|null}`; `result` devuelve
`{protocolVersion, invocationId, state}`.

Invocation: `{protocolVersion, invocationId, requestId, sessionId, taskId,
deviceId, instanceId, generation, toolId, arguments: {text}, expiresAtUnixMs}`.
No se considera autorización ni se ejecuta por recibir un mensaje de chat.

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
2. **Integración app/panel:** bucle nativo ligado a Atlas, instancia única, interruptor,
   atajo y control Hub Panel; aborto local independiente de red y estado recuperable.
   Probar salida normal, crash, pérdida de red, suspensión y reinicios antes de ejecutar.
3. **Delegación e identidad:** entradas autenticadas, continuaciones no bloqueantes,
   orbe/etiquetas, snapshots atómicos de tareas/confirmaciones/cursor; mantener Atlas
   operativo mientras Hefesto espera. Medir resultados, latencia y tokens.
4. **Lecturas y archivos:** raíces seguras, permisos de exportación, creación exclusiva,
   planes para cambios existentes, escritura atómica y auditoría minimizada.
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
- 2026-10-28: commit, push y despliegue de prueba autorizados por el usuario.
  El despliegue, las credenciales de prueba y su verificación se registran al ejecutarlos.
