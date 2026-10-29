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

  it('conserva agent y taskId del continuador', () => {
    const event: MessageAppended = {
      ...appended('assistant', 'req-2'),
      agent: 'hefesto',
      taskId: 'tsk-1',
    }
    const s = conversationReducer(initialConversationState, {
      type: 'conversation.message_appended',
      event,
    })
    expect(s.messages[0]).toMatchObject({ agent: 'hefesto', taskId: 'tsk-1' })
  })

  it('el continuador con requestId propio no colisiona con el interino', () => {
    const interino = appended('assistant', 'req-1')
    const continuador: MessageAppended = {
      ...appended('assistant', 'req-2'),
      agent: 'hefesto',
      taskId: 'tsk-1',
    }
    let s = conversationReducer(initialConversationState, {
      type: 'conversation.message_appended',
      event: interino,
    })
    s = conversationReducer(s, {
      type: 'conversation.message_appended',
      event: continuador,
    })
    expect(s.messages).toHaveLength(2)
  })

  it('completed limpia el request en vuelo', () => {    const s1 = conversationReducer(initialConversationState, {
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

  it('nueva sesión reinicia el panel', () => {
    const s1 = conversationReducer(initialConversationState, {
      type: 'conversation.message_appended',
      event: appended('user'),
    })
    expect(s1.messages).toHaveLength(1)
    const other = { ...appended('assistant'), sessionId: 'ses-2', requestId: 'req-9' }
    const s2 = conversationReducer(s1, {
      type: 'conversation.message_appended',
      event: other,
    })
    expect(s2.sessionId).toBe('ses-2')
    expect(s2.messages).toHaveLength(1)
    expect(s2.messages[0].role).toBe('assistant')
  })

  it('hydrate conserva historial y aparca estados a medias en idle', () => {    const next = conversationReducer(initialConversationState, {
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

  it('hydrate con la misma sesión conserva el mensaje que ya llegó por evento', () => {
    const live = conversationReducer(initialConversationState, {
      type: 'conversation.message_appended',
      event: { ...appended('assistant'), content: 'respuesta' },
    })
    const next = conversationReducer(live, {
      type: 'conversation.hydrate',
      session: {
        sessionId: 'ses-1',
        state: 'responding',
        messages: [
          {
            requestId: 'req-1', sessionId: 'ses-1', role: 'user', content: 'hola',
            source: 'local', timestamp: '2026-10-12T00:00:00.000Z',
          },
          {
            requestId: 'req-1', sessionId: 'ses-1', role: 'assistant',
            content: 'respuesta', source: 'local',
            timestamp: '2026-10-12T00:00:01.000Z',
          },
        ],
      },
    })
    expect(next.messages.map((m) => `${m.requestId}:${m.role}`)).toEqual([
      'req-1:user',
      'req-1:assistant',
    ])
  })

  it('hydrate con snapshot anterior al último evento no borra ni reordena', () => {
    const withUser = conversationReducer(initialConversationState, {
      type: 'conversation.message_appended',
      event: appended('user'),
    })
    const live = conversationReducer(withUser, {
      type: 'conversation.message_appended',
      event: { ...appended('assistant'), content: 'respuesta' },
    })
    const stale = conversationReducer(live, {
      type: 'conversation.hydrate',
      session: {
        sessionId: 'ses-1',
        state: 'responding',
        messages: [
          {
            requestId: 'req-1', sessionId: 'ses-1', role: 'user', content: 'hola',
            source: 'local', timestamp: '2026-10-12T00:00:00.000Z',
          },
        ],
      },
    })
    expect(stale.messages).toHaveLength(2)
    expect(stale.messages[1].role).toBe('assistant')
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

describe('confirmaciones', () => {
  const confirmation = {
    type: 'permission.confirmation_required' as const,
    requestId: 'req-9',
    sessionId: 'ses-1',
    timestamp: '2026-10-12T00:00:00.000Z',
    toolId: 'calendar_write',
    summary: 'Crear «Dentista» en Deberes',
    risk: 'write' as const,
    expiresAt: '2026-10-12T00:05:00.000Z',
  }

  it('acumula por requestId sin duplicar', () => {
    const s1 = conversationReducer(initialConversationState, {
      type: 'permission.confirmation_required',
      event: confirmation,
    })
    const s2 = conversationReducer(s1, {
      type: 'permission.confirmation_required',
      event: confirmation,
    })
    expect(s2.confirmations).toHaveLength(1)
  })

  it('resolved retira la tarjeta', () => {
    const s1 = conversationReducer(initialConversationState, {
      type: 'permission.confirmation_required',
      event: confirmation,
    })
    const s2 = conversationReducer(s1, {
      type: 'permission.resolved',
      requestId: 'req-9',
    })
    expect(s2.confirmations).toHaveLength(0)
  })
})
