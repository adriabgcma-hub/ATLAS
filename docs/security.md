# Seguridad y privacidad

## Reglas principales

- No guardar claves API, tokens, contraseñas, cookies ni claves privadas en el repositorio.
- Mantener los secretos en el llavero del sistema o en archivos locales ignorados por Git.
- No exponer secretos, rutas privadas ni datos personales en logs, prompts o mensajes de interfaz.
- Aplicar el principio de mínimo privilegio a cada herramienta, integración y sesión.
- Mantener desactivadas las capacidades no solicitadas por el usuario.

## Permisos y confirmaciones

Las herramientas deben declarar su nivel de riesgo y sus permisos requeridos. Las operaciones de escritura, comunicación externa, ejecución de comandos, cambios de configuración y borrado requieren confirmación explícita.

La ausencia de respuesta nunca equivale a autorización. Las confirmaciones deben caducar, estar vinculadas a una única petición y poder rechazarse o cancelarse.

## Datos

Atlas debe distinguir entre:

- **Públicos**: pueden mostrarse o enviarse según la configuración.
- **Locales**: permanecen en el dispositivo.
- **Sensibles**: requieren minimización y consentimiento.
- **Secretos**: nunca se incluyen en prompts, logs ni eventos de interfaz.

Los datos enviados a proveedores externos deben estar limitados al contexto necesario y reflejarse en la configuración de privacidad.

## Audio y sensores

El micrófono, la escucha permanente, el wake word, STT y TTS deben permanecer desactivados hasta que el usuario los active expresamente. La interfaz debe mostrar cuándo un sensor está en uso y ofrecer una forma inmediata de detenerlo.

## Auditoría

Las operaciones deben poder relacionarse mediante `requestId` y `sessionId`. La auditoría puede registrar estados, herramientas y resultados generales, pero debe redactar tokens, cabeceras de autorización, cookies, rutas sensibles y contenido privado innecesario.

