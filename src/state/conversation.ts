import type {
  ActiveSession,
  ChatMessage,
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
}

export const initialConversationState: ConversationState = {
  sessionId: null,
  sessionState: 'idle',
  messages: [],
  pendingRequestIds: [],
  lastError: null,
}

export type ConversationAction =
  | { type: 'session.state_changed'; event: SessionEvent }
  | { type: 'conversation.message_appended'; event: MessageAppended }
  | { type: 'conversation.error'; message: string; requestId?: string }
  | { type: 'conversation.hydrate'; session: ActiveSession }
  | { type: 'conversation.reset' }

/** Tope de mensajes en el frontal (el cerebro guarda 100; margen local). */
const MAX_MESSAGES = 200

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

export function conversationReducer(
  state: ConversationState,
  action: ConversationAction,
): ConversationState {
  switch (action.type) {
    case 'session.state_changed': {
      const { event } = action
      const pending =
        event.state === 'cancelled' || event.state === 'completed' || event.state === 'failed'
          ? state.pendingRequestIds.filter((id) => id !== event.requestId)
          : state.pendingRequestIds.includes(event.requestId)
            ? state.pendingRequestIds
            : [...state.pendingRequestIds, event.requestId]
      return {
        ...state,
        sessionId: event.sessionId,
        sessionState: event.state,
        pendingRequestIds: pending,
        lastError: event.state === 'failed' ? (event.message ?? 'unknown_error') : state.lastError,
      }
    }
    case 'conversation.message_appended': {
      const message = toChatMessage(action.event)
      if (state.messages.some((m) => m.requestId === message.requestId && m.role === message.role)) {
        return state
      }
      const messages = [...state.messages, message]
      return {
        ...state,
        sessionId: action.event.sessionId,
        messages: messages.length > MAX_MESSAGES ? messages.slice(-MAX_MESSAGES) : messages,
      }
    }
    case 'conversation.error': {
      return { ...state, sessionState: 'failed', lastError: action.message }
    }
    case 'conversation.hydrate': {
      // Al reabrir, una operación a medias ya murió con el proceso anterior:
      // se conserva el historial pero el estado vuelve a reposo.
      const settled = isSessionSettled(action.session.state)
      return {
        sessionId: action.session.sessionId,
        sessionState: settled ? action.session.state : 'idle',
        messages: action.session.messages,
        pendingRequestIds: [],
        lastError: null,
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
