import type {
  ActiveSession,
  ChatMessage,
  ConfirmationRequest,
  MessageAppended,
  SessionEvent,
  SessionState,
} from '../types/contracts'
import type { DeriveOrbStateArgs } from './orbState'

/**
 * Estado de conversación del frontend (Fase 2).
 * Separado del estado visual: la escena solo recibe `OrbState` derivado.
 * Se alimenta de eventos IPC (`session.state_changed`,
 * `conversation.message_appended`); nunca fuerza transiciones.
 */
export interface ConversationState {
  sessionId: string | null
  sessionState: SessionState
  messages: ChatMessage[]
  /** requestIds en vuelo (thinking/responding): sirven para cancelar. */
  pendingRequestIds: string[]
  lastError: string | null
  /** Confirmaciones pendientes de la UI (una por requestId). */
  confirmations: ConfirmationRequest[]
}

export const initialConversationState: ConversationState = {
  sessionId: null,
  sessionState: 'idle',
  messages: [],
  pendingRequestIds: [],
  lastError: null,
  confirmations: [],
}

export type ConversationAction =
  | { type: 'session.state_changed'; event: SessionEvent }
  | { type: 'conversation.message_appended'; event: MessageAppended }
  | { type: 'conversation.error'; message: string; requestId?: string }
  | { type: 'conversation.hydrate'; session: ActiveSession }
  | { type: 'permission.confirmation_required'; event: ConfirmationRequest }
  | { type: 'permission.resolved'; requestId: string }
  | { type: 'conversation.reset' }

/** Tope de mensajes en el frontal (el cerebro guarda 100; margen local). */
const MAX_MESSAGES = 200

function messageKey(message: ChatMessage): string {
  return `${message.requestId}:${message.role}`
}

/**
 * Fusiona la instantánea con los mensajes recibidos en vivo: la hidratación
 * y el drenado corren en paralelo, así que el snapshot puede ser anterior al
 * último evento y no debe borrarlo (el frontal volvería a imprimirlo).
 */
function mergeHydrated(local: ChatMessage[], snapshot: ChatMessage[]): ChatMessage[] {
  const known = new Set(snapshot.map(messageKey))
  const lastKey = snapshot.length > 0 ? messageKey(snapshot[snapshot.length - 1]) : ''
  const tail = local.slice(local.findIndex((m) => messageKey(m) === lastKey) + 1)
  const merged = [...snapshot, ...tail.filter((m) => !known.has(messageKey(m)))]
  return merged.length > MAX_MESSAGES ? merged.slice(-MAX_MESSAGES) : merged
}

const TERMINAL: ReadonlySet<SessionState> = new Set([
  'idle',
  'cancelled',
  'completed',
  'failed',
])

function toChatMessage(event: MessageAppended): ChatMessage {
  return {
    requestId: event.requestId,
    sessionId: event.sessionId,
    role: event.role,
    content: event.content,
    source: event.source,
    timestamp: event.timestamp,
  }
}

/**
 * Nueva sesión en los eventos = panel nuevo. Sin esto, cada `/clear`
 * acumulaba su "Historial reiniciado" sobre las sesiones anteriores.
 */
function ensureSession(state: ConversationState, sessionId: string): ConversationState {
  if (state.sessionId === null || state.sessionId === sessionId) return state
  return { ...initialConversationState, sessionId }
}

export function conversationReducer(
  state: ConversationState,
  action: ConversationAction,
): ConversationState {
  switch (action.type) {
    case 'session.state_changed': {
      const { event } = action
      const base = ensureSession(state, event.sessionId)
      const pending =
        event.state === 'cancelled' || event.state === 'completed' || event.state === 'failed'
          ? base.pendingRequestIds.filter((id) => id !== event.requestId)
          : base.pendingRequestIds.includes(event.requestId)
            ? base.pendingRequestIds
            : [...base.pendingRequestIds, event.requestId]
      return {
        ...base,
        sessionId: event.sessionId,
        sessionState: event.state,
        pendingRequestIds: pending,
        lastError: event.state === 'failed' ? (event.message ?? 'unknown_error') : base.lastError,
      }
    }
    case 'conversation.message_appended': {
      const base = ensureSession(state, action.event.sessionId)
      const message = toChatMessage(action.event)
      if (base.messages.some((m) => m.requestId === message.requestId && m.role === message.role)) {
        return base
      }
      const messages = [...base.messages, message]
      return {
        ...base,
        sessionId: action.event.sessionId,
        messages: messages.length > MAX_MESSAGES ? messages.slice(-MAX_MESSAGES) : messages,
      }
    }
    case 'conversation.error': {
      return { ...state, sessionState: 'failed', lastError: action.message }
    }
    case 'permission.confirmation_required': {
      const base = ensureSession(state, action.event.sessionId)
      const rest = base.confirmations.filter(
        (c) => c.requestId !== action.event.requestId,
      )
      return { ...base, confirmations: [...rest, action.event] }
    }
    case 'permission.resolved': {
      return {
        ...state,
        confirmations: state.confirmations.filter(
          (c) => c.requestId !== action.requestId,
        ),
      }
    }
    case 'conversation.hydrate': {
      // Al reabrir, una operación a medias ya murió con el proceso anterior:
      // se conserva el historial pero el estado vuelve a reposo.
      const settled = isSessionSettled(action.session.state)
      const sameSession = state.sessionId === action.session.sessionId
      return {
        sessionId: action.session.sessionId,
        sessionState: settled ? action.session.state : 'idle',
        messages: sameSession
          ? mergeHydrated(state.messages, action.session.messages)
          : action.session.messages,
        pendingRequestIds: [],
        lastError: null,
        // Las confirmaciones son en vivo: al recargar caducan solas.
        confirmations: [],
      }
    }
    case 'conversation.reset': {
      return initialConversationState
    }
  }
}

/** ¿La sesión está en un estado sin actividad en curso? */
export function isSessionSettled(state: SessionState): boolean {
  return TERMINAL.has(state)
}

/**
 * Mapea `SessionState` (núcleo) a la entrada de `deriveOrbState` (visual).
 * La escena no conoce sesiones: solo recibe intensidad/pulso/error.
 */
export function sessionStateToOrbInput(
  state: SessionState,
  hasContent: boolean,
): DeriveOrbStateArgs {
  switch (state) {
    case 'thinking':
    case 'awaiting_confirmation':
    case 'executing':
      return { isStreaming: true, hasContent, isMuted: false, isSpeaking: false, lastError: false }
    case 'responding':
      return { isStreaming: true, hasContent: true, isMuted: false, isSpeaking: false, lastError: false }
    case 'failed':
      return { isStreaming: false, hasContent, isMuted: false, isSpeaking: false, lastError: true }
    case 'idle':
    case 'cancelled':
    case 'completed':
      return { isStreaming: false, hasContent, isMuted: false, isSpeaking: false, lastError: false }
  }
}
