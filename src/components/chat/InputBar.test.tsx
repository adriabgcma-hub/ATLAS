import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { InputBar } from './InputBar'

const base = {
  onSubmit: vi.fn(),
  onCancel: vi.fn(),
  onTogglePanel: vi.fn(),
  thinking: false,
  expanded: false,
  hasConfirmations: false,
}

function render(pending: boolean) {
  return renderToStaticMarkup(<InputBar {...base} pending={pending} />)
}

describe('barra de transmisión', () => {
  it('con petición en curso los controles quedan activos para cancelar', () => {
    const html = render(true)
    expect(html).toContain('Petición en curso — Enter para cancelar')
    expect(html).toContain('Cancelar petición')
    expect(html).not.toContain('disabled')
  })

  it('en reposo sin texto el botón de enviar sigue deshabilitado', () => {
    const html = render(false)
    expect(html).toContain('disabled=""')
    expect(html).toContain('Enviar mensaje')
  })
})
