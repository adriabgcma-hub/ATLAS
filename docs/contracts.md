# Contratos del sistema

Estos contratos describen las fronteras públicas de Atlas. Son una especificación inicial; cuando se implementen deben convertirse en tipos estrictos compartidos o equivalentes validados en Rust y TypeScript.

## Identificadores

Todos los eventos de una operación deben incluir:

```ts
type Correlation = {
  requestId: string
  sessionId: string
  timestamp: string // ISO-8601 UTC
}
```

Los identificadores se generan en el núcleo y nunca contienen secretos ni datos personales.

## Ciclo de una sesión

```ts
type SessionState =
  | 'idle'
  | 'thinking'
  | 'awaiting_confirmation'
  | 'executing'
  | 'responding'
  | 'cancelled'
  | 'completed'
  | 'failed'
```

```ts
type SessionEvent = Correlation & {
  type: 'session.state_changed'
  state: SessionState
  message?: string
}
```

El frontend solo representa este estado; no puede forzar una transición de ejecución sin pasar por IPC y permisos.

## Mensajes IPC

```ts
type UserMessage = Correlation & {
  type: 'conversation.send'
  content: string
  attachments?: Array<{ id: string; mediaType: string; name: string }>
}

type CancelRequest = Correlation & {
  type: 'conversation.cancel'
}
```

El núcleo debe rechazar mensajes sin `requestId`, con contenido vacío o con adjuntos que no hayan sido autorizados por la sesión.

## Herramientas

```ts
type RiskLevel = 'read' | 'write' | 'external' | 'destructive'

type ToolManifest = {
  id: string
  version: string
  description: string
  risk: RiskLevel
  inputSchema: unknown
  outputSchema: unknown
  requiresConfirmation: boolean
}

type ToolCall = Correlation & {
  type: 'tool.call'
  toolId: string
  arguments: unknown
}

type ToolResult = Correlation & {
  type: 'tool.result'
  toolId: string
  status: 'succeeded' | 'failed' | 'rejected' | 'cancelled'
  output?: unknown
  errorCode?: string
}
```

Los argumentos se validan contra `inputSchema` antes de ejecutar el adaptador. Los resultados externos se normalizan antes de devolverse al modelo.

## Confirmaciones

```ts
type ConfirmationRequest = Correlation & {
  type: 'permission.confirmation_required'
  toolId: string
  summary: string
  risk: RiskLevel
  expiresAt: string
}

type ConfirmationResponse = Correlation & {
  type: 'permission.respond'
  decision: 'allow_once' | 'deny'
}
```

Nunca se debe interpretar la ausencia de respuesta como autorización. Las confirmaciones caducan y se vinculan a un único `requestId`.

## Origen del mensaje

```ts
type Source = 'local' | 'telegram'
```

`local` es la barra de input de la app; `telegram` es el bot consultado por
polling saliente. El núcleo trata ambos igual; solo cambia el origen registrado.

## Mensajes añadidos a la conversación

```ts
type MessageRole = 'user' | 'assistant'

type MessageAppended = Correlation & {
  type: 'conversation.message_appended'
  role: MessageRole
  content: string // no vacío, máximo 4000 caracteres
  source: Source
}
```

El frontend suscribe este evento y representa el mensaje sin distinguir el origen:
la cadencia visual es idéntica en ambos canales.

## Errores

Los errores expuestos al frontend deben ser seguros para mostrar:

```ts
type AtlasError = {
  code: string
  message: string
  retryable: boolean
  requestId?: string
}
```

Los detalles internos, rutas privadas, tokens y respuestas completas de proveedores solo pueden aparecer en logs locales protegidos, y nunca en eventos de UI.

