# Plan de limpieza de pruebas y documentación obsoleta

> **Estado:** aprobado y ejecutado el 2026-11-02. Se borraron los archivos
> efímeros de `/tmp` y del home, `.superpowers/`, los `__pycache__/` y las
> specs/planes antiguos; se conservaron las de teclado, multimedia y ventanas.
> No se ejecutó `cargo clean` y se conservaron `hefesto-control`/`hefesto-echo`.
> Lo dudoso quedó en `/private/tmp/_revision/`.

**Objetivo:** retirar los archivos de prueba efímeros de las entregas de Hefesto
y decidir qué documentación de planificación ya no hace falta, sin tocar la
identidad mTLS, las apps instaladas, los LaunchAgents del usuario ni las suites
automatizadas.

**Criterio de seguridad:** primero inventario y confirmación; después borrado por
ruta exacta. Nada de `rm -rf` sobre rutas amplias, `pkill` globales ni borrado de
archivos que no hayamos creado en estas pruebas. Lo que no esté claro, se mueve
a una carpeta `_revision/` en vez de borrarse.

---

## 1. Archivos de prueba fuera del repo (`/private/tmp`)

Creados por las pruebas en vivo de lectura, escritura, teclado, multimedia y
ventanas. Propuesta: **borrar** salvo lo marcado como conservar.

| Ruta | Tamaño / fecha | Propuesta |
| --- | --- | --- |
| `hefesto_live.py` | 2 KB, 2026-11-01 | Conservar hasta la revisión: es el driver de las pruebas en vivo |
| `hefesto-kbd.txt` | 22 B, 2026-11-01 | Borrar (texto de la prueba de teclado) |
| `kbd_base` | 2 B, 2026-11-01 | Borrar (estado auxiliar de la prueba) |
| `secure_dialog.log` | 0 B, 2026-11-01 | Borrar |
| `atlas-sample.txt` | 274 KB, 2026-10-30 | Borrar (muestra de lectura) |
| `atlas-sample2.txt` | 281 KB, 2026-10-30 | Borrar (muestra de lectura) |
| `x` | 14 MB, Mach-O arm64 | Borrar (binario auxiliar de prueba) |
| `atlas-charge-fix.sh`, `atlas-smc-read.sh` | scripts, 2026-10-30 | Revisar antes: pueden ser diagnósticos puntuales; borrar si no se reutilizan |
| `atlas_allow.py`, `atlas_bubble_test.py`, `atlas_confirm_and_card.py`, `atlas_trigger_write.py` | scripts, 2026-10-31 | Borrar (drivers de la prueba de burbuja/escritura) |
| `atlas-live-34.py`, `atlas-live-test.py` | scripts, 2026-10-31 | Borrar (drivers de pruebas anteriores) |
| `atlas_seq.env` | 7 B, 2026-11-01 | **Inspeccionar antes de borrar**: podría contener un identificador o credencial de prueba |
| `atlas.log`, `atlas-test.log`, `atlas-check.log`, `atlas-clippy.log`, `clippy-atlas-mac.log`, `clippy-m3.log` | logs | Borrar |
| `atlas-t5-suite.log`, `atlas-task4-workspace*.log`, `atlas-tauri-build*.log`, `atlas-tauri-dev.log`, `atlas-vite*.log`, `atlas-relaunch.log`, `atlas-launch.log` | logs | Borrar |
| `atlas-diag.log`, `atlas-diag.pid`, `atlas-vite.pid` | diagnóstico antiguo | Borrar tras comprobar que no hay proceso vivo con esos PIDs |

**No tocar:** `/private/tmp/powerlog` (es de root y no es nuestro), archivos de
otros usuarios y cualquier ruta fuera de esta lista.

## 2. Archivos de prueba en el home

| Ruta | Propuesta |
| --- | --- |
| `~/Documents/atlas-prueba-lectura.txt` (56 B) | Borrar (LUNA-42 de la prueba de lectura) |
| `~/Documents/atlas-nota-hefesto-2.txt` (17 B) | Borrar (nota de la prueba de escritura) |
| `~/atlas-hefesto/__pycache__/` | Borrar (caché de los helpers de prueba) |
| `~/atlas-hefesto/hefesto-control` (3 KB) | Decidir: conservar como herramienta manual o borrar; no es necesario para producción |
| `~/atlas-hefesto/hefesto-echo` (6 KB) | Decidir: conservar como herramienta manual o borrar |

**No tocar:** `~/atlas-hefesto/{ca,controller,device,server}.pem|key`,
`peers.json`, `ca.cnf` y `.ext` (identidad mTLS del dispositivo y del
controlador), `/Applications/Atlas.app`, `/Applications/Hub Panel.app` y
`~/Library/LaunchAgents/Hub Panel.plist` (app del usuario con autoarranque
pedido).

## 3. Artefactos ignorados del repo

