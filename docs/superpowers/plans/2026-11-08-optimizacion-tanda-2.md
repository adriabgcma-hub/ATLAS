# Optimización tanda 2 — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Aplicar las seis optimizaciones de la tanda 2 acordadas con el usuario: workspaces de npm, deduplicar tests Python, single-flight + TTL en CalDAV, puerta temporal al sweep de Hefesto, extraer el bloque Hefesto de `lib.rs` y quitar `rustls-tls` del panel.

**Architecture:** Cada optimización es independiente y se entrega en su propio commit con su propio ciclo de verificación. Las tres primeras (workspaces, tests, CalDAV) no cambian comportamiento observable; la puerta del sweep retrasa el barrido de vencimientos como mucho 5 s (aceptable: los plazos de Hefesto son de 15 s o más); las dos de Rust son movimientos/recortes sin cambio de comportamiento.

**Tech Stack:** npm 11 (workspaces), Vite 8, Python 3 (unittest, stdlib), Rust/Tauri 2.11 (cargo workspace en la raíz: `src-tauri`, `crates/*`, `apps/panel-app/src-tauri`), relay en `atlas-hub` (systemd `atlas-relay`).

**Spec:** sin documento aparte; los requisitos son los seis puntos de la tanda 2 (ver más abajo), tal como quedaron acordados con el usuario el 2026-11-08. Este plan los concreta con archivos, código y pruebas.

## Global Constraints

- Nada de secretos en el repo; `.env` nunca se commitea (`git status` limpio antes de cada commit salvo los archivos de la tarea).
- Los servidores de dev se lanzan solo mientras hacen falta y se matan al terminar (matar por PID propio, nunca `pkill`).
- Commit solo con permiso explícito del usuario; cuando se concede, push inmediato al remoto.
- Los cambios en `server/` se despliegan con `./scripts/deploy-relay.sh` (tras commit) y se comprueba `/v1/health`.
- Los cambios que tocan el binario macOS exigen regenerar el bundle con la identidad **Apple Development** y verificar la firma profunda (`codesign --verify --deep --strict --verbose=2`).
- Suites de referencia (deben quedar igual o mejor): Rust `cargo test --workspace` (211 + 2 ignoradas), `cargo clippy --workspace --all-targets -- -D warnings` y `cargo fmt --check` limpios; Python `python3 -m unittest discover -s . -p 'test_*.py'` todo verde (598 ejecuciones hoy; bajará el número total tras la tarea 2 sin perder cobertura); frontend `npm run build`, `npm run lint`, `npm run test` (61 tests) limpios.
- No ejecutar `npm run build` dentro de `apps/panel` (regeneraría `apps/panel/dist`, que está versionado); usar `npm run build:ui` dentro de `apps/panel-app` (escribe en `apps/panel-app/dist`, ignorado).
- No ejecutar las pruebas Rust marcadas `#[ignore]` (reinician el relay real o tocan iCloud) sin permiso explícito.
- Cada prueba contra iCloud en vivo requiere permiso explícito del usuario; este plan no incluye ninguna (todo se verifica offline con dobles).

## Requisitos (la tanda 2, uno a uno)

1. **npm workspaces** — deduplicar los `node_modules` repetidos entre la raíz (191 MB), `apps/panel` (67 MB) y `apps/panel-app` (15 MB). Las versiones ya coinciden entre los tres `package.json`.
2. **Deduplicar tests Python** — en `server/test_hefesto.py` la clase base `JournalTests` tiene 38 tests que heredan 9 subclases: 380 ejecuciones para 117 tests únicos. Hay que separar los helpers en una fixture sin tests.
3. **Single-flight + TTL en CalDAV** — `calendar_calendars()` llama a `discover()` (3 PROPFIND contra iCloud) en cada petición. Hay que cachear con TTL y evitar descubrimientos concurrentes duplicados.
4. **Puerta temporal al sweep de Hefesto** — cada `dispatch()` del journal ejecuta `_sweep` (dos UPDATE con COMMIT). Hay que barrer como mucho cada 5 s.
5. **Extraer el bloque Hefesto de `lib.rs`** — el bloque «Hefesto (Fase 4)» (líneas 279–631, ~350 líneas) vive mezclado en `src-tauri/src/lib.rs` (921 líneas); moverlo a `src-tauri/src/hefesto_runtime.rs`.
6. **Quitar `rustls-tls` del reqwest del panel** — el panel solo habla HTTP (`http://atlas-hub.local:8765`); el TLS sobra en sus builds propios (`tauri build` compila solo el panel).

