# Delegación por intención a Hefesto — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** El relay delega encargos a Hefesto por intención del LLM, sin bloquear el chat, y publica el resultado como mensaje continuador etiquetado HEFESTO (violeta en la UI), con seguimiento también en Telegram.

**Architecture:** El `Brain` del relay actúa como controlador en proceso del journal Hefesto (mTLS listener ya existente en `server/hefesto.py`). Una herramienta `hefesto_echo` encola el encargo (`submit`) y vuelve al instante; un hilo de seguimiento sondea la nueva operación `outcome` hasta un estado terminal y publica `conversation.message_appended` con `agent: 'hefesto'`, `taskId` de correlación y requestId propio. Sin eventos de estado nuevos.

**Tech Stack:** Python 3.11+ (solo stdlib, unittest), Rust (serde, atlas-core/atlas-next), TypeScript + React (vitest, react-dom/server para tests de componentes).

**Spec:** [docs/superpowers/specs/2026-10-29-hefesto-delegacion-design.md](../specs/2026-10-29-hefesto-delegacion-design.md) — el plan argumenta desde la spec; ejecutar leyendo ambos.

## Global Constraints

- **Sin commits sin permiso explícito del usuario**: cada paso «Commit» exige preguntar antes; si lo deniega, se sigue sin commitear. Nunca push ni reescritura de historial sin permiso.
- **Ciclo de vida de procesos en el Mac (AGENTS.md, obligatorio)**: los servidores de dev (`npm run dev`) se lanzan solo mientras se prueban y se matan por su propio PID al terminar; antes de dar la tarea por terminada auditar `ps`/`pgrep`/`launchctl list` y reportar qué queda vivo. Nada de autoarranque ni procesos huérfanos.
- **Nunca commitear `.env`** (tokens reales).
- **El relay corre en `atlas-hub`**: desplegar solo con `./scripts/deploy-relay.sh` desde la raíz del repo (reinicia el servicio compartido `atlas-relay` → requiere permiso del usuario); usar siempre `atlas-hub.local`, nunca la IP. El listener Hefesto ya está activo en atlas-hub (8767, mTLS, PKI de prueba).
- **Contratos espejo obligatorio**: cualquier cambio de contrato se hace a la vez en `docs/contracts.md`, `crates/atlas-core/src/contracts.rs` y `src/types/contracts.ts`.
- **Documentación en español** y registro obligatorio del trabajo en `docs/implementation-plan.md`, `docs/hefesto-plan.md` §8 y `docs/implemented.md`.
- **Comandos de verificación**: pruebas Python `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q` desde la raíz (el archivo nuevo de pruebas debe llamarse `test_hefesto_delegation.py` para que el discover lo encuentre); frontend `npm run test`, `npm run build`, `npm run lint`; Rust `cargo check --workspace`, `cargo test --workspace`.
- **El eco sigue siendo la única operación**: esta entrega no ejecuta nada en el Mac ni cambia la matriz de consentimiento.
- **El eco es trivial, sin confirmación**; no se añaden flujos de confirmación en esta entrega.

---

### Task 1: Protocolo v1 — operación `outcome` en el journal

**Files:**
- Modify: `server/hefesto.py:26-33` (`_FIELDS`), `server/hefesto.py:221-224` (RBAC), `server/hefesto.py:336-337` (insertar rama en `_operate`)
- Test: `server/test_hefesto.py` (clase `JournalTests`)
- Docs: `docs/hefesto-plan.md:131-146` (tabla y respuestas), `docs/contracts.md:167-171` (sección Hefesto v1), `docs/security.md:46-52` (párrafo del journal)

**Interfaces:**
- Consumes: `Journal.dispatch` con `Peer("controller", …)`; convenciones de `test_hefesto.py` (`ok`, `error`, `ready`, `send`, `next`, `result_fields`).
- Produces: `POST /v1/hefesto/outcome` con cuerpo `{"protocolVersion": 1, "invocationId": "…"}` → 200 `{"protocolVersion": 1, "invocationId": "…", "state": "<pending|dispatched|succeeded|cancelled|expired|unknown>", "outputText": string|null}`; errores: 403 `forbidden` (rol device), 404 `invocation_not_found`, 400 `invalid_fields`/`invalid_id`. Lectura pura: no muta la invocación.

- [ ] **Step 1: Escribir los tests que fallan**

Añadir a la clase `JournalTests` de `server/test_hefesto.py` (después de los tests existentes):

```python
    def test_outcome_exige_rol_controller(self):
        self.ready()
        self.send()
        self.error("forbidden", "outcome", peer=DEVICE, invocationId="inv-1")

    def test_outcome_devuelve_estado_y_resultado(self):
        self.ready()
        ack = self.send(text="héllo 世界")
        outcome = self.ok("outcome", peer=CONTROLLER, invocationId=ack["invocationId"])
        self.assertEqual(outcome, {"protocolVersion": 1, "invocationId": ack["invocationId"],
                                   "state": "pending", "outputText": None})
        invocation = self.next()
        self.ok("result", peer=DEVICE, **self.result_fields(invocation))
        outcome = self.ok("outcome", peer=CONTROLLER, invocationId=ack["invocationId"])
        self.assertEqual(outcome["state"], "succeeded")
        self.assertEqual(outcome["outputText"], "héllo 世界")

    def test_outcome_desconocido_devuelve_404(self):
        self.error("invocation_not_found", "outcome", peer=CONTROLLER, invocationId="no-existe")

    def test_outcome_exige_invocation_id(self):
        self.error("invalid_fields", "outcome", peer=CONTROLLER)

    def test_outcome_es_lectura_pura(self):
        self.ready()
        ack = self.send()
        self.next()
        self.ok("outcome", peer=CONTROLLER, invocationId=ack["invocationId"])
        self.assertEqual(self.ok("status")["activeInvocationId"], ack["invocationId"])
```

- [ ] **Step 2: Ejecutar y confirmar que fallan**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: los 5 tests nuevos FALLAN (`not_found` 404 en lugar de 200/forbidden: la operación aún no existe).

- [ ] **Step 3: Implementar `outcome` en `server/hefesto.py`**

En `_FIELDS` (línea 26-33), añadir la entrada `"outcome": {"invocationId"},` (por ejemplo, antes de `"submit"`):

```python
_FIELDS = {
    "register": {"instanceId"},
    "heartbeat": {"instanceId"},
    "control": {"action", "expectedGeneration"},
    "outcome": {"invocationId"},
    "submit": {"submissionId", "requestId", "sessionId", "taskId", "toolId", "arguments"},
    "next": {"instanceId", "generation"},
    "result": {"instanceId", "generation", "invocationId", "output"},
}
```

En `dispatch` (línea 222-223), ampliar el control de rol:

```python
                    if ((operation in ("register", "heartbeat", "next", "result") and peer.role != "device")
                            or (operation in ("submit", "outcome") and peer.role != "controller")):
                        raise _Rejected("forbidden", 403)
```

En `_operate`, insertar la rama entre el final de la rama `submit` (tras su `return`, línea 336) y `if operation == "next":`:

```python
        if operation == "outcome":
            invocation = self._db.execute(
                "SELECT * FROM invocations WHERE invocation_id=? AND device_id=?",
                (data["invocationId"], device_id)).fetchone()
            if invocation is None:
                raise _Rejected("invocation_not_found", 404)
            return {"protocolVersion": 1, "invocationId": invocation["invocation_id"],
                    "state": invocation["state"], "outputText": invocation["output_text"]}
```

Nota: no tocar `_sweep` ni ninguna escritura; `dispatch` ya hace `BEGIN IMMEDIATE` + `_sweep` + `COMMIT` para todas las operaciones, y `_sweep` solo muta por lease/expiración, no por leer.

- [ ] **Step 4: Ejecutar la suite completa**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: TODO pasa (las ~60 existentes + 5 nuevas).

