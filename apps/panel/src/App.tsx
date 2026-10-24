import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  j, post, usePoll,
  type Health, type LogsResp, type OpResp, type SessionResp, type Snapshot, type Stats,
} from './api'
import {
  Card, ConfirmCard, LogsCard, MacCard, PeticionesCard,
  PortsCard, RelayCard, UnitsCard,
} from './components/Cards'

interface Pending { op: string; kind: string; summary: string }

export default function App() {
  const [fadeOut, setFadeOut] = useState(true)
  useEffect(() => {
    const id = setTimeout(() => setFadeOut(false), 30)
    return () => clearTimeout(id)
  }, [])

  const { data: snap, error: sysErr } = usePoll<Snapshot>(
    () => j<Snapshot>('/v1/system'), 5000)
  const { data: health } = usePoll<Health>(() => j<Health>('/v1/health'), 10000)
  const { data: session } = usePoll<SessionResp>(() => j<SessionResp>('/v1/session'), 8000)
  const { data: stats } = usePoll<Stats>(() => j<Stats>('/v1/stats'), 30000)
  const { data: logs } = usePoll<{ lines: string[]; total: number }>(
    () => j<LogsResp>('/v1/logs?tail=140'), 15000)

  const [pending, setPending] = useState<Pending | null>(null)
  const [resolving, setResolving] = useState(false)
  const [toast, setToast] = useState<{ text: string; err?: boolean } | null>(null)
  const flash = useCallback((text: string, err = false) => setToast({ text, err }), [])
  useEffect(() => {
    if (!toast) return
    const id = setTimeout(() => setToast(null), 6000)
    return () => clearTimeout(id)
  }, [toast])

  const ask = useCallback(async (kind: 'service' | 'cancel_all',
                                 fields: Record<string, string>) => {
    try {
      const op = await post<OpResp>('/v1/system/asks', { kind, ...fields })
      setPending({ op: op.op, kind: op.kind, summary: op.summary })
    } catch (exc) {
      flash(`No se pudo pedir la acción: ${(exc as Error).message}`, true)
    }
  }, [flash])

  const decide = useCallback(async (decision: 'allow_once' | 'deny') => {
    if (!pending) return
    setResolving(true)
    try {
      const res = await post<{ ok: boolean; executed: boolean; result?: Record<string, unknown> }>(
        '/v1/system/resolve', { op: pending.op, decision })
      if (!res.executed) {
        flash('Acción denegada.')
      } else if (res.result?.ok) {
        flash(`✅ ${pending.summary}`)
      } else {
        flash(`Falló: ${res.result?.error ?? res.result?.output ?? 'sin detalle'}`, true)
      }
    } catch (exc) {
      flash((exc as Error).message, true)
    } finally {
      setResolving(false)
      setPending(null)
    }
  }, [pending, flash])

  const viewer = useMemo(() => {
    const err = sysErr ?? null
    if (err) {
      return <Card title="Relay inaccesible" tone="err"><div className="empty">{err}</div></Card>
    }
    if (!snap) return null
    const relayUnit = snap.units.find(u => u.unit === 'atlas-relay.service')
    return (
      <>
        <UnitsCard snap={snap} onAct={(unit, verb) => ask('service', { unit, verb })} />
        <PortsCard snap={snap} />
        <MacCard snap={snap} />
        <RelayCard health={health} stats={stats} relayUnit={relayUnit} />
        <PeticionesCard health={health} session={session}
                        onCancelAll={() => ask('cancel_all', {})} />
        <LogsCard lines={logs?.lines ?? []} total={logs?.total ?? 0} />
      </>
    )
  }, [snap, sysErr, health, stats, session, logs, ask])

  return (
    <div style={{ opacity: fadeOut ? 0.65 : 1, transition: 'opacity 1.2s ease' }}>
      <div className="app-label">
        ATLAS · PANEL
        <small>atlas-hub · {snap?.generated_at?.slice(11, 19) ?? '—'} UTC</small>
      </div>
      <main className="grid">{viewer}</main>
      {pending && (
        <ConfirmCard summary={pending.summary} resolving={resolving} onDecide={decide} />
      )}
      {toast && <div className={`toast${toast.err ? ' err' : ''}`}>{toast.text}</div>}
    </div>
  )
}
