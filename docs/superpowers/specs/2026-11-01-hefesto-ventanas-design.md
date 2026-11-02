# Ventanas de Hefesto (puerta 7, corte 3) — Diseño

**Fecha:** 2026-11-01
**Estado:** diseño decidido por el agente bajo mandato del usuario («arranca multimedia
y luego ventanas y todo lo que quede pendiente sin parar hasta acabar»)
**Antecedentes:** [control de apps](2026-10-31-hefesto-apps-design.md), [teclado](2026-11-01-hefesto-teclado-design.md), [multimedia](2026-11-01-hefesto-multimedia-design.md)

## 1. Contexto y objetivo

Hefesto abre/cierra apps (v4), teclea (v5) y controla multimedia (v6). El corte 3
de la puerta 7 pide **ventanas**: listar las ventanas de las apps en ejecución,
moverlas, redimensionarlas y traerlas al frente. La vía pública es la **API de
Accesibilidad** (`AXUIElement`, ApplicationServices), que no necesita subprocess ni
API privada y comparte el permiso TCC de Accesibilidad ya concedido a `Atlas.app`.

## 2. Decisiones de diseño

1. **Una sola herramienta** `hefesto_windows` con `action ∈ {list, move, resize,
   focus}`. Listar es lectura; mover/redimensionar/traer son efectos.
2. **Objetivo por app + ventana:** `app` (nombre o bundleId, obligatorio salvo en
   `list`) y, opcionalmente, `title` (coincidencia exacta y luego contiene, sin
   distinguir mayúsculas) e `index` (1-based entre las ventanas estándar de la app).
   Sin `title`/`index`, se usa la ventana principal (`AXMain`) o la primera estándar.
   Si hay varias coincidencias por título → error estable, nunca se elige una al azar.
3. **Solo ventanas estándar** (`AXWindow` + `AXStandardWindow`): se excluyen
   diálogos, hojas, popovers y ventanas de apps sin interfaz. Máximo 30 en `list`.
4. **Coordenadas absolutas** en puntos, origen arriba-izquierda (el de AX). `move`
   exige `x`,`y`; `resize` exige `width`,`height` (100–10000). El puente **acota**
   la posición al rectángulo que une todas las pantallas (`NSScreen`) para que la
   barra de título quede alcanzable; el tamaño se acota a 100–10000.
5. **Traer al frente** = `AXRaise` + desminimizar + foco de ventana + activar la app
   con `NSRunningApplication` (API pública). No cambia tamaño ni posición.
6. **Tarjeta condicional:** `list` no abre tarjeta (lectura, como `hefesto_apps
   list`); `move`, `resize` y `focus` abren **exactamente una** tarjeta por llamada
   válida y **ninguna** con entrada inválida (`requiresConfirmation: True` +
   `planConfirmation: True`, con la tarjeta dentro de `_windows`). Sin burbuja
   `agent: hefesto`: la tarjeta es la superficie de consentimiento y el modelo
   reporta el resultado.
7. **Permiso:** `AXIsProcessTrusted()` antes de cualquier operación. Sin permiso →
   texto estable y nada se toca. No se abre un diálogo nuevo: el permiso ya se pidió
   y concedió para el teclado; si se revoca, el texto lo explica.
8. **Hilo principal y timeout:** AppKit (`NSWorkspace`, `NSScreen`) y AX se reenvían
   al hilo principal con el `run_on_main_thread` + canal ya usado por apps/teclado,
   con `AXUIElementSetMessagingTimeout(2 s)` para que una app colgada no bloquee.
9. **Journal:** `hefesto.windows` es efecto **sin contenido** (solo acción, objetivo
   y geometría). Por seguridad, cualquier invocación en vuelo —incluido `list`— queda
   `unknown` tras un reinicio y no se reentrega.
10. **Sin capturas, sin visión, sin AppleScript ni shell.**

## 3. Requisitos

### 3.1 Protocolo y wire

1. `protocolVersion` sube de **6 a 7** en los dos extremos y en todos los fixtures;
   ambos lados rechazan versiones distintas.
