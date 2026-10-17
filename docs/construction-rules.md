# Reglas de construcción

Estas reglas definen cómo se debe ampliar Atlas. Se aplican al frontend, Rust, documentación, integraciones y publicación.

## Alcance del producto

- Conserva la ventana, la escena visual, el orbe y las órbitas como superficies principales del producto.
- Atlas será un asistente de escritorio con IA, pero las capacidades se añadirán por capas y de forma opcional.
- No añadas audio, STT, wake word, TTS, agentes autónomos, herramientas externas, prompts de sistema ni claves API sin una petición explícita del usuario.
- No mezcles lógica de IA, proveedores o herramientas dentro de los componentes visuales.

## Antes de cambiar código

- Lee el código y la documentación directamente relacionados con el cambio.
- Consulta `docs/index.md` como entrada principal de documentación.
- Ejecuta `git status --short` y conserva los cambios ajenos.
- Usa `rg` para localizar usos antes de retirar una API, comando IPC, dependencia o contrato.
- No recorras ni reestructures todo el repositorio si el cambio no lo requiere.

## Arquitectura

- React y Three.js se encargan de la presentación, interacción y estados visuales.
- Rust y Tauri se encargan de capacidades nativas, IPC, permisos, persistencia y coordinación.
- Usa contratos tipados para eventos, comandos, errores y estados compartidos.
- Mantén los proveedores de IA detrás de interfaces reemplazables.
- Mantén cada herramienta o integración en un adaptador aislado y desactivable.
- El frontend nunca debe conocer claves, tokens ni detalles internos de proveedores.

## Seguridad y permisos

- Nunca guardes secretos, tokens, contraseñas, cookies o claves privadas en el repositorio.
- Usa el llavero del sistema o configuración local ignorada por Git.
- Valida las entradas antes de ejecutar herramientas y normaliza las respuestas externas antes de mostrarlas.
- Requiere confirmación explícita para escritura, comunicación externa, ejecución de comandos, cambios de configuración y borrado.
- La falta de respuesta nunca significa autorización.
- Incluye cancelación, timeouts y estados de error en operaciones que puedan tardar o producir efectos.
- No expongas secretos, rutas privadas, datos personales ni contenido sensible en logs o respuestas.

## TypeScript y Rust

- Mantén TypeScript estricto; no rebajes la configuración del compilador para ocultar errores.
- Mantén Rust modular y devuelve errores estructurados a través de IPC.
- Evita `unwrap` y `expect` en rutas controladas por el usuario o dependientes del entorno.
- Versiona los eventos públicos, payloads y transiciones de estado cuando cambien.
- Prefiere interfaces pequeñas, funciones acotadas y dependencias explícitas.
- No introduzcas estado global mutable si una sesión o un módulo puede ser su propietario.

## Pruebas y verificación

- Ejecuta, cuando estén disponibles, `npm run build`, `npm run lint` y `cargo check --workspace`.
- Comprueba manualmente los estados visuales afectados por un cambio de frontend.
- Verifica cancelación, errores y permisos en cualquier flujo que ejecute acciones.
- Si cambia el bundle de macOS, regénéralo con la identidad Apple Development disponible y verifica la firma profunda.
- No consideres terminado un cambio solo porque compile: debe respetar contratos y documentación.

## Documentación

- Actualiza la documentación cuando cambien arquitectura, contratos, permisos o comportamiento visible.
- Añade implementaciones terminadas a `docs/implemented.md` usando el nombre exacto del commit.
- Mantén el plan en `docs/implementation-plan.md` alineado con el estado real.
- Usa ejemplos concretos y documenta límites, errores y permisos.

## Commits y publicación en la nube

- Crea un commit únicamente cuando el usuario dé permiso explícito.
- Cuando se conceda el permiso, publica inmediatamente el commit en el remoto de GitHub configurado.
- No dejes un commit autorizado únicamente en el repositorio local.
- Antes de publicar, verifica la URL del remoto, la rama de destino y el estado del árbol de trabajo.
- No hagas push, reescribas el historial ni cambies de rama sin permiso explícito del usuario.
- El relay corre en `homelab` como servicio de usuario (`atlas-relay.service`): cuando el cambio afecte a `server/`, despliega con `./scripts/deploy-relay.sh` (commitea, sube, baja los cambios en `homelab`, reinicia el servicio y comprueba `/v1/health`).
