import type { SessionState } from '../types/contracts'
import { planPhaseOrbit } from './orbitPlan'

/** Etiqueta de órbita (misma forma que `OrbitInfo` de Orbits). */
export interface OrbitLabel {
  title: string
  detail: string
  color?: string
}

/** Lo mínimo que el controlador necesita del puerto de órbitas. */
export interface PhaseOrbitSink {
  spawnOrbit(info?: OrbitLabel): number
  retireOrbit(id: number): void
  retireAll(): void
  burstAll(): void
}

export interface PhaseOrbits {
  /** La petición acaba de salir: órbita con el texto del usuario. */
  request(requestId: string, text: string, source: string): void
  /** Evento de estado del turno: sustituye la fase viva. */
  apply(state: SessionState, requestId: string, message?: string): void
  /** Retira la fase viva (hidratación, desmontaje). */
  clear(): void
}

const PHASE_DETAIL_MAX = 140

function truncate(text: string): string {
  return text.length > PHASE_DETAIL_MAX ? `${text.slice(0, PHASE_DETAIL_MAX)}…` : text
}

/**
 * Órbitas por fase dirigidas por eventos, no por render: los eventos del relay
 * llegan en lote y React colapsa `executing`→`responding` en un solo render,
 * así que la órbita se crea al procesar cada evento (el render no la vería).
 */
export function createPhaseOrbits(sink: () => PhaseOrbitSink | null): PhaseOrbits {
  let current: { key: string; id: number } | null = null

  const retireCurrent = (target: PhaseOrbitSink) => {
    if (!current) return
    target.retireOrbit(current.id)
    current = null
  }

  const show = (target: PhaseOrbitSink, key: string, info: OrbitLabel) => {
    if (current?.key === key) return
    retireCurrent(target)
    current = { key, id: target.spawnOrbit(info) }
  }

  return {
    request(requestId, text, source) {
      const target = sink()
      if (!target) return
      const key = `request:${requestId}`
      // El efecto de render se repite mientras dura `thinking`: solo la
      // primera vez abre órbita (y estalla las retiradas de la anterior).
      if (current?.key === key) return
      target.burstAll()
      show(target, key, {
        title: source === 'telegram' ? 'Telegram · Tú' : 'Tú',
        detail: truncate(text),
      })
    },
    apply(state, _requestId, message) {
      const target = sink()
      if (!target) return
      if (state === 'completed' || state === 'cancelled' || state === 'failed') {
        current = null
        target.retireAll()
        return
      }
      const plan = planPhaseOrbit(state, message)
      if (plan) show(target, plan.key, { title: plan.title, detail: plan.detail, color: plan.color })
    },
    clear() {
      const target = sink()
      if (target) retireCurrent(target)
    },
  }
}
