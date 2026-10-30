import { describe, expect, it } from 'vitest'
import { createPhaseOrbits, type OrbitLabel } from './phaseOrbits'
import { ATLAS_ORBIT_COLOR, HEFESTO_ORBIT_COLOR } from './orbitPlan'

class FakeSink {
  spawned: OrbitLabel[] = []
  retired: number[] = []
  retiredAll = 0
  bursts = 0
  private nextId = 1

  spawnOrbit(info?: OrbitLabel): number {
    this.spawned.push(info ?? { title: '', detail: '' })
    return this.nextId++
  }
  retireOrbit(id: number): void {
    this.retired.push(id)
  }
  retireAll(): void {
    this.retiredAll += 1
  }
  burstAll(): void {
    this.bursts += 1
  }
}

function setup() {
  const sink = new FakeSink()
  const orbits = createPhaseOrbits(() => sink)
  return { sink, orbits }
}

describe('createPhaseOrbits', () => {
  it('pide la órbita de la petición con el texto del usuario', () => {
    const { sink, orbits } = setup()
    orbits.request('req-1', 'hola', 'local')
    expect(sink.bursts).toBe(1)
    expect(sink.spawned).toHaveLength(1)
    expect(sink.spawned[0]).toMatchObject({ title: 'Tú', detail: 'hola' })
  })

  it('repetir la petición durante thinking no duplica ni reestalla', () => {
    const { sink, orbits } = setup()
    orbits.request('req-1', 'hola', 'local')
    orbits.request('req-1', 'hola', 'local')
    expect(sink.spawned).toHaveLength(1)
    expect(sink.bursts).toBe(1)
  })

  it('thinking por sí solo no abre órbita (la abre el mensaje del usuario)', () => {
    const { sink, orbits } = setup()
    orbits.apply('thinking', 'req-2')
    expect(sink.spawned).toHaveLength(0)
  })

  it('executing sustituye la petición por la fase de Hefesto en violeta', () => {
    const { sink, orbits } = setup()
    orbits.request('req-1', 'lee el archivo', 'local')
    orbits.apply('executing', 'req-1', 'hefesto_read_file')
    expect(sink.retired).toEqual([1])
    expect(sink.spawned[1]).toMatchObject({
      title: 'Hefesto · Leyendo archivo',
      color: HEFESTO_ORBIT_COLOR,
    })
  })

  it('responding sustituye la fase por la respuesta de Atlas en azul', () => {
    const { sink, orbits } = setup()
    orbits.apply('responding', 'req-1')
    expect(sink.spawned[0]).toMatchObject({
      title: 'Atlas',
      detail: 'Redactando respuesta',
      color: ATLAS_ORBIT_COLOR,
    })
  })

  it('el estado terminal retira todas las órbitas y limpia la fase', () => {
    const { sink, orbits } = setup()
    orbits.apply('responding', 'req-1')
    orbits.apply('completed', 'req-1')
    expect(sink.retiredAll).toBe(1)
    orbits.apply('responding', 'req-1')
    expect(sink.spawned).toHaveLength(2)
  })

  it('sin puerto montado no falla', () => {
    const orbits = createPhaseOrbits(() => null)
    expect(() => {
      orbits.request('req-1', 'hola', 'local')
      orbits.apply('executing', 'req-1', 'hefesto_list_dir')
      orbits.apply('completed', 'req-1')
    }).not.toThrow()
  })
})
