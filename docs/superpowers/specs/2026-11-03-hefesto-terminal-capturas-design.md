# Terminal general y capturas de Hefesto — diseño

**Fecha:** 2026-11-03
**Alcance:** cerrar los dos pendientes de Hefesto: terminal general (puerta 6) y
capturas con lectura local (puerta 7). La ventana de tablas/CSV se especifica en
`docs/implementation-plan.md` y no forma parte del protocolo Hefesto.

## Decisiones

- El terminal general ejecuta **argv directo**, nunca una shell. El modelo elige
  `program` y `args`; el usuario ve la línea exacta en una tarjeta y decide.
- La captura guarda un PNG en `~/Documents/Atlas/capturas/` y reconoce el texto
  con Apple Vision en el Mac. **La imagen no sale del Mac**; solo el texto
  reconocido vuelve al modelo, con consentimiento por operación.
- Ambas herramientas suben el protocolo a **v9**. Cada invocación es un efecto:
  tras un reinicio queda `unknown` y no se reejecuta.

## Terminal general

Herramienta del relay `hefesto_terminal` (`requiresConfirmation: True`,
`planConfirmation: True`) con `{program, args}`. El modelo no escribe una shell:
`program` es una ruta absoluta y `args` una lista de cadenas. Límites: programa
≤1024 B, 0–32 argumentos, cada uno ≤1024 B, línea completa ≤4096 B, sin NUL.

Validación antes de la tarjeta: ruta absoluta, fichero regular ejecutable,
raíces permitidas (`/usr/bin`, `/bin`, `/usr/sbin`, `/sbin`, `/usr/local/bin`,
`/opt/homebrew/bin`) y denegación explícita de elevación o persistencia
(`sudo`, `su`, `doas`, `launchctl`, `systemsetup`, `shutdown`, `reboot`, `halt`).
La tarjeta muestra `program` y `args` citados, riesgo `destructive`, y avisa de
que la salida irá al modelo.

Ejecución en el dispositivo: `env_clear` con `PATH`, `LANG=C`, `LC_ALL=C`,
stdin nulo, `cwd=/`, stdout y stderr capturados por separado, grupo de procesos
propio, timeout de **10 s** y `SIGKILL` al grupo al vencer. La salida se corta a
**48 KiB** combinados (stdout + stderr) y se añade `… (recortado)`. El resultado
es texto: `Salida (código N):\n…`, con `[stderr]` si hubo error, o un texto
estable (`el comando no respondió a tiempo`, `programa no permitido`,
`no se pudo ejecutar el comando`). Sin burbuja.

Privacidad: los argumentos pueden llevar datos del usuario. El journal no los
persiste: guarda `commandBytes`/`commandSha256` y mantiene la invocación
completa solo en memoria hasta la entrega; el log del relay no escribe los
argumentos y registra el resultado solo por longitud.

## Capturas con OCR local

Herramienta del relay `hefesto_capture` (`requiresConfirmation: True`,
`planConfirmation: True`) con `{target: "screen"}`. La tarjeta avisa de que se
capturará la pantalla, se guardará localmente y el texto reconocido saldrá hacia
el modelo; riesgo `external`. Cada invocación abre su tarjeta y no hay burbuja.

En el dispositivo: preflight de TCC con `CGPreflightScreenCaptureAccess`; si
falta, se solicita una vez por proceso con `CGRequestScreenCaptureAccess` y se
devuelve un texto estable pidiendo activar *Grabación de pantalla* y reiniciar
Atlas. Con permiso, ejecuta `/usr/sbin/screencapture -x -t png <ruta>` con argv
fijo, timeout de 10 s y grupo de procesos propio, guardando en
`~/Documents/Atlas/capturas/captura-AAAAMMDD-HHMMSS.png` (directorio creado si
falta). Después reconoce texto con `VNRecognizeTextRequest` (nivel preciso,
español e inglés) y devuelve `Captura guardada: <nombre>\nTexto reconocido:\n…`
o `no se reconoció texto`. La ruta absoluta no se devuelve; el nombre basta.

Límites: una captura por invocación, salida ≤64 KiB (el texto OCR se recorta),
sin captura de ventanas concretas ni grabación de vídeo en este corte. Privacidad:
el PNG queda en el Mac; el log del relay no escribe argumentos ni texto OCR y
registra el resultado solo por longitud. Un reinicio con la captura en vuelo la
deja `unknown`.

## Protocolo v9

`protocolVersion: 9` en wire, journal y relay; ambos extremos rechazan 8.
`hefesto.terminal` (`{program, args}`) y `hefesto.capture` (`{target}`) se
añaden al catálogo. `hefesto_capture` y `hefesto_terminal` usan
`requiresConfirmation` + `planConfirmation`; el journal valida claves cerradas,
`bad_action`/`bad_argument` y deja `unknown` las invocaciones en vuelo.

## Ventana de tablas/CSV

Fuera de Hefesto. `MarkdownMessage` detecta tablas GFM y ofrece «Abrir en
ventana»; el comando Tauri guarda la tabla en memoria y abre una ventana
`table-*` que carga `table.html`. Desde ahí, «Exportar CSV» escribe en
`~/Documents/Atlas/tablas/` con nombre saneado; nunca se acepta una ruta desde
el webview. La tabla no se persiste: vive en memoria mientras la ventana exista.

## Criterios de aceptación

- `hefesto_terminal` no usa shell, no acepta programas fuera de las raíces y
  mata el grupo al vencer el timeout; la tarjeta muestra el comando exacto.
- `hefesto_capture` no envía la imagen fuera del Mac y sin TCC no captura nada.
- El journal no persiste argumentos del terminal ni texto OCR.
- La ventana de tablas abre, muestra la tabla y exporta un CSV válido; en el
  navegador de desarrollo el botón no aparece.
- Sin procesos huérfanos ni servicios nuevos al cerrar las pruebas.
