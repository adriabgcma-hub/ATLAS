import { useCallback, useEffect, useReducer, useRef, type RefObject } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Orbits } from './components/orb/Orbits'
import { AtlasScene, type ScenePort } from './components/scene/Scene'
import { InputBar } from './components/chat/InputBar'
import { ConversationPanel } from './components/chat/ConversationPanel'
import {
  conversationReducer,
  initialConversationState,
  sessionStateToOrbInput,
} from './state/conversation'
import { deriveOrbState } from './state/orbState'
import type {
  ActiveSession,
  AtlasError,
  MessageAppended,
  MessageRole,
  SendResult,
  SessionEvent,
  SessionState,
  Source,
} from './types/contracts'
import './App.css'

const LETTER_CADENCE_MS = 26
/** Cada cuánto se drena el relay (el cursor evita reprocesar). */
const RELAY_DRAIN_MS = 4000

/** true solo dentro del webview de Tauri (con `withGlobalTauri`). */
function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI__' in window
}

function errorCode(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'code' in error) {
    const code = (error as AtlasError).code
    if (typeof code === 'string' && code.length > 0) return code
  }
  return 'invoke_failed'
}

interface LetterJob {
  text: string
  index: number
}

/** Fila de eventos del cerebro (`GET /v1/events`). Solo dev. */
interface RelayEventRow {
  seq: number
  kind: string
  type: string
  requestId: string
  sessionId: string
  timestamp: string
  role?: MessageRole
  content?: string
  source?: Source
  state?: SessionState
  message?: string
}

interface LetterPump {
  queue: RefObject<LetterJob[]>
  port: RefObject<ScenePort | null>
  timers: RefObject<number[]>
  pumping: RefObject<boolean>
  retries: RefObject<number>
}

/** Reintentos si la escena aún no está lista (p. ej. envío nada más abrir). */
const PUMP_RETRY_MAX = 10
const PUMP_RETRY_MS = 500

/**
 * Bomba FIFO de letras en ámbito de módulo (sin auto-referencias de hooks).
 * Si el puerto de la escena no está listo, reintenta de forma acotada en vez
 * de dejar trabajos atascados hasta el próximo envío.
 */
function pumpQueue(pump: LetterPump): void {
  if (pump.pumping.current) return
  if (pump.queue.current.length === 0) {
    pump.retries.current = 0
    // Sin trabajos ni bombeo no quedan timers vivos: poda del registro
    // (misma identidad de array para la limpieza al desmontar).
    pump.timers.current.length = 0
    return
  }
  if (!pump.port.current) {
    if (pump.retries.current < PUMP_RETRY_MAX) {
      pump.retries.current += 1
      pump.timers.current.push(window.setTimeout(() => pumpQueue(pump), PUMP_RETRY_MS))
    }
    return
  }
  pump.retries.current = 0
  pump.pumping.current = true
  step()

  // Declaración hoisted: la cadena de pasos se auto-perpetúa.
  function step(): void {
    const job = pump.queue.current[0]
    const port = pump.port.current
    if (!job || !port) {
      pump.pumping.current = false
      if (pump.queue.current.length > 0) pumpQueue(pump)
      return
    }
    if (job.index >= job.text.length) {
      pump.queue.current = pump.queue.current.slice(1)
      pump.pumping.current = false
      pumpQueue(pump)
      return
    }
    port.spawnLetter(job.text[job.index])
    pump.queue.current = [
      { text: job.text, index: job.index + 1 },
      ...pump.queue.current.slice(1),
    ]
    pump.timers.current.push(window.setTimeout(step, LETTER_CADENCE_MS))
  }
}

