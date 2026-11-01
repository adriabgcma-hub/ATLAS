# Multimedia de Hefesto (puerta 5) — Diseño

**Fecha:** 2026-11-01
**Estado:** diseño decidido por el agente bajo mandato del usuario («arranca multimedia
y luego ventanas y todo lo que quede pendiente sin parar hasta acabar»)
**Antecedentes:** [control de apps](2026-10-31-hefesto-apps-design.md), [teclado](2026-11-01-hefesto-teclado-design.md)

## 1. Contexto y objetivo

Hefesto ya abre/cierra apps (v4) y teclea (v5). La puerta 5 del plan pide
«multimedia». En macOS, la forma pública y sin permisos nuevos de controlar
volumen, brillo y reproducción es **postear las teclas de sistema** (las que
envía el teclado físico) por CGEvent, que ya usamos para el teclado y para las
que la app tiene permiso de Accesibilidad.

Objetivo: subir/bajar volumen, silenciar, subir/bajar brillo y controlar la
reproducción (play/pausa, siguiente, anterior), con confirmación y sin leer ni
exponer contenido multimedia.

## 2. Decisiones de diseño

1. **Solo teclas de sistema**, no valores absolutos: volumen y brillo por pasos
   (`count` 1–10), como las teclas F11/F12 y F1/F2 del teclado. El volumen
   absoluto (0–100) exigiría CoreAudio/AppleScript (subprocess o binding nuevo)
   y queda fuera de este corte.
2. **Una sola herramienta** `hefesto_media` con `action ∈ {volume_up,
   volume_down, mute, brightness_up, brightness_down, play_pause, next,
   previous}` y `count` opcional (1–10, por defecto 1). `mute`, `play_pause`,
   `next` y `previous` ignoran `count`.
3. **Sin subprocess y sin API privada:** `CGEvent` con los *keycodes* de sistema
   (`NX_KEYTYPE_*`) y el flag de función, al hilo principal, con el mismo puente
   de entrada del teclado.
4. **Siempre tarjeta:** cambiar el volumen o pausar música es un efecto del
   usuario; `requiresConfirmation: True` y `planConfirmation: True` (una tarjeta
   por llamada, ninguna con entrada inválida), como el teclado.
5. **Sin burbuja** y sin contenido: el resultado («Subido 2 pasos de volumen»,
   «Silenciado», «Pausado/reanudado») vuelve al modelo; la tarjeta es la
   visibilidad.
6. **Entrada segura y permiso:** se mantienen las comprobaciones del teclado
   (entrada segura → `no tecleo ahí`; sin permiso → error estable). Reutiliza el
   puente y el TCC ya concedidos.
7. **Nada de leer el estado** (volumen actual, pista o app en reproducción):
   requeriría APIs privadas (MediaRemote) o consultas que este corte no
   necesita. El modelo no puede preguntar «¿a qué volumen estoy?».

## 3. Requisitos

### 3.1 Protocolo y wire

1. `protocolVersion` sube de **5 a 6** en los dos extremos y en todos los
   fixtures; ambos lados rechazan versiones distintas.
2. `toolId` del dispositivo: **`hefesto.media`** con
   `MediaPayload { action: MediaAction, count?: u8 }`.
3. Salida `{ text }` con el resumen estable.
4. El relay expone **una** herramienta `hefesto_media` con `action` enum y
   `count` (1–10), `requiresConfirmation: True` + `planConfirmation: True`.

### 3.2 Comportamiento

5. Cada acción postea su clave NX (`SOUND_UP`, `SOUND_DOWN`, `MUTE`,
   `BRIGHTNESS_UP`, `BRIGHTNESS_DOWN`, `PLAY`, `NEXT`, `PREVIOUS`) con el flag
   de función, `count` veces (1–10), down y up por pulsación, sin dormir entre
   pulsaciones.
6. `count` solo aplica a volumen y brillo; se rechaza `count` fuera de 1–10 y
   `count` en acciones que no lo admiten (validación estable, sin tarjeta).
7. Antes de postear: entrada segura → error estable; sin permiso → error
   estable. Nada se postea en ninguno de los dos casos.
8. La app en primer plano **no** importa: las teclas de sistema son globales.

### 3.3 Errores (texto estable)

9. `action inválida (volume_up, volume_down, mute, brightness_up,
   brightness_down, play_pause, next o previous)`.
10. `count fuera de rango (1–10)`; `count no aplica a esta acción`.
11. Entrada segura: `hay un campo con entrada segura en primer plano; no pulso
    teclas del sistema`.
12. Sin permiso: `falta el permiso de Accesibilidad; actívalo en Ajustes del
    Sistema › Privacidad y seguridad › Accesibilidad`.
13. Fallo del puente: `no se pudo pulsar la tecla del sistema: {err}`.
14. Timeout/cancelación/reinicio: ruta común (unknown sin reejecutar).

### 3.4 Persistencia y privacidad

15. Journal: `hefesto.media` es **efecto sin contenido** (solo acción y count);
    claves cerradas `action|count`; invocación en vuelo → `unknown`.
16. Logs: sin contenido sensible; se puede registrar la acción y el count. Sin
    burbuja.

## 4. Fuera de alcance

- Volumen/brillo absolutos (0–100) y lectura de su estado.
- Pista, artista, app en reproducción o carátulas (MediaRemote es privada).
- Capturas de pantalla y modelo de visión.
- Teclas multimedia de apps concretas (se controlan con `hefesto.keyboard` si se
  necesitan atajos propios).

## 5. Pruebas

5.1 Rust puro: mapa acción→clave NX, validación de `count` (rango y acciones que
lo admiten), textos estables, puente falso (éxito, entrada segura, sin permiso,
`count` repetido). Wire v6 con fixtures.
5.2 Relay: catálogo con tarjeta, validación antes de delegar, tarjeta con el
resumen correcto, sin burbuja, sin log de contenido (no hay contenido), test de
una tarjeta por llamada y cero con entrada inválida.
5.3 Frontend: órbita `hefesto_media` → «Hefesto · Multimedia».
5.4 Contratos y registro.
5.5 En vivo: subir/bajar volumen dos pasos y silenciar (el usuario lo nota),
brillo un paso, play/pausa en una app de música si hay una abierta; matriz de
errores de validación por HTTP.

## 6. Riesgos

- Postear teclas de sistema requiere el mismo permiso de Accesibilidad que el
  teclado: ya concedido; si se revoca, el error estable lo explica.
- Las teclas de sistema no devuelven estado: un «subido» no garantiza que el
  audio estuviera activo o que el brillo cambie (p. ej. pantalla externa).
- `count` alto (10) en brillo puede llevar al mínimo/máximo: aceptado, es lo
  mismo que mantener pulsada la tecla.
