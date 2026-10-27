import { useEffect, useRef, useState } from 'react'
import type { ChatMessage } from '../../types/contracts'
import { MarkdownMessage } from './MarkdownMessage'

const STREAM_CADENCE_MS = 16
const STREAM_MS_PER_CHAR = 18
const STREAM_MIN_MS = 400
const STREAM_MAX_MS = 6000

/**
 * Duración del efecto de escritura según la longitud: las respuestas cortas
 * mantienen el ritmo de máquina de escribir y las largas se aceleran hasta
 * un techo de 6 s para no eternizar la lectura.
 */
function streamDuration(length: number): number {
  return Math.min(STREAM_MAX_MS, Math.max(STREAM_MIN_MS, length * STREAM_MS_PER_CHAR))
}

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
  const startedAtRef = useRef(0)

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

  // Cada mensaje nuevo arranca su propio reloj de escritura
  useEffect(() => {
    startedAtRef.current = performance.now()
  }, [currentLastId])

  // Revelado progresivo — sin depender del estado de sesión.
  // El avance se calcula por tiempo transcurrido (no por tick), así una
  // pestaña en segundo plano que congela los timers no alarga la espera:
  // al volver, el texto se pone al día de golpe.
  useEffect(() => {
    if (!isAssistant) return
    const length = lastContent.length
    if (revealed >= length) return
    const duration = streamDuration(length)
    timerRef.current = window.setTimeout(() => {
      const elapsed = performance.now() - startedAtRef.current
      const target = Math.ceil((elapsed / duration) * length)
      setRevealed((r) => Math.min(length, Math.max(r + 1, target)))
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
              {message.role === 'assistant' ? (
                <div className="chat-msg-text chat-md">
                  <MarkdownMessage text={text} streaming={showCursor} />
                </div>
              ) : (
                <p className="chat-msg-text">
                  {message.content}
                </p>
              )}
            </article>
          )
        })}
      </div>
    </section>
  )
}
