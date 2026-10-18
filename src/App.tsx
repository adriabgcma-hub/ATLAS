import { useCallback, useEffect, useReducer, useRef, useState, type RefObject } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Orbits, type OrbitPort } from './components/orb/Orbits'
import { AtlasScene, type ScenePort } from './components/scene/Scene'
import { InputBar } from './components/chat/InputBar'
import { ConversationPanel } from './components/chat/ConversationPanel'
import { ConfirmationPanel } from './components/chat/ConfirmationPanel'
import {
  conversationReducer,
  initialConversationState,
  sessionStateToOrbInput,
} from './state/conversation'
import { deriveOrbState } from './state/orbState'
import type {
  ActiveSession,
  AtlasError,
  ConfirmationDecision,
  ConfirmationRequest,
  MessageAppended,
  MessageRole,
  SendResult,
  SessionEvent,
  SessionState,
  Source,
} from './types/contracts'
import './App.css'

const LETTER_CADENCE_MS = 26
/** URL del relay. En dev (navegador) se sondea directo; en Tauri el núcleo
 *  drena el relay vía ATLAS_RELAY_URL y el frontend usa IPC. */
const RELAY_URL = import.meta.env.VITE_RELAY_URL || 'http://192.168.100.26:8765'
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

/** Recorta el detalle de la burbuja (sin estilos de desborde). */
function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…` : text
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
  decision?: string
  toolId?: string
  summary?: string
  risk?: string
  expiresAt?: string
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

// Palabras que cuentan como confirmación explícita (mismo conjunto que el relay).
const CONFIRM_WORDS = new Set([
  'sí', 'si', 'sií', 'síí', 'adelante', 'vale', 'ok', 'okey', 'oké',
  'confirmo', 'confirmar', 'ejecutar', 'hazlo', 'dále', 'dale', 'vamos',
  'sí adelante', 'si adelante', 'confirmado', 'acepto', 'autorizo',
])
// Palabras que cuentan como denegación explícita.
const DENY_WORDS = new Set([
  'no', 'no quiero', 'no gracias', 'cancela', 'cancelar', 'stop',
  'detener', 'parar', 'olvídalo', 'olvidalo', 'déjalo', 'dejalo',
  'no ejecutes', 'no hacerlo', 'deniego', 'rechazo',
])

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

  // Zona de chat: expandida (panel + barra) o compacta (solo barra).
  // Persiste entre recargas.
  const [chatExpanded, setChatExpanded] = useState<boolean>(() => {
    try {
      return window.localStorage.getItem('atlas.chatOpen') !== '0'
    } catch {
      return true
    }
  })

  useEffect(() => {
    try {
      window.localStorage.setItem('atlas.chatOpen', chatExpanded ? '1' : '0')
    } catch {
      /* almacenamiento no disponible */
    }
  }, [chatExpanded])

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
    fetch(`${RELAY_URL}/v1/events?since=${devRelayCursorRef.current}`)
      .then((response) => {
        if (!response.ok) throw new Error('relay_http')
        return response.json() as Promise<{ events?: RelayEventRow[] }>
      })
      .then((data) => {
        const rows = data.events ?? []
        if (!devSyncedRef.current) {
          // Primera sincronización: marcar todos los eventos como vistos,
          // pero procesar confirmaciones pendientes (que no están en /v1/session).
          for (const row of rows) {
            devRelayCursorRef.current = Math.max(devRelayCursorRef.current, row.seq)
            if (
              row.kind === 'confirmation' &&
              typeof row.toolId === 'string' &&
              typeof row.summary === 'string' &&
              (row.risk === 'read' ||
                row.risk === 'write' ||
                row.risk === 'external' ||
                row.risk === 'destructive') &&
              typeof row.expiresAt === 'string'
            ) {
              dispatch({
                type: 'permission.confirmation_required',
                event: {
                  type: 'permission.confirmation_required',
                  requestId: row.requestId,
                  sessionId: row.sessionId,
                  timestamp: row.timestamp,
                  toolId: row.toolId,
                  summary: row.summary,
                  risk: row.risk,
                  expiresAt: row.expiresAt,
                },
              })
              setChatExpanded(true)
            } else if (
              row.kind === 'confirmation_resolved' &&
              typeof row.requestId === 'string'
            ) {
              dispatch({
                type: 'permission.resolved',
                requestId: row.requestId,
              })
              const port = orbitPortRef.current
              const orbitId = confirmOrbitIdRef.current.get(row.requestId)
              if (port && orbitId !== undefined) {
                port.retireOrbit(orbitId)
                confirmOrbitIdRef.current.delete(row.requestId)
                orbitedConfirmsRef.current.delete(row.requestId)
              }
            }
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
            row.kind === 'confirmation_resolved' &&
            typeof row.requestId === 'string'
          ) {
            // La confirmación fue resuelta (por botón o por texto en Telegram).
            dispatch({
              type: 'permission.resolved',
              requestId: row.requestId,
            })
            const port = orbitPortRef.current
            const orbitId = confirmOrbitIdRef.current.get(row.requestId)
            if (port && orbitId !== undefined) {
              port.retireOrbit(orbitId)
              confirmOrbitIdRef.current.delete(row.requestId)
              orbitedConfirmsRef.current.delete(row.requestId)
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
          } else if (
            row.kind === 'confirmation' &&
            typeof row.toolId === 'string' &&
            typeof row.summary === 'string' &&
            (row.risk === 'read' ||
              row.risk === 'write' ||
              row.risk === 'external' ||
              row.risk === 'destructive') &&
            typeof row.expiresAt === 'string'
          ) {
            dispatch({
              type: 'permission.confirmation_required',
              event: {
                type: 'permission.confirmation_required',
                requestId: row.requestId,
                sessionId: row.sessionId,
                timestamp: row.timestamp,
                toolId: row.toolId,
                summary: row.summary,
                risk: row.risk,
                expiresAt: row.expiresAt,
              },
            })
            setChatExpanded(true)
          }
        }
      })
      .catch(() => {
        /* relay no disponible */
      })
  }, [enqueueLetters, rememberMessage])

  // Órbitas por operación: cada petición viva (incluidos inputs) tiene su
  // órbita con etiqueta al hover; al terminar se retira con explosión.
  const orbitPortRef = useRef<OrbitPort | null>(null)
  const orbitedRequestsRef = useRef<Set<string>>(new Set())
  const orbitedConfirmsRef = useRef<Set<string>>(new Set())
  /** Mapea confirmation.requestId → id de órbita en Orbits. */
  const confirmOrbitIdRef = useRef<Map<string, number>>(new Map())

  useEffect(() => {
    const port = orbitPortRef.current
    if (!port) return
    const { sessionState, pendingRequestIds, messages, confirmations } = conversation
    if (sessionState === 'thinking' && pendingRequestIds.length > 0) {
      const requestId = pendingRequestIds[pendingRequestIds.length - 1]
      if (!orbitedRequestsRef.current.has(requestId)) {
        orbitedRequestsRef.current.add(requestId)
        const lastUser = [...messages].reverse().find((m) => m.role === 'user')
        port.burstAll()
        port.spawnOrbit({
          title: lastUser?.source === 'telegram' ? 'Telegram · Tú' : 'Tú',
          detail: lastUser ? truncate(lastUser.content, 140) : 'Petición en curso',
        })
      }
    } else if (
      sessionState === 'completed' ||
      sessionState === 'cancelled' ||
      sessionState === 'failed'
    ) {
      port.retireAll()
      orbitedRequestsRef.current.clear()
      orbitedConfirmsRef.current.clear()
    }
    // Retirar órbitas de confirmaciones que ya se resolvieron (por botón o texto).
    const currentConfirmIds = new Set(confirmations.map((c) => c.requestId))
    for (const [requestId, orbitId] of confirmOrbitIdRef.current) {
      if (!currentConfirmIds.has(requestId)) {
        port.retireOrbit(orbitId)
        confirmOrbitIdRef.current.delete(requestId)
        orbitedConfirmsRef.current.delete(requestId)
      }
    }
    // Crear órbitas para confirmaciones nuevas.
    for (const confirmation of confirmations) {
      if (!orbitedConfirmsRef.current.has(confirmation.requestId)) {
        orbitedConfirmsRef.current.add(confirmation.requestId)
        const orbitId = port.spawnOrbit({
          title: 'Confirmación pendiente',
          detail: truncate(confirmation.summary, 140),
        })
        confirmOrbitIdRef.current.set(confirmation.requestId, orbitId)
      }
    }
  })

  // Suscripción a eventos del núcleo + hidratación + drenado del relay.
  useEffect(() => {
    const timers = letterTimersRef.current
    const unsubs: Array<() => void> = []
    let cancelled = false
    let drainTimer = 0
    let hydrateTimer = 0

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
      listen<ConfirmationRequest>('permission.confirmation_required', (event) => {
        dispatch({ type: 'permission.confirmation_required', event: event.payload })
        // Una confirmación necesita acción: expande la zona sola.
        setChatExpanded(true)
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
      // Hidratar historial (incluye mensajes de Telegram que llegaron mientras
      // el frontend estaba cerrado). Se re-hidrata cada 30s para no perder
      // mensajes que el sondeo no capture.
      const hydrate = () => {
        fetch(`${RELAY_URL}/v1/session`)
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
      }
      hydrate()
      pollDevRelay()
      drainTimer = window.setInterval(pollDevRelay, RELAY_DRAIN_MS)
      hydrateTimer = window.setInterval(hydrate, 30_000)
    }

    return () => {
      cancelled = true
      for (const unsub of unsubs) unsub()
      window.clearInterval(drainTimer)
      window.clearInterval(hydrateTimer)
      for (const timer of timers) clearTimeout(timer)
    }
  }, [enqueueLetters, pollDevRelay, rememberMessage, seedSeen])

  // Envío local: en Tauri va al cerebro por IPC; en dev va al cerebro por

  const handlePermissionRespond = useCallback(
    (requestId: string, decision: ConfirmationDecision) => {
      const port = orbitPortRef.current
      const orbitId = confirmOrbitIdRef.current.get(requestId)
      const done = () => {
        dispatch({ type: 'permission.resolved', requestId })
        if (port && orbitId !== undefined) {
          port.retireOrbit(orbitId)
          confirmOrbitIdRef.current.delete(requestId)
        }
      }
      if (!inTauri()) {
        fetch(`${RELAY_URL}/v1/confirm`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ requestId, decision }),
        })
          .then(() => done())
          .catch(() => {
            /* relay no disponible: se conserva la tarjeta */
          })
        return
      }
      invoke<boolean>('permission_respond', { requestId, decision })
        .then(() => done())
        .catch(() => {
          /* relay no disponible: se conserva la tarjeta */
        })
    },
    [],
  )

  // Cuando hay confirmaciones pendientes, el texto del input se interpreta
  // como decisión (sí/no) en lugar de un mensaje nuevo para el LLM.
  const handleSubmit = useCallback(
    (content: string) => {
      const lower = content.trim().toLowerCase()
      // Si hay confirmaciones pendientes, intentar responder a la primera.
      if (conversation.confirmations.length > 0) {
        const firstConfirm = conversation.confirmations[0]
        if (CONFIRM_WORDS.has(lower)) {
          handlePermissionRespond(firstConfirm.requestId, 'allow_once')
          return
        }
        if (DENY_WORDS.has(lower)) {
          handlePermissionRespond(firstConfirm.requestId, 'deny')
          return
        }
        // Texto no reconocido: ignorar (o se podría avisar)
        return
      }
      if (!inTauri()) {
        fetch(`${RELAY_URL}/v1/send`, {
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
    [conversation.confirmations, handlePermissionRespond, enqueueLetters, pollDevRelay],
  )

  const handleCancel = useCallback(() => {
    // Si hay confirmaciones pendientes, cancelar = denegar la primera.
    if (conversation.confirmations.length > 0) {
      handlePermissionRespond(conversation.confirmations[0].requestId, 'deny')
      return
    }
    if (!inTauri()) return
    const requestId = conversation.pendingRequestIds[0]
    if (!requestId) return
    invoke('conversation_cancel', { requestId }).catch((error: unknown) => {
      dispatch({ type: 'conversation.error', message: errorCode(error) })
    })
  }, [conversation.confirmations, conversation.pendingRequestIds, handlePermissionRespond])

  const orbState = deriveOrbState(
    sessionStateToOrbInput(conversation.sessionState, conversation.messages.length > 0),
  )

  const thinking =
    conversation.sessionState === 'thinking' ||
    conversation.sessionState === 'responding' ||
    conversation.sessionState === 'awaiting_confirmation' ||
    conversation.sessionState === 'executing'

  return (
    <main className="app-shell">
      <Orbits asleep={false} portRef={orbitPortRef} />
      <AtlasScene orbState={orbState} levelRef={levelRef} portRef={scenePortRef} />
      <div className="app-label" aria-label="Atlas">ATLAS</div>
      <div className={`chat-stack${chatExpanded ? ' open' : ''}`} aria-hidden={!chatExpanded}>
        {(conversation.messages.length > 0 || conversation.confirmations.length > 0) && (
          <>
            {conversation.confirmations.length > 0 && (
              <ConfirmationPanel
                items={conversation.confirmations}
                onRespond={handlePermissionRespond}
              />
            )}
            {conversation.messages.length > 0 && (
              <ConversationPanel messages={conversation.messages} />
            )}
          </>
        )}
      </div>
      <InputBar
        onSubmit={handleSubmit}
        onCancel={handleCancel}
        onTogglePanel={() => setChatExpanded((expanded) => !expanded)}
        pending={conversation.pendingRequestIds.length > 0}
        thinking={thinking}
        expanded={chatExpanded}
        hasConfirmations={conversation.confirmations.length > 0}
      />
    </main>
  )
}
