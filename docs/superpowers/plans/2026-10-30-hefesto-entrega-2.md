# Cierre de la entrega 2 de Hefesto — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cerrar la entrega 2 de Hefesto: atajo local acabado y testeado, control de suspensión desde el Hub Panel, y la matriz de fallos ejecutada y registrada.

**Architecture:** El relay ya actúa como controlador del journal Hefesto (entrega 3). El journal ya expone `status` (GET, legible por el controller) y `control` (POST abort), y el dispositivo ya observa la suspensión remota (rechazo → `status` → `suspended` → autosuspensión con marcador). Solo falta el lado controlador en el relay (métodos + endpoints HTTP) y la UI del panel. El atajo global `cmd+shift+a` ya existe; se extrae su lógica a una función testeable. La matriz de fallos es validación en vivo.

**Tech Stack:** Python 3.11+ (solo stdlib, unittest), Rust (atlas-mac/atlas-next), TypeScript + React (panel en `apps/panel`).

**Spec:** [docs/superpowers/specs/2026-10-30-hefesto-entrega-2-design.md](../specs/2026-10-30-hefesto-entrega-2-design.md) — el plan argumenta desde la spec; ejecutar leyendo ambos.

## Global Constraints

- **Sin commits sin permiso explícito del usuario**: cada paso «Commit» exige preguntar antes; si lo deniega, se sigue sin commitear. Nunca push ni reescritura de historial sin permiso.
- **Ciclo de vida de procesos en el Mac (AGENTS.md, obligatorio)**: los servidores de dev (`npm run dev`, `tauri dev`, vite) se lanzan solo mientras se prueban y se matan por su propio PID al terminar; antes de dar la tarea por terminada auditar `ps`/`pgrep`/`launchctl list` y reportar qué queda vivo.
- **Nunca commitear `.env`** (tokens reales).
- **El relay corre en `atlas-hub`**: desplegar solo con `./scripts/deploy-relay.sh` desde la raíz del repo (reinicia el servicio compartido `atlas-relay` → requiere permiso del usuario); usar siempre `atlas-hub.local`, nunca la IP.
- **Contratos espejo obligatorio** si cambia un contrato (docs/contracts.md + crates/atlas-core + src/types/contracts.ts). Esta entrega **no** cambia contratos: `status` y `control` ya existen en el protocolo v1.
- **Documentación en español** y registro del trabajo en `docs/hefesto-plan.md` §8 y `docs/implemented.md` (solo con el nombre exacto del commit publicado).
- **La suspensión remota solo invalida**: no hay `resume` remoto; la reactivación es manual y en la app.
- **El eco sigue siendo la única operación**: esta entrega no ejecuta nada en el Mac ni cambia la matriz de consentimiento.
- **Comandos de verificación**: Python `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q` desde la raíz; panel `cd apps/panel && npm run build`; Rust `cargo check --workspace`, `cargo test -p atlas-next`.

---

### Task 1: Control remoto de Hefesto en el relay (métodos + endpoints + docs)

**Files:**
- Modify: `server/atlas_relay.py:927` (tras `hefesto_stop`, métodos del Brain), `server/atlas_relay.py:1603` (do_GET), `server/atlas_relay.py:1637` (do_POST)
- Test: `server/test_hefesto_delegation.py` (clases nuevas `ControlJournal` y `RemoteControlTests`)
- Docs: `docs/relay.md` (sección de endpoints)

**Interfaces:**
- Consumes: `hefesto.Peer("controller", device_id)` y `journal.dispatch(peer, method, path, body)` (ya en uso en `_delegate_echo`); `json`; `self._hefesto_journal`/`self._hefesto_device` (atributos del Brain).
- Produces:
  - `Brain.hefesto_status() -> dict` — `{"configured": bool, "deviceId": str|null, "suspended": bool|null, "generation": int|null}`.
  - `Brain.hefesto_suspend() -> dict` — `{"ok": bool, "suspended": bool?}` o `{"ok": False, "error": str}`.
  - HTTP `GET /v1/hefesto/status` → 200 con `hefesto_status()`; `POST /v1/hefesto/suspend` → 200 con `hefesto_suspend()`.

- [ ] **Step 1: Escribir los tests que fallan**

Añadir a `server/test_hefesto_delegation.py`, tras `CancellingJournal`:

