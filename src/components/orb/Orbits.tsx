import { useEffect, useRef, useState, type CSSProperties, type RefObject } from 'react'

/** Órbita viva: una bola de energía + estela de Atlas. */
interface OrbitalParticle {
  radius: number
  // Fase actual, integrada frame a frame. NUNCA recalcular desde tiempo
  // absoluto: multiplicar tiempo acumulado por un multiplicador de velocidad
  // que cambia (thinking/responding/…) reposiciona todas las partículas de golpe.
  angle: number
  baseSpeed: number
  size: number
  color: string
  tiltX: number
  tiltY: number
  /** ms (performance.now) de creación; la órbita sale del orbe. */
  bornAt: number
  /** ms de retirada (fin de tarea + 5 s) o null mientras la tarea sigue viva. */
  retireAt: number | null
  /** Última posición visible de la bola (para la explosión aunque expire oculta). */
  seenX: number
  seenY: number
  seenR: number
  /** Qué representa la órbita (se muestra en la mini-burbuja al acercarse). */
  title: string
  detail: string
  /** Identificador único para el hover. */
  id: number
}

/** Etiqueta de la órbita para la mini-burbuja. */
export interface OrbitInfo {
  title: string
  detail: string
}

/** Contenido de la mini-burbuja de proximidad. */
interface OrbitHover extends OrbitInfo {
  id: number
  color: string
}

/** Chispa de la mini-explosión de retirada (espacio pantalla, px y px/s). */
interface BurstSpark {
  x: number
  y: number
  vx: number
  vy: number
  bornAt: number
  sprite: HTMLCanvasElement
  size: number
}

/** Puerto imperativo (cero re-renders, como ScenePort): App crea una órbita
 *  al recibir cada petición y retira las vivas al terminar la tarea.
 *  Atlas es único: un solo color; cada petición crea su órbita fresca. */
export interface OrbitPort {
  /** Crea una órbita de Atlas con su etiqueta. Devuelve el id de la órbita. */
  spawnOrbit: (info?: OrbitInfo) => number
  /** Retira una órbita concreta por su id (vive 5 s más y estalla). */
  retireOrbit: (id: number) => void
  /** Las órbitas en vivo pasan a retiradas: viven 5 s más y terminan con
   *  una mini-explosión de su color. */
  retireAll: () => void
  /** Estalla de inmediato las retiradas (al empezar una petición nueva). */
  burstAll: () => void
}

const TRAIL_LENGTH = 24
// Arco total (radianes) que abarca la estela: FIJO para todas las partículas,
// independiente de su velocidad. Con estela basada en tiempo, al ralentizar
// las órbitas los puntos se amontonaban detrás de la bola (todo fusionado).
// TRAIL_LENGTH crece parejo con TRAIL_ARC para mantener la densidad de puntos.
const TRAIL_ARC = 0.6
const CAMERA_Z = 4
const FOV = (50 * Math.PI) / 180
// Tope de dt: si el hilo principal se congela, al reanudar el tiempo simulado
// avanza como mucho MAX_DT por frame renderizado (sin saltos de posición).
const MAX_DT = 0.05

// Vida extra de la órbita tras el fin de la tarea + mini-explosión final.
const ORBIT_LINGER_MS = 5_000
const BURST_LIFE_MS = 550
const BURST_SPARKS = 14
// Aparición: la órbita NACE EN EL ORBE (radio 0) y se expande en espiral
// hasta su radio con ease-out (ver radiusNow abajo). CONGELADO con el resto
// de parámetros visuales de Orbits (ver AGENTS.md): no se reajusta.
const SPAWN_IN_MS = 900
// Mini-burbuja: aparece tras este tiempo con el puntero encima (evita que un
// roce accidental de ratón abra la etiqueta) y se oculta si el puntero se va
// antes de este tiempo (evita el parpadeo de un Bubble on/off).
const HOVER_DELAY_MS = 300
const HOVER_COMMIT_MS = 1_000