- [ ] **Step 5: Documentar la operación**

En `docs/hefesto-plan.md` §5, añadir la fila a la tabla (línea 131-139):

```markdown
| POST `/v1/hefesto/outcome` | controller | `invocationId` |
```

Y en el párrafo de respuestas (línea 143-146) añadir tras `result`:

```markdown
`{protocolVersion, invocationId, state}`; `outcome` devuelve
`{protocolVersion, invocationId, state, outputText: string|null}` (lectura pura).
```

En `docs/contracts.md` sección «Hefesto v1», tras la línea 170 («No añade herramientas al catálogo del LLM»), añadir:

```markdown
Operación nueva del protocolo v1: `POST /v1/hefesto/outcome` (rol controller)
`{invocationId}` → `{protocolVersion, invocationId, state, outputText|null}`.
Lectura pura del journal.
```

En `docs/security.md`, en el párrafo del journal (líneas 46-52), añadir al final:

```markdown
La operación `outcome` exige rol controller y es lectura pura: no muta estado.
```

- [ ] **Step 6: Commit (solo con permiso explícito del usuario)**

```bash
git add server/hefesto.py server/test_hefesto.py docs/hefesto-plan.md docs/contracts.md docs/security.md
git commit -m "feat(hefesto): operación outcome de lectura para el controlador"
```

---

### Task 2: El listener entrega el journal y deriva el dispositivo

**Files:**
- Modify: `server/atlas_relay.py:1561-1616` (`hefesto_listener`)
- Test: `server/test_hefesto_integration.py` (clase nueva `ListenerYieldTests`)

**Interfaces:**
- Consumes: `hefesto.load_peers` (fingerprint → `Peer` con `role`/`device_id`), `fail_config`.
- Produces: `hefesto_listener(host, data_dir)` pasa a `yield` — `None` si el listener está desactivado; `{"journal": hefesto.Journal, "device_id": str | None}` si está activo (`device_id` = único peer con `role == "device"`; `None` si no hay dispositivos; **más de uno → `fail_config`** con salida 2). Los tests existentes no capturan el valor del `with`, así que siguen pasando sin cambios.

- [ ] **Step 1: Escribir los tests que fallan**

Añadir a `server/test_hefesto_integration.py`:

```python
class ListenerYieldTests(unittest.TestCase):
    def test_deshabilitado_devuelve_none(self):
        with patch.dict("os.environ", {}, clear=True):
            with atlas_relay.hefesto_listener("127.0.0.1", "unused") as ctx:
                self.assertIsNone(ctx)

    def test_habilitado_entrega_journal_y_dispositivo_unico(self):
        peers = {"a" * 64: atlas_relay.hefesto.Peer("device", "mac-one"),
                 "b" * 64: atlas_relay.hefesto.Peer("controller", "mac-one")}
        with patch.dict("os.environ", SETTINGS, clear=True), \
                patch.object(atlas_relay, "log"), \
                patch.object(atlas_relay.hefesto, "load_peers", return_value=peers), \
                patch.object(atlas_relay.hefesto, "Journal") as journal, \
                patch.object(atlas_relay.hefesto, "create_server") as create:
            with atlas_relay.hefesto_listener("127.0.0.1", "unused") as ctx:
                self.assertEqual(ctx, {"journal": journal.return_value,
                                       "device_id": "mac-one"})
            create.assert_called_once()

    def test_sin_dispositivo_entrega_journal_con_device_none(self):
        peers = {"a" * 64: atlas_relay.hefesto.Peer("controller", "mac-one")}
        with patch.dict("os.environ", SETTINGS, clear=True), \
                patch.object(atlas_relay, "log"), \
                patch.object(atlas_relay.hefesto, "load_peers", return_value=peers), \
                patch.object(atlas_relay.hefesto, "Journal") as journal, \
                patch.object(atlas_relay.hefesto, "create_server") as create:
            with atlas_relay.hefesto_listener("127.0.0.1", "unused") as ctx:
                self.assertEqual(ctx, {"journal": journal.return_value,
                                       "device_id": None})
            create.assert_called_once()

    def test_mas_de_un_dispositivo_falla_la_configuracion(self):
        peers = {"a" * 64: atlas_relay.hefesto.Peer("device", "mac-one"),
                 "b" * 64: atlas_relay.hefesto.Peer("device", "mac-two")}
        with patch.dict("os.environ", SETTINGS, clear=True), \
                patch.object(atlas_relay, "log"), \
                patch.object(atlas_relay.hefesto, "load_peers", return_value=peers), \
                patch.object(atlas_relay.hefesto, "Journal") as journal, \
                patch.object(atlas_relay.hefesto, "create_server") as create:
            with self.assertRaises(SystemExit) as error:
                with atlas_relay.hefesto_listener("127.0.0.1", "unused"):
                    self.fail("Multi-dispositivo debe fallar")
            self.assertEqual(error.exception.code, 2)
            journal.assert_not_called()
            create.assert_not_called()
```

- [ ] **Step 2: Ejecutar y confirmar que fallan**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: `test_deshabilitado_devuelve_none` FALLA (yield `None` hoy no devuelve nada) y los otros tres FALLAN (`ctx` es `None` / no hay SystemExit).

- [ ] **Step 3: Implementar el yield en `hefesto_listener`**

En `server/atlas_relay.py`, cambiar la rama desactivada (línea 1565-1567):

```python
    if not any(name in os.environ for name in names):
        yield None
        return
```

Y dentro del `with ExitStack()`, derivar el dispositivo justo tras cargar pares y antes de crear el journal (línea 1578-1582):

```python
    with ExitStack() as resources:
        try:
            peers = hefesto.load_peers(os.environ["HEFESTO_PEERS_FILE"])
            devices = {peer.device_id for peer in peers.values()
                       if peer.role == "device"}
            if len(devices) > 1:
                fail_config("El registro de pares de Hefesto solo admite un dispositivo.")
            device_id = next(iter(devices)) if devices else None
            journal = hefesto.Journal(os.path.join(data_dir, "hefesto.sqlite"))
            resources.callback(journal.close)
```

Y el `yield` final (línea 1616):

```python
        log("Hefesto: listener mTLS habilitado (solo eco).")
        yield {"journal": journal, "device_id": device_id}
```

- [ ] **Step 4: Ejecutar la suite completa**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: TODO pasa (los tests existentes de ciclo de vida no capturan el yield; con `load_peers → {}` el `device_id` es `None` y no falla).

- [ ] **Step 5: Commit (solo con permiso explícito del usuario)**

```bash
git add server/atlas_relay.py server/test_hefesto_integration.py
git commit -m "feat(hefesto): el listener entrega el journal y deriva el dispositivo"
```

---

### Task 3: Herramienta `hefesto_echo` y catálogo dinámico de herramientas

**Files:**
- Modify: `server/atlas_relay.py:130-176` (`TOOL_DEFS` queda; añadir `HEFESTO_TOOL` y cambiar `tool_payload`), `server/atlas_relay.py:316-339` (`Brain.__init__`), `server/atlas_relay.py:726-727` (`_tool_def`), `server/atlas_relay.py:743-765` (`execute_tool`), `server/atlas_relay.py:956` (`groq_completion`), `server/atlas_relay.py:1072` (`_handle_tool_call`)
- Test: `server/test_hefesto_delegation.py` (nuevo; el nombre empieza por `test_hefesto` para que el discover lo recoja)