```python
class ControlJournal:
    """Journal falso para el control remoto: status (GET) y control (POST)."""

    def __init__(self, generation=3, suspended=False):
        self.generation = generation
        self.suspended = suspended
        self.controls = []

    def dispatch(self, peer, method, path, raw=b""):
        operation = path.rsplit("/", 1)[-1]
        if operation == "status":
            return (200, {"protocolVersion": 1, "deviceId": peer.device_id,
                          "generation": self.generation,
                          "suspended": self.suspended})
        if operation == "control":
            body = json.loads(raw)
            self.controls.append((body["action"], body["expectedGeneration"]))
            if body["expectedGeneration"] != self.generation:
                return (409, {"protocolVersion": 1, "error": "stale_generation"})
            self.generation += 1
            self.suspended = True
            return (200, {"protocolVersion": 1, "deviceId": peer.device_id,
                          "generation": self.generation, "suspended": True})
        return (404, {"protocolVersion": 1, "error": "not_found"})


class RacedControlJournal(ControlJournal):
    """Simula un abort ajeno: el primer control choca con la generación."""

    def __init__(self):
        super().__init__(generation=3)
        self.first_control = True

    def dispatch(self, peer, method, path, raw=b""):
        if path.endswith("control") and self.first_control:
            self.first_control = False
            self.generation += 1
            return (409, {"protocolVersion": 1, "error": "stale_generation"})
        return super().dispatch(peer, method, path, raw)
```

Y una clase nueva al final (junto a `TrackerTests`):

```python
class RemoteControlTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
        self.addCleanup(self.temp.cleanup)

    def test_status_sin_listener(self):
        brain = make_brain(self.temp.name)
        self.assertEqual(brain.hefesto_status(),
                         {"configured": False, "deviceId": None,
                          "suspended": None, "generation": None})

    def test_status_con_listener(self):
        brain = make_brain(self.temp.name)
        brain._hefesto_journal = ControlJournal()
        brain._hefesto_device = "mac-one"
        self.assertEqual(brain.hefesto_status(),
                         {"configured": True, "deviceId": "mac-one",
                          "suspended": False, "generation": 3})

    def test_suspend_envia_abort_con_la_generacion_actual(self):
        brain = make_brain(self.temp.name)
        fake = ControlJournal()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        self.assertEqual(brain.hefesto_suspend(), {"ok": True, "suspended": True})
        self.assertEqual(fake.controls, [("abort", 3)])

    def test_suspend_sin_listener(self):
        brain = make_brain(self.temp.name)
        self.assertEqual(brain.hefesto_suspend(),
                         {"ok": False, "error": "hefesto_disabled"})

    def test_suspend_reintenta_si_la_generacion_queda_obsoleta(self):
        brain = make_brain(self.temp.name)
        fake = RacedControlJournal()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        self.assertEqual(brain.hefesto_suspend(), {"ok": True, "suspended": True})
        self.assertEqual(fake.controls, [("abort", 3), ("abort", 4)])
```

- [ ] **Step 2: Ejecutar y confirmar que fallan**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: los 5 tests nuevos FALLAN (`AttributeError: 'Brain' object has no attribute 'hefesto_status'`).

- [ ] **Step 3: Implementar los métodos del Brain**

En `server/atlas_relay.py`, tras `hefesto_stop` (línea ~927), añadir:

```python
    def hefesto_status(self) -> dict:
        """Estado del dispositivo Hefesto visto por el controlador (lectura)."""
        with self.lock:
            journal, device_id = self._hefesto_journal, self._hefesto_device
        if journal is None or device_id is None:
            return {"configured": False, "deviceId": None,
                    "suspended": None, "generation": None}
        status, response = journal.dispatch(
            hefesto.Peer("controller", device_id), "GET", "/v1/hefesto/status")
        if status != 200:
            return {"configured": True, "deviceId": device_id,
                    "suspended": None, "generation": None,
                    "error": response.get("error")}
        return {"configured": True, "deviceId": device_id,
                "suspended": response["suspended"],
                "generation": response["generation"]}

    def hefesto_suspend(self) -> dict:
        """Suspende Hefesto por control remoto (solo invalida; no reactiva)."""
        with self.lock:
            journal, device_id = self._hefesto_journal, self._hefesto_device
        if journal is None or device_id is None:
            return {"ok": False, "error": "hefesto_disabled"}
        peer = hefesto.Peer("controller", device_id)
        for _ in range(2):
            status, body = journal.dispatch(peer, "GET", "/v1/hefesto/status")
            if status != 200:
                return {"ok": False, "error": body.get("error", "status_failed")}
            status, body = journal.dispatch(
                peer, "POST", "/v1/hefesto/control",
                json.dumps({"protocolVersion": 1, "action": "abort",
                            "expectedGeneration": body["generation"]}).encode("utf-8"))
            if status == 200:
                return {"ok": True, "suspended": True}
            if body.get("error") != "stale_generation":
                return {"ok": False, "error": body.get("error", "control_failed")}
        return {"ok": False, "error": "stale_generation"}
```