// Sprites pre-renderizados en lugar de arc() + shadowBlur por punto: el blur
// gaussiano de canvas2d saturaba el hilo principal y congelaba la UI.
// Dos tipos de textura por color (misma fuente `SPRITE_CACHE`):
//   - ANILLO (energía): halo suave + aro nítido + núcleo brillante, estética
//     HUD de Atlas (nada de blob de plasma). Opaco en el aro: tapa su estela.
//   - ESTELA: disco plano bien aclarado (TRAIL_TINT) → tono más claro que el
//     aro, para que el rastro se lea como una cola brillante.
const SPRITE_CACHE = new Map<string, HTMLCanvasElement>()

function shadeChannels(color: string, amount: number, dark: boolean): [number, number, number] {
  const r = parseInt(color.slice(1, 3), 16)
  const g = parseInt(color.slice(3, 5), 16)
  const b = parseInt(color.slice(5, 7), 16)
  const f = dark
    ? (c: number) => Math.round(c * (1 - amount))
    : (c: number) => Math.round(c + (255 - c) * amount)
  return [f(r), f(g), f(b)]
}

function lighten(color: string, amount: number): string {
  const [r, g, b] = shadeChannels(color, amount, false)
  return `rgb(${r}, ${g}, ${b})`
}

// Color con alpha a partir del hex ORIGINAL + tinte + alpha. `dark` oscurece
// hacia negro (cada canal hacia 0), si no aclara. Parsea hex, no rgb(): los callers
// deben pasar el color base de Atlas (hex).
function rgba(color: string, amount: number, alpha: number, dark = false): string {
  const [r, g, b] = shadeChannels(color, amount, dark)
  return `rgba(${r}, ${g}, ${b}, ${alpha})`
}

const TRAIL_TINT = 0.5

// Anillo de energía de Atlas: halo + aro + núcleo (ver getOrbitSprite).
// El aro tapa su estela (source-over) y al solaparse no acumula brillo.
// El cuerpo visible vive hasta ~60% del radio del sprite: multiplicador de
// drawSize para que el aro lea al tamaño de la bola original (2.4), dejando
// el halo fuera.
const ORB_BALL_DRAW = 3.0

function getOrbitSprite(color: string): HTMLCanvasElement {
  const key = `orbit:${color}`
  const cached = SPRITE_CACHE.get(key)
  if (cached) return cached

  const SIZE = 64
  const half = SIZE / 2
  const sprite = document.createElement('canvas')
  sprite.width = SIZE
  sprite.height = SIZE
  const sctx = sprite.getContext('2d')!
  // Halo exterior suave.
  const glow = sctx.createRadialGradient(half, half, 0, half, half, half - 1)
  glow.addColorStop(0, rgba(color, 0.55, 0.28))
  glow.addColorStop(0.55, rgba(color, 0.3, 0.12))
  glow.addColorStop(1, rgba(color, 0.3, 0))
  sctx.fillStyle = glow
  sctx.fillRect(0, 0, SIZE, SIZE)
  // Aro de energía: trazo nítido.
  sctx.strokeStyle = rgba(color, 0.35, 0.95)
  sctx.lineWidth = 5
  sctx.beginPath()
  sctx.arc(half, half, half * 0.52, 0, Math.PI * 2)
  sctx.stroke()
  // Filo brillante en el arco superior (lectura de luz).
  sctx.strokeStyle = 'rgba(255, 255, 255, 0.85)'
  sctx.lineWidth = 1.5
  sctx.beginPath()
  sctx.arc(half, half, half * 0.52, -Math.PI * 0.8, -Math.PI * 0.2)
  sctx.stroke()
  // Núcleo: punto caliente que funde a transparente.
  const core = sctx.createRadialGradient(half, half, 0, half, half, half * 0.3)
  core.addColorStop(0, 'rgba(255, 255, 255, 0.95)')
  core.addColorStop(0.5, rgba(color, 0.5, 0.9))
  core.addColorStop(1, rgba(color, 0.5, 0))
  sctx.fillStyle = core
  sctx.beginPath()
  sctx.arc(half, half, half * 0.3, 0, Math.PI * 2)
  sctx.fill()

  SPRITE_CACHE.set(key, sprite)
  return sprite
}