export default function App() {
  const levelRef = useRef(0)
  const scenePortRef = useRef<ScenePort | null>(null)
  // Timers de la cadencia de letras (se limpian al desmontar).
  const letterTimersRef = useRef<number[]>([])
  // Cola FIFO de letras: dos mensajes seguidos no entrelazan caracteres.
  const letterQueueRef = useRef<LetterJob[]>([])
  const pumpingRef = useRef(false)
  const pumpRetryRef = useRef(0)
  // Cursor del sondeo dev (sin Tauri no hay núcleo que lo guarde).
  const devRelayCursorRef = useRef(0)
  // Vistos (requestId:rol): las letras solo vuelan para mensajes nuevos,
  // nunca para re-juegos del historial (recargas, HMR, cursor atrasado).
  const seenRef = useRef<Set<string>>(new Set())
  const rememberMessage = useCallback((requestId: string, role: string): boolean => {
    const key = `${requestId}:${role}`
    if (seenRef.current.has(key)) return false
    seenRef.current.add(key)
    if (seenRef.current.size > 1000) {
      const oldest = seenRef.current.values().next().value
      if (oldest !== undefined) seenRef.current.delete(oldest)
    }
    return true
  }, [])
  const seedSeen = useCallback((messages: Array<{ requestId: string; role: string }>) => {
    for (const message of messages) {
      seenRef.current.add(`${message.requestId}:${message.role}`)
    }
  }, [])

  const [conversation, dispatch] = useReducer(conversationReducer, initialConversationState)

  const enqueueLetters = useCallback((text: string) => {
    if (!text) return
    letterQueueRef.current = [...letterQueueRef.current, { text, index: 0 }]
    pumpQueue({
      queue: letterQueueRef,
      port: scenePortRef,
      timers: letterTimersRef,
      pumping: pumpingRef,
      retries: pumpRetryRef,
    })
  }, [])

  // Primera pasada del sondeo: situarse en el borde vivo sin re-jugar nada
  // (el panel se hidrata por /v1/session).
  const devSyncedRef = useRef(false)

  // Sondeo dev de eventos del cerebro (navegador). Extraído para poder
  // llamarlo justo tras enviar, sin esperar al intervalo. Las letras solo
  // vuelan para mensajes de usuario: la respuesta del LLM vive en el panel
  // y en el brillo del orbe, no en la barra de input.
  const pollDevRelay = useCallback(() => {
    fetch(`http://127.0.0.1:8765/v1/events?since=${devRelayCursorRef.current}`)
      .then((response) => {
        if (!response.ok) throw new Error('relay_http')
        return response.json() as Promise<{ events?: RelayEventRow[] }>
      })
      .then((data) => {
        const rows = data.events ?? []
        if (!devSyncedRef.current) {
          for (const row of rows) {
            devRelayCursorRef.current = Math.max(devRelayCursorRef.current, row.seq)
          }
          devSyncedRef.current = true
          return
        }
        for (const row of rows) {
          devRelayCursorRef.current = Math.max(devRelayCursorRef.current, row.seq)
          if (
            row.kind === 'message' &&
            (row.role === 'user' || row.role === 'assistant') &&
            (row.source === 'local' || row.source === 'telegram') &&
            typeof row.content === 'string' &&
            row.content.trim()
          ) {
            dispatch({
              type: 'conversation.message_appended',
              event: {
                type: 'conversation.message_appended',
                requestId: row.requestId,
                sessionId: row.sessionId,
                timestamp: row.timestamp,
                role: row.role,
                content: row.content,
                source: row.source,
              },
            })
            if (row.role === 'user' && rememberMessage(row.requestId, row.role)) {
              enqueueLetters(row.content)
            }
          } else if (
            row.kind === 'state' &&
            typeof row.state === 'string' &&
            (
              row.state === 'idle' ||
              row.state === 'thinking' ||
              row.state === 'awaiting_confirmation' ||
              row.state === 'executing' ||
              row.state === 'responding' ||
              row.state === 'cancelled' ||
              row.state === 'completed' ||
              row.state === 'failed'
            )
          ) {
            dispatch({
              type: 'session.state_changed',
              event: {
                type: 'session.state_changed',
                requestId: row.requestId,
                sessionId: row.sessionId,
                timestamp: row.timestamp,
                state: row.state,
                ...(typeof row.message === 'string' ? { message: row.message } : {}),
              },
            })
          }
        }
      })
      .catch(() => {
        /* relay no disponible */
      })
  }, [enqueueLetters, rememberMessage])

  // Suscripción a eventos del núcleo + hidratación + drenado del relay.
  useEffect(() => {
    const timers = letterTimersRef.current
    const unsubs: Array<() => void> = []
    let cancelled = false
    let drainTimer = 0

    if (inTauri()) {
      invoke<ActiveSession | null>('session_get')
        .then((session) => {
          if (!cancelled && session) {
            seedSeen(session.messages)
            dispatch({ type: 'conversation.hydrate', session })
          }
        })
        .catch(() => {
          /* núcleo aún no listo: se hidratará con los eventos */
        })
      listen<SessionEvent>('session.state_changed', (event) => {
        dispatch({ type: 'session.state_changed', event: event.payload })
      })
        .then((unsub) => {
          unsubs.push(unsub)
        })
        .catch(() => {
          /* sin eventos: modo degradado */
        })
      listen<MessageAppended>('conversation.message_appended', (event) => {
        dispatch({ type: 'conversation.message_appended', event: event.payload })
        if (event.payload.role === 'user' && rememberMessage(event.payload.requestId, event.payload.role)) {
          enqueueLetters(event.payload.content)
        }
      })
        .then((unsub) => {
          unsubs.push(unsub)
        })
        .catch(() => {
          /* sin eventos: modo degradado */
        })
      // El relay es opcional: si está caído se ignora en silencio.
      drainTimer = window.setInterval(() => {
        invoke('relay_drain').catch(() => {
          /* relay no disponible */
        })
      }, RELAY_DRAIN_MS)
    } else {
      // DEV (navegador, sin Tauri ni núcleo): hidrata el historial y sondea
      // eventos nuevos con los mismos visuales. Vía canónica: IPC.
      fetch('http://127.0.0.1:8765/v1/session')
        .then((response) => {
          if (!response.ok) throw new Error('relay_http')
          return response.json() as Promise<{ session: ActiveSession | null }>
        })
        .then((data) => {
          if (cancelled) return
          if (data.session) {
            seedSeen(data.session.messages)
            dispatch({ type: 'conversation.hydrate', session: data.session })
          }
        })
        .catch(() => {
          /* relay no disponible */
        })
      pollDevRelay()
      drainTimer = window.setInterval(pollDevRelay, RELAY_DRAIN_MS)
    }

    return () => {
      cancelled = true
      for (const unsub of unsubs) unsub()
      window.clearInterval(drainTimer)
      for (const timer of timers) clearTimeout(timer)
    }
  }, [enqueueLetters, pollDevRelay, rememberMessage, seedSeen])

  // Envío local: en Tauri va al cerebro por IPC; en dev va al cerebro por
  // HTTP directo (mismo bucle de eventos). Sin relay, eco visual.
  const handleSubmit = useCallback(
    (content: string) => {
      if (!inTauri()) {
        fetch('http://127.0.0.1:8765/v1/send', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ content, source: 'local' }),
        })
          .then((response) => {
            if (!response.ok) throw new Error('relay_http')
            pollDevRelay()
          })
          .catch(() => enqueueLetters(content))
        return
      }
      invoke<SendResult>('conversation_send', { content, source: 'local' }).catch((error: unknown) => {
        dispatch({ type: 'conversation.error', message: errorCode(error) })
      })
    },
    [enqueueLetters, pollDevRelay],
  )

  const handleCancel = useCallback(() => {
    if (!inTauri()) return
    const requestId = conversation.pendingRequestIds[0]
    if (!requestId) return
    invoke('conversation_cancel', { requestId }).catch((error: unknown) => {
      dispatch({ type: 'conversation.error', message: errorCode(error) })
    })
  }, [conversation.pendingRequestIds])

  const orbState = deriveOrbState(
    sessionStateToOrbInput(conversation.sessionState, conversation.messages.length > 0),
  )

  return (
    <main className="app-shell">
      <Orbits asleep={false} />
      <AtlasScene agentTone="neutral" orbState={orbState} levelRef={levelRef} portRef={scenePortRef} />
      <div className="app-label" aria-label="Atlas">ATLAS</div>
      {conversation.messages.length > 0 && (
        <ConversationPanel messages={conversation.messages} />
      )}
      <InputBar
        onSubmit={handleSubmit}
        onCancel={handleCancel}
        pending={conversation.pendingRequestIds.length > 0}
      />
    </main>
  )
}