## Estructura de archivos

- `package.json` (raíz) — añade `workspaces`; `package-lock.json` se regenera con `npm install`.
- `server/test_hefesto.py` — restructura de clases (solo tests).
- `server/atlas_relay.py` — `Brain`: caché + lock de calendarios; `calendar_calendars()` reescrito.
- `server/test_hefesto_integration.py` — pruebas nuevas de la caché CalDAV.
- `server/hefesto.py` — `Journal`: puerta del sweep.
- `server/test_hefesto.py` — pruebas nuevas de la puerta.
- `src-tauri/src/hefesto_runtime.rs` (nuevo) + `src-tauri/src/lib.rs` (adelgazado).
- `apps/panel-app/src-tauri/Cargo.toml` — `reqwest` sin `rustls-tls`.
- `docs/implemented.md`, `docs/implementation-plan.md`, `docs/panel-app-plan.md` — registro final.

---

### Task 1: npm workspaces

**Files:**
- Modify: `package.json` (raíz)
- Modify: `package-lock.json` (regenerado por `npm install`)

**Interfaces:**
- Consumes: nada (primera tarea).
- Produces: hoisting de dependencias a la raíz; los scripts existentes (`npm run dev|build|lint|test` en la raíz, `npm run dev|build` en `apps/panel`, `npm run dev:ui|build:ui|tauri` en `apps/panel-app`) siguen funcionando sin cambios.

- [ ] **Step 1: Añadir el campo `workspaces`**

En `package.json` (raíz), tras `"type": "module",` añadir:

```json
  "workspaces": [
    "apps/panel",
    "apps/panel-app"
  ],
```

- [ ] **Step 2: Reinstalar con hoisting**

Run: `cd /Users/adria/Developer/project-1/atlas-next && npm install`
Expected: termina sin errores; npm mueve las dependencias a `node_modules` de la raíz.

- [ ] **Step 3: Comprobar la deduplicación**

Run: `du -sh node_modules apps/panel/node_modules apps/panel-app/node_modules`
Expected: `apps/panel/node_modules` y `apps/panel-app/node_modules` bajan a menos de 1 MB cada uno (quedan solo enlaces/binarios locales); la raíz crece poco (ya tenía casi todo).

- [ ] **Step 4: Verificar las suites del frontend**

Run:
```bash
npm run build && npm run lint && npm run test
```
Expected: build OK, lint sin errores, 61 tests pasan.

- [ ] **Step 5: Verificar el pipeline del panel**

Run: `cd apps/panel-app && npm run build:ui`
Expected: build OK y la salida queda en `apps/panel-app/dist` (ignorado por Git). **No** ejecutar `npm run build` dentro de `apps/panel`.

- [ ] **Step 6: Commit**

```bash
git add package.json package-lock.json
git commit -m "chore: npm workspaces para deduplicar node_modules entre raíz y panel"
```

---

### Task 2: Deduplicar los tests del journal

**Files:**
- Modify: `server/test_hefesto.py:53-99` (clase base) y las 9 líneas de subclase

**Interfaces:**
- Consumes: nada.
- Produces: `JournalFixture` (mixin con `setUp`, `new_journal`, `call`, `ok`, `error`, `ready`, `send`, `next`, `result_fields`, `restart`); `JournalTests(JournalFixture, unittest.TestCase)` conserva los 38 tests del ciclo de vida; las 9 subclases de herramienta heredan de `JournalFixture` y conservan solo sus tests propios.

- [ ] **Step 1: Convertir la base en fixture sin tests**

En `server/test_hefesto.py`, cambiar la declaración de la línea 53:

```python
class JournalTests(unittest.TestCase):
```

por:

```python
class JournalFixture:
```

y cerrar la clase justo después de `restart()` (tras la línea 99, antes de `test_full_echo_and_exact_response_contract`) añadiendo la clase que conserva los 38 tests:

```python
class JournalTests(JournalFixture, unittest.TestCase):
    pass
```

- [ ] **Step 2: Reenraizar las 9 subclases**

Cambiar cada una de estas declaraciones (de `(JournalTests)` a `(JournalFixture, unittest.TestCase)`):

