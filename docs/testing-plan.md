# Plan de pruebas de herramientas (Atlas + Hefesto)

Fecha: 2026-11-04. Objetivo: probar cada herramienta de forma exhaustiva,
mezclando escenarios cotidianos, y arreglar en caliente lo que falle.

## Método

- Línea base: `cargo test --workspace`, suite Python, `npm test`, `npm run lint`,
  `npm run build`.
- En vivo contra el relay desplegado: `POST /v1/send` con frase natural,
  `GET /v1/events?since=N` para observar llamadas y confirmaciones, y
  `POST /v1/confirm {requestId, decision}` para decidir cada tarjeta.
- Cada herramienta: camino feliz, entradas inválidas/vacías/canceladas, tarjeta
  (cuando aplique), privacidad y una mezcla con otras herramientas.
- Al terminar: auditoría de procesos (sin huérfanos) y registro en
  `docs/implemented.md`.

## Por herramienta

1. **echo** — texto ≤256 B; sin tarjeta; texto vacío/inválido rechazado.
2. **list_dir / read_file** — dentro de raíces; fuera de raíces rechazado;
   binario rechazado; tarjeta de lectura; salida ≤16 KiB.
3. **write_file** — crear sin pisar; sobrescribir con hash; fuera de raíces;
   contenido nunca en log ni journal.
4. **rename** — mover sin pisar; destino existente; fuera de raíces; mismo volumen.
5. **apps** — open/close/list; alias localizado («Buscar»); app inexistente;
   cierre suave; sin tarjeta.
6. **keyboard** — type y combo; tarjeta siempre; texto sin persisistir.
7. **media** — volumen/subir/bajar/silencio/brillo/reproducción; `count` 1–10;
   restauración del estado.
8. **windows** — list/move/resize/focus; ambigüedad; claves cerradas por acción.
9. **system** — os/disk/uptime/memory; sin tarjeta; acción inválida.
10. **terminal** — comando válido; programa fuera de raíces; denegado por nombre;
    timeout; sin shell.
11. **capture** — tarjeta y rechazo estable por falta de TCC de Grabación de
    pantalla (la ruta OCR real exige conceder el permiso).
12. **comandos de chat** — `/help`, `/log`, `/clear`.
13. **calendar_read/write** — requiere permiso explícito por prueba contra iCloud
    (throttle de Apple); fuera de esta tanda salvo petición expresa.
14. **Mezclas** — leer+escribir+renombrar; apps+keyboard (abrir TextEdit y
    teclear); windows+system; terminal+echo.

## Criterio de éxito

Ninguna herramienta devuelve un fallo inesperado; las tarjetas aparecen solo
cuando corresponde; los datos sensibles no salen en logs; y no quedan procesos
ni apps de prueba abiertas al final.
