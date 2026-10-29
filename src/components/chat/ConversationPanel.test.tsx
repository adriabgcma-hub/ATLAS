import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ConversationPanel } from './ConversationPanel'
import type { ChatMessage } from '../../types/contracts'

function msg(partial: Partial<ChatMessage>): ChatMessage {
  return {
    requestId: 'req-1',
    sessionId: 'ses-1',
    role: 'assistant',
    content: 'eco',
    source: 'local',
    timestamp: '2026-10-29T00:00:00.000Z',
    ...partial,
  }
}

describe('etiquetas de la conversación', () => {
  it('marca HEFESTO el mensaje continuador', () => {
    const html = renderToStaticMarkup(
      <ConversationPanel messages={[msg({ agent: 'hefesto', taskId: 'tsk-1' })]} />,
    )
    expect(html).toContain('HEFESTO')
    expect(html).toContain('chat-msg-hefesto')
  })

  it('mantiene ATLAS para el resto de mensajes assistant', () => {
    const html = renderToStaticMarkup(
      <ConversationPanel messages={[msg({}), msg({ requestId: 'req-2' })]} />,
    )
    expect(html).toContain('ATLAS')
    expect(html).not.toContain('HEFESTO')
  })
})