- línea 609: `class ReadToolTests(JournalTests):`
- línea 676: `class WriteJournalTests(JournalTests):`
- línea 762: `class AppControlJournalTests(JournalTests):`
- línea 925: `class KeyboardJournalTests(JournalTests):`
- línea 993: `class MediaJournalTests(JournalTests):`
- línea 1067: `class WindowsJournalTests(JournalTests):`
- línea 1259: `class SystemJournalTests(JournalTests):`
- línea 1315: `class TerminalJournalTests(JournalTests):`
- línea 1364: `class CaptureJournalTests(JournalTests):`

- [ ] **Step 3: Verificar el recuento**

Run: `python3 -m unittest test_hefesto -v 2>&1 | tail -3`
Expected: `Ran 117 tests` (antes eran 380) y `OK`.

- [ ] **Step 4: Verificar la suite completa**

Run: `python3 -m unittest discover -s . -p 'test_*.py' 2>&1 | tail -3`
Expected: `OK`; el número total baja ~263 (de 598 a ~335) porque desaparecen las ejecuciones redundantes, sin perder ningún test único.

- [ ] **Step 5: Commit**

```bash
git add server/test_hefesto.py
git commit -m "test: deduplica los tests del journal (fixture sin herencia redundante)"
```

---

### Task 3: Single-flight y TTL en el descubrimiento CalDAV

**Files:**
- Modify: `server/atlas_relay.py:76` (constante), `server/atlas_relay.py:748-752` (init de `Brain`), `server/atlas_relay.py:915-921` (`calendar_calendars`)
- Test: `server/test_hefesto_integration.py` (clase nueva `CalendarCacheTests`)

**Interfaces:**
- Consumes: `Brain.__init__` actual; `atlas_caldav.CalDAVClient.discover() -> list[dict]` (sin cambios).
- Produces: `Brain.calendar_calendars() -> list[dict]` con caché TTL de 300 s y ejecución en serie de los descubrimientos (`self._cal_lock`); en error se descarta la caché y se propaga `CalDAVError`.

- [ ] **Step 1: Escribir los tests que fallan**

Añadir a `server/test_hefesto_integration.py` (al final, tras `AtlasRegressionTests`) esta clase. El archivo ya importa `tempfile`, `threading`, `mock.patch`, `atlas_relay`; añadir `import time` arriba si no está.

```python
class CalendarCacheTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        with patch.dict("os.environ", {}, clear=True):
            cfg = atlas_relay.read_env()
        cfg["data_dir"] = temporary.name
        self.brain = atlas_relay.Brain(cfg)

    def test_calendar_calendars_reutiliza_el_descubrimiento_dentro_del_ttl(self):
        calls = []

        class FakeClient:
            def discover(self):
                calls.append(1)
                return [{"id": "/cal/1", "name": "Prueba"}]

        with patch.object(self.brain, "_caldav_client", return_value=FakeClient()):
            first = self.brain.calendar_calendars()
            second = self.brain.calendar_calendars()
        self.assertEqual(first, [{"id": "/cal/1", "name": "Prueba"}])
        self.assertEqual(second, first)
        self.assertEqual(len(calls), 1)

    def test_calendar_calendars_redescubre_tras_el_ttl(self):
        calls = []
        clock = {"t": 0.0}

        class FakeClient:
            def discover(self):
                calls.append(1)
                return [{"id": "/cal/1", "name": "Prueba"}]

        with patch.object(self.brain, "_caldav_client", return_value=FakeClient()), \
             patch.object(atlas_relay.time, "monotonic", side_effect=lambda: clock["t"]):
            self.brain.calendar_calendars()
            clock["t"] = atlas_relay.CAL_CALENDARS_TTL + 1
            self.brain.calendar_calendars()
        self.assertEqual(len(calls), 2)

    def test_calendar_calendars_descarta_la_cache_tras_un_error(self):
        calls = []

        class FakeClient:
            def discover(self):
                calls.append(1)
                if len(calls) == 1:
                    return [{"id": "/cal/1", "name": "Prueba"}]
                raise atlas_caldav.CalDAVError("http_500")

        with patch.object(self.brain, "_caldav_client", return_value=FakeClient()):
            self.brain.calendar_calendars()
            with self.assertRaises(atlas_caldav.CalDAVError):
                self.brain.calendar_calendars()
            self.brain.calendar_calendars()
        self.assertEqual(len(calls), 3)

    def test_calendar_calendars_un_solo_descubrimiento_concurrente(self):
        calls = []

        class FakeClient:
            def discover(self):
                calls.append(1)
                time.sleep(0.05)
                return [{"id": "/cal/1", "name": "Prueba"}]

        results = [None, None, None]

        def worker(index):
            results[index] = self.brain.calendar_calendars()

        threads = [threading.Thread(target=worker, args=(i,)) for i in range(3)]
        with patch.object(self.brain, "_caldav_client", return_value=FakeClient()):
            for thread in threads:
                thread.start()
            for thread in threads:
                thread.join()
        self.assertEqual(len(calls), 1)
        self.assertTrue(all(r == [{"id": "/cal/1", "name": "Prueba"}]
                            for r in results))
```

