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

Para Hefesto rige la matriz de [su plan](./hefesto-plan.md): prohibición > permiso
individual > plan > adaptador trivial. Crear no permite sobrescribir, un plan no
permite saltar la confirmación destructiva y no se admite elevación mediante `sudo`.
Las excepciones triviales describen herramientas futuras: la base solo admite eco.

## Frontera de Hefesto (base local, no desplegada)

El HTTP de Atlas no autoriza acciones del Mac. Hefesto usa un listener distinto con
mTLS, CA explícita y registro de huellas de certificados ligado a rol/dispositivo.
La huella se comprueba al terminar el handshake, antes de procesar HTTP, y un
certificado no listado se cierra sin respuesta; cada conexión tiene un plazo
absoluto que la cierra aunque el cliente gotee bytes, para que el trabajo lento
previo a la autenticación no agote las plazas del listener.
No hay activación por defecto, downgrade, credenciales en frontend ni ejecución desde
mensajes de chat. Las entradas y confirmaciones de delegación real siguen pendientes.

El ejecutor empieza suspendido; las instancias, generaciones, leases e invocaciones
impiden reutilizar entregas antiguas. Un resultado desconocido no se reejecuta. El
aborto local no depende de la red y no revierte efectos ya producidos; falta todavía
integrarlo en los controles de app/panel antes de habilitar capacidades reales.

Las pruebas usan certificados efímeros y texto ficticio. No provisionar certificados,
TCC ni secretos reales sin permiso. El journal de eco persiste texto de prueba para
validar duplicados; no admite documentos privados ni hace llamadas a proveedores.

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

