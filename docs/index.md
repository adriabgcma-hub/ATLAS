# Atlas

Documentación del proyecto Atlas.

## Documentos

- [Arquitectura](./architecture.md): describe las capas de Atlas, sus responsabilidades y el flujo de una petición.
- [Contratos](./contracts.md): define los mensajes IPC, estados de sesión, herramientas, permisos y errores compartidos.
- [Plan de implementación](./implementation-plan.md): organiza las fases previstas para construir Atlas.
- [Hefesto](./hefesto-plan.md): plan revisado del especialista del Mac, fronteras de confianza, protocolo y puertas de validación.
- [Implementado](./implemented.md): mantiene el historial de implementaciones y el nombre de cada commit asociado.
- [Seguridad y privacidad](./security.md): define el tratamiento de secretos, permisos, datos y sensores.
- [Release](./release.md): describe las comprobaciones y comandos para generar y publicar bundles.
- [Tools](./tools.md): catálogo de herramientas del cerebro.
- [API del cerebro](./relay-api.md): documenta la API HTTP del relay Python (eventos, sesiones, límites y entorno).
- [Relay: manual de operaciones](./relay.md): qué es el relay, dónde corre (atlas-hub) y el panel — para colaboradores y otras IAs.
- [Hub Panel: app macOS independiente](./panel-app-plan.md): plan y estado de la app de barra de menús (`apps/panel-app`) para monitorizar y reiniciar el relay aunque esté colgado (F1–F6: bundle firmado e instalado).
- [Atlas Hub: servidor de servicios y acceso remoto](./atlas-hub-plan.md): diseño del servidor de servicios (Nextcloud, Jellyfin), acceso remoto (Headscale) y consola (Cockpit) sobre `atlas-hub`.