Nota: la prueba del error es de caracterización (pasa antes y después); las otras tres deben fallar ahora.

- [ ] **Step 2: Verificar que fallan (RED)**

Run: `python3 -m unittest test_hefesto_integration.CalendarCacheTests -v 2>&1 | tail -12`
Expected: `test_calendar_calendars_reutiliza_el_descubrimiento_dentro_del_ttl`, `test_calendar_calendars_redescubre_tras_el_ttl` y `test_calendar_calendars_un_solo_descubrimiento_concurrente` FAIL (2 y 3 llamadas respectivamente); `test_calendar_calendars_descarta_la_cache_tras_un_error` OK.

- [ ] **Step 3: Implementar caché + lock**

En `server/atlas_relay.py`, junto a `DEFAULT_TIMEZONE = "Europe/Madrid"` (línea 76), añadir:

```python
CAL_CALENDARS_TTL = 300.0
```

En `Brain.__init__`, junto a `self._cal_cache: dict = {}` (línea 750), añadir:

```python
        self._cal_cache: dict = {}
        self._cal_cal_cache: tuple[float, list[dict]] | None = None
        self._cal_lock = threading.Lock()
```

Sustituir `calendar_calendars` (líneas 915-921) por:

```python
    def calendar_calendars(self) -> list[dict]:
        # TTL corto + ejecución en serie: dos peticiones a la vez no duplican
        # los PROPFIND contra iCloud (Apple penaliza las ráfagas).
        with self.lock:
            cached = self._cal_cal_cache
            if cached and time.monotonic() - cached[0] < CAL_CALENDARS_TTL:
                return cached[1]
        with self._cal_lock:
            with self.lock:
                cached = self._cal_cal_cache
                if cached and time.monotonic() - cached[0] < CAL_CALENDARS_TTL:
                    return cached[1]
            try:
                calendars = self._caldav_client().discover()
            except atlas_caldav.CalDAVError:
                with self.lock:
                    self._cal_cal_cache = None
                    self._caldav = None  # redescubrir la próxima vez
                raise
            with self.lock:
                self._cal_cal_cache = (time.monotonic(), calendars)
            return calendars
```

- [ ] **Step 4: Verificar que pasan (GREEN)**

Run: `python3 -m unittest test_hefesto_integration.CalendarCacheTests -v 2>&1 | tail -8`
Expected: 4 OK.

- [ ] **Step 5: Verificar la suite completa**

Run: `python3 -m unittest discover -s . -p 'test_*.py' 2>&1 | tail -3`
Expected: `OK`.

- [ ] **Step 6: Commit**

```bash
git add server/atlas_relay.py server/test_hefesto_integration.py
git commit -m "perf(relay): single-flight y caché TTL para el descubrimiento CalDAV"
```

---

### Task 4: Puerta temporal al sweep del journal

**Files:**
- Modify: `server/hefesto.py:20` (constante), `server/hefesto.py:457-458` (`Journal.__init__`), `server/hefesto.py:509-513` (`dispatch`)
- Test: `server/test_hefesto.py` (clase `JournalTests`, que tras la Task 2 se ejecuta una sola vez)

**Interfaces:**
- Consumes: `Journal` actual; `INVOCATION_SECONDS = 15` (plazo del eco); `Clock` del test (arranca en `mono=100.0`).
- Produces: `Journal._sweep(now, wall)` se ejecuta como mucho cada `SWEEP_MIN_INTERVAL_S = 5.0` segundos monótonos; el primer `dispatch` de un journal nuevo siempre barre. Los vencimientos se retrasan como mucho 5 s (los plazos de Hefesto son de 15 s o más).

