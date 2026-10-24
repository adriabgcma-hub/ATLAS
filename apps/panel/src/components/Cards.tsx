import { fmtDuration, gb, type Health, type Snapshot, type Stats, type Unit } from '../api'

/* Léxico visual: tarjetas sueltas; nada de framework (tokens en panel.css). */

export function Card({ title, badge, wide, tone, children }: {
  title: string; badge?: string; wide?: boolean; tone?: 'err'; children: React.ReactNode
}) {
  return (
    <section className={`card${wide ? ' wide' : ''}${tone === 'err' ? ' err' : ''}`}>
      <h2>{title}{badge ? <span className="badge">{badge}</span> : null}</h2>
      {children}
    </section>
  )
}

export const dotClass = (active?: string | null, load?: string | null): string => {
  if (load === 'masked' || active === 'failed') return 'err'
  if (active === 'activating') return 'warn'
  if (active === 'active') return 'ok'
  return 'dim'
}

export const unitLabel = (u: string) => u.replace(/\.service$/, '')

export function UnitRow({ unit, onAct }: { unit: Unit; onAct?: (unit: string, verb: string) => void }) {
  const busy = unit.active === 'activating'
  return (
    <div className="row">
      <span className={`dot ${dotClass(unit.active, unit.load)}`}></span>
      <div className="grow">
        <div className="name">{unitLabel(unit.unit)}</div>
        <div className="sub">
          {unit.description ?? '—'}{unit.since ? ` · ${unit.since}` : ''}
          {unit.mem_bytes ? ` · ${(unit.mem_bytes / 1e6).toFixed(0)} MB` : ''}
        </div>
      </div>
      {unit.actionable && onAct && (
        <div className="actions" style={{ display: 'flex', gap: 6 }}>
          {(['start', 'stop', 'restart'] as const).map(verb => (
            <button key={verb} className="btn" disabled={busy}
                    onClick={() => onAct(unit.unit, verb)}>
              {verb}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

export function UnitsCard({ snap, onAct }: { snap: Snapshot; onAct: (unit: string, verb: string) => void }) {
  return (
    <Card title="Servicios (systemd)" badge={snap.units.length + ' unidades'}>
      {snap.units.map(u => <UnitRow key={u.unit} unit={u} onAct={onAct} />)}
    </Card>
  )
}

export function PortsCard({ snap }: { snap: Snapshot }) {
  return (
    <Card title="Puertos a la escucha" badge={String(snap.ports.length)}>
      <div>
        {snap.ports.map(p => (
          <span key={p.port} className="chip">{p.port}<small>{p.service ?? p.address}</small></span>
        ))}
      </div>
    </Card>
  )
}

export function MacCard({ snap }: { snap: Snapshot }) {
  const r = snap.resources
  const disk = r.disk_root
  return (
    <Card title="Este Mac (homelab)" badge={fmtDuration(r.uptime_s)}>
      <div className="metrics">
        <div className="metric"><div className="k">Memoria</div>
          <div className="v">{r.mem_used_pct ?? '—'}<small> %</small></div></div>
        <div className="metric"><div className="k">Disco</div>
          <div className="v">{disk
            ? (Math.round((100 * (disk.total - disk.free)) / disk.total) + '% · ' + gb(disk.free) + ' libres')
            : '—'}</div></div>
        <div className="metric"><div className="k">Temperatura</div>
          <div className="v">{r.temp_c != null ? r.temp_c.toFixed(0) : '—'}<small> °C</small></div></div>
        <div className="metric"><div className="k">Carga (1m)</div>
          <div className="v">{r.load_1 ?? '—'}</div></div>
        <div className="metric"><div className="k">Batería</div>
          <div className="v">{r.battery ? (`${r.battery.cap_pct}% · ${r.battery.status}`) : '—'}</div></div>
        <div className="metric"><div className="k">Red</div>
          <div className="v">{r.ac_connected ? 'AC ✓' : 'AC ✗'}</div></div>
      </div>
    </Card>
  )
}

export function RelayCard({ health, stats, relayUnit }: {
  health: Health | null; stats: Stats | null; relayUnit: Unit | undefined
}) {
  const usage = stats?.token_usage
  return (
    <Card title="Relay Atlas · Telegram" badge={`llm: ${health?.llm ?? '—'} · cal: ${health?.calendar ?? '—'}`}>
      <div className="row">
        <span className={`dot ${health?.telegram === 'enabled' ? 'ok' : 'dim'}`}></span>
        <div className="grow">
          <div className="name">
            Telegram {health?.telegram ?? '—'}
            {health?.bot?.username ? ` · @${health.bot.username}` : ''}
          </div>
          <div className="sub">
            {relayUnit?.since ? `arrancado ${relayUnit.since}` : 'unidad no vista'}
            {relayUnit?.mem_bytes ? ` · ${(relayUnit.mem_bytes / 1e6).toFixed(1)} MB` : ''}
          </div>
        </div>
        <span className={`state-chip ${health && health.inflight > 0 ? 'warn' : 'on'}`}>
          {health ? `vuelo ${health.inflight} · cola ${health.queue}` : '—'}
        </span>
      </div>
      <div className="metrics">
        <div className="metric"><div className="k">Llamadas LLM</div>
          <div className="v">{usage?.calls ?? 0}</div></div>
        <div className="metric"><div className="k">Tokens total</div>
          <div className="v">{(usage?.total ?? 0).toLocaleString('es-ES')}</div></div>
        <div className="metric"><div className="k">Events buffer</div>
          <div className="v">{health?.events ?? 0}</div></div>
      </div>
      {usage && Object.entries(usage.by_model).length > 0 && (
        <div style={{ marginTop: 6 }}>
          {Object.entries(usage.by_model).map(([model, m]) => (
            <span key={model} className="chip">{model}<small>{m.total.toLocaleString('es-ES')} tok</small></span>
          ))}
        </div>
      )}
    </Card>
  )
}

export function PeticionesCard({ health, session, onCancelAll }: {
  health: Health | null
  session: import('../api').SessionResp | null
  onCancelAll: () => void
}) {
  const msgs = session?.session?.messages ?? []
  const state = session?.session?.state ?? 'idle'
  const chip = ['idle', 'completed', 'failed', 'cancelled'].includes(state) ? '' : 'warn'
  return (
    <Card title="Peticiones" badge={state}>
      <div className="row">
        <span className={`state-chip ${chip}`}>{state}</span>
        {health && health.inflight > 0 && (
          <button className="btn danger" onClick={onCancelAll}>cancelar en vuelo ({health.inflight})</button>
        )}
      </div>
      {msgs.length === 0 && <div className="empty">Sin mensajes en la sesión activa.</div>}
      {msgs.slice(-12).map(m => (
        <div className="row" key={m.requestId + m.role}>
          <span className={`state-chip ${m.role === 'user' ? 'on' : ''}`}>
            {m.role === 'user'
              ? (m.source === 'telegram' ? 'TG · TÚ' : 'TÚ')
              : 'ATLAS'}
          </span>
          <div className="grow">
            <div className="sub" style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {m.content.length > 140 ? m.content.slice(0, 140) + '…' : m.content}
            </div>
          </div>
        </div>
      ))}
    </Card>
  )
}

export function LogsCard({ lines, total }: { lines: string[]; total: number }) {
  return (
    <Card title="Log del relay" badge={`${lines.length} lineas · ${total} en total`} wide>
      <pre className="logs">{lines.join('\n') || 'Log vacío.'}</pre>
    </Card>
  )
}

export function ConfirmCard({ summary, resolving, onDecide }: {
  summary: string
  resolving: boolean
  onDecide: (decision: 'allow_once' | 'deny') => void
}) {
  return (
    <div className="confirm-card">
      <h3>Acción del panel — requiere confirmación</h3>
      <p>{summary}</p>
      <div className="actions">
        <button className="btn deny" disabled={resolving} onClick={() => onDecide('deny')}>Denegar</button>
        <button className="btn allow" disabled={resolving} onClick={() => onDecide('allow_once')}>Permitir una vez</button>
      </div>
    </div>
  )
}
