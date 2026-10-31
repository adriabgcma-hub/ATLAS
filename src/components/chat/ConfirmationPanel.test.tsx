import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { ConfirmationPanel } from './ConfirmationPanel'
import type { ConfirmationRequest } from '../../types/contracts'

function req(partial: Partial<ConfirmationRequest>): ConfirmationRequest {
  return {
    requestId: 'req-1',
    sessionId: 'ses-1',
    timestamp: '2026-10-29T00:00:00.000Z',
    type: 'permission.confirmation_required',
    toolId: 'hefesto_write_file',
    summary: 'Crear «/x/a.txt»',
    risk: 'write',
    expiresAt: '2026-10-29T00:05:00.000Z',
    ...partial,
  }
}

describe('etiqueta de la tarjeta de confirmación', () => {
  it('marca HEFESTO las herramientas de Hefesto', () => {
    const html = renderToStaticMarkup(
      <ConfirmationPanel items={[req({})]} onRespond={vi.fn()} />,
    )
    expect(html).toContain('HEFESTO')
    expect(html).not.toContain('ATLAS')
  })

  it('mantiene ATLAS para el resto de herramientas', () => {
    const html = renderToStaticMarkup(
      <ConfirmationPanel
        items={[req({ toolId: 'send_email', risk: 'external' })]}
        onRespond={vi.fn()}
      />,
    )
    expect(html).toContain('ATLAS')
    expect(html).not.toContain('HEFESTO')
  })
})
