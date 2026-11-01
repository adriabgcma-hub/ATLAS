# Teclado sintético de Hefesto (puerta 7, corte 2) — Diseño

**Fecha:** 2026-11-01
**Estado:** aprobado por el usuario (alcance, destino, protecciones y permiso TCC)
**Antecedente:** [control de apps (corte 1)](2026-10-31-hefesto-apps-design.md)

## 1. Contexto y objetivo

Hefesto ya abre, cierra y lista aplicaciones (protocolo v4). El segundo corte de la
puerta 7 le permite **escribir texto y pulsar un atajo** en la app que el usuario tiene
en primer plano, siempre con confirmación individual.

Objetivo acotado: cubrir los dos usos reales —dictar en un campo y guardar/cerrar con
un atajo— con la superficie mínima y sin abrir un canal de ejecución encubierto.

## 2. Decisiones de diseño (alcance acordado)

1. **Dos acciones, una herramienta:** `type` (texto) y `combo` (un atajo). No hay
   secuencias con esperas ni macros.
2. **Solo app en primer plano:** Hefesto no activa apps, no cambia de ventana y no
   elige el campo. El usuario prepara el foco antes de aprobar la tarjeta.
3. **Siempre tarjeta:** teclear es un efecto sobre el mundo, nunca trivial;
   `requiresConfirmation: True` y ninguna ruta sin confirmación.
4. **Sin lista de apps bloqueadas** (decisión del usuario): teclear en Terminal es
   posible si el usuario lo enfoca y aprueba; la tarjeta con el texto exacto es la
   salvaguarda. Riesgo aceptado y documentado.
5. **Prohibidos los saltos de línea** en el texto y **prohibido el Enter suelto** en
   los atajos: al menos un modificador por atajo.
6. **Entrada segura respetada:** si macOS marca entrada segura (campos de contraseña),
   Hefesto no teclea.
7. **Permiso TCC de Accesibilidad:** se pide **una vez por arranque** al primer uso; si
   no se concede, error estable y ningún efecto.
8. **Sin burbuja violeta:** el texto tecleado no se replica fuera de la app; el
   resultado vuelve al modelo y la tarjeta es la visibilidad.

## 3. Requisitos

### 3.1 Protocolo y wire

1. `protocolVersion` sube de **4 a 5** en los dos extremos (wire Rust y Python) y en
   todos los fixtures; ambos lados rechazan versiones distintas.
2. `toolId` del dispositivo: **`hefesto.keyboard`** con payload
   `KeyboardPayload { action: "type"|"combo", text?: string, combo?: string }`.
3. Salida `{ text }` con el resumen estable (no viaja el texto tecleado completo).
4. El relay expone **una sola** herramienta `hefesto_keyboard` con `action` enum,
   `text` y `combo`, y `requiresConfirmation: True`.

### 3.2 Texto y gramática del atajo

5. `type`: `text` obligatorio, no vacío, **≤4096 bytes UTF-8**, sin `\n`, `\r`, `\t`
   ni caracteres de control (`char::is_control`). Se escribe con eventos **Unicode** de
   CGEvent (independiente del layout), en trozos de ≤20 unidades UTF-16.
6. `combo`: cadena `mods+tecla` (p. ej. `cmd+shift+s`). Modificadores cerrados:
   `cmd`, `shift`, `alt`, `ctrl`; **1–3**, sin repetir. Teclas cerradas: `a`–`z`,
   `0`–`9`, `space`, `return`, `tab`, `esc`, `left`, `right`, `up`, `down`, `delete`.
   Se traducen a *keycodes* ANSI (posición física), que es como funcionan los atajos en
   cualquier layout. `combo` ≤64 bytes.
7. El texto y el atajo se validan en el relay antes de delegar y otra vez en el
   dispositivo (defensa en profundidad); los errores son texto estable.

### 3.3 Permisos y entrada segura

8. Antes de teclear se comprueba el permiso de publicación de eventos con
   `CGPreflightPostEventAccess` (es el permiso de Accesibilidad que macOS muestra en
   Ajustes). Si falta, se solicita el diálogo del sistema **una sola vez por arranque**
   con `CGRequestPostEventAccess` y se devuelve el error estable; sin permiso no se
   postea ningún evento. (`AXIsProcessTrusted` cubre la misma intención, pero su
   variante con aviso exige construir un `CFDictionary`; la API de CoreGraphics es
   pública, equivalente y más simple.)
9. Antes de teclear se comprueba `IsSecureEventInputEnabled`. Si está activa, no se
   postea nada y se devuelve el error estable.
10. La app en primer plano se lee de `NSWorkspace.frontmostApplication` para el
    resumen; si no se puede, se usa «la app en primer plano».

### 3.4 Flujo de datos

11. El modelo emite `hefesto_keyboard {action:"type", text:"…"}` → el relay valida,
    abre **una** tarjeta con el texto recortado y, tras `allow_once`, delega
    `hefesto.keyboard` con `invocationId` y `taskId` nuevos y espera el `outcome`.
