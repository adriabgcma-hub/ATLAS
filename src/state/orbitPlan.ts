import type { SessionState } from '../types/contracts'

/** Órbita de una fase viva del turno (proceso o respuesta de Atlas). */
export interface PhaseOrbit {
  /** Identidad de la fase: si no cambia, la órbita no se recrea. */
  key: string
  title: string
  detail: string
  color: string
}

/** Mismo azul que la órbita de la petición (ver Orbits). */
export const ATLAS_ORBIT_COLOR = '#3FA4FF'
/** Violeta de Hefesto (coincide con la etiqueta HEFESTO del chat). */
export const HEFESTO_ORBIT_COLOR = '#A78BFA'

const HEFESTO_PHASES: Record<string, string> = {
  hefesto_read_file: 'Hefesto · Leyendo archivo',
  hefesto_list_dir: 'Hefesto · Listando carpeta',
  hefesto_echo: 'Hefesto · Encargo',
}

/**
 * Traduce el estado del turno a la órbita de fase. `thinking` no está aquí:
 * su órbita es la petición y necesita el mensaje del usuario, que vive en App.
 * El `message` de `executing` es el toolId (nunca rutas ni contenido).
 */
export function planPhaseOrbit(state: SessionState, message?: string): PhaseOrbit | null {
  if (state === 'executing') {
    if (!message) {
      return { key: 'executing', title: 'Herramienta', detail: '', color: ATLAS_ORBIT_COLOR }
    }
    const hefesto = HEFESTO_PHASES[message]
    return {
      key: `executing:${message}`,
      title: hefesto ?? `Herramienta · ${message}`,
      detail: '',
      color: hefesto ? HEFESTO_ORBIT_COLOR : ATLAS_ORBIT_COLOR,
    }
  }
  if (state === 'responding') {
    return {
      key: 'responding',
      title: 'Atlas',
      detail: 'Redactando respuesta',
      color: ATLAS_ORBIT_COLOR,
    }
  }
  return null
}
