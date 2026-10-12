import { describe, expect, it } from 'vitest'
import {
  conversationReducer,
  initialConversationState,
  isSessionSettled,
  sessionStateToOrbInput,
  type ConversationState,
} from './conversation'
import type { MessageAppended, SessionEvent } from '../types/contracts'
import { deriveOrbState } from './orbState'

function stateChanged(
  state: ConversationState['sessionState'],
  requestId = 'req-1',
): SessionEvent {
  return {
    type: 'session.state_changed',
    requestId,
    sessionId: 'ses-1',
    timestamp: '2026-10-12T00:00:00.000Z',
    state,
  }
}

function appended(role: 'user' | 'assistant', requestId = 'req-1'): MessageAppended {
  return {
    type: 'conversation.message_appended',
    requestId,
    sessionId: 'ses-1',
    timestamp: '2026-10-12T00:00:00.000Z',
    role,
    content: 'hola',
    source: 'local',
  }
}

describe('conversationReducer', () => {
  it('registra thinking y el request en vuelo', () => {
    const next = conversationReducer(initialConversationState, {
      type: 'session.state_changed',
      event: stateChanged('thinking'),
    })
    expect(next.sessionState).toBe('thinking')
    expect(next.pendingRequestIds).toEqual(['req-1'])
  })

  it('añade mensajes sin duplicar el mismo rol+request', () => {
    const s1 = conversationReducer(initialConversationState, {
      type: 'conversation.message_appended',
      event: appended('user'),
    })
    const s2 = conversationReducer(s1, {
      type: 'conversation.message_appended',
      event: appended('user'),
    })
    expect(s2.messages).toHaveLength(1)
    const s3 = conversationReducer(s2, {
      type: 'conversation.message_appended',
      event: appended('assistant'),
    })
    expect(s3.messages).toHaveLength(2)
  })

  it('completed limpia el request en vuelo', () => {
    const s1 = conversationReducer(initialConversationState, {
      type: 'session.state_changed',
      event: stateChanged('thinking'),
    })
    const s2 = conversationReducer(s1, {
      type: 'session.state_changed',
      event: stateChanged('completed'),
    })
    expect(s2.pendingRequestIds).toEqual([])
    expect(s2.sessionState).toBe('completed')
  })

  it('failed guarda el mensaje de error', () => {
    const event = { ...stateChanged('failed'), message: 'relay_unreachable' }
    const next = conversationReducer(initialConversationState, {
      type: 'session.state_changed',
      event,
    })
    expect(next.lastError).toBe('relay_unreachable')
  })

  it('hydrate conserva historial y aparca estados a medias en idle', () => {
    const next = conversationReducer(initialConversationState, {
      type: 'conversation.hydrate',
      session: {
        sessionId: 'ses-1',
        state: 'thinking',
        messages: [
          {
            requestId: 'req-1',
            sessionId: 'ses-1',
            role: 'user',
            content: 'hola',
            source: 'telegram',
            timestamp: '2026-10-12T00:00:00.000Z',
          },
        ],
      },
    })
    expect(next.sessionState).toBe('idle')
    expect(next.messages).toHaveLength(1)
    expect(next.pendingRequestIds).toEqual([])
  })
})

describe('sessionStateToOrbInput + deriveOrbState', () => {
  it('thinking -> thinking visual', () => {
    expect(deriveOrbState(sessionStateToOrbInput('thinking', false))).toBe('thinking')
  })
  it('responding -> responding visual', () => {
    expect(deriveOrbState(sessionStateToOrbInput('responding', true))).toBe('responding')
  })
  it('failed -> error visual', () => {
    expect(deriveOrbState(sessionStateToOrbInput('failed', true))).toBe('error')
  })
  it('completed/cancelled/idle -> idle visual', () => {
    expect(deriveOrbState(sessionStateToOrbInput('completed', true))).toBe('idle')
    expect(deriveOrbState(sessionStateToOrbInput('cancelled', true))).toBe('idle')
    expect(deriveOrbState(sessionStateToOrbInput('idle', false))).toBe('idle')
  })
  it('isSessionSettled distingue activos de terminales', () => {
    expect(isSessionSettled('thinking')).toBe(false)
    expect(isSessionSettled('responding')).toBe(false)
    expect(isSessionSettled('completed')).toBe(true)
    expect(isSessionSettled('cancelled')).toBe(true)
  })
})
