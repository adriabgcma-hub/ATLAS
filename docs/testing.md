# Testing

## Objetivo

Las pruebas deben validar el comportamiento de la interfaz, la coordinación nativa y los contratos entre TypeScript, Rust y Python. Un build correcto no sustituye las pruebas de comportamiento.

## Frontend TypeScript

- Ejecuta `npm run build` para comprobar tipos y compilación de Vite.
- Ejecuta `npm run lint` para detectar errores de estilo, hooks y contratos de React.
- Verifica manualmente la ventana transparente, el orbe, las órbitas, el redimensionado y los estados visuales.
- Comprueba que una animación detenida o un error no bloquee la interfaz.
- Si se añaden pruebas automatizadas, cubre componentes puros, transformaciones de estado y eventos antes que detalles visuales frágiles.

## Tauri y Rust

- Ejecuta `cargo check --workspace` después de cualquier cambio en Rust o configuración Tauri.
- Valida comandos IPC con entradas válidas, inválidas, vacías y canceladas.
- Comprueba que los errores se devuelvan de forma estructurada y segura para mostrar.
- Verifica permisos, confirmaciones, timeouts y cancelación de operaciones.
- Comprueba el ciclo de vida de la ventana: mostrar, ocultar, enfocar, minimizar y cerrar.
- Para cambios de bundle, ejecuta `npm run tauri build -- --debug` y verifica que el artefacto se genere correctamente.

## Python

Python puede utilizarse para scripts auxiliares, fixtures, validación de recursos o pruebas de integración. No debe convertirse en una dependencia implícita del runtime de la aplicación sin documentarlo.

- Fija la versión mínima de Python requerida por cada script.
- Mantén las dependencias en un archivo de requisitos separado si aparecen.
- Haz que los scripts fallen con códigos de salida claros.
- No uses Python para modificar archivos del proyecto cuando una edición normal sea suficiente.
- No incluyas entornos virtuales, cachés, reportes locales ni secretos en Git.
- Valida entradas y rutas de los scripts para evitar escribir fuera de los directorios esperados.

## Contratos e integración

- Prueba que los eventos emitidos por Rust coincidan con los tipos consumidos por TypeScript.
- Comprueba `requestId`, `sessionId`, estados y códigos de error en cada flujo.
- Simula proveedores de IA y herramientas externas en pruebas locales; no dependas de una API real para pruebas unitarias.
- Verifica que una herramienta no pueda ejecutarse sin registro ni permiso.
- Comprueba que una confirmación rechazada no produzca efectos secundarios.

## Regresión y entrega

Antes de entregar un cambio, ejecuta como mínimo:

```bash
npm run build
npm run lint
cargo check --workspace
```

Si el cambio afecta Tauri o el bundle, añade:

```bash
npm run tauri build -- --debug
```

Si el cambio afecta a `server/` (el relay), despliégalo y verifica su salud:

```bash
./scripts/deploy-relay.sh
```

El script reinicia el servicio `atlas-relay` en `homelab` y comprueba `/v1/health`; si falla, revisa `journalctl --user -u atlas-relay -n 20` (desde `homelab`).

Documenta los fallos conocidos, el alcance de las pruebas y cualquier dependencia externa necesaria para reproducirlas.