function getDiscSprite(fill: string): HTMLCanvasElement {
  const key = `disc:${fill}`
  const cached = SPRITE_CACHE.get(key)
  if (cached) return cached

  const SIZE = 64
  const half = SIZE / 2
  const sprite = document.createElement('canvas')
  sprite.width = SIZE
  sprite.height = SIZE
  const sctx = sprite.getContext('2d')!
  // Disco plano: círculo sólido; el navegador anti-aliasa el borde del path.
  sctx.fillStyle = fill
  sctx.beginPath()
  sctx.arc(half, half, half - 1, 0, Math.PI * 2)
  sctx.fill()

  SPRITE_CACHE.set(key, sprite)
  return sprite
}

export function Orbits({ asleep = false, portRef }: { asleep?: boolean; portRef?: RefObject<OrbitPort | null> }) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const bubbleRef = useRef<HTMLDivElement>(null)
  const animationRef = useRef<number>(0)
  // Espejo del prop para el bucle rAF (sin re-crear el efecto al alternar).
  const asleepRef = useRef(asleep)
  // Mini-burbuja de proximidad: el contenido viaja por estado (cambia poco);
  // la posición se escribe directa en el DOM cada frame (cero re-renders).
  const [hover, setHover] = useState<OrbitHover | null>(null)
  const hoverIdRef = useRef<number | null>(null)
  const mouseRef = useRef({ x: -9999, y: -9999, inside: false })

  useEffect(() => {
    asleepRef.current = asleep
  }, [asleep])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const dpr = Math.min(window.devicePixelRatio || 1, 2)

    const resize = () => {
      canvas.width = window.innerWidth * dpr
      canvas.height = window.innerHeight * dpr
      canvas.style.width = `${window.innerWidth}px`
      canvas.style.height = `${window.innerHeight}px`
      // setTransform (no scale): la escala no se acumula entre resize.
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    }
    resize()
    window.addEventListener('resize', resize)

    // Puntero para la mini-burbuja (coords CSS px: el canvas ocupa todo).
    const onPointerMove = (event: PointerEvent) => {
      mouseRef.current.x = event.clientX
      mouseRef.current.y = event.clientY
      mouseRef.current.inside = true
    }
    const onPointerLeave = () => {
      mouseRef.current.inside = false
    }
    window.addEventListener('pointermove', onPointerMove)
    document.documentElement.addEventListener('pointerleave', onPointerLeave)

    // En reposo NO hay órbitas: se crean por demanda vía portRef (a petición
    // de App) y se retiran al terminar la tarea (+5 s y explosión).
    const orbits: OrbitalParticle[] = []
    const bursts: BurstSpark[] = []
    let spawnCount = 0

    // Sprites perezosos por color (caché de módulo compartida).
    const spritesByColor = new Map<string, { ball: HTMLCanvasElement; trail: HTMLCanvasElement }>()
    const spritesFor = (color: string) => {
      const cached = spritesByColor.get(color)
      if (cached) return cached
      const entry = {
        ball: getOrbitSprite(color),
        trail: getDiscSprite(lighten(color, TRAIL_TINT)),
      }
      spritesByColor.set(color, entry)
      return entry
    }

    // Atlas es único: un solo color azul eléctrico para todas las órbitas.
    const ATLAS_ORBIT_COLOR = '#3FA4FF'

    const spawnOrbit = (info?: OrbitInfo): number => {
      const color = ATLAS_ORBIT_COLOR
      spritesFor(color)
      const direction = Math.random() < 0.5 ? 1 : -1
      // Banda 1.02–1.30 casi frontal (±0.25 de inclinación): el anillo
      // proyectado nunca toca el orbe (radio 0.5) ni pasa por debajo de él.
      // Varias órbitas simultáneas se reparten la banda sin solaparse.
      const baseRadius = 1.02 + (spawnCount % 5) * 0.07
      const orbitId = spawnCount
      spawnCount += 1

      orbits.push({
        radius: baseRadius + (Math.random() - 0.5) * 0.05,
        angle: Math.random() * Math.PI * 2,
        // Velocidad base fija y lenta (rad/s): las órbitas NO reaccionan al
        // estado del chat (decisión de diseño: sin acelerones por estado).
        baseSpeed: (0.15 + Math.random() * 0.4) * direction,
        // Tamaño único (el del más grande): todas las órbitas iguales.
        size: 2.6,
        color,
        tiltX: (Math.random() - 0.5) * 0.5,
        tiltY: (Math.random() - 0.5) * 0.5,
        bornAt: performance.now(),
        retireAt: null,
        seenX: -1,
        seenY: -1,
        seenR: 0,
        title: info?.title ?? 'ATLAS',
        detail: info?.detail ?? '',
        id: orbitId,
      })
      return orbitId
    }

    const retireOrbit = (id: number) => {
      const until = performance.now() + ORBIT_LINGER_MS
      for (const o of orbits) {
        if (o.id === id && o.retireAt === null) {
          o.retireAt = until
          break
        }
      }
    }

    const retireAll = () => {
      const until = performance.now() + ORBIT_LINGER_MS
      for (const o of orbits) {
        if (o.retireAt === null) o.retireAt = until
      }
    }

    // Estallido inmediato de las retiradas: la petición nueva no espera
    // sus 5 s (expiran en el próximo frame, con explosión).
    const burstAll = () => {
      const now = performance.now()
      for (const o of orbits) {
        if (o.retireAt !== null) o.retireAt = Math.min(o.retireAt, now)
      }
    }

    if (portRef)
      portRef.current = { spawnOrbit, retireOrbit, retireAll, burstAll }

    let lastFrame = -1
    // Factor de sueño (0 despierto → 1 dormido): transición suave ~2 s.
    let sleepF = 0
    // Histéresis de la mini-burbuja: `hoverRawId` es la órbita bajo el
    // puntero ESTE frame (aún sin decidir si se muestra) y `hoverShownId` la
    // que está visible. Evita que un roce de ratón abra y cierre la etiqueta.
    let hoverRawId: number | null = null
    let hoverSince = 0
    let hoverShownId: number | null = null
    let bubbleX = 0
    let bubbleY = 0

    const animate = (now: number) => {
      // dt real con clamp (independiente del framerate; un stall nunca
      // acumula catch-up). El primer frame no integra.
      const dt = lastFrame >= 0 ? Math.min((now - lastFrame) / 1000, MAX_DT) : 0
      lastFrame = now

      // Sueño: las órbitas frenan a 1/4 de velocidad y se atenúan.
      const sleepTarget = asleepRef.current ? 1 : 0
      sleepF += (sleepTarget - sleepF) * Math.min(1, dt * 1.2)
      const sleepDim = 1 - 0.5 * sleepF

      // En reposo (sin órbitas ni explosiones) no hay nada que pintar.
      if (orbits.length === 0 && bursts.length === 0) {
        animationRef.current = requestAnimationFrame(animate)
        return
      }

      ctx.clearRect(0, 0, window.innerWidth, window.innerHeight)

      const centerX = window.innerWidth / 2
      const centerY = window.innerHeight / 2
      const focalLength = window.innerHeight / (2 * Math.tan(FOV / 2))

      // Mezcla aditiva una vez por frame (look neón a bajo coste).
      ctx.globalCompositeOperation = 'lighter'

      // Candidata a burbuja: la órbita viva más cercana al puntero.
      let candidate: OrbitalParticle | null = null
      let candX = 0
      let candY = 0
      let bestD2 = Infinity

      for (let i = orbits.length - 1; i >= 0; i--) {
        const p = orbits[i]
        // Integración de fase: la velocidad solo afecta al movimiento futuro.
        p.angle += p.baseSpeed * (1 - 0.75 * sleepF) * dt

        // Aparición desde el orbe (radio 0 → radio final con ease-out) +
        // vida: 0 en el frame de expiración (invisible; la explosión releva).
        const outT = Math.min(1, Math.max(0, (now - p.bornAt) / SPAWN_IN_MS))
        const outF = 1 - Math.pow(1 - outT, 3)
        const bornF = outT * outT * (3 - 2 * outT)
        const alive = p.retireAt === null || now < p.retireAt
        const lifeF = alive ? bornF : 0
        const radiusNow = p.radius * outF

        const { ball: ballSprite, trail: trailSprite } = spritesByColor.get(p.color)!
        const dir = p.baseSpeed >= 0 ? 1 : -1

        const cosTY = Math.cos(p.tiltY)
        const sinTY = Math.sin(p.tiltY)
        const cosTX = Math.cos(p.tiltX)
        const sinTX = Math.sin(p.tiltX)

        // Centro de la explosión este frame (-1 = oculta ahora; abajo se
        // usa la última posición visible guardada en seen*).
        let ballX = -1
        let ballY = -1
        let ballR = 0

        for (let t = TRAIL_LENGTH; t >= 0; t--) {
          // Estela de arco fijo hacia atrás en el sentido del movimiento.
          const angle = p.angle - t * (TRAIL_ARC / TRAIL_LENGTH) * dir

          const rawX = Math.cos(angle) * radiusNow
          const rawY = Math.sin(angle) * radiusNow

          const x3d = rawX * cosTY - rawY * sinTY * sinTX
          const y3d = rawY * cosTX
          const z3d = rawX * sinTY + rawY * cosTY * sinTX

          const zDist = CAMERA_Z - z3d
          if (zDist <= 0.1) continue
          const scale = focalLength / zDist
          const x2d = centerX + x3d * scale
          const y2d = centerY - y3d * scale

          // Culling barato: descarta puntos fuera de pantalla o subpíxel.
          if (
            x2d < -40 || x2d > window.innerWidth + 40 ||
            y2d < -40 || y2d > window.innerHeight + 40
          ) continue

          // Sin descarte ni atenuado por profundidad: todo el anillo siempre
          // visible (decisión del propietario).

          const fade = 1 - t / TRAIL_LENGTH
          const rBase = p.size * (scale / focalLength) * 8

          if (t === 0) {
            // Bola de energía: source-over (no aditivo). Al ser opaca cubre la
            // estela que hay debajo y al solaparse no acumula brillo entre
            // bolas (ORB_BALL_DRAW compensa el cuerpo al 60% del sprite).
            const drawSize = rBase * ORB_BALL_DRAW
            ballX = x2d
            ballY = y2d
            ballR = drawSize
            p.seenX = x2d
            p.seenY = y2d
            p.seenR = drawSize
            if (drawSize < 1) continue
            ctx.globalCompositeOperation = 'source-over'
            ctx.globalAlpha = lifeF * sleepDim
            ctx.drawImage(ballSprite, x2d - drawSize / 2, y2d - drawSize / 2, drawSize, drawSize)
            ctx.globalCompositeOperation = 'lighter'
          } else {
            // Estela: disco plano de tono claramente MÁS CLARO que la bola.
            const r = rBase * fade
            const drawSize = r * 1.7
            if (drawSize < 1) continue
            ctx.globalAlpha = fade * fade * 0.85 * lifeF * sleepDim
            ctx.drawImage(trailSprite, x2d - drawSize / 2, y2d - drawSize / 2, drawSize, drawSize)
          }
        }

        // Proximidad del puntero (tocando o muy cerca): solo órbitas vivas
        // y ya aparecidas. Gana la más cercana.
        if (alive && bornF > 0.6 && ballX >= 0 && mouseRef.current.inside) {
          const hr = ballR + 12
          const dx = ballX - mouseRef.current.x
          const dy = ballY - mouseRef.current.y
          const d2 = dx * dx + dy * dy
          if (d2 <= hr * hr && d2 < bestD2) {
            bestD2 = d2
            candidate = p
            candX = ballX
            candY = ballY
          }
        }

        // Fin de vida: la órbita estalla en chispas de su color y se retira.
        // Si la bola está tras la cámara este frame, se usa su última
        // posición visible (nunca se esfuma en silencio).
        if (!alive) {
          const ex = ballX >= 0 ? ballX : p.seenX
          const ey = ballX >= 0 ? ballY : p.seenY
          const er = ballX >= 0 ? ballR : p.seenR
          if (ex >= 0) {
            const { trail } = spritesByColor.get(p.color)!
            for (let s = 0; s < BURST_SPARKS; s++) {
              const a = Math.random() * Math.PI * 2
              const speed = 60 + Math.random() * 160
              bursts.push({
                x: ex,
                y: ey,
                vx: Math.cos(a) * speed,
                vy: Math.sin(a) * speed,
                bornAt: now,
                sprite: trail,
                size: Math.max(2, er * (0.25 + Math.random() * 0.3)),
              })
            }
          }
          orbits.splice(i, 1)
        }
      }

      // Mini-burbuja: contenido por estado (solo al cambiar de órbita),
      // posición escrita directa en el DOM cada frame (cero re-renders).
      //
      // Reglas: aparece 300 ms después de que el puntero esté encima de forma
      // continua (un roce no la abre) y, si el puntero se va antes de 1 s,
      // aguanta ese segundo en vez de parpadear.
      const rawId = candidate ? candidate.id : null
      if (rawId !== hoverRawId) {
        hoverRawId = rawId
        hoverSince = now
      }
      let visible: OrbitalParticle | null = null
      if (candidate) {
        if (now - hoverSince >= HOVER_DELAY_MS) visible = candidate
        bubbleX = Math.min(Math.max(candX + 18, 8), window.innerWidth - 256)
        bubbleY = Math.max(candY - 84, 8)
      } else if (hoverShownId !== null && now - hoverSince < HOVER_COMMIT_MS) {
        // Sigue dentro del periodo de gracia: se mantiene si la órbita no ha
        // expirado (puede haber estallado con el puntero encima).
        visible = orbits.find((o) => o.id === hoverShownId) ?? null
      }
      const visibleId = visible ? visible.id : null
      if (visibleId !== hoverIdRef.current) {
        hoverIdRef.current = visibleId
        hoverShownId = visibleId
        setHover(
          visible
            ? { id: visible.id, title: visible.title, detail: visible.detail, color: visible.color }
            : null,
        )
      }
      const bubble = bubbleRef.current
      if (bubble && visible) {
        bubble.style.transform = `translate(${bubbleX}px, ${bubbleY}px)`
      }

      // Mini-explosiones en curso (aditivas, 550 ms de vida).
      for (let i = bursts.length - 1; i >= 0; i--) {
        const b = bursts[i]
        const t = (now - b.bornAt) / BURST_LIFE_MS
        if (t >= 1) {
          bursts.splice(i, 1)
          continue
        }
        const ageS = (now - b.bornAt) / 1000
        const fade = 1 - t
        const drawSize = Math.max(1, b.size * fade)
        ctx.globalAlpha = fade * 0.9 * sleepDim
        ctx.drawImage(
          b.sprite,
          b.x + b.vx * ageS - drawSize / 2,
          b.y + b.vy * ageS - drawSize / 2,
          drawSize,
          drawSize,
        )
      }

      ctx.globalAlpha = 1
      ctx.globalCompositeOperation = 'source-over'

      animationRef.current = requestAnimationFrame(animate)
    }

    animationRef.current = requestAnimationFrame(animate)

    // Fase 1.4: con la ventana oculta/minimizada, detener el bucle por completo.
    // Al reanudar, lastFrame = -1 ⇒ dt = 0 en el primer frame (sin salto).
    const handleVisibilityChange = () => {
      if (document.hidden) {
        if (animationRef.current !== null) {
          cancelAnimationFrame(animationRef.current)
          animationRef.current = 0
        }
      } else if (animationRef.current === 0) {
        lastFrame = -1
        animationRef.current = requestAnimationFrame(animate)
      }
    }
    document.addEventListener('visibilitychange', handleVisibilityChange)

    return () => {
      cancelAnimationFrame(animationRef.current)
      window.removeEventListener('resize', resize)
      window.removeEventListener('pointermove', onPointerMove)
      document.documentElement.removeEventListener('pointerleave', onPointerLeave)
      document.removeEventListener('visibilitychange', handleVisibilityChange)
      if (portRef) portRef.current = null
    }
    // Solo al montar: portRef es un objeto ref estable de App (solo se
    // asigna .current, nunca se reemplaza).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <>
      <canvas
        ref={canvasRef}
        style={{
          position: 'absolute',
          inset: 0,
          zIndex: 1,
          pointerEvents: 'none',
        }}
      />
      <div
        ref={bubbleRef}
        className={`orbit-bubble${hover ? ' visible' : ''}`}
        style={{ '--orbit-color': hover?.color ?? '#ffffff' } as CSSProperties}
        aria-hidden="true"
      >
        <div className="orbit-bubble-title">{hover?.title}</div>
        {hover?.detail && <div className="orbit-bubble-detail">{hover.detail}</div>}
      </div>
    </>
  )
}