- [ ] **Step 1: Escribir el test que falla**

En `server/test_hefesto.py`, dentro de `JournalTests` (la base), junto a los demás tests de vencimiento (línea ~358), añadir:

```python
    def test_el_barrido_se_retrasa_hasta_la_siguiente_ventana(self):
        self.ready()                                        # t=100: primer barrido
        self.send()                                         # t=100: pendiente, vence en t+15
        self.clock.advance(hefesto.INVOCATION_SECONDS - 2)  # t=113: barrido #2 (13>=5)
        self.ok("status")
        self.clock.advance(2)                               # t=115: el plazo ya venció
        self.assertIsNotNone(self.next())                   # puerta cerrada (2<5): sigue pendiente
        self.clock.advance(5)                               # t=120: ventana abierta
        self.ok("status")                                   # barrido #3: dispatched vencida
        self.assertIsNone(self.next())                      # -> unknown, no se entrega
```

- [ ] **Step 2: Verificar que falla (RED)**

Run: `python3 -m unittest test_hefesto.JournalTests.test_el_barrido_se_retrasa_hasta_la_siguiente_ventana -v`
Expected: FAIL en `self.assertIsNotNone(self.next())` (hoy cada dispatch barre y el eco vencido expira en t=115).

- [ ] **Step 3: Implementar la puerta**

En `server/hefesto.py`, junto a `REQUEST_DEADLINE = 5.0` (línea 23), añadir:

```python
SWEEP_MIN_INTERVAL_S = 5.0
```

En `Journal.__init__` (tras `self._lock = threading.RLock()`, línea 458), añadir:

```python
        self._last_sweep = float("-inf")  # el primer dispatch siempre barre
```

En `dispatch`, sustituir las líneas 510-512:

```python
                self._db.execute("BEGIN IMMEDIATE")
                now, wall = self._monotonic(), self._wall_time()
                self._sweep(now, wall)
```

por:

```python
                self._db.execute("BEGIN IMMEDIATE")
                now, wall = self._monotonic(), self._wall_time()
                if now - self._last_sweep >= SWEEP_MIN_INTERVAL_S:
                    self._sweep(now, wall)
                    self._last_sweep = now
```

- [ ] **Step 4: Verificar que pasa (GREEN)**

Run: `python3 -m unittest test_hefesto.JournalTests.test_el_barrido_se_retrasa_hasta_la_siguiente_ventana -v`
Expected: OK.

- [ ] **Step 5: Añadir la guarda del primer barrido**

En `JournalTests`, añadir:

```python
    def test_tras_reiniciar_el_primer_dispatch_barre_los_vencidos(self):
        self.ready()
        self.send()                                        # pendiente, vence en t+15
        self.restart()                                     # journal nuevo: _last_sweep=-inf
        self.clock.advance(hefesto.INVOCATION_SECONDS + 1)
        self.ok("status")                                  # primer dispatch del nuevo
        self.assertIsNone(self.next())
```

Run: `python3 -m unittest test_hefesto.JournalTests.test_tras_reiniciar_el_primer_dispatch_barre_los_vencidos -v`
Expected: OK (es de caracterización: protege el `-inf` inicial frente a un `0` futuro).

- [ ] **Step 6: Auditar la suite completa**

Run: `python3 -m unittest discover -s . -p 'test_*.py' 2>&1 | tail -5`
Expected: `OK`. Si algún test de vencimiento falla porque avanza el reloj menos de `SWEEP_MIN_INTERVAL_S`, subir ese avance a `hefesto.SWEEP_MIN_INTERVAL_S` (los que avanzan 5 o 15 ya pasan; la comparación es `>=`).

- [ ] **Step 7: Commit**

```bash
git add server/hefesto.py server/test_hefesto.py
git commit -m "perf(hefesto): barrido de vencimientos como mucho cada 5 s"
```

---

### Task 5: Extraer el bloque Hefesto de `lib.rs`

**Files:**
- Create: `src-tauri/src/hefesto_runtime.rs` (contenido movido de `lib.rs:279-631`)
- Modify: `src-tauri/src/lib.rs` (borrar el bloque, añadir `mod` y `use`, ajustar imports)

