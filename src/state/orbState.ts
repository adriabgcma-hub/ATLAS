/**
 * Estado reactivo del Orb. Derivable desde el estado global del chat.
 */

/** Rama cinemática (intensidad, pulso, error). */
export type OrbState = 'idle' | 'thinking' | 'responding' | 'speaking' | 'error'

export interface DeriveOrbStateArgs {
  isStreaming: boolean
  hasContent: boolean
  isMuted: boolean
  isSpeaking: boolean
  lastError: boolean
}

export function deriveOrbState(args: DeriveOrbStateArgs): OrbState {
  if (args.lastError) {
    return 'error'
  }
  if (args.isSpeaking && !args.isMuted) {
    return 'speaking'
  }
  if (args.isStreaming && args.hasContent) {
    return 'responding'
  }
  if (args.isStreaming) {
    return 'thinking'
  }
  return 'idle'
}

