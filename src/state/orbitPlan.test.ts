import { describe, expect, it } from 'vitest'
import { ATLAS_ORBIT_COLOR, HEFESTO_ORBIT_COLOR, planPhaseOrbit } from './orbitPlan'

describe('planPhaseOrbit', () => {
  it('dibuja la lectura de Hefesto en violeta con la acción concreta', () => {
    const plan = planPhaseOrbit('executing', 'hefesto_read_file')
    expect(plan).toEqual({
      key: 'executing:hefesto_read_file',
      title: 'Hefesto · Leyendo archivo',
      detail: '',
      color: HEFESTO_ORBIT_COLOR,
    })
  })

  it('distingue el listado de carpetas', () => {
    const plan = planPhaseOrbit('executing', 'hefesto_list_dir')
    expect(plan?.title).toBe('Hefesto · Listando carpeta')
    expect(plan?.color).toBe(HEFESTO_ORBIT_COLOR)
  })

  it('el encargo asíncrono también es Hefesto', () => {
    const plan = planPhaseOrbit('executing', 'hefesto_echo')
    expect(plan?.title).toBe('Hefesto · Encargo')
    expect(plan?.color).toBe(HEFESTO_ORBIT_COLOR)
  })

  it('otras herramientas usan el nombre del catálogo en azul', () => {
    const plan = planPhaseOrbit('executing', 'calendar_create_event')
    expect(plan).toEqual({
      key: 'executing:calendar_create_event',
      title: 'Herramienta · calendar_create_event',
      detail: '',
      color: ATLAS_ORBIT_COLOR,
    })
  })

  it('sin nombre de herramienta no inventa etiqueta', () => {
    const plan = planPhaseOrbit('executing')
    expect(plan?.title).toBe('Herramienta')
    expect(plan?.key).toBe('executing')
  })

  it('la respuesta la dibuja Atlas en azul', () => {
    const plan = planPhaseOrbit('responding')
    expect(plan).toEqual({
      key: 'responding',
      title: 'Atlas',
      detail: 'Redactando respuesta',
      color: ATLAS_ORBIT_COLOR,
    })
  })

  it('los estados sin fase propia no dibujan órbita', () => {
    for (const state of ['idle', 'thinking', 'awaiting_confirmation', 'completed', 'cancelled', 'failed'] as const) {
      expect(planPhaseOrbit(state)).toBeNull()
    }
  })
})
