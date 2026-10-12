# Release

## Preparación

Antes de generar una versión:

- Comprueba `git status --short` y confirma que los cambios incluidos son los correctos.
- Ejecuta `npm run build` para validar el frontend.
- Ejecuta `npm run lint`.
- Ejecuta `cargo check --workspace`.
- Revisa que no haya secretos, archivos `.env`, claves, certificados ni artefactos locales incluidos.
- Verifica que la versión de `package.json`, `src-tauri/tauri.conf.json` y los metadatos relevantes sea coherente.

## Bundle de depuración

Para regenerar el bundle de Tauri en modo debug:

```bash
npm run tauri build -- --debug
```

Este comando debe ejecutarse después de que el build del frontend y las comprobaciones de Rust hayan pasado. El resultado se genera dentro de `src-tauri/target/`, que está excluido de Git.

## Bundle de distribución

Para generar el bundle de distribución:

```bash
npm run tauri build
```

Si se modifica el bundle de macOS:

- Regénéralo con la identidad Apple Development disponible.
- Verifica la firma profunda de la aplicación.
- Comprueba que los binarios auxiliares y recursos incluidos correspondan a la versión actual.
- No publiques el bundle si la firma o los permisos no se pueden verificar.

## Publicación

- Actualiza `docs/implemented.md` con el nombre exacto del commit.
- Crea el commit solo con permiso explícito del usuario.
- Verifica el remoto y la rama antes de publicar.
- Publica el commit en GitHub inmediatamente después de crearlo.

