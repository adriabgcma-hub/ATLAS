# Plan de implementación

Este documento organiza la evolución de Atlas hacia un asistente de escritorio con IA, manteniendo la interfaz visual como núcleo del producto.

## Fase 1 — Base visual estable

> **Estado: completada** — commit `Anclar documentación y cerrar fase 1` (2026-10-12). Build, lint y `cargo check --workspace` verificados.

- [x] Mantener la ventana Tauri transparente.
- [x] Consolidar la escena WebGL, el orbe y las órbitas.
- [x] Mantener estados visuales claros: `idle`, `thinking`, `executing` y `error`.
- [x] Verificar build, lint y comprobación de Rust.

## Fase 2 — Interacción y sesiones

- Añadir la interfaz de conversación.
- Crear sesiones identificadas y cancelables.
- Implementar los eventos y comandos definidos en `contracts.md`.
- Separar el estado visual del estado de conversación.

## Fase 3 — Núcleo de IA

- Crear una interfaz de proveedor de modelos.
- Añadir construcción de contexto y límites de tamaño.
- Incorporar respuestas progresivas y cancelación.
- Mantener las credenciales fuera del frontend y del repositorio.

## Fase 4 — Memoria local

- Persistir sesiones y preferencias localmente.
- Separar memoria temporal, memoria de conversación y memoria autorizada del usuario.
- Añadir consulta, edición y borrado de memoria.

## Fase 5 — Herramientas y permisos

- Crear el registro de herramientas.
- Validar manifiestos, argumentos y resultados.
- Implementar confirmaciones para escritura, acciones externas y operaciones destructivas.
- Registrar auditoría sin secretos.

## Fase 6 — Capacidades opcionales

- Añadir integraciones con archivos, calendario, navegador y otros servicios mediante adaptadores.
- Incorporar audio, STT, wake word y TTS solo como módulos opcionales y con consentimiento explícito.
- Mantener cada capacidad desactivable de forma independiente.

## Regla de avance

Una fase solo puede considerarse terminada cuando su comportamiento está implementado, probado, documentado y asociado a un commit publicado en GitHub.

