import type {
  ConfirmationDecision,
  ConfirmationRequest,
} from '../../types/contracts'

/**
 * Confirmaciones pendientes (Fase herramientas).
 * La ausencia de respuesta nunca autoriza: el cerebro caduca la petición.
 * Solo presenta y recoge la decisión; la ejecuta el cerebro.
 */
export function ConfirmationPanel({
  items,
  onRespond,
}: {
  items: ConfirmationRequest[]
  onRespond: (requestId: string, decision: ConfirmationDecision) => void
}) {
  if (items.length === 0) return null

  return (
    <section className="chat-confirm" aria-label="Confirmaciones pendientes">
      {items.map((item) => (
        <article key={item.requestId} className="chat-confirm-card">
          <span className="chat-msg-tag">
            {item.toolId.startsWith('hefesto') ? 'HEFESTO' : 'ATLAS'} ·{' '}
            {item.toolId} · {item.risk.toUpperCase()}
          </span>
          <p className="chat-confirm-text">{item.summary}</p>
          <div className="chat-confirm-actions">
            <button
              type="button"
              className="chat-confirm-allow"
              onClick={() => onRespond(item.requestId, 'allow_once')}
            >
              Permitir una vez
            </button>
            <button
              type="button"
              className="chat-confirm-deny"
              onClick={() => onRespond(item.requestId, 'deny')}
            >
              Denegar
            </button>
          </div>
        </article>
      ))}
    </section>
  )
}
