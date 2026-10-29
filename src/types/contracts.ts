/**
 * Contratos compartidos TS (Fase 2). Espejo de `docs/contracts.md` y de
 * `crates/atlas-core/src/contracts.rs`: cualquier cambio debe hacerse en los
 * tres sitios a la vez. Sin enums (tsconfig usa `erasableSyntaxOnly`).
 */

export type Source = 'local' | 'telegram'

export type MessageRole = 'user' | 'assistant'

export type Agent = 'atlas' | 'hefesto'

export type SessionState =
  | 'idle'
  | 'thinking'
  | 'awaiting_confirmation'
  | 'executing'
  | 'responding'
  | 'cancelled'
  | 'completed'
  | 'failed'

export interface Correlation {
  requestId: string
  sessionId: string
  /** ISO-8601 UTC. */
  timestamp: string
}

export interface SessionEvent extends Correlation {
  type: 'session.state_changed'
  state: SessionState
  message?: string
}

export interface MessageAppended extends Correlation {
  type: 'conversation.message_appended'
  role: MessageRole
  content: string
  source: Source
  /** Agente emisor del mensaje; ausente = Atlas. */
  agent?: Agent
  /** Correlación con la delegación (taskId del protocolo Hefesto). */
  taskId?: string
}

export interface AtlasError {
  code: string
  message: string
  retryable: boolean
  requestId?: string
}

export type ConfirmationDecision = 'allow_once' | 'deny'

export interface ConfirmationRequest extends Correlation {
  type: 'permission.confirmation_required'
  toolId: string
  summary: string
  risk: 'read' | 'write' | 'external' | 'destructive'
  expiresAt: string
}

/** Mensaje guardado en el estado de conversación del frontend. */
export interface ChatMessage {
  requestId: string
  sessionId: string
  role: MessageRole
  content: string
  source: Source
  timestamp: string
  agent?: Agent
  taskId?: string
}

/** Resultado de `conversation_send` / mensajes drenados del relay. */
export interface SendResult {
  requestId: string
  sessionId: string
}

/** Resultado de `relay_drain`. */
export interface DrainResult {
  delivered: number
  skipped: number
}

/** Sesión activa devuelta por `session_get`. */
export interface ActiveSession {
  sessionId: string
  state: SessionState
  messages: ChatMessage[]
}

/** Fase del dispositivo Hefesto (runtime nativo dentro de la app). */
export type HefestoPhase =
  | 'unconfigured'
  | 'connecting'
  | 'suspended'
  | 'ready'
  | 'offline'
  | 'error'

/** Estado que publica el bucle nativo de Hefesto. */
export interface HefestoSnapshot {
  phase: HefestoPhase
  deviceId: string | null
  generation: number | null
  lastEcho: string | null
  /** Código estable corto; nunca rutas, certificados ni secretos. */
  detail: string | null
}

/** Evento IPC `hefesto-state-changed`. */
export interface HefestoEvent extends HefestoSnapshot {
  type: 'hefesto.state_changed'
}
