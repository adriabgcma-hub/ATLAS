import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'

interface TablePayload {
  title: string
  headers: string[]
  rows: string[][]
}

export function TableWindow() {
  const [payload, setPayload] = useState<TablePayload | null>(null)
  const [error, setError] = useState('')
  const [status, setStatus] = useState('')
  const [busy, setBusy] = useState(false)
  const label = getCurrentWindow().label

  useEffect(() => {
    let alive = true
    invoke<TablePayload>('table_payload', { label })
      .then((value) => {
        if (alive) setPayload(value)
      })
      .catch(() => {
        if (alive) setError('No se pudo cargar la tabla.')
      })
    return () => {
      alive = false
    }
  }, [label])

  async function exportCsv() {
    setBusy(true)
    setStatus('')
    try {
      const path = await invoke<string>('export_table_csv', { label })
      setStatus(`Exportado a ${path}`)
    } catch {
      setStatus('No se pudo exportar el CSV.')
    } finally {
      setBusy(false)
    }
  }

  if (error) {
    return (
      <main className="table-window">
        <p className="table-error" role="alert">{error}</p>
      </main>
    )
  }
  if (!payload) {
    return (
      <main className="table-window">
        <p className="table-loading">Cargando tabla…</p>
      </main>
    )
  }
  return (
    <main className="table-window">
      <header className="table-header">
        <div>
          <p className="table-kicker">ATLAS · TABLA</p>
          <h1>{payload.title || 'Tabla'}</h1>
          <p className="table-meta">
            {payload.rows.length} filas · {payload.headers.length} columnas
          </p>
        </div>
        <button type="button" onClick={exportCsv} disabled={busy}>
          {busy ? 'Exportando…' : 'Exportar CSV'}
        </button>
      </header>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              {payload.headers.map((header, index) => (
                <th key={`${index}-${header}`} scope="col">{header}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {payload.rows.map((row, rowIndex) => (
              <tr key={rowIndex}>
                {row.map((cell, cellIndex) => (
                  <td key={cellIndex}>{cell}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="table-status" role="status" aria-live="polite">{status}</p>
    </main>
  )
}