2. `toolId` del dispositivo: **`hefesto.windows`** con
   `WindowsPayload { action, app?, title?, index?, x?, y?, width?, height? }`.
   El `action` del **wire** usa valores distintos de los de `hefesto.apps` para que
   el enum `untagged` no confunda `{"action":"list"}` con una app:
   `list_windows`, `move_window`, `resize_window` y `focus_window`. El relay traduce
   las acciones del modelo (`list`, `move`, `resize`, `focus`) a esos valores.
3. Salida: `{windows:[{app, title, x, y, width, height, focused, minimized}]}` para
   `list` (≤30) y `{text}` con el resumen para los efectos.
4. El relay expone **una** herramienta `hefesto_windows` con `action` enum,
   `requiresConfirmation: True` + `planConfirmation: True`.

### 3.2 Comportamiento

5. `list` enumera las ventanas estándar de las apps regulares en ejecución (o solo
   las de `app` si se indica), con título, posición, tamaño, foco y minimizado.
6. `move` fija `(x,y)`; `resize` fija `(width,height)`; ambos conservan el resto y
   no activan la app. `focus` trae la ventana al frente y desminimiza.
7. Validación estable antes de tocar nada: acción, app, título, índice, coordenadas.

### 3.3 Errores (texto estable)

8. `action inválida (list, move, resize o focus)`.
9. `falta app` (salvo en `list`).
10. `index fuera de rango (1–99)`; `x/y fuera de rango (-20000–20000)`;
    `width/height fuera de rango (100–10000)`.
11. `falta el permiso de Accesibilidad; actívalo en Ajustes del Sistema › Privacidad
    y seguridad › Accesibilidad`.
12. `«X» no está abierta`; `no encuentro una ventana de «X»`; `hay N ventanas de «X»
    que coinciden; concreta el título o el índice`.
13. Fallo del puente: `no se pudo {mover|redimensionar|traer} la ventana: {err}`.
14. Timeout/cancelación/reinicio: ruta común (`unknown` sin reejecutar).

### 3.4 Persistencia y privacidad

15. Journal: claves cerradas por acción, tipos estrictos, sin `_contents`; en vuelo
    tras reinicio → `unknown`. Los títulos de ventana son datos del usuario: se
    guardan en la submission (necesarios para el comando), no se registran en logs
    más allá de la tarjeta y no se emiten en burbujas.
16. Logs: se registra la acción y el objetivo recortado, nunca contenido de la app.

## 4. Fuera de alcance

- Cerrar ventanas (destructivo; tendría su propio corte con confirmación).
- Minimizar, maximizar, pantalla completa, mosaico o zonas.
- Orden Z más allá de traer al frente.
- Espacios de trabajo, Mission Control y múltiples pantallas más allá del acotado.
- Leer el contenido de la ventana, capturas o OCR.

## 5. Pruebas

5.1 Rust puro: validación por acción, rangos, textos estables, resolución de
objetivo (título exacto/contiene/ambiguo/índice) y puente falso (éxito, sin permiso,
sin app, sin ventana, error del puente). Wire v7 con fixtures.
5.2 Executor: `hefesto.windows` se enruta al `WindowBridge`; sin puente → texto.
5.3 Journal: submit/next/result, validación por acción, `unknown` tras reinicio.
5.4 Relay: catálogo, `list` sin tarjeta, efectos con una tarjeta y ninguna con
entrada inválida, resumen, sin burbuja.
5.5 Frontend: órbita `hefesto_windows` → «Hefesto · Ventanas».
5.6 En vivo (requiere pantalla desbloqueada): listar ventanas reales de TextEdit,
mover a una posición, redimensionar, traer al frente desde otra app, ambigüedad por
título y matriz de errores de validación por HTTP.

## 6. Riesgos

- La API de Accesibilidad puede no responder si la app objetivo está colgada: el
  timeout de mensajes de 2 s lo acota y devuelve error estable.
- Las apps sin permiso o con ventanas no estándar no aparecen; se documenta.
- En pantalla bloqueada `loginwindow` retiene la entrada segura y el foco; las
  pruebas en vivo de ventanas deben hacerse con la sesión desbloqueada.
