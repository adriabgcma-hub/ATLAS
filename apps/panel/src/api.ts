/** API del panel: todo pasa por el relay (misma LAN, sin auth — perímetro de red). */

export const RELAY_URL =
  (import.meta.env.VITE_RELAY_URL as string | undefined) ?? window.location.origin

export async function j<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(RELAY_URL + path, {
    headers: { Accept: 'application/json' },
    ...init,
  })
  if (!res.ok) {
    const text = await res.text().catch(() => '')
    throw new Error(`HTTP ${res.status}: ${text}`, { cause: res.status })
  }
  return res.json() as Promise<T>
}

export const post = <T>(path: string, body: unknown) =>
  j<T>(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })

// ------------------- tipos (espejo de server/system_monitor.py) -------------------

export interface Unit {
  unit: string
  scope: 'system' | 'user'
  description?: string | null
  load?: string | null
  active?: string | null
  sub?: string | null
  since?: string | null
  pid?: number | null
  mem_bytes?: number | null
  actionable?: boolean
  error?: string
}

export interface ContainerInfo {
  id?: string
  name?: string
  image?: string
  state?: string
  status?: string
  compose_service?: string | null
  compose_project?: string | null
}

export interface Snapshot {
  generated_at: string
  units: Unit[]
  ports: { port: number; address: string; service?: string | null }[]
  docker: { available: boolean; containers: ContainerInfo[] }
  cloud: {
    compose_project: string
    containers: ContainerInfo[]
    nextcloud: { status: string | null; version: string | null; error?: string | null; maintenance?: boolean }
    tailscale_serve: { url: string | null; raw?: string | null }
  }
  resources: {
    mem_total_kb?: number
    mem_available_kb?: number
    mem_used_pct?: number | null
    disk_root?: { total: number; free: number } | null
    uptime_s?: number
    load_1?: number
    temps?: { src: string; label: string | null; celsius: number }[]
    temp_c?: number | null
    battery?: { cap_pct: number; status: string } | null
    ac_connected?: boolean | null
  }
  actions: { verbs: string[]; system_units: string[]; user_units: string[]; compose_targets: string[] }
}

export interface Health {
  ok: boolean
  telegram: string
  llm: string
  calendar: string
  events: number
  queue: number
  inflight: number
  bot?: { id?: number; username?: string; first_name?: string }
}

export interface Msg {
  requestId: string
  role: string
  content: string
  source: 'local' | 'telegram' | string
  timestamp: string
}

export interface SessionResp {
  session: { sessionId: string; state: string; messages: Msg[] } | null
  cursor: number
}

export interface Stats {
  token_usage: {
    prompt: number
    completion: number
    total: number
    calls: number
    by_model: Record<string, { prompt: number; completion: number; total: number; calls: number }>
  }
}

export interface OpResp { ok: boolean; op: string; kind: string; summary: string }

export interface LogsResp { lines: string[]; total: number }

// ------------------- hooks -------------------

import { useEffect, useRef, useState } from 'react'

export function usePoll<T>(fn: () => Promise<T>, ms: number, active = true) {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)
  const fnRef = useRef(fn)
  fnRef.current = fn
  useEffect(() => {
    if (!active) return
    let alive = true
    let timer: ReturnType<typeof setTimeout> | undefined
    const tick = async () => {
      try {
        const value = await fnRef.current()
        if (alive) { setData(value); setError(null) }
      } catch (exc) {
        if (alive) setError(String((exc as Error).message ?? exc))
      } finally {
        if (alive) timer = setTimeout(tick, ms)
      }
    }
    tick()
    return () => { alive = false; if (timer) clearTimeout(timer) }
  }, [ms, active])
  return { data, error }
}

export function fmtDuration(s?: number | null): string {
  if (s == null || !Number.isFinite(s)) return '—'
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  return d > 0 ? `${d}d ${h}h` : h > 0 ? `${h}h ${m}m` : `${m}m`
}

export function gb(bytes?: number | null): string {
  if (bytes == null) return '—'
  return `${(bytes / 1e9).toFixed(0)} GB`
}