**Interfaces:**
- Consumes: `Peer("controller", device_id)` de `hefesto`; `new_id`, `dlog`; contrato `submit` del journal (Task 1 sin cambios).
- Produces:
  - `Brain.tool_defs() -> list[dict]` — `TOOL_DEFS` + `HEFESTO_TOOL` solo si `self._hefesto_journal is not None and self._hefesto_device is not None`.
  - `tool_payload(defs: list[dict]) -> list[dict]` — mismo formato OpenAI function calling que hoy.
  - `Brain.execute_tool(tool_id: str, args: dict, job: dict) -> str` — **firma cambiada**: ahora recibe `job` (el único call site es `_handle_tool_call`).
  - `Brain._delegate_echo(job: dict, args: dict) -> str` — encola vía `journal.dispatch(peer, "POST", "/v1/hefesto/submit", …)`; en 200 registra la delegación en `self._delegations[invocation_id]` (en memoria; la persistencia llega en Task 4); en otro caso devuelve `"Hefesto no está disponible (<error>)"`.
  - `Brain._hefesto_journal: hefesto.Journal | None`, `Brain._hefesto_device: str | None`, `Brain._delegations: dict[str, dict]` (nuevos atributos, inicializados en `__init__`).

- [ ] **Step 1: Escribir los tests que fallan**

Crear `server/test_hefesto_delegation.py`:

```python
"""Pruebas de delegación por intención del cerebro (entrega 3)."""

from __future__ import annotations

import json
import tempfile
import unittest

import atlas_relay


class FakeJournal:
    """Journal falso: respuestas de submit en cola y outcomes prefijados."""

    def __init__(self):
        self.submits = []
        self.submit_results = []
        self.outcomes = {}

    def dispatch(self, peer, method, path, raw):
        operation = path.rsplit("/", 1)[-1]
        body = json.loads(raw)
        if operation == "submit":
            self.submits.append(body)
            return self.submit_results.pop(0)
        if operation == "outcome":
            entry = self.outcomes.get(body["invocationId"])
            if entry is None:
                return (404, {"protocolVersion": 1, "error": "invocation_not_found"})
            return (200, {"protocolVersion": 1,
                          "invocationId": body["invocationId"], **entry})
        return (404, {"protocolVersion": 1, "error": "not_found"})

    def accept(self, invocation_id="inv-1"):
        self.submit_results.append((200, {"protocolVersion": 1,
                                          "invocationId": invocation_id,
                                          "state": "pending"}))

    def reject(self, code):
        self.submit_results.append((409, {"protocolVersion": 1, "error": code}))


def make_brain(data_dir):
    cfg = {
        "token": "", "allowed": set(), "groq_keys": [],
        "groq_model": atlas_relay.DEFAULT_GROQ_MODEL,
        "groq_temperature": 0.7, "timezone": "Europe/Madrid",
        "caldav_user": "", "caldav_password": "",
        "editable": set(), "editable_names": [],
        "host": "127.0.0.1", "port": 8765,
        "data_dir": data_dir, "cors_origin": "",
    }
    return atlas_relay.Brain(cfg)


JOB = {"request_id": "req-1", "session_id": "ses-1", "source": "local",
       "chat_id": None, "deadline": 0.0}


class ToolAvailabilityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
        self.addCleanup(self.temp.cleanup)

    def test_sin_listener_la_herramienta_no_se_ofrece(self):
        brain = make_brain(self.temp.name)
        self.assertNotIn("hefesto_echo", [t["id"] for t in brain.tool_defs()])
        self.assertEqual(brain.execute_tool("hefesto_echo", {"text": "hola"}, JOB),
                         "Hefesto no está disponible (desactivado)")

    def test_con_listener_y_dispositivo_la_herramienta_se_ofrece(self):
        brain = make_brain(self.temp.name)
        brain._hefesto_journal = FakeJournal()
        brain._hefesto_device = "mac-one"
        self.assertIn("hefesto_echo", [t["id"] for t in brain.tool_defs()])
        payload = atlas_relay.tool_payload(brain.tool_defs())
        self.assertIn("hefesto_echo", [f["function"]["name"] for f in payload])

    def test_delegacion_aceptada_registra_la_entrada(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        fake.accept()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        result = brain.execute_tool("hefesto_echo", {"text": "hola"}, JOB)
        self.assertIn("delegado a Hefesto", result)
        entry = brain._delegations.get("inv-1")
        self.assertIsNotNone(entry)
        self.assertEqual(entry["request_id"], "req-1")
        self.assertEqual(entry["session_id"], "ses-1")
        submitted = fake.submits[0]
        self.assertEqual(submitted["toolId"], "hefesto.echo")
        self.assertEqual(submitted["arguments"], {"text": "hola"})
        self.assertEqual(submitted["requestId"], "req-1")

    def test_delegacion_rechazada_no_registra(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        fake.reject("device_suspended")
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        result = brain.execute_tool("hefesto_echo", {"text": "hola"}, JOB)
        self.assertIn("Hefesto no está disponible (device_suspended)", result)
        self.assertEqual(brain._delegations, {})

    def test_texto_vacio_o_largo_se_rechaza_sin_encolar(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        self.assertIn("texto vacío",
                      brain.execute_tool("hefesto_echo", {"text": "  "}, JOB))
        self.assertIn("demasiado largo",
                      brain.execute_tool("hefesto_echo", {"text": "á" * 200}, JOB))
        self.assertEqual(fake.submits, [])
```

- [ ] **Step 2: Ejecutar y confirmar que fallan**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: los 5 tests nuevos FALLAN (no existe `test_hefesto_delegation` → `ImportError`/`AttributeError`; si el discover no lo recoge, comprobar el nombre del archivo).

- [ ] **Step 3: Implementar catálogo dinámico y `hefesto_echo`**

En `server/atlas_relay.py`, tras `TOOL_DEFS` (línea 165) añadir:

```python
HEFESTO_TOOL = {
    "id": "hefesto_echo",
    "risk": "external",
    "requiresConfirmation": False,
    "description": ("Encarga una tarea a Hefesto, el especialista del Mac de Atlas, "
                    "y vuelve al instante con el estado de la delegación. Si el "
                    "encargo se acepta, díselo al usuario y no inventes el "
                    "resultado: llegará aparte como mensaje de Hefesto."),
    "schema": {
        "type": "object",
        "properties": {
            "text": {"type": "string",
                     "description": "tarea para Hefesto (máx. 256 bytes)"},
        },
        "required": ["text"],
    },
}
```

Cambiar `tool_payload` (línea 172-176) a función de parámetro:

```python
def tool_payload(defs: list[dict]) -> list[dict]:
    """Catálogo en formato OpenAI function calling para Groq."""
    return [{"type": "function",
             "function": {"name": t["id"], "description": t["description"],
                          "parameters": t["schema"]}} for t in defs]
```

En `Brain.__init__` (junto a los demás atributos, antes de `self._load()`, línea 339):

```python
        # Hefesto: journal/controlador en proceso y delegaciones en vuelo.
        self._hefesto_journal: hefesto.Journal | None = None
        self._hefesto_device: str | None = None
        self._delegations: dict[str, dict] = {}
```

Cambiar `_tool_def` (línea 726-727):

```python
    def tool_defs(self) -> list[dict]:
        if self._hefesto_journal is not None and self._hefesto_device is not None:
            return TOOL_DEFS + [HEFESTO_TOOL]
        return TOOL_DEFS

    def _tool_def(self, tool_id: str) -> dict | None:
        return next((t for t in self.tool_defs() if t["id"] == tool_id), None)
```

Cambiar la firma de `execute_tool` (línea 743) y añadir la rama:

```python
    def execute_tool(self, tool_id: str, args: dict, job: dict) -> str:
        """Ejecuta una herramienta. Devuelve texto para el modelo."""
        if tool_id == "hefesto_echo":
            return self._delegate_echo(job, args)
        if tool_id == "calendar_read":
```

Añadir `_delegate_echo` tras `execute_tool`:

```python
    def _delegate_echo(self, job: dict, args: dict) -> str:
        journal, device_id = self._hefesto_journal, self._hefesto_device
        if journal is None or device_id is None:
            return "Hefesto no está disponible (desactivado)"
        text = str(args.get("text", "")).strip()
        if not text:
            return "delegación rechazada: texto vacío"
        if len(text.encode("utf-8")) > 256:
            return "delegación rechazada: texto demasiado largo (máx. 256 bytes)"
        submission = {
            "protocolVersion": 1,
            "submissionId": new_id("sub"),
            "requestId": job["request_id"],
            "sessionId": job["session_id"],
            "taskId": new_id("tsk"),
            "toolId": "hefesto.echo",
            "arguments": {"text": text},
        }
        status, response = journal.dispatch(
            hefesto.Peer("controller", device_id), "POST",
            "/v1/hefesto/submit", json.dumps(submission).encode("utf-8"))
        if status != 200:
            dlog(self, "HEFESTO",
                 f"submit rejected error={response.get('error')} request={job['request_id']}")
            return f"Hefesto no está disponible ({response.get('error', 'desconocido')})"
        invocation_id = response["invocationId"]
        with self.lock:
            self._delegations[invocation_id] = {
                "invocation_id": invocation_id,
                "request_id": job["request_id"],
                "session_id": job["session_id"],
                "task_id": submission["taskId"],
                "source": job.get("source", "local"),
                "chat_id": job.get("chat_id"),
            }
        dlog(self, "HEFESTO", f"delegated invocation={invocation_id} "
                              f"request={job['request_id']} task={submission['taskId']}")
        return f"delegado a Hefesto ({invocation_id})"
```

En `groq_completion` (línea 956) cambiar:

```python
            "tools": tool_payload(self.tool_defs()),
```

En `_handle_tool_call` (línea 1072) cambiar el call site:

```python
        return self.execute_tool(name, args, job)
```

- [ ] **Step 4: Ejecutar la suite completa**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: TODO pasa (los tests de la Task 1 y 2 incluidos).

- [ ] **Step 5: Commit (solo con permiso explícito del usuario)**

```bash
git add server/atlas_relay.py server/test_hefesto_delegation.py
git commit -m "feat(hefesto): herramienta hefesto_echo y catálogo dinámico de tools"
```

---

### Task 4: Continuador, persistencia de delegaciones y cancelación

**Files:**
- Modify: `server/atlas_relay.py:316-339` (`__init__`), `server/atlas_relay.py:346-372` (`_load`), `server/atlas_relay.py:677-703` (`cancel`)
- Test: `server/test_hefesto_delegation.py` (clase nueva `ContinuationTests`)

**Interfaces:**
- Consumes: `_delegations`/`_delegate_echo` de la Task 3; `atomic_write`, `new_id`, `utc_now`, `STORE_HISTORY`, `MAX_CONTENT_LEN`, `_push_event`.
- Produces:
  - `Brain._delegations_path() -> str` (archivo `hefesto-delegations.json` en `data_dir`), `Brain._load_delegations() -> dict`, `Brain._save_delegations() -> None` (vía `atomic_write`).
  - `Brain._hefesto_finish(entry: dict, state: str, output_text: str | None) -> None` — publica el continuador y elimina la delegación; **si la sesión no existe o ya no es la activa (por `/clear`), descarta en silencio** (un continuador de una sesión abandonada no debe arrastrar el panel a la sesión vieja).
  - `Brain.cancel(request_id)` además elimina las delegaciones cuyo `request_id` coincida (sin continuador).

- [ ] **Step 1: Escribir los tests que fallan**

Añadir a `server/test_hefesto_delegation.py`:

```python
class ContinuationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
        self.addCleanup(self.temp.cleanup)

    def delegate(self, brain, fake, invocation_id="inv-1", job=None):
        fake.accept(invocation_id)
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        brain.execute_tool("hefesto_echo", {"text": "hola"}, job or JOB)
        return brain._delegations[invocation_id]

    def test_continuador_exitoso_con_etiqueta_y_task(self):
        brain = make_brain(self.temp.name)
        session_id = brain._active()["id"]
        fake = FakeJournal()
        entry = self.delegate(brain, fake, job={**JOB, "session_id": session_id})
        brain._hefesto_finish(entry, "succeeded", "hola mundo")
        last = brain.sessions[session_id]["messages"][-1]
        self.assertEqual(last["role"], "assistant")
        self.assertEqual(last["content"], "hola mundo")
        self.assertEqual(last["agent"], "hefesto")
        self.assertEqual(last["taskId"], entry["task_id"])
        event = brain.events[-1]
        self.assertEqual(event["type"], "conversation.message_appended")
        self.assertEqual(event["agent"], "hefesto")
        self.assertEqual(event["taskId"], entry["task_id"])
        self.assertNotEqual(event["requestId"], entry["request_id"])
        self.assertNotIn(entry["invocation_id"], brain._delegations)

    def test_continuador_avisa_por_telegram(self):
        brain = make_brain(self.temp.name)
        session_id = brain._active()["id"]
        fake = FakeJournal()
        entry = self.delegate(brain, fake,
                              job={**JOB, "session_id": session_id,
                                   "source": "telegram", "chat_id": 42})
        with mock.patch.object(brain, "_send_telegram") as send:
            brain._hefesto_finish(entry, "succeeded", "eco")
        send.assert_called_once_with(42, "Hefesto: eco")

    def test_cancelar_el_turno_descarta_la_delegacion(self):
        brain = make_brain(self.temp.name)
        session_id = brain._active()["id"]
        fake = FakeJournal()
        self.delegate(brain, fake, job={**JOB, "session_id": session_id})
        brain.cancel("req-1")
        self.assertEqual(brain._delegations, {})
        self.assertEqual(
            brain._load_delegations(), {})

    def test_continuador_de_sesion_abandonada_se_descarta(self):
        brain = make_brain(self.temp.name)
        first = brain._active()["id"]
        fake = FakeJournal()
        entry = self.delegate(brain, fake, job={**JOB, "session_id": first})
        brain._local_reply("/clear", "local")
        brain._hefesto_finish(entry, "succeeded", "eco")
        self.assertNotIn("eco", [m["content"] for m in brain.sessions[first]["messages"]])
        self.assertNotIn(entry["invocation_id"], brain._delegations)

    def test_delegaciones_persisten_entre_instancias(self):
        brain1 = make_brain(self.temp.name)
        session_id = brain1._active()["id"]
        fake = FakeJournal()
        self.delegate(brain1, fake, job={**JOB, "session_id": session_id})
        brain2 = make_brain(self.temp.name)
        self.assertIn("inv-1", brain2._delegations)
```

(Importar `mock` al inicio del archivo: `from unittest import mock`.)

- [ ] **Step 2: Ejecutar y confirmar que fallan**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: los 5 tests nuevos FALLAN (`AttributeError: _hefesto_finish` / delegaciones no persistidas).

- [ ] **Step 3: Implementar persistencia, continuador y cancelación**

En `_load` (al final, línea 372), añadir:

```python
        self._delegations = self._load_delegations()
```

Añadir los métodos de persistencia (junto a `_save_offset`, línea 386-388):

```python
    def _delegations_path(self) -> str:
        return os.path.join(self.cfg["data_dir"], "hefesto-delegations.json")

    def _load_delegations(self) -> dict[str, dict]:
        try:
            with open(self._delegations_path(), encoding="utf-8") as fh:
                data = json.load(fh)
            if not isinstance(data, dict):
                return {}
            return {key: entry for key, entry in data.items()
                    if isinstance(entry, dict)
                    and isinstance(entry.get("invocation_id"), str)}
        except (OSError, ValueError, AttributeError):
            return {}

    def _save_delegations(self) -> None:
        atomic_write(self._delegations_path(), self._delegations)
```

En `_delegate_echo`, dentro del `with self.lock:` añadir `self._save_delegations()` tras insertar la entrada.

Añadir `_hefesto_finish` (después de `_delegate_echo`):

