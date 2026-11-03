/**
 * Estado reactivo del Orb. Derivable desde el estado global del chat.
 */

/** Rama cinemática (intensidad, pulso, error). */
export type OrbState = 'idle' | 'thinking' | 'responding' | 'error'

export interface DeriveOrbStateArgs {
  isStreaming: boolean
  hasContent: boolean
  lastError: boolean
}

export function deriveOrbState(args: DeriveOrbStateArgs): OrbState {
  if (args.lastError) {
    return 'error'
  }
  if (args.isStreaming && args.hasContent) {
    return 'responding'
  }
  if (args.isStreaming) {
    return 'thinking'
  }
  return 'idle'
}

