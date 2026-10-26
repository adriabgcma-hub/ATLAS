import { useCallback, useEffect, useMemo, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import {
  IN_TAURI, fmtDuration, gb, j, post, restartRelay, usePoll,
  type Health, type LogsResp, type OpResp, type SessionResp, type Snapshot, type Stats,
} from './api'
import {
  Card, ConfirmCard, LogsCard, PeticionesCard,
  PortsCard, RelayCard, RestartCard, UnitsCard,
} from './components/Cards'

interface Pending { op: string; kind: string; summary: string }

/** KPI de la portada: cifra grande + contexto. */
function Kpi({ label, value, unit, sub, bar }: {
  label: string; value: string | number; unit?: string; sub?: string; bar?: number
}) {
  return (
    <div className="kpi">
      <div className="kpi-k">{label}</div>
      <div className="kpi-v">{value}{unit && value !== '—' ? <small>{unit}</small> : null}</div>
      {bar != null && (
        <div className="kpi-bar"><span style={{ width: `${Math.max(0, Math.min(100, bar))}%` }} /></div>
      )}
      {sub ? <div className="kpi-sub">{sub}</div> : null}
    </div>
  )
}

export default function App() {
  const [fadeOut, setFadeOut] = useState(true)
  useEffect(() => {
    const id = setTimeout(() => setFadeOut(false), 30)
    return () => clearTimeout(id)
  }, [])

  const { data: snap, error: sysErr } = usePoll<Snapshot>(
    () => j<Snapshot>('/v1/system'), 2000)
  const { data: health } = usePoll<Health>(() => j<Health>('/v1/health'), 2000)
  const { data: session } = usePoll<SessionResp>(() => j<SessionResp>('/v1/session'), 2000)
  const { data: stats } = usePoll<Stats>(() => j<Stats>('/v1/stats'), 5000)
  const { data: logs } = usePoll<{ lines: string[]; total: number }>(
    () => j<LogsResp>('/v1/logs?tail=140'), 2000)

  const [pending, setPending] = useState<Pending | null>(null)
  const [resolving, setResolving] = useState(false)
  const [toast, setToast] = useState<{ text: string; err?: boolean } | null>(null)
  const flash = useCallback((text: string, err = false) => setToast({ text, err }), [])
  useEffect(() => {
    if (!toast) return
    const id = setTimeout(() => setToast(null), 6000)
    return () => clearTimeout(id)
  }, [toast])

  // Reinicio del relay (solo app macOS). El menú de la barra emite este evento
  // para abrir la confirmación; la acción real la ejecuta Rust por SSH.
  const [restartStep, setRestartStep] = useState<0 | 1 | 2>(0)
  const [restarting, setRestarting] = useState(false)
  useEffect(() => {
    if (!IN_TAURI) return
    let dispose: (() => void) | undefined
    let cancelled = false
    listen('panel://restart-requested', () => setRestartStep(1))
      .then(fn => { if (cancelled) fn(); else dispose = fn })
      .catch(() => {})
    return () => { cancelled = true; dispose?.() }
  }, [])

  const doRestart = useCallback(async () => {
    setRestarting(true)
    try {
      const res = await restartRelay()
      if (res.ok) flash(`Relay reiniciado (${res.state})`)
      else flash(`El relay no quedó activo: ${res.state || 'sin estado'}`, true)
    } catch (exc) {
      flash(`No se pudo reiniciar: ${(exc as Error).message}`, true)
    } finally {
      setRestarting(false)
      setRestartStep(0)
    }
  }, [flash])

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
        flash(pending.summary)
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
        {/* Orden de la portada: primero el relay (lo que se vigila), luego
            servicios/puertos y la actividad, y al final el log. Los datos del
            Mac viven en los KPIs: no se duplican en una tarjeta. */}
        <RelayCard health={health} stats={stats} relayUnit={relayUnit} />
        <UnitsCard snap={snap} onAct={(unit, verb) => ask('service', { unit, verb })} />
        <PortsCard snap={snap} />
        <PeticionesCard health={health} session={session}
                        onCancelAll={() => ask('cancel_all', {})} />
        <LogsCard lines={logs?.lines ?? []} total={logs?.total ?? 0} />
      </>
    )
  }, [snap, sysErr, health, stats, session, logs, ask])

  const units = snap?.units ?? []
  const res = snap?.resources
  const disk = res?.disk_root
  const diskPct = disk ? Math.round((100 * (disk.total - disk.free)) / disk.total) : null
  const activeUnits = units.filter(u => u.active === 'active').length
  const tone = health?.ok ? 'ok' : health ? 'warn' : sysErr ? 'down' : 'dim'
  const statusText = { ok: 'RELAY OK', warn: 'DEGRADADO', down: 'SIN CONEXIÓN', dim: 'CONECTANDO' }[tone]
  const statusSub = health
    ? `${activeUnits}/${units.length || '—'} servicios · ${health.inflight} en vuelo · ${health.queue} en cola`
    : 'atlas-hub no responde'

  return (
    <div className="shell" style={{ opacity: fadeOut ? 0.65 : 1, transition: 'opacity 1.2s ease' }}>
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">◈</span>
          <div>
            <div className="brand-name">HUB PANEL</div>
            <div className="brand-sub">
              atlas-hub · {snap?.generated_at?.slice(11, 19) ?? '—'} UTC
            </div>
          </div>
        </div>

        <div className={`relay-pill ${tone}`}>
          <span className="relay-led" aria-hidden="true" />
          <div className="relay-copy">
            <div className="relay-state">{statusText}</div>
            <div className="relay-sub">{statusSub}</div>
          </div>
        </div>

        {IN_TAURI && (
          <div className="topbar-actions">
            <button className="btn restart" disabled={restartStep !== 0}
                    onClick={() => setRestartStep(1)}>
              Reiniciar relay
            </button>
          </div>
        )}
      </header>

      {snap && (
        <section className="kpis" aria-label="Resumen">
          <Kpi label="Servicios" value={`${activeUnits}/${units.length}`} sub="unidades activas" />
          <Kpi label="Peticiones" value={health?.inflight ?? 0}
               sub={`cola ${health?.queue ?? 0} · ${health?.events ?? 0} eventos`} />
          <Kpi label="Memoria" value={res?.mem_used_pct ?? '—'} unit="%"
               bar={res?.mem_used_pct ?? 0} sub={`carga ${res?.load_1 ?? '—'}`} />
          <Kpi label="Disco" value={diskPct ?? '—'} unit="%"
               bar={diskPct ?? 0}
               sub={res?.disk_root ? `${gb(res.disk_root.free)} libres` : 'sin datos'} />
          <Kpi label="Uptime" value={res?.uptime_s != null ? fmtDuration(res.uptime_s) : '—'}
               sub={res?.temp_c != null ? `${res.temp_c.toFixed(0)} °C` : 'servidor'} />
          <Kpi label="Energía" value={res?.battery ? `${res.battery.cap_pct}%` : (res?.ac_connected ? 'AC' : '—')}
               sub={res?.battery ? `${res.battery.status} · ${res?.ac_connected ? 'AC' : 'batería'}` : 'sin batería'} />
        </section>
      )}

      <main className="grid">{viewer}</main>
      {pending && (
        <ConfirmCard summary={pending.summary} resolving={resolving} onDecide={decide} />
      )}
      {restartStep !== 0 && (
        <RestartCard step={restartStep} busy={restarting}
                     onCancel={() => setRestartStep(0)}
                     onNext={() => setRestartStep(2)}
                     onRestart={doRestart} />
      )}
      {toast && <div className={`toast${toast.err ? ' err' : ''}`}>{toast.text}</div>}
    </div>
  )
}
