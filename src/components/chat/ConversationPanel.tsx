import { useEffect, useRef, useState } from 'react'
import type { ChatMessage } from '../../types/contracts'

const STREAM_CADENCE_MS = 18

/**
 * Historial de la conversación (Fase 3).
 * Presenta mensajes de usuario y del LLM vengan del canal que vengan;
 * no decide nada: lee del reducer alimentado por eventos IPC.
 * Simula streaming: revela el último mensaje assistant progresivamente.
 */
export function ConversationPanel({ messages }: { messages: ChatMessage[] }) {
  const listRef = useRef<HTMLDivElement>(null)
  const [revealed, setRevealed] = useState(0)
  const [lastId, setLastId] = useState('')
  const timerRef = useRef(0)

  const last = messages[messages.length - 1]
  const currentLastId = last ? `${last.requestId}-${last.role}` : ''
  const lastContent = last?.role === 'assistant' ? last.content : ''
  const isAssistant = last?.role === 'assistant'
  const done = revealed >= lastContent.length

  // Ajuste en fase de render: reiniciar al cambiar mensaje
  if (lastId !== currentLastId) {
    setLastId(currentLastId)
    setRevealed(0)
  }

  // Timer de revelado progresivo — sin depender del estado de sesión
  useEffect(() => {
    if (!isAssistant) return
    if (revealed >= lastContent.length) return
    timerRef.current = window.setTimeout(() => {
      setRevealed((r) => r + 1)
    }, STREAM_CADENCE_MS)
    return () => window.clearTimeout(timerRef.current)
  }, [isAssistant, revealed, lastContent])

  useEffect(() => {
    const list = listRef.current
    if (list) list.scrollTop = list.scrollHeight
  }, [messages, revealed])

  if (messages.length === 0) return null

  return (
    <section
      className="chat-panel"
      aria-label="Conversación con Atlas"
      aria-live="polite"
    >
      <div ref={listRef} className="chat-panel-list">
        {messages.map((message, i) => {
          const isLastAssistant =
            i === messages.length - 1 && message.role === 'assistant'
          const showCursor = isLastAssistant && !done
          const text =
            isLastAssistant
              ? (done ? message.content : message.content.slice(0, revealed))
              : message.content
          return (
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
              <p className="chat-msg-text">
                {text}
                {showCursor && <span className="stream-cursor" aria-hidden="true" />}
              </p>
            </article>
          )
        })}
      </div>
    </section>
  )
}