```python
    def _hefesto_finish(self, entry: dict, state: str,
                        output_text: str | None) -> None:
        if state == "succeeded" and output_text is not None:
            content = output_text
        elif state == "expired":
            content = "Hefesto no respondió a tiempo y el encargo caducó."
        elif state == "cancelled":
            content = "El encargo a Hefesto se canceló."
        else:
            content = "El encargo a Hefesto quedó en un estado desconocido."
        with self.lock:
            session = self.sessions.get(entry["session_id"])
            if session is None or session["id"] != self.active_id:
                self._delegations.pop(entry["invocation_id"], None)
                self._save_delegations()
                return
            request_id = new_id("req")
            session["messages"].append({
                "requestId": request_id, "sessionId": session["id"],
                "role": "assistant", "content": content, "source": "local",
                "agent": "hefesto", "taskId": entry["task_id"],
                "timestamp": utc_now(),
            })
            session["messages"] = session["messages"][-STORE_HISTORY:]
            self._save_sessions()
            self._delegations.pop(entry["invocation_id"], None)
            self._save_delegations()
            self._push_event({
                "kind": "message", "type": "conversation.message_appended",
                "requestId": request_id, "sessionId": session["id"],
                "timestamp": utc_now(), "role": "assistant", "content": content,
                "source": "local", "agent": "hefesto",
                "taskId": entry["task_id"],
            })
        if entry.get("chat_id") is not None:
            self._send_telegram(entry["chat_id"],
                                f"Hefesto: {content}"[:MAX_CONTENT_LEN])
        dlog(self, "HEFESTO", f"continuation invocation={entry['invocation_id']} "
                              f"state={state} session={entry['session_id']}")
```

En `cancel` (dentro del `with self.lock:`, tras el bloque de confirmaciones, línea 703):

```python
            removed = [iid for iid, entry in self._delegations.items()
                       if entry.get("request_id") == request_id]
            if removed:
                for iid in removed:
                    self._delegations.pop(iid, None)
                self._save_delegations()
```

- [ ] **Step 4: Ejecutar la suite completa**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: TODO pasa.

- [ ] **Step 5: Commit (solo con permiso explícito del usuario)**

```bash
git add server/atlas_relay.py server/test_hefesto_delegation.py
git commit -m "feat(hefesto): continuador persistente y cancelación de delegaciones"
```

---

### Task 5: Hilo de seguimiento, arranque/parada y `main()`

**Files:**
- Modify: `server/atlas_relay.py:316-339` (`__init__`), nuevo bloque tras `_hefesto_finish`, `server/atlas_relay.py:1661-1673` (`main`)
- Test: `server/test_hefesto_delegation.py` (clase nueva `TrackerTests`)

**Interfaces:**
- Consumes: `_delegations`/`_hefesto_finish` (Tasks 3-4), `hefesto_listener` yield (Task 2), `Peer`.
- Produces:
  - `Brain.hefesto_start(journal, device_id: str | None) -> None` — fija journal/dispositivo; si hay `device_id` arranca el hilo `hefesto-tracker` (daemon); si no, no hay hilo (y la herramienta no se ofrece, Task 3).
  - `Brain.hefesto_stop() -> None` — marca parada y une el hilo (timeout 2 s).
  - `Brain._hefesto_poll() -> None` — una pasada síncrona: consulta `outcome` por cada delegación y llama `_hefesto_finish` en estados terminales (`succeeded`, `expired`, `cancelled`, `unknown`); ignora `pending`/`dispatched` y errores no 200.
  - `Brain._hefesto_tracker() -> None` — bucle con `Event.wait(1.0)` que llama `_hefesto_poll`.
  - `main()` llama `hefesto_start` con el yield del listener y `hefesto_stop` al salir (la recuperación tras reinicio la resuelve el propio journal: al reabrir, `pending→cancelled` y `dispatched→unknown`, y la primera pasada publica los continuadores).

- [ ] **Step 1: Escribir los tests que fallan**

Añadir a `server/test_hefesto_delegation.py`:

```python
class TrackerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
        self.addCleanup(self.temp.cleanup)

    def delegate(self, brain, fake, invocation_id="inv-1"):
        fake.accept(invocation_id)
        brain._hefesto_journal = fake
        brain._hefesto_device = "mac-one"
        session_id = brain._active()["id"]
        brain.execute_tool("hefesto_echo", {"text": "hola"},
                           {**JOB, "session_id": session_id})
        return session_id

    def test_poll_resuelve_estados_terminales(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        session_id = self.delegate(brain, fake)
        fake.outcomes["inv-1"] = {"state": "expired", "outputText": None}
        brain._hefesto_poll()
        last = brain.sessions[session_id]["messages"][-1]
        self.assertEqual(last["agent"], "hefesto")
        self.assertIn("caducó", last["content"])
        self.assertNotIn("inv-1", brain._delegations)

    def test_poll_no_toca_pendientes(self):
        brain = make_brain(self.temp.name)
        fake = FakeJournal()
        session_id = self.delegate(brain, fake)
        fake.outcomes["inv-1"] = {"state": "dispatched", "outputText": None}
        brain._hefesto_poll()
        self.assertIn("inv-1", brain._delegations)
        self.assertEqual(brain.sessions[session_id]["messages"], [])

    def test_poll_desconocido_y_cancelado_publican_continuador(self):
        for state, needle in (("unknown", "desconocido"), ("cancelled", "canceló")):
            with self.subTest(state=state):
                temp = tempfile.TemporaryDirectory(prefix="atlas-delegation-")
                self.addCleanup(temp.cleanup)
                brain = make_brain(temp.name)
                fake = FakeJournal()
                session_id = self.delegate(brain, fake)
                fake.outcomes["inv-1"] = {"state": state, "outputText": None}
                brain._hefesto_poll()
                last = brain.sessions[session_id]["messages"][-1]
                self.assertIn(needle, last["content"])

    def test_recuperacion_tras_reinicio_del_brain(self):
        brain1 = make_brain(self.temp.name)
        fake1 = FakeJournal()
        self.delegate(brain1, fake1)
        brain2 = make_brain(self.temp.name)
        fake2 = FakeJournal()
        brain2.hefesto_start(fake2, "mac-one")
        try:
            self.assertIn("inv-1", brain2._delegations)
            fake2.outcomes["inv-1"] = {"state": "unknown", "outputText": None}
            brain2._hefesto_poll()
            self.assertNotIn("inv-1", brain2._delegations)
        finally:
            brain2.hefesto_stop()

    def test_tracker_arranca_y_para_con_el_brain(self):
        brain = make_brain(self.temp.name)
        brain.hefesto_start(FakeJournal(), "mac-one")
        try:
            self.assertTrue(brain._hefesto_thread.is_alive())
        finally:
            brain.hefesto_stop()
        self.assertFalse(brain._hefesto_thread.is_alive())

    def test_sin_dispositivo_no_arranca_tracker(self):
        brain = make_brain(self.temp.name)
        brain.hefesto_start(FakeJournal(), None)
        self.assertIsNone(getattr(brain, "_hefesto_thread", None))
        self.assertNotIn("hefesto_echo", [t["id"] for t in brain.tool_defs()])
```

- [ ] **Step 2: Ejecutar y confirmar que fallan**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: los 6 tests nuevos FALLAN (`AttributeError: hefesto_start` / `_hefesto_poll`).

- [ ] **Step 3: Implementar el hilo y el cableado de `main()`**

En `__init__`, junto a los atributos de la Task 3, añadir:

```python
        self._hefesto_stop: threading.Event | None = None
        self._hefesto_thread: threading.Thread | None = None
```

Tras `_hefesto_finish`, añadir:

