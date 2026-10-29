import type { HefestoPhase, HefestoSnapshot } from '../../types/contracts'

const PHASE_LABELS: Record<HefestoPhase, string> = {
  unconfigured: 'Sin configurar',
  connecting: 'Conectando',
  suspended: 'Suspendido',
  ready: 'Activo',
  offline: 'Sin conexión',
  error: 'Error',
}

interface HefestoBadgeProps {
  state: HefestoSnapshot
  onResume: () => void
  onAbort: () => void
}

/** Indicador del dispositivo Hefesto, espejo de la etiqueta ATLAS.
 *  Sin configuración no existe: la app no muestra un Hefesto inerte. */
export function HefestoBadge({ state, onResume, onAbort }: HefestoBadgeProps) {
  if (state.phase === 'unconfigured') return null
  const pending = state.phase === 'suspended' && state.detail === 'abort_pending'
  const label = pending ? 'Suspendiendo…' : PHASE_LABELS[state.phase]
  const active = state.phase === 'ready'
  const canToggle = active || (state.phase === 'suspended' && !pending)
  return (
    <div
      className={`hefesto-badge hefesto-${state.phase}`}
      title={state.detail ?? undefined}
      aria-label={`Hefesto: ${label}`}
    >
      <span className="hefesto-dot" aria-hidden="true" />
      <span className="hefesto-name">HEFESTO</span>
      <span className="hefesto-phase">{label}</span>
      {canToggle && (
        <button
          type="button"
          className="hefesto-toggle"
          onClick={active ? onAbort : onResume}
        >
          {active ? 'Suspender' : 'Activar'}
        </button>
      )}
      {state.lastEcho !== null && (
        <span className="hefesto-echo" title={state.lastEcho}>
          eco: «{state.lastEcho}»
        </span>
      )}
    </div>
  )
}