| Ruta | Propuesta |
| --- | --- |
| `.superpowers/` (2 MB) | Borrar entero: ledgers, briefs, informes y diffs de las tandas SDD; es scratch y está ignorado por Git |
| `server/__pycache__/` y demás `__pycache__/` | Borrar |
| `target/` (16 GB) | Opcional: `cargo clean` para recuperar disco. No es un archivo de prueba y obliga a recompilar; decidir aparte |
| `node_modules/`, `dist/`, `apps/panel/dist/` | **No borrar**: son dependencias y el panel web versionado que se despliega |

## 4. Documentación candidata a retirar

Las specs y planes de las tandas ya cerradas son 16 archivos versionados. Su
resultado y sus commits ya están resumidos en `docs/implemented.md` y en
`docs/hefesto-plan.md` §8, y el historial de Git los conserva aunque se borren.

**Candidatos a borrar** (tras actualizar los enlaces de «Documentación
relacionada» en `implemented.md` y revisar que `docs/index.md` no los enlace):

- `docs/superpowers/specs/2026-10-29-hefesto-delegacion-design.md` y su plan
- `docs/superpowers/specs/2026-10-30-hefesto-entrega-2-design.md` y su plan
- `docs/superpowers/specs/2026-10-30-hefesto-lecturas-design.md` y su plan
- `docs/superpowers/specs/2026-10-30-hefesto-escritura-design.md` y su plan
- `docs/superpowers/specs/2026-10-31-hefesto-apps-design.md` y su plan
- `docs/superpowers/specs/2026-11-01-hefesto-teclado-design.md` y su plan
- `docs/superpowers/specs/2026-11-01-hefesto-multimedia-design.md` y su plan
- `docs/superpowers/specs/2026-11-01-hefesto-ventanas-design.md` y su plan

**Recomendación:** borrar primero las tandas antiguas (delegación, entrega 2,
lecturas, escritura, apps) y conservar teclado, multimedia y ventanas hasta que
el usuario revise esta propuesta; son las más recientes y contienen la evidencia
en vivo más detallada.

**Conservar:** `docs/index.md`, `docs/contracts.md`, `docs/hefesto-plan.md`,
`docs/implemented.md`, `docs/implementation-plan.md`, `docs/relay.md`,
`docs/relay-api.md`, `docs/panel-app-plan.md`, `docs/security.md`,
`docs/tools.md`, `docs/release.md`, `docs/architecture.md`.

## 5. Suites de pruebas que **no** se tocan

- `server/test_hefesto*.py` (journal, delegación e integración).
- `crates/atlas-mac/tests/*` y pruebas unitarias de los crates.
- `src/**/*.test.ts(x)` y pruebas del panel.
- `scripts/deploy-*.sh` y las comprobaciones de salud.

Son la red de seguridad del producto, no archivos efímeros.

## 6. Orden de ejecución propuesto (solo tras aprobación)

1. Comprobar que no queda ningún proceso de prueba (`pgrep` de `Atlas.app`,
   `TextEdit`, `Safari`, Vite/Tauri y los drivers de `/tmp`) y que
   `git status --short` no tiene cambios ajenos.
2. Borrar por ruta exacta los archivos de §1 y §2 confirmados; mover a
   `_revision/` lo dudoso (`atlas_seq.env`, `atlas-charge-fix.sh`,
   `atlas-smc-read.sh`, `hefesto-control`, `hefesto-echo`).
3. Borrar `.superpowers/` y los `__pycache__/`.
4. Decidir `target/` aparte (`cargo clean` opcional).
5. Documentación: actualizar enlaces, borrar los candidatos aprobados y
   comprobar que `docs/index.md` sigue coherente.
6. Verificación: `cargo test --workspace`, Python, frontend y `npm run build`;
   `git status --short`; revisar que `/v1/health` sigue OK y que las apps
   instaladas arrancan.
7. Commit único de limpieza, con permiso explícito, y push si se autoriza.

## 7. Decisiones tomadas al ejecutar

1. Se conservan las specs/planes de teclado, multimedia y ventanas; se borraron
   las cinco tandas antiguas.
2. Se conservan `hefesto-control` y `hefesto-echo` para pruebas manuales.
3. No se ejecutó `cargo clean`; `target/` sigue con 16 GB. Se puede limpiar
   aparte cuando no haya una compilación en curso.
4. `atlas_seq.env`, `atlas-charge-fix.sh` y `atlas-smc-read.sh` se movieron sin
   abrirlos a `/private/tmp/_revision/` para que el usuario decida.

## 8. Trabajo pendiente de Hefesto (fuera de esta limpieza)

- **Puerta 6 — terminal de consulta:** adaptadores cerrados, presupuestos de
  tiempo/salida y supervisión de procesos. Requiere una spec y un plan nuevos;
  no se ha empezado.
- **Capturas de la puerta 7:** dependen del permiso TCC de grabación de pantalla
  y de elegir el modelo de visión y el consentimiento de envío; siguen
  explícitamente fuera de alcance hasta que el usuario lo decida.