**Interfaces:**
- Consumes: `EVENT_HEFESTO` (sigue en `lib.rs:73`, ahora referenciada como `crate::EVENT_HEFESTO`), tipos de `atlas_mac` y de `tauri`.
- Produces: `hefesto_runtime::{Hefesto, GateAction, hefesto_gate, hefesto_is_suspended, hefesto_write_suspended, hefesto_clear_suspended, hefesto_status, hefesto_resume, hefesto_abort}` (todo `pub(crate)`); `lib.rs` conserva `run()` con `.manage(Hefesto::new())`, `app.state::<Hefesto>().start(...)` y el `generate_handler!` intacto.

- [ ] **Step 1: Crear `hefesto_runtime.rs`**

Crear `src-tauri/src/hefesto_runtime.rs` con el contenido de `lib.rs:279-631` (del comentario `// ── Hefesto (Fase 4) ──…` hasta `hefesto_abort` inclusive), con estos ajustes:

1. Cabecera de imports (mover desde `lib.rs` lo que solo usa este bloque y añadir lo necesario):
```rust
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};
use atlas_mac::{Clock, HefestoClient, OpaqueId, Phase, Roots, Runtime, Snapshot};
```
(si `cargo check` señala alguno sin usar, quitarlo).

2. En `hefesto_publish`, donde se emite en `EVENT_HEFESTO`, usar `crate::EVENT_HEFESTO`.

3. Visibilidad: `pub(crate)` en `struct Hefesto`, `enum GateAction`, y en las funciones `hefesto_gate`, `hefesto_is_suspended`, `hefesto_write_suspended`, `hefesto_clear_suspended`, `hefesto_status`, `hefesto_resume`, `hefesto_abort`. El resto (constantes, `HefestoEvent`, `HefestoCommand`, `hefesto_transport`, `hefesto_clock`, `hefesto_loop`) queda privado del módulo.

- [ ] **Step 2: Adelgazar `lib.rs`**

1. Tras `mod table;` (línea 42) añadir `mod hefesto_runtime;`.
2. Borrar de `lib.rs` las líneas 279-631 (el bloque movido).
3. Sustituir la línea 13 (`use atlas_mac::{Clock, HefestoClient, OpaqueId, Phase, Roots, Runtime, Snapshot};`) por:
```rust
use atlas_mac::Phase;
```
4. Cerca del uso, añadir:
```rust
use hefesto_runtime::{Hefesto, hefesto_abort, hefesto_resume, hefesto_status};
```
5. En el `mod tests` (línea 726), sustituir las dos líneas del `use super::*` ampliado (729-730) para que los tests de `hefesto_gate`/marcadores apunten al módulo nuevo:
```rust
        crate::hefesto_runtime::{
            hefesto_clear_suspended, hefesto_gate, hefesto_is_suspended,
            hefesto_write_suspended, GateAction,
        },
        Phase, EVENT_CONFIRM, EVENT_HEFESTO, EVENT_MESSAGE, EVENT_STATE,
```

- [ ] **Step 3: Verificar compilación y pruebas**

Run:
```bash
cargo check --workspace && cargo test --workspace 2>&1 | tail -5
```
Expected: check OK; `cargo test --workspace` con el mismo recuento que la línea base (211 + 2 ignoradas), todo verde. Si faltara un import, corregirlo y repetir (es un movimiento, no cambia comportamiento).

- [ ] **Step 4: Clippy y formato**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: limpios.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/hefesto_runtime.rs
git commit -m "refactor(atlas): extrae el runtime de Hefesto de lib.rs a hefesto_runtime.rs"
```

---

### Task 6: Quitar `rustls-tls` del reqwest del panel

**Files:**
- Modify: `apps/panel-app/src-tauri/Cargo.toml:16`
- Modify: `docs/panel-app-plan.md` (nota HTTP-only)

**Interfaces:**
- Consumes: nada.
- Produces: `reqwest` del panel con `default-features = false, features = ["json"]` — solo HTTP plano. El panel solo llama a `relay_url()` (por defecto `http://atlas-hub.local:8765`); un `ATLAS_RELAY_URL` con `https://` ya no funcionaría.

- [ ] **Step 1: Cambiar la feature**

En `apps/panel-app/src-tauri/Cargo.toml`, línea 16, de:

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
```

a:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json"] }
```