- [ ] **Step 4: Ejecutar y confirmar que pasan**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: TODO pasa (93 = 88 previas + 5 nuevas).

- [ ] **Step 5: Añadir los endpoints HTTP**

En `do_GET` (tras el bloque `/v1/stats`, antes del 404 de la línea 1611):

```python
            if parsed.path == "/v1/hefesto/status":
                self._json(200, brain.hefesto_status())
                return
```

En `do_POST` (tras el bloque `/v1/confirm`, línea ~1637):

```python
            if path == "/v1/hefesto/suspend":
                self._json(200, brain.hefesto_suspend())
                return
```

Nota: `do_POST` ya exige cuerpo JSON (`_read_json()`); el panel siempre envía `{}` con `post()`. Sin cuerpo, responde 400 `bad_body` (aceptable).

- [ ] **Step 6: Verificación de sintaxis**

Run: `python3 -m py_compile server/atlas_relay.py server/hefesto.py`
Expected: sin salida (compila).

- [ ] **Step 7: Documentar los endpoints**

En `docs/relay.md`, añadir en la sección de la API HTTP:

```markdown
- `GET /v1/hefesto/status` → `{configured, deviceId, suspended, generation}`
  (estado del dispositivo Hefesto visto por el controlador; solo LAN).
- `POST /v1/hefesto/suspend` (cuerpo `{}`) → `{ok: true, suspended: true}` o
  `{ok: false, error}`. Suspende Hefesto por control remoto (solo invalida;
  la reactivación es manual en la app). Latencia ~1 s (sondeo del dispositivo).
```

- [ ] **Step 8: Commit (solo con permiso explícito del usuario)**

```bash
git add server/atlas_relay.py server/test_hefesto_delegation.py docs/relay.md
git commit -m "feat(hefesto): suspensión remota de Hefesto desde el relay"
```

---

### Task 2: Hub Panel — tarjeta Hefesto y botón Suspender

**Files:**
- Modify: `apps/panel/src/api.ts` (tipos + fetchers), `apps/panel/src/components/Cards.tsx` (componente), `apps/panel/src/App.tsx` (estado + montaje)

**Interfaces:**
- Consumes: `j`/`post`/`usePoll` de `apps/panel/src/api.ts`; `Card` de `Cards.tsx` (`title`, `badge`, `tone?: 'err'`, `children`).
- Produces: `HefestoStatus` (tipo), `fetchHefestoStatus()`, `suspendHefesto()`, `HefestoCard` (componente).

- [ ] **Step 1: Añadir tipos y fetchers en `api.ts`**

Tras la interfaz `Health` (línea ~112), añadir:

```ts
export interface HefestoStatus {
  configured: boolean
  deviceId: string | null
  suspended: boolean | null
  generation: number | null
}

export const fetchHefestoStatus = () =>
  j<HefestoStatus>('/v1/hefesto/status')

export const suspendHefesto = () =>
  post<{ ok: boolean; suspended?: boolean; error?: string }>('/v1/hefesto/suspend', {})
```

- [ ] **Step 2: Añadir el componente `HefestoCard` en `Cards.tsx`**

Al final de `Cards.tsx` (reusa `Card` y los estilos existentes del panel):

```tsx
export function HefestoCard({ status, busy, onSuspend }: {
  status: HefestoStatus | null
  busy: boolean
  onSuspend: () => void
}) {
  const configured = status?.configured ?? false
  const suspended = status?.suspended
  const label = !status
    ? 'Consultando…'
    : !configured
      ? 'Sin configurar'
      : suspended
        ? 'Suspendido'
        : 'Activo'
  return (
    <Card title="Hefesto" badge={configured ? (status!.deviceId ?? '') : '—'}>
      <div className="kpi-row">
        <span className={`relay-led${suspended === false ? ' on' : ''}`} aria-hidden="true" />
        <span>{label}</span>
      </div>
      {configured && suspended === false && (
        <button className="btn danger" onClick={onSuspend} disabled={busy}>
          {busy ? 'Suspendiendo…' : 'Suspender'}
        </button>
      )}
    </Card>
  )
}
```