```python
    def hefesto_start(self, journal, device_id: str | None) -> None:
        with self.lock:
            self._hefesto_journal = journal
            self._hefesto_device = device_id
        if device_id is None:
            return
        self._hefesto_stop = threading.Event()
        self._hefesto_thread = threading.Thread(
            target=self._hefesto_tracker, name="hefesto-tracker", daemon=True)
        self._hefesto_thread.start()
        dlog(self, "HEFESTO", f"tracker started device={device_id} "
                              f"delegations={len(self._delegations)}")

    def hefesto_stop(self) -> None:
        stop = self._hefesto_stop
        if stop is not None:
            stop.set()
        thread = self._hefesto_thread
        if thread is not None and thread.ident is not None:
            thread.join(timeout=2)

    def _hefesto_tracker(self) -> None:
        while not self._hefesto_stop.wait(1.0):
            self._hefesto_poll()

    def _hefesto_poll(self) -> None:
        with self.lock:
            journal, device_id = self._hefesto_journal, self._hefesto_device
            entries = [dict(entry) for entry in self._delegations.values()]
        if journal is None or device_id is None or not entries:
            return
        peer = hefesto.Peer("controller", device_id)
        for entry in entries:
            status, response = journal.dispatch(
                peer, "POST", "/v1/hefesto/outcome",
                json.dumps({"protocolVersion": 1,
                            "invocationId": entry["invocation_id"]}).encode("utf-8"))
            if status != 200:
                dlog(self, "HEFESTO",
                     f"outcome error={response.get('error')} "
                     f"invocation={entry['invocation_id']}")
                continue
            if response.get("state") in ("succeeded", "expired", "cancelled", "unknown"):
                self._hefesto_finish(entry, response["state"],
                                     response.get("outputText"))
```

En `main()` (línea 1663), capturar el yield y arrancar/parar:

```python
        with hefesto_listener(host, cfg["data_dir"]) as hefesto_ctx:
            if hefesto_ctx is not None:
                brain.hefesto_start(hefesto_ctx["journal"], hefesto_ctx["device_id"])
            threading.Thread(target=brain.worker, daemon=True, name="brain-worker").start()
            if cfg["token"]:
                threading.Thread(target=brain.telegram_loop, daemon=True,
                                 name="telegram-poll").start()
            log(f"escuchando en {host}:{cfg['port']} (solo red local {network}).")
            log(f"panel estático en /panel/ ({sysmon.panel_dist}).")
            try:
                server.serve_forever()
            except KeyboardInterrupt:
                pass
            finally:
                brain.hefesto_stop()
```

- [ ] **Step 4: Ejecutar la suite completa**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: TODO pasa.

- [ ] **Step 5: Commit (solo con permiso explícito del usuario)**

```bash
git add server/atlas_relay.py server/test_hefesto_delegation.py
git commit -m "feat(hefesto): hilo de seguimiento con recuperación tras reinicio"
```

---

### Task 6: Contratos Rust (núcleo, wire y Tauri) + `docs/contracts.md`

**Files:**
- Modify: `crates/atlas-core/src/contracts.rs:48-54` (enum `Agent`), `crates/atlas-core/src/contracts.rs:129-139` (`MessageAppended`), `crates/atlas-core/src/contracts.rs:192-218` (tests), `crates/atlas-core/src/lib.rs:10-13` (re-export), `crates/atlas-core/src/relay.rs:12-15` (import), `crates/atlas-core/src/relay.rs:313-324` (`SnapshotMessage`), `crates/atlas-core/src/relay.rs:350+` (tests), `src-tauri/src/lib.rs:93-103` (`MessageView`), `src-tauri/src/lib.rs:214-229` (`session_get`)
- Docs: `docs/contracts.md:136-150` (bloque `MessageAppended` y párrafo de continuadores)

**Interfaces:**
- Consumes: contrato Python ya emitido (eventos y sesión con `agent`/`taskId`).
- Produces:
  - `atlas_core::Agent` (`Atlas` | `Hefesto`, serde lowercase, re-exportado en `lib.rs`).
  - `MessageAppended { agent: Option<Agent>, task_id: Option<String> }` con `skip_serializing_if = "Option::is_none"` y `rename = "agent"` / `"taskId"`.
  - `SnapshotMessage { agent: Option<Agent>, task_id: Option<String> }` (Deserialize; los mensajes antiguos sin campos deserializan a `None`).
  - `MessageView` (lib.rs) con los mismos campos opcionales para la hidratación.

- [ ] **Step 1: Escribir los tests que fallan**

En `crates/atlas-core/src/contracts.rs`, dentro del módulo `tests`:

```rust
    #[test]
    fn message_appended_con_agente_serializa_campos_opcionales() {
        let event = MessageAppended {
            correlation: Correlation::new("ses-1"),
            kind: EventType::MessageAppended,
            role: Role::Assistant,
            content: "eco".into(),
            source: Source::Local,
            agent: Some(Agent::Hefesto),
            task_id: Some("tsk-1".into()),
        };
        let json = serde_json::to_value(&event).expect("serializa");
        assert_eq!(json["agent"], "hefesto");
        assert_eq!(json["taskId"], "tsk-1");
        let without = MessageAppended {
            agent: None,
            task_id: None,
            ..event.clone()
        };
        let json = serde_json::to_value(&without).expect("serializa");
        assert!(json.get("agent").is_none());
        assert!(json.get("taskId").is_none());
    }
```

En `crates/atlas-core/src/relay.rs`, dentro del módulo `tests`:

```rust
    #[test]
    fn snapshot_message_lee_agent_y_task() {
        let url = fake_brain(vec![(
            "/v1/session",
            r#"{"session":{"sessionId":"ses-1","state":"idle","messages":[{"requestId":"req-1","sessionId":"ses-1","role":"assistant","content":"eco","source":"local","timestamp":"2026-10-29T00:00:00.000Z","agent":"hefesto","taskId":"tsk-1"}]},"cursor":0}"#,
        )]);
        let (snapshot, _) = get_session(&url, Duration::from_secs(2)).expect("ok");
        let message = &snapshot.expect("sesión").messages[0];
        assert_eq!(message.agent, Some(Agent::Hefesto));
        assert_eq!(message.task_id.as_deref(), Some("tsk-1"));
    }
```

- [ ] **Step 2: Ejecutar y confirmar que fallan**

Run: `cargo test -p atlas-core`
Expected: FALLAN por compilación (`Agent` no existe; campos `agent`/`task_id` no existen).

- [ ] **Step 3: Implementar los contratos Rust**

En `crates/atlas-core/src/contracts.rs`, tras el enum `Role` (línea 51-54):

```rust
/// Agente emisor de un mensaje; ausente = Atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Atlas,
    Hefesto,
}
```

En `MessageAppended` (línea 131-139), añadir los campos:

```rust
    pub role: Role,
    pub content: String,
    pub source: Source,
    #[serde(rename = "agent", skip_serializing_if = "Option::is_none")]
    pub agent: Option<Agent>,
    #[serde(rename = "taskId", skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
```

En `crates/atlas-core/src/lib.rs` línea 10-13, añadir `Agent` al re-export:

```rust
pub use contracts::{
    Agent, AtlasError, ConfirmationRequest, Correlation, EventType, MessageAppended, OutEvent,
    RiskLevel, Role, SessionEvent, SessionState, Source, codes, now_iso8601, MAX_CONTENT_LEN,
};
```

En `crates/atlas-core/src/relay.rs`, import (línea 12-15):

```rust
use super::contracts::{
    Agent, AtlasError, ConfirmationRequest, Correlation, EventType, MessageAppended, OutEvent,
    RiskLevel, Role, SessionEvent, SessionState, Source, codes,
};
```

En `SnapshotMessage` (línea 314-324), añadir:

```rust
    pub source: Source,
    pub timestamp: String,
    #[serde(rename = "agent")]
    pub agent: Option<Agent>,
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
```