- [ ] **Step 2: Compilar y probar el panel solo**

Run: `cargo build -p atlas-panel && cargo test -p atlas-panel 2>&1 | tail -5`
Expected: build OK y los tests del panel pasan (las 2 pruebas reales marcadas `#[ignore]` siguen ignoradas; no ejecutarlas).

- [ ] **Step 3: Anotarlo en la documentación del panel**

En `docs/panel-app-plan.md`, en la sección de la URL del relay, añadir una línea:

```markdown
- El cliente HTTP del panel es **solo HTTP** (`reqwest` sin TLS): `ATLAS_RELAY_URL`
  debe ser `http://…`; una URL `https://` fallaría al hacer la petición.
```

- [ ] **Step 4: Commit**

```bash
git add apps/panel-app/src-tauri/Cargo.toml docs/panel-app-plan.md
git commit -m "chore(panel): reqwest sin TLS (el panel solo habla HTTP con el relay)"
```

---

### Task 7: Despliegue, verificación global, bundles y registro

**Files:**
- Modify: `docs/implemented.md`, `docs/implementation-plan.md` (registro)

**Interfaces:**
- Consumes: las seis tareas anteriores (sus commits están en `main`).

- [ ] **Step 1: Desplegar el relay (pedir permiso al usuario primero)**

Run: `./scripts/deploy-relay.sh`
Expected: `✅ Relay desplegado y sano … /v1/health` con `{"ok": true, …}`.

- [ ] **Step 2: Verificación global**

Run:
```bash
npm run build && npm run lint && npm run test
cargo check --workspace
cargo test --workspace 2>&1 | tail -3
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cd server && python3 -m unittest discover -s . -p 'test_*.py' 2>&1 | tail -3
```
Expected: todo verde; Rust 211 + 2 ignoradas; frontend 61; Python ~335 ejecuciones (misma cobertura que antes).

- [ ] **Step 3: Medir y anotar el ahorro**

Run:
```bash
du -sh node_modules apps/panel/node_modules apps/panel-app/node_modules
cd server && time python3 -m unittest test_hefesto 2>&1 | tail -2
```
Expected: los `apps/*/node_modules` casi desaparecidos; el tiempo de `test_hefesto` claramente menor que antes de la deduplicación. Anotar ambas cifras en el registro.

- [ ] **Step 4: Bundles (pedir permiso y participación del usuario)**

Cambios en binarios macOS (tareas 5 y 6) ⇒ regenerar con la identidad Apple Development:

```bash
cd apps/panel-app && APPLE_SIGNING_IDENTITY="Apple Development: …" npm run build
codesign --verify --deep --strict --verbose=2 "target/release/bundle/macos/Hub Panel.app"
ditto "target/release/bundle/macos/Hub Panel.app" "$HOME/Applications/Hub Panel.app"
```

y para la app principal (`npm run tauri build` en la raíz con la misma identidad, verificación profunda e instalación en `/Applications/Atlas.app` según `docs/release.md`). Comprobar con el usuario que la app abre, Hefesto registra y el panel sigue en verde.

- [ ] **Step 5: Registro**

Añadir a `docs/implemented.md` una entrada por commit (tareas 1-6) con el nombre exacto de cada commit, y una viñeta «Optimización tanda 2» en `docs/implementation-plan.md` con las cifras del Step 3. Commit final:

```bash
git add docs/implemented.md docs/implementation-plan.md
git commit -m "docs: registra la optimización tanda 2"
git push
```

---

## Self-review

**Cobertura de requisitos:** 1→Task 1; 2→Task 2; 3→Task 3; 4→Task 4; 5→Task 5; 6→Task 6; despliegue/registro→Task 7. Sin huecos.

**Placeholders:** ninguno — cada paso lleva el comando o el código literal.

**Consistencia de tipos:** `SWEEP_MIN_INTERVAL_S` se define en Task 4 y se usa igual en sus tests; `CAL_CALENDARS_TTL` en Task 3; `hefesto_runtime::*` en Task 5 coincide con los nombres actuales de `lib.rs` (verificados contra el código). El orden importa: Task 2 antes de Task 4 (los tests de la puerta van en `JournalTests`, que tras la deduplicación corre una sola vez), y Task 5 después de Task 3 (independientes, pero el despliegue único de Task 7 cubre las tres tareas de `server/`).