Nota: importa `HefestoStatus` desde `../api` en `Cards.tsx` (o declara las props inline). Si `relay-led`/`btn danger` no existen en el CSS del panel, añade una regla mínima en la hoja del panel siguiendo los tokens existentes (no inventes colores: reusa los ya definidos).

- [ ] **Step 3: Cablear en `App.tsx`**

Añadir el poll y el handler (junto a los otros `usePoll`, línea ~38-44):

```tsx
const { data: hefesto } = usePoll<HefestoStatus>(() => fetchHefestoStatus(), 2000)
const [suspending, setSuspending] = useState(false)
const doSuspend = useCallback(async () => {
  setSuspending(true)
  try {
    const res = await suspendHefesto()
    if (res.ok) flash('Hefesto suspendido.')
    else flash(`No se pudo suspender: ${res.error ?? 'sin detalle'}`, true)
  } catch (exc) {
    flash((exc as Error).message, true)
  } finally {
    setSuspending(false)
  }
}, [flash])
```

Importar `fetchHefestoStatus`, `suspendHefesto` y `HefestoStatus` desde `./api` y `HefestoCard` desde `./components/Cards`. Montar en el `viewer` (tras `<RelayCard …/>`, línea ~138):

```tsx
<HefestoCard status={hefesto} busy={suspending} onSuspend={doSuspend} />
```

- [ ] **Step 4: Verificar build del panel**

Run: `cd apps/panel && npm run build`
Expected: build sin errores de TypeScript ni de Vite. (No hay harness de tests en el panel; la verificación es de compilación + visual.)

- [ ] **Step 5: Commit (solo con permiso explícito del usuario)**

```bash
git add apps/panel/src/api.ts apps/panel/src/components/Cards.tsx apps/panel/src/App.tsx
git commit -m "feat(panel): tarjeta Hefesto con suspensión remota"
```

---

### Task 3: Atajo local — lógica extraída y testeada

**Files:**
- Modify: `src-tauri/src/lib.rs:553-575` (handler del atajo), `src-tauri/src/lib.rs:589` (módulo de tests)

**Interfaces:**
- Consumes: `tauri_plugin_global_shortcut::ShortcutState`; `app.get_webview_window("main")`.
- Produces: `enum VisibilityAction { Show, Hide }` y `fn next_visibility(visible: bool, focused: bool) -> VisibilityAction` (puros y testeables).

- [ ] **Step 1: Escribir el test que falla**

En el módulo `tests` de `src-tauri/src/lib.rs` (línea 590), añadir:

```rust
    #[test]
    fn el_atajo_oculta_solo_si_esta_visible_y_en_foco() {
        use super::{next_visibility, VisibilityAction};
        assert!(matches!(next_visibility(true, true), VisibilityAction::Hide));
        assert!(matches!(next_visibility(true, false), VisibilityAction::Show));
        assert!(matches!(next_visibility(false, true), VisibilityAction::Show));
        assert!(matches!(next_visibility(false, false), VisibilityAction::Show));
    }
```

Y ampliar el `use super::{...}` de la cabecera del módulo con `next_visibility, VisibilityAction` (o usar los caminos `super::` dentro del test como arriba).

- [ ] **Step 2: Ejecutar y confirmar que falla**

Run: `cargo test -p atlas-next`
Expected: FALLA por compilación (`next_visibility` no existe).

- [ ] **Step 3: Extraer la función y el enum**

En `src-tauri/src/lib.rs`, antes de `pub fn run()` (línea ~534), añadir:

```rust
enum VisibilityAction {
    Show,
    Hide,
}

/// El atajo alterna: oculta solo si la ventana está visible y enfocada;
/// en cualquier otro caso la trae al frente.
fn next_visibility(visible: bool, focused: bool) -> VisibilityAction {
    if visible && focused {
        VisibilityAction::Hide
    } else {
        VisibilityAction::Show
    }
}
```