En `src-tauri/src/lib.rs`, `MessageView` (línea 93-103):

```rust
    content: String,
    source: Source,
    timestamp: String,
    #[serde(rename = "agent", skip_serializing_if = "Option::is_none")]
    agent: Option<atlas_core::Agent>,
    #[serde(rename = "taskId", skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
```

Y en el mapeo de `session_get` (línea 220-227):

```rust
                .map(|m| MessageView {
                    request_id: m.request_id,
                    session_id: m.session_id,
                    role: m.role,
                    content: m.content,
                    source: m.source,
                    timestamp: m.timestamp,
                    agent: m.agent,
                    task_id: m.task_id.clone(),
                })
```

- [ ] **Step 4: Ejecutar las verificaciones Rust**

Run: `cargo check --workspace` y `cargo test --workspace`
Expected: compila y TODO pasa.

- [ ] **Step 5: Actualizar `docs/contracts.md`**

Sustituir el bloque «Mensajes añadidos a la conversación» (línea 136-150) por:

```ts
type MessageRole = 'user' | 'assistant'

type Agent = 'atlas' | 'hefesto'

type MessageAppended = Correlation & {
  type: 'conversation.message_appended'
  role: MessageRole
  content: string // no vacío, máximo 4000 caracteres
  source: Source
  /** Agente emisor del mensaje; ausente = Atlas. */
  agent?: Agent
  /** Correlación con la delegación (taskId del protocolo Hefesto). */
  taskId?: string
}
```

El frontend suscribe este evento y representa el mensaje sin distinguir el origen:
la cadencia visual es idéntica en ambos canales. Un continuador de Hefesto llega
como `message_appended` con `agent: 'hefesto'` y **requestId propio** (nunca
reutiliza el del turno): el interino y el resultado no colisionan. Los
continuadores no emiten eventos de estado de sesión.
```

- [ ] **Step 6: Commit (solo con permiso explícito del usuario)**

```bash
git add crates/atlas-core/src/contracts.rs crates/atlas-core/src/lib.rs crates/atlas-core/src/relay.rs src-tauri/src/lib.rs docs/contracts.md
git commit -m "feat(contracts): agent y taskId en MessageAppended (Rust)"
```

---

### Task 7: Contratos TypeScript y reducer de conversación

**Files:**
- Modify: `src/types/contracts.ts:50-63` (`MessageAppended`), `src/types/contracts.ts:76-84` (`ChatMessage`), `src/state/conversation.ts:59-67` (`toChatMessage`)
- Test: `src/state/conversation.test.ts`

**Interfaces:**
- Consumes: contrato Rust de la Task 6 (nombres idénticos).
- Produces:
  - `export type Agent = 'atlas' | 'hefesto'`.
  - `MessageAppended { agent?: Agent; taskId?: string }` y `ChatMessage { agent?: Agent; taskId?: string }`.
  - `toChatMessage` copia `agent` y `taskId` del evento; la clave de dedupe (`requestId:role`) NO cambia.

- [ ] **Step 1: Escribir los tests que fallan**

En `src/state/conversation.test.ts`:

```ts
  it('conserva agent y taskId del continuador', () => {
    const event: MessageAppended = {
      ...appended('assistant', 'req-2'),
      agent: 'hefesto',
      taskId: 'tsk-1',
    }
    const s = conversationReducer(initialConversationState, {
      type: 'conversation.message_appended',
      event,
    })
    expect(s.messages[0]).toMatchObject({ agent: 'hefesto', taskId: 'tsk-1' })
  })

  it('el continuador con requestId propio no colisiona con el interino', () => {
    const interino = appended('assistant', 'req-1')
    const continuador: MessageAppended = {
      ...appended('assistant', 'req-2'),
      agent: 'hefesto',
      taskId: 'tsk-1',
    }
    let s = conversationReducer(initialConversationState, {
      type: 'conversation.message_appended',
      event: interino,
    })
    s = conversationReducer(s, {
      type: 'conversation.message_appended',
      event: continuador,
    })
    expect(s.messages).toHaveLength(2)
  })
```

- [ ] **Step 2: Ejecutar y confirmar que fallan**

Run: `npm run test`
Expected: los 2 tests nuevos FALLAN (tipos: `agent`/`taskId` no existen en `MessageAppended` → error de compilación de vitest).

- [ ] **Step 3: Implementar los contratos TS y el mapeo**

En `src/types/contracts.ts`, tras `MessageRole` (línea 4):

```ts
export type Agent = 'atlas' | 'hefesto'
```

En `MessageAppended` (línea 50-58), añadir tras `source`:

```ts
  source: Source
  /** Agente emisor del mensaje; ausente = Atlas. */
  agent?: Agent
  /** Correlación con la delegación (taskId del protocolo Hefesto). */
  taskId?: string
```

En `ChatMessage` (línea 76-84), añadir tras `source`:

```ts
  source: Source
  timestamp: string
  agent?: Agent
  taskId?: string
```

En `src/state/conversation.ts`, `toChatMessage` (línea 59-67):

```ts
function toChatMessage(event: MessageAppended): ChatMessage {
  return {
    requestId: event.requestId,
    sessionId: event.sessionId,
    role: event.role,
    content: event.content,
    source: event.source,
    timestamp: event.timestamp,
    agent: event.agent,
    taskId: event.taskId,
  }
}
```

- [ ] **Step 4: Ejecutar las verificaciones del frontend**

Run: `npm run test` y `npm run build`
Expected: TODO pasa y compila (el lint en el Step siguiente).

- [ ] **Step 5: Lint**

Run: `npm run lint`
Expected: sin avisos nuevos.

- [ ] **Step 6: Commit (solo con permiso explícito del usuario)**

```bash
git add src/types/contracts.ts src/state/conversation.ts src/state/conversation.test.ts
git commit -m "feat(contracts): agent y taskId en MessageAppended (TS)"
```

---

### Task 8: UI — etiqueta HEFESTO violeta

**Files:**
- Modify: `src/components/chat/ConversationPanel.tsx:79-113` (etiquetas y clases), `src/App.css` (tras `.chat-msg-assistant`, línea 488)
- Test: `src/components/chat/ConversationPanel.test.tsx` (nuevo; mismo estilo que `MarkdownMessage.test.tsx` con `renderToStaticMarkup`)

**Interfaces:**
- Consumes: `ChatMessage.agent` de la Task 7.
- Produces: mensajes con `agent === 'hefesto'` muestran la etiqueta `HEFESTO` y la clase `chat-msg-hefesto` (además de `chat-msg-assistant`); el resto de asistentes siguen con `ATLAS`. El efecto de escritura ya cubre al continuador (es el último mensaje assistant).

- [ ] **Step 1: Escribir el test que falla**

Crear `src/components/chat/ConversationPanel.test.tsx`:

```tsx
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ConversationPanel } from './ConversationPanel'
import type { ChatMessage } from '../../types/contracts'

function msg(partial: Partial<ChatMessage>): ChatMessage {
  return {
    requestId: 'req-1',
    sessionId: 'ses-1',
    role: 'assistant',
    content: 'eco',
    source: 'local',
    timestamp: '2026-10-29T00:00:00.000Z',
    ...partial,
  }
}