12. El dispositivo enruta por `toolId`, reenvía al hilo principal y ejecuta con CGEvent.
13. El relay devuelve el resumen al modelo **sin** publicar burbuja `agent: hefesto`.

### 3.5 Manejo de errores (texto estable en el turno, sin lanzar nada)

14. Sin permiso: `falta el permiso de Accesibilidad; actívalo en Ajustes del Sistema ›
    Privacidad y seguridad › Accesibilidad`.
15. Entrada segura: `hay un campo con entrada segura en primer plano; no tecleo ahí`.
16. Texto inválido: `texto no válido (sin saltos de línea ni controles)`.
17. Atajo inválido: `atajo no válido`.
18. Falta argumento: el relay no delega y responde `falta text para type` o
    `falta combo para combo`.
19. Timeout / dispositivo sin respuesta: ruta común del relay («Hefesto no respondió a
    tiempo»); una invocación en vuelo tras reinicio queda `unknown`, sin reejecutar.
20. Cancelación a mitad de turno: se revalida antes de delegar, como en escritura y
    apps.

### 3.6 Persistencia y privacidad

21. Journal: `hefesto.keyboard` es **efecto con contenido**. El texto no se persiste
    (solo `contentBytes`/`contentSha256`), vive en memoria para entregarlo al
    dispositivo y una invocación sin entregar tras reiniciar queda `unknown`. El
    conjunto de claves se restringe a `action|text|combo` antes de guardar.
22. Logs: el relay **nunca** escribe el texto en el log (misma redacción que
    `hefesto_write_file`). El resumen tampoco contiene el texto.
23. Sin burbuja: el texto no se replica a Telegram ni a otros canales.

## 4. Fuera de alcance (cortes posteriores)

- Secuencias de teclas con esperas, macros y repetición.
- Clics de ratón, arrastrar y soltar, rueda.
- Activar apps o elegir ventana antes de teclear.
- Atajos globales del sistema (Mission Control, Spotlight…) como caso especial: entran
  por la gramática general si el usuario los aprueba.
- Ventanas (mover/redimensionar/traer), capturas y modelo de visión.

## 5. Pruebas

5.1 **Dispositivo (Rust, `crates/atlas-mac`)**
- Puras: gramática del atajo (válidos, sin modificador, modificador repetido, tecla
  fuera del conjunto, texto con `\n`/control, límites), y construcción del plan.
- Wire: `KeyboardPayload`/salida (serialización, `action` inválida, `type` sin texto,
  `combo` inválido) con fixtures v5.
- Despacho: el ejecutor enruta `hefesto.keyboard` por `toolId` sin tocar las demás
  ramas, con un puente falso (éxito, entrada segura, sin permiso).

5.2 **Relay (Python, `test_hefesto_*`)**
- Catálogo: `hefesto_keyboard` con `requiresConfirmation: True`.
- Tarjeta con el texto recortado; `allow_once` delega y devuelve el texto del `outcome`.
- Validación previa: `action` fuera de vocabulario, `type` sin texto, texto con salto de
  línea o demasiado largo, `combo` con formato inválido → no se submit.
- El log del relay no contiene el texto tecleado.
- Sin burbuja `agent: hefesto` en el éxito.
- Journal: sin contenido persistido, claves cerradas, `unknown` tras reinicio.

5.3 **Frontend (vitest)**
- `HEFESTO_PHASES` incluye `hefesto_keyboard` → «Hefesto · Tecleando».

5.4 **Contratos**
- Bump a v5 en los tres espejos documentales y filas de `hefesto.keyboard`.

5.5 **Prueba en vivo (con el usuario, tras desplegar)**
- Escribir texto en TextEdit (aparece el texto exacto), `cmd+a` en TextEdit (selecciona),
  diálogo con respuesta oculta (`with hidden answer`) → rechazo por entrada segura.
- Sin permiso de Accesibilidad: primer uso → diálogo del sistema; concedido → funciona.
- Comprobar que no quedan procesos ni RAM añadida al cerrar la prueba.

5.6 **Umbral de aceptación**
- `cargo test --workspace`, clippy `-D warnings`, `cargo fmt --check`,
  `python3 -m unittest discover`, `npm test` + `npm run build`, y la matriz en vivo.

## 6. Riesgos y puntos de vigilancia

- **TCC de Accesibilidad** es un permiso potente: la app queda con capacidad de
  controlar el Mac. Se limita por diseño (una herramienta, dos acciones, tarjeta
  siempre) y se documenta en el plan y en Ajustes.
- **Entrada segura** depende de que la app que muestra el campo la active; los
  diálogos de macOS lo hacen. Se prueba con un diálogo de respuesta oculta, sin
  manejar contraseñas reales.
- **Eventos Unicode** pueden no llegar a apps que exigen *keycodes* reales (juegos,
  algunos editores); queda fuera de alcance y se detecta en la prueba en vivo.
- **CGEvent** es API pública de ApplicationServices; no se usan APIs privadas.
