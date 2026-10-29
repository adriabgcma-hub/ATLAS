/** API del panel: todo pasa por el relay (misma LAN, sin auth — perímetro de red).
 *
 *  Hay dos entornos:
 *  - Web (`/panel/` servido por el relay): `fetch` directo con CORS.
 *  - App macOS (Tauri, `apps/panel-app`): las peticiones van por los comandos
 *    IPC `api_get` / `api_post` (reqwest en Rust), sin CORS y sin que el
 *    webview conozca la URL del relay.
 */

import { invoke, isTauri } from '@tauri-apps/api/core'

export const RELAY_URL =
  (import.meta.env.VITE_RELAY_URL as string | undefined) ?? window.location.origin

const IN_TAURI = isTauri()

/** true cuando la UI corre dentro de la app macOS (apps/panel-app). */
export { IN_TAURI }

export interface RestartResp {
  ok: boolean
  state: string
  output: string
}

/** Reinicia el relay por SSH (solo app Tauri; comando fijo en authorized_keys). */
export const restartRelay = () => invoke<RestartResp>('restart_relay')

/** El menú de la barra pidió reiniciar antes de crear la ventana. */
export const hasPendingRestart = () => invoke<boolean>('pending_restart')

export const clearPendingRestart = () => invoke<void>('clear_restart_request')

async function tauriRequest<T>(path: string, init?: RequestInit): Promise<T> {
  const method = (init?.method ?? 'GET').toUpperCase()
  try {
    if (method === 'GET') {
      return await invoke<T>('api_get', { path })
    }
    const raw = init?.body
    const body = typeof raw === 'string' ? JSON.parse(raw) : (raw ?? null)
    return await invoke<T>('api_post', { path, body })
  } catch (exc) {
    throw new Error(String((exc as Error)?.message ?? exc), { cause: exc })
  }
}

export async function j<T>(path: string, init?: RequestInit): Promise<T> {
  if (IN_TAURI) return tauriRequest<T>(path, init)

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

export interface Snapshot {
  generated_at: string
  units: Unit[]
  ports: { port: number; address: string; service?: string | null }[]
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
  actions: { verbs: string[]; system_units: string[] }
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

export interface HefestoStatus {
  configured: boolean
  deviceId: string | null
  suspended: boolean | null
  generation: number | null
}

export const fetchHefestoStatus = () =>
  j<HefestoStatus>('/v1/hefesto/status')

export const suspendHefesto = () =>
  post<{ ok: boolean; suspended?: boolean; error?: string }>('/v1/hefesto/suspend', {})

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