describe('etiquetas de la conversación', () => {
  it('marca HEFESTO el mensaje continuador', () => {
    const html = renderToStaticMarkup(
      <ConversationPanel messages={[msg({ agent: 'hefesto', taskId: 'tsk-1' })]} />,
    )
    expect(html).toContain('HEFESTO')
    expect(html).toContain('chat-msg-hefesto')
  })

  it('mantiene ATLAS para el resto de mensajes assistant', () => {
    const html = renderToStaticMarkup(
      <ConversationPanel messages={[msg({}), msg({ requestId: 'req-2' })]} />,
    )
    expect(html).toContain('ATLAS')
    expect(html).not.toContain('HEFESTO')
  })
})
```

- [ ] **Step 2: Ejecutar y confirmar que falla**

Run: `npm run test`
Expected: FALLA (`HEFESTO` no aparece).

- [ ] **Step 3: Implementar etiqueta y estilo**

En `src/components/chat/ConversationPanel.tsx`, dentro del `map` (sustituir el bloque de la etiqueta y la clase, líneas ~90-99):

```tsx
          const isHefesto = message.agent === 'hefesto'
          const tag = message.role === 'user'
            ? message.source === 'telegram'
              ? 'TG · TÚ'
              : 'TÚ'
            : isHefesto
              ? 'HEFESTO'
              : 'ATLAS'
          return (
            <article
              key={`${message.requestId}-${message.role}`}
              className={
                message.role === 'user'
                  ? 'chat-msg chat-msg-user'
                  : isHefesto
                    ? 'chat-msg chat-msg-assistant chat-msg-hefesto'
                    : 'chat-msg chat-msg-assistant'
              }
            >
              <span className="chat-msg-tag">{tag}</span>
```

En `src/App.css`, tras `.chat-msg-assistant` (línea 488):

```css
.chat-msg-hefesto {
  border: 1px solid rgba(186, 124, 255, 0.35);
  background: rgba(32, 16, 52, 0.55);
}

.chat-msg-hefesto .chat-msg-tag {
  color: rgba(206, 166, 255, 0.75);
}
```

- [ ] **Step 4: Ejecutar las verificaciones**

Run: `npm run test`, `npm run build`, `npm run lint`
Expected: TODO pasa.

- [ ] **Step 5: Comprobación visual en dev (junto con el usuario)**

Run: `npm run dev` (guardar el PID; http://localhost:5173). No hay datos reales aún sin el relay desplegado: comprobar que la app arranca y el chat sigue pintando los mensajes normales de ATLAS. Matar el dev server por su PID al terminar y auditar `ps`/`pgrep`/`launchctl list`.

- [ ] **Step 6: Commit (solo con permiso explícito del usuario)**

```bash
git add src/components/chat/ConversationPanel.tsx src/components/chat/ConversationPanel.test.tsx src/App.css
git commit -m "feat(chat): etiqueta HEFESTO violeta para continuadores"
```

---

### Task 9: Verificación global y revisión de cobertura

**Files:** ninguno nuevo (solo corrección si aparece algo).

- [ ] **Step 1: Suite Python completa**

Run: `python3 -m unittest discover -s server -p 'test_hefesto*.py' -q`
Expected: TODO pasa (~60 previas + 14 nuevas).

- [ ] **Step 2: Suites Rust y frontend completas**

Run: `cargo check --workspace && cargo test --workspace && npm run test && npm run build && npm run lint`
Expected: TODO pasa.

- [ ] **Step 3: Cobertura de la spec**

Repasar la spec (`docs/superpowers/specs/2026-10-29-hefesto-delegacion-design.md`) requisito a requisito y marcar la tarea que lo implementa:

| Requisito de la spec | Tarea |
| --- | --- |
| Interino no bloqueante (tool vuelve al instante) | 3 |
| Continuador `agent: hefesto` violeta, requestId propio | 4, 7, 8 |
| Solo se delega con Hefesto disponible | 3 |
| No disponible → LLM lo comunica, sin continuador | 3 |
| `outcome` controller, lectura pura | 1 |
| Relay como controlador (journal + dispositivo único) | 2, 5 |
| Cancelación del turno sin continuador ni aborto del dispositivo | 4 |
| Recuperación tras reinicio (`unknown` → continuador) | 5 |
| Telegram: seguimiento al chat de origen | 4 |
| Eco sin confirmación; sin acciones reales | 3 (nada más se toca) |
| Contratos espejo en los tres sitios | 6, 7 (+ docs en 1 y 6) |
| dlog HEFESTO con latencia/estado | 3, 4, 5 |

Anotar cualquier hueco y corregirlo antes de seguir.

- [ ] **Step 4: Sin procesos huérfanos**

Run: `pgrep -fl 'vite|tauri|atlas_relay' ; launchctl list | grep -i atlas`
Expected: nada vivo de esta sesión de trabajo en el Mac (el relay vive en atlas-hub, no aquí).

---

### Task 10: Despliegue, prueba en vivo y registro

**Files:**
- Docs: `docs/implementation-plan.md`, `docs/hefesto-plan.md` §8, `docs/implemented.md`

**Prerrequisitos:** cambios commiteados (los pasos de commit anteriores piden permiso; sin commits no se puede desplegar) y **permiso explícito del usuario para desplegar** (reinicia el servicio compartido `atlas-relay` en atlas-hub) y para la prueba con Telegram.

- [ ] **Step 1: Desplegar el relay**

Pide permiso al usuario y ejecuta:

Run: `./scripts/deploy-relay.sh`
Expected: sube los commits, reinicia `atlas-relay` y `/v1/health` responde ok. Comprobar el log: `ssh adria_admin@atlas-hub 'journalctl -u atlas-relay -n 40 --no-pager'` — debe aparecer «Hefesto: listener mTLS habilitado» y «tracker started device=…».

- [ ] **Step 2: Arrancar el frontal en dev**

Run: `npm run dev` (guardar el PID en una variable; se mata al final por ese PID).

- [ ] **Step 3: Delegación local**

Con Hefesto activo en la app (chip listo; si está suspendido, Activar), enviar por el input local: «Pasa un eco a Hefesto con el texto "hola delegación"».
Expected: mensaje interino de ATLAS («se lo paso a Hefesto» o similar) y, al poco, mensaje violeta etiquetado HEFESTO con «hola delegación». En `journalctl -u atlas-relay` (atlas-hub), líneas `HEFESTO: delegated …` y `HEFESTO: continuation … state=succeeded`.

- [ ] **Step 4: Delegación por Telegram**

Enviar el mismo encargo por el bot.
Expected: interino por Telegram y seguimiento «Hefesto: hola delegación» al mismo chat.

- [ ] **Step 5: Cancelación a medias**

Enviar un encargo y cancelar el turno de inmediato (UI).
Expected: sin continuador; el dispositivo queda intacto.

- [ ] **Step 6: Suspensión con encargo en vuelo**

Enviar un encargo y pulsar Suspender en la app.
Expected: continuador «El encargo a Hefesto se canceló.» (el abort cancela la invocación pendiente en el journal).

- [ ] **Step 7: Reinicio del relay con delegación en vuelo**

Enviar un encargo y, antes del resultado, reiniciar el relay con `./scripts/deploy-relay.sh` (o pedir al usuario el reinicio vía Hub Panel).
Expected: al volver, continuador «quedó en un estado desconocido» o el eco si el dispositivo terminó antes.

- [ ] **Step 8: Limpieza de procesos**

Matar el dev server por su PID (`kill <pid>`); ejecutar `ps`/`pgrep`/`launchctl list` y reportar qué queda vivo y cuánta memoria. Nada de Atlas puede quedar huérfano.

- [ ] **Step 9: Registro de trabajo**

Actualizar (solo tras los commits con permiso, para poder citar sus hashes):
- `docs/implementation-plan.md`: marcar el ítem «Entrega 3: delegación por intención (2026-10-29)» con los resultados.
- `docs/hefesto-plan.md` §8: entrada de registro con implementación, pruebas (unitarias y en vivo) y resultados (delegación, cancelación, suspensión, reinicio).
- `docs/implemented.md`: añadir las implementaciones terminadas con el nombre exacto de los commits.

- [ ] **Step 10: Commit de la documentación (solo con permiso explícito del usuario)**

```bash
git add docs/implementation-plan.md docs/hefesto-plan.md docs/implemented.md
git commit -m "docs: registrar la entrega 3 de Hefesto (delegación por intención)"
```
