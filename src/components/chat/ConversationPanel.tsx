import { useEffect, useRef } from 'react'
import type { ChatMessage } from '../../types/contracts'

/**
 * Historial de la conversación (Fase 3).
 * Presenta mensajes de usuario y del LLM vengan del canal que vengan;
 * no decide nada: lee del reducer alimentado por eventos IPC.
 */
export function ConversationPanel({ messages }: { messages: ChatMessage[] }) {
  const listRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const list = listRef.current
    if (list) list.scrollTop = list.scrollHeight
  }, [messages.length])

  if (messages.length === 0) return null

  return (
    <section className="chat-panel" aria-label="Conversación con Atlas" aria-live="polite">
      <div ref={listRef} className="chat-panel-list">
        {messages.map((message) => (
          <article
            key={`${message.requestId}-${message.role}`}
            className={
              message.role === 'user' ? 'chat-msg chat-msg-user' : 'chat-msg chat-msg-assistant'
            }
          >
            <span className="chat-msg-tag">
              {message.role === 'user'
                ? message.source === 'telegram'
                  ? 'TG · TÚ'
                  : 'TÚ'
                : 'ATLAS'}
            </span>
            <p className="chat-msg-text">{message.content}</p>
          </article>
        ))}
      </div>
    </section>
  )
}