Y sustituir el cuerpo del handler (líneas ~556-573) para usar la función:

```rust
                    if let Some(window) = app.get_webview_window("main") {
                        match next_visibility(
                            window.is_visible().unwrap_or(false),
                            window.is_focused().unwrap_or(false),
                        ) {
                            VisibilityAction::Hide => {
                                let _ = window.hide();
                            }
                            VisibilityAction::Show => {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                        }
                    }
```

(Conserva la comprobación `if event.state() != ShortcutState::Pressed { return; }` intacta.)

- [ ] **Step 4: Ejecutar y confirmar que pasa**

Run: `cargo test -p atlas-next`
Expected: TODO pasa (las 4 existentes + 1 nueva).

- [ ] **Step 5: Commit (solo con permiso explícito del usuario)**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(chat): el atajo global alterna la ventana con lógica testeada"
```

---

### Task 4: Despliegue y matriz de fallos en vivo

**Files:**
- Docs: `docs/hefesto-plan.md` §8, `docs/implemented.md` (solo al publicar)

**Interfaces:**
- Consumes: relay desplegado (Task 1), panel construido (Task 2), app reconstruida (Task 3).
- Produces: registro de la matriz de fallos y despliegue en atlas-hub.

- [ ] **Step 1: Publicar y desplegar (solo con permiso explícito del usuario)**

- Construir el panel web y versionar el `dist`: `cd apps/panel && npm run build` y `git add -f apps/panel/dist` (AGENTS.md: se commitea a mano; comprobar que se borran los assets viejos).
- Reconstruir el bundle de la app (atajo): `APPLE_SIGNING_IDENTITY="Apple Development: adria_bgcma@hotmail.com (Y329B5UZ34)" npm run tauri build` (si no, resolver con `security find-identity -v -p codesigning`), verificar firma profunda (`codesign --verify --deep --strict`) e instalar con `ditto` en `/Applications/Atlas.app` (AGENTS.md).
- Desplegar el relay: `./scripts/deploy-relay.sh` (reinicia `atlas-relay` en atlas-hub) y comprobar `/v1/health`.

- [ ] **Step 2: Ejecutar la matriz de fallos con el usuario**

Con Hefesto activo en la app (lanzar el bundle con las variables mTLS, como en la entrega 3) y el usuario delante, ejecutar y anotar el resultado de cada escenario (checklist de la spec §2.3):

1. **Salida normal** (Cmd+Q): relanzar y comprobar arranque limpio; `ps`/`pgrep` sin huérfanos.
2. **Crash** (`kill -9` del PID de la app): relanzar, reconectar; encargos pendientes resueltos como caducados/`unknown`.
3. **Pérdida de red** (parar `atlas-relay` o cortar la LAN): el dispositivo reintenta sin bloquearse; al volver, reconecta solo.
4. **Suspensión** desde la app **y** desde el Hub Panel (botón Suspender): marcador escrito, encargo en vuelo cancelado, nada se ejecuta, y la app muestra el indicador «Suspendido».
5. **Reinicio** del relay (y del Mac si procede): recuperación del journal sin reejecutar nada.

En cada escenario: verificar también el **atajo** `cmd+shift+a` (trae/oculta la ventana) y el `/v1/health` del relay.

- [ ] **Step 3: Registrar resultados en docs**

En `docs/hefesto-plan.md` §8, añadir una entrada `2026-10-30` con: los tres frentes cerrados, el resultado de cada escenario de la matriz (con lo observado), los commits de Tasks 1-3, y la auditoría final de procesos (`ps`/`pgrep`/`launchctl list` + qué queda vivo).

En `docs/implemented.md`, añadir la entrada «Cierre de la entrega 2 de Hefesto» con los nombres exactos de los commits publicados y fecha.

- [ ] **Step 4: Auditoría final de procesos (AGENTS.md)**

Parar por PID cualquier dev server lanzado (`npm run dev`, `tauri dev`); comprobar `ps`/`pgrep`/`launchctl list` y reportar qué queda vivo en el Mac. En atlas-hub quedan solo el relay y el listener 8767 (servicio gestionado).

- [ ] **Step 5: Commit de docs (solo con permiso explícito del usuario)**

```bash
git add docs/hefesto-plan.md docs/implemented.md
git commit -m "docs(hefesto): registrar el cierre de la entrega 2"
```
