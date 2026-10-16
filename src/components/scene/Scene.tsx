import { useRef, useMemo, useEffect, useCallback } from 'react'
import { Canvas, useFrame, useThree } from '@react-three/fiber'
import * as THREE from 'three'
import type { RefObject } from 'react'
import type { OrbState } from '../../state/orbState'

/**
 * Escena WebGL ÚNICA (optimización B): fusiona el grid cibernético y el
 * EnergyCore que antes vivían en DOS contextos GL independientes.
 *
 * Equivalencia visual con el sistema antiguo (parámetros congelados):
 * - Cámara única en z=4 (la que tenía el Orb). El grid estaba a distancia
 *   relativa 7 (cam z=5, plano z=-2) → aquí el plano va a z=-3 para
 *   conservar EXACTAMENTE la misma proyección.
 * - Apilado DOM: este canvas ocupa el hueco del Orb (z-index 2), por
 *   encima de Órbitas (z1). Único delta aceptado: las líneas del grid,
 *   aditivas, ahora brillan sobre las partículas al cruzarse (antes las
 *   tapaban).
 * - El orb es opaco: escribe depth y recorta las líneas que tenga detrás,
 *   igual que hacía su canvas superior.
 *
 * Pilar 4 (2026-08-23): LetterStream añade las letras de energía — al ENVIAR
 * un mensaje, sus caracteres vuelan en cascada pausada desde la barra de
 * input hasta el orb (sprites con la textura del glifo, pool fijo) y cada
 * llegada empuja uPulse del orb. La cadencia la decide App.tsx vía el puerto.
 *
 * Pilar 5 ligero (2026-08-24): StarDust (polvo estelar en 3 capas con
 * parallax por velocidad diferencial) + glow de horizonte en el propio
 * shader del grid. La nebulosa clásica se DESCARTÓ por diseño: competía
 * con las estelas tipo Jarvis.
 */

const vertexShaderGrid = `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`

const fragmentShaderGrid = `
uniform float uTime;
uniform float uHorizonBoost;
uniform float uWaveR;
uniform float uWaveA;
uniform float uBootFade;
uniform float uDim;
uniform vec3 uToneColor;
varying vec2 vUv;

void main() {
  // Centro de la pantalla
  vec2 center = vec2(0.5, 0.5);
  vec2 uv = vUv - center;

  // Distancia al centro
  float dist = length(uv);

  // Grid infinito que se mueve hacia el espectador
  float gridScale = 20.0;
  float speed = 0.3;

  // Coordenadas de la grid con movimiento
  vec2 gridUv = uv * gridScale;
  gridUv.y += uTime * speed;

  // Líneas de la grid (MUY FINAS)
  vec2 grid = abs(fract(gridUv) - 0.5);
  float line = min(grid.x, grid.y);

  // Suavizado de líneas (muy estrechas)
  float lineWidth = 0.015;
  float gridLine = 1.0 - smoothstep(0.0, lineWidth, line);

  // Perspectiva: líneas más brillantes cerca del horizonte
  float perspective = 1.0 - abs(uv.y);
  gridLine *= perspective;

  // Tinte de Atlas: color único y fijo de la interfaz.
  vec3 tintColor = uToneColor;

  // ═══════════════════════════════════════════════════════════
  // SOLO LAS LÍNEAS BRILLAN - FONDO NEGRO
  // ═══════════════════════════════════════════════════════════

  // Color base: NEGRO (transparente) excepto donde hay línea
  vec3 color = vec3(0.0, 0.0, 0.0);

  // Solo donde hay línea, aplicamos el brillo neón
  if (gridLine > 0.01) {
    // Retícula siempre blanca (petición del propietario): el tinte de
    // Atlas vive en el horizonte y la onda expansiva, no en las líneas.
    vec3 lineColor = vec3(0.95, 0.97, 1.0);

    // Capas de energía neón. Multiplicadores moderados: si la suma aditiva
    // satura (>1 por canal), ACES lava cualquier tinte a blanco puro.
    float core   = gridLine * 2.2;
    float neon   = pow(gridLine, 3.0) * 2.8;
    float bloom  = pow(gridLine, 2.0) * 1.8;
    float spread = exp(-line * 40.0) * 1.1;

    // Soft-limit de energía: mantiene el matiz y evita el quemado a blanco.
    float energy = core + neon + bloom + spread;
    energy = energy / (1.0 + energy * 0.45);

    color = lineColor * energy;

    // Núcleo blanco fino solo en el centro exacto de la línea, para no perder luminosidad.
    color += vec3(1.0) * pow(gridLine, 8.0) * 0.5;
  }

  // Fade hacia los bordes (viñeta) - solo afecta a las líneas
  float fade = 1.0 - smoothstep(0.3, 0.8, dist);
  color *= fade;

  // Alpha: completamente transparente donde no hay línea
  float alpha = gridLine * fade * 0.9;

  // ── Glow de horizonte (Pilar 5 ligero) ─────────────────────
  // «Aurora»: modulación lenta por senos superpuestos — la banda respira
  // de forma orgánica sin meter ruido caro en este shader.
  float auroraMod = 0.8 + 0.2 * sin(uTime * 0.35 + sin(uTime * 0.13) * 1.7)
                                 * cos(uTime * 0.11 + vUv.x * 2.4);
  float horizonBand = exp(-abs(vUv.y - 0.58) * 9.0) * auroraMod;
  float horizonEnergy = horizonBand * fade * (0.085 + uHorizonBoost * 0.075);
  color += tintColor * horizonEnergy;
  alpha += horizonBand * fade * (0.05 + uHorizonBoost * 0.05);

  // ── Onda expansiva sobre el suelo ──────────────────────────
  // Anillo tenue que nace bajo el orb cuando se absorben letras
  // (CyberGrid detecta el salto de orbPulseRef). Compensada en Y por la
  // rotación del plano para leerse circular en pantalla.
  float dWave = length((vUv - vec2(0.5, 0.5)) * vec2(1.0, 1.45));
  float ring = exp(-pow((dWave - uWaveR) * 16.0, 2.0)) * uWaveA;
  color += tintColor * ring * fade * 0.3;
  alpha += ring * fade * 0.2;

  // Boot: el grid entero nace del negro.
  // Modo dormido: el suelo se apaga a medias (uDim 0→1).
  float sleepFade = 1.0 - uDim * 0.55;
  gl_FragColor = vec4(color * uBootFade * sleepFade, alpha * uBootFade * sleepFade);
}
`


// Tope de dt por frame compartido por ambos sistemas: un stall del hilo
// principal nunca "teletransporta" la simulación (ver §11.3).
const MAX_DT = 0.05

// ═══════════════════════════════════════════════════════════════
// PILAR 4 — LETRAS DE ENERGÍA
// ═══════════════════════════════════════════════════════════════

// Deben coincidir con la cámara del <Canvas> (única fuente: este fichero).
const CAMERA_Z = 4
const CAMERA_FOV = 50

const MAX_LETTERS = 48
const LETTER_SIZE = 0.22
const ORB_PULSE_PER_LETTER = 0.32
const LETTER_LIFETIME_MIN = 0.8
const LETTER_LIFETIME_MAX = 1.2
// La barra de input vive abajo; el origen DOM se convierte a mundo z=0.
const INPUT_BAR_SELECTOR = '.input-bar'
// Destino de todas las letras: el centro exacto del EnergyCore.
const ORB_TARGET = new THREE.Vector3(0, 0, 0)
// Vector temporal reutilizable en el bucle de frames (single-threaded).
const tmpVec = new THREE.Vector3()

// Atlas es único: colores fijos, sin tonos por agente (idénticos al
// «neutral» anterior: cero cambio visual).
const ATLAS_TITANIUM_RGB: [number, number, number] = [0.949, 0.969, 1.0]
const ATLAS_LETTER_RGB: [number, number, number] = [0.12, 0.55, 1.0]
const ATLAS_DUST_TONE = new THREE.Color(...ATLAS_TITANIUM_RGB)
const ATLAS_LETTER_TONE = new THREE.Color(...ATLAS_LETTER_RGB)

// ── Secuencia de arranque (boot, 2026-08-24) ─────────────────
// Un solo reloj (BootClock) avanza en el primer useFrame del canvas;
// cada capa lee su ventana temporal. Si no hay boot, t arranca en 999
// y todas las ventanas dan factor 1 (coste cero en estado estable).
const BOOT_TOTAL_S = 2.4
const BOOT_START_S = -0.35 // pequeño periodo a negro antes de despertar

/** Reloj compartido del arranque; t en segundos desde el inicio. */
interface BootClock {
  t: number
}

/** Progreso suavizado (smoothstep) de una ventana temporal del boot. */
function bootWindow(t: number, start: number, end: number): number {
  const x = Math.min(1, Math.max(0, (t - start) / (end - start)))
  return x * x * (3 - 2 * x)
}

/** Overshoot elástico para la «implosión» del núcleo (llega con rebote). */
function easeOutBack(x: number): number {
  const c1 = 1.70158
  const c3 = c1 + 1
  const p = x - 1
  return 1 + c3 * p * p * p + c1 * p * p
}

/** Opciones por letra: multiplicadores de tamaño y de vida del vuelo. */
export interface LetterSpawnOpts {
  sizeMul?: number
  lifeMul?: number
}

export interface ScenePort {
  spawnLetter: (char: string, opts?: LetterSpawnOpts) => void
}

interface LetterSlot {
  active: boolean
  t: number
  duration: number
  sizeMul: number
  start: THREE.Vector3
  control: THREE.Vector3
  spin: number
}

interface LetterPool {
  group: THREE.Group
  sprites: THREE.Sprite[]
  slots: LetterSlot[]
}

function makeLetterTexture(char: string): THREE.CanvasTexture {
  const size = 128
  const canvas = document.createElement('canvas')
  canvas.width = size
  canvas.height = size
  const ctx = canvas.getContext('2d')

  if (ctx) {
    ctx.clearRect(0, 0, size, size)
    ctx.font = '700 92px system-ui, -apple-system, sans-serif'
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    // Glow azul eléctrico saturado (nada de blanco pleno: petición del propietario)
    ctx.shadowColor = 'rgba(20, 130, 255, 0.95)'
    ctx.shadowBlur = 24
    ctx.fillStyle = '#3fa4ff'
    ctx.fillText(char, size / 2, size / 2 + 4)
    // Segunda pasada con núcleo CELESTE (no blanco) para legibilidad
    ctx.shadowBlur = 0
    ctx.fillStyle = '#a8dcff'
    ctx.fillText(char, size / 2, size / 2 + 4)
  }

  const texture = new THREE.CanvasTexture(canvas)
  texture.needsUpdate = true
  return texture
}

// Caché de glifos compartida entre montajes (StrictMode la reutiliza).
// Teclado real rara vez supera ~100 símbolos distintos.
const letterTextureCache = new Map<string, THREE.CanvasTexture>()

function getLetterTexture(char: string): THREE.CanvasTexture {
  const cached = letterTextureCache.get(char)
  if (cached) return cached

  if (letterTextureCache.size >= 128) {
    for (const texture of letterTextureCache.values()) texture.dispose()
    letterTextureCache.clear()
  }

  const texture = makeLetterTexture(char)
  letterTextureCache.set(char, texture)
  return texture
}

/** Convierte un punto DOM (px de viewport) a coords mundo en el plano z=0. */
function domPointToWorld(px: number, py: number, width: number, height: number): THREE.Vector3 {
  const ndcX = (px / width) * 2 - 1
  const ndcY = -(py / height) * 2 + 1
  const halfH = Math.tan(THREE.MathUtils.degToRad(CAMERA_FOV / 2)) * CAMERA_Z
  const halfW = halfH * (width / height)
  return new THREE.Vector3(ndcX * halfW, ndcY * halfH, 0)
}

function CyberGrid({
  orbState,
  orbPulseRef,
  bootClockRef,
  sleepRef,
}: {
  orbState: OrbState
  orbPulseRef: RefObject<number>
  bootClockRef: RefObject<BootClock | null>
  sleepRef?: RefObject<{ v: number }>
}) {
  const materialRef = useRef<THREE.ShaderMaterial>(null)
  const simTimeRef = useRef(0)
  const lastFrameRef = useRef(0)
  // Onda expansiva: se dispara al detectar un salto brusco del pulso del
  // orb (letra absorbida). Un único anillo activo; nuevos saltos lo relanzan.
  const waveRef = useRef({ active: false, r: 0 })
  const prevPulseRef = useRef(0)

  const uniforms = useMemo(() => ({
    uTime: { value: 0 },
    uHorizonBoost: { value: 0 },
    uWaveR: { value: 1 },
    uWaveA: { value: 0 },
    // El primer useFrame lo pone al valor correcto (nace a negro con boot).
    uBootFade: { value: 0 },
    uDim: { value: 0 },
    uToneColor: { value: new THREE.Color(...ATLAS_TITANIUM_RGB) },
  }), [])

  useFrame(() => {
    if (!materialRef.current) return
    const now = performance.now()
    const elapsed = lastFrameRef.current > 0 ? (now - lastFrameRef.current) / 1000 : 0
    lastFrameRef.current = now
    const dt = Math.min(elapsed, MAX_DT)
    simTimeRef.current += dt

    const u = materialRef.current.uniforms
    u.uTime.value = simTimeRef.current

    // Boot: el grid emerge del negro durante la primera fase.
    const bootT = bootClockRef.current?.t ?? 999
    u.uBootFade.value = bootWindow(bootT, BOOT_START_S, 1.15)

    // Sueño: apagado suave del suelo.
    u.uDim.value = sleepRef?.current?.v ?? 0

    // El horizonte "respira" cuando Atlas habla/responde (Pilar 5 ligero).
    const boostTarget = orbState === 'responding' || orbState === 'speaking' ? 1 : 0
    u.uHorizonBoost.value += (boostTarget - u.uHorizonBoost.value) * 0.04

    // Onda expansiva: salto de pulso → relanzar anillo desde el centro.
    const pulse = orbPulseRef.current
    const wave = waveRef.current
    if (!wave.active && pulse - prevPulseRef.current > 0.12) {
      wave.active = true
      wave.r = 0.04
    }
    prevPulseRef.current = pulse

    if (wave.active) {
      wave.r += dt * 0.38
      if (wave.r >= 1.05) {
        wave.active = false
        u.uWaveA.value = 0
      } else {
        u.uWaveR.value = wave.r
        u.uWaveA.value = Math.max(0, 1 - wave.r * 1.05)
      }
    }
  })

  return (
    // Suelo hundido 0.65 y en z=-3 (cam z=4): el offset Y evita que las
    // líneas crucen por detrás del orb y de las estelas de Órbitas — el
    // orb «flota» sobre el suelo (ajuste visual pedido 2026-08-24; ya no
    // es la proyección EXACTA del antiguo Background, es deliberada).
    <mesh position={[0, -0.65, -3]} rotation={[-Math.PI / 4, 0, 0]}>
      <planeGeometry args={[20, 20, 1, 1]} />
      <shaderMaterial
        ref={materialRef}
        vertexShader={vertexShaderGrid}
        fragmentShader={fragmentShaderGrid}
        uniforms={uniforms}
        transparent
        depthWrite={false}
        blending={THREE.AdditiveBlending}
      />
    </mesh>
  )
}

// ── Pilar 5 ligero: polvo estelar con parallax ────────────────
// 3 capas de puntos a profundidades distintas; cada capa deriva hacia
// abajo (misma dirección visual que el grid) a velocidad propia — el
// movimiento relativo entre capas lee como profundidad con cámara fija.
const DUST_LAYERS = [
  { count: 120, z: -2.4, speed: 0.01, size: 0.02, opacity: 0.35 },
  { count: 100, z: -1.5, speed: 0.022, size: 0.028, opacity: 0.55 },
  { count: 80, z: -0.7, speed: 0.04, size: 0.036, opacity: 0.8 },
]
const DUST_BOUNDS_X = 3.2
const DUST_BOUNDS_Y = 1.9
const DUST_BASE_COLOR = new THREE.Color(0.62, 0.77, 1.0)
const DUST_TONE_MIX = 0.35

let dustTextureCache: THREE.CanvasTexture | null = null
function getDustTexture(): THREE.CanvasTexture {
  if (!dustTextureCache) {
    const size = 64
    const canvas = document.createElement('canvas')
    canvas.width = size
    canvas.height = size
    const ctx = canvas.getContext('2d')
    if (ctx) {
      const gradient = ctx.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2)
      gradient.addColorStop(0, 'rgba(255,255,255,1)')
      gradient.addColorStop(0.4, 'rgba(210,228,255,0.55)')
      gradient.addColorStop(1, 'rgba(210,228,255,0)')
      ctx.fillStyle = gradient
      ctx.fillRect(0, 0, size, size)
    }
    dustTextureCache = new THREE.CanvasTexture(canvas)
    dustTextureCache.colorSpace = THREE.SRGBColorSpace
  }
  return dustTextureCache
}

interface DustLayer {
  points: THREE.Points
  speed: number
  baseOpacity: number
}

function StarDust({
  orbState,
  bootClockRef,
  sleepRef,
}: {
  orbState: OrbState
  bootClockRef: RefObject<BootClock | null>
  sleepRef?: RefObject<{ v: number }>
}) {
  const { scene } = useThree()
  const layersRef = useRef<DustLayer[] | null>(null)
  // El polvo acelera mientras Atlas piensa/responde/habla (Pilar 5 ligero).
  const boostRef = useRef(1)

  // Pool construido en efecto y añadido imperativamente (regla react-compiler:
  // ninguna instancia GL durante el render). Dispose completo al desmontar.
  useEffect(() => {
    const texture = getDustTexture()
    const layers: DustLayer[] = []

    for (const layer of DUST_LAYERS) {
      const positions = new Float32Array(layer.count * 3)
      for (let i = 0; i < layer.count; i += 1) {
        positions[i * 3] = (Math.random() * 2 - 1) * DUST_BOUNDS_X
        positions[i * 3 + 1] = (Math.random() * 2 - 1) * DUST_BOUNDS_Y
      }
      const geometry = new THREE.BufferGeometry()
      geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3))
      const material = new THREE.PointsMaterial({
        map: texture,
        size: layer.size,
        transparent: true,
        opacity: layer.opacity,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
        color: DUST_BASE_COLOR.clone().lerp(ATLAS_DUST_TONE, DUST_TONE_MIX),
        sizeAttenuation: true,
      })
      const points = new THREE.Points(geometry, material)
      points.position.z = layer.z
      scene.add(points)
      layers.push({ points, speed: layer.speed, baseOpacity: layer.opacity })
    }

    layersRef.current = layers
    return () => {
      for (const layer of layers) {
        scene.remove(layer.points)
        layer.points.geometry.dispose()
        ;(layer.points.material as THREE.PointsMaterial).dispose()
      }
      layersRef.current = null
    }
  }, [scene])

  useFrame((_, delta) => {
    const layers = layersRef.current
    if (!layers) return
    const dt = Math.min(delta, MAX_DT)

    const boostTarget = orbState === 'idle' ? 1 : 1.6
    boostRef.current += (boostTarget - boostRef.current) * 0.02

    // Sueño: el polvo casi se detiene y se atenúa.
    const sleepV = sleepRef?.current?.v ?? 0

    // Bucle indexado: el for...of sobre el array del ref dispara
    // react-hooks/immutability en cuanto hay una asignación directa.
    for (let li = 0; li < layers.length; li += 1) {
      const layer = layers[li]
      const material = layer.points.material as THREE.PointsMaterial

      // Deriva vertical + wrap con re-sorteo de x para que no se note el ciclo.
      // Vía API del BufferAttribute (getX/setY): la escritura por índice
      // directo en arrays alcanzados por refs la prohíbe react-compiler.
      const attr = layer.points.geometry.getAttribute('position') as THREE.BufferAttribute
      const count = attr.count
      const step = layer.speed * boostRef.current * dt * (1 - 0.85 * sleepV)
      for (let i = 0; i < count; i += 1) {
        let y = attr.getY(i) - step
        if (y < -DUST_BOUNDS_Y) {
          y = DUST_BOUNDS_Y
          attr.setX(i, (Math.random() * 2 - 1) * DUST_BOUNDS_X)
        }
        attr.setY(i, y)
      }
      attr.needsUpdate = true

      // Opacidad = arranque × sueño (escritura continua: son 3 capas).
      const bootT = bootClockRef.current?.t ?? 999
      material.opacity =
        layer.baseOpacity * bootWindow(bootT, 0.25, 1.6) * (1 - 0.55 * sleepV)
    }
  })

  return null
}

// Micro-parallax: la cámara oscila levemente siguiendo el puntero (lerp
// suave) — vende la profundidad del polvo en capas. Amplitud mínima para
// no romper el mapeo DOM→mundo de las letras (domPointToWorld).
const PARALLAX_MAX_X = 0.08
const PARALLAX_MAX_Y = 0.05

/** prefers-reduced-motion: el SO pide menos movimiento → sin fugaces ni
 *  parallax. Se lee UNA vez al cargar módulo (el ajuste no cambia en caliente). */
const REDUCED_MOTION =
  typeof window !== 'undefined' &&
  window.matchMedia('(prefers-reduced-motion: reduce)').matches

function CameraRig({
  bootClockRef,
  sleepRef,
}: {
  bootClockRef: RefObject<BootClock | null>
  sleepRef?: RefObject<{ v: number }>
}) {
  // Mutar el objeto `camera` del hook directo lo prohíbe react-compiler
  // (valor de scope de render); se accede vía ref, patrón poolRef.
  const camera = useThree((state) => state.camera)
  const cameraRef = useRef<THREE.Camera | null>(null)
  const targetRef = useRef({ x: 0, y: 0 })

  useEffect(() => {
    cameraRef.current = camera
    const onMove = (event: PointerEvent) => {
      targetRef.current.x = (event.clientX / window.innerWidth) * 2 - 1
      targetRef.current.y = (event.clientY / window.innerHeight) * 2 - 1
    }
    window.addEventListener('pointermove', onMove)
    return () => window.removeEventListener('pointermove', onMove)
  }, [camera])

  useFrame(({ clock }) => {
    const cam = cameraRef.current
    if (!cam) return

    // Boot: la cámara despierta MÁS CERCA y algo baja, y retrocede hasta
    // su posición final mientras el grid emerge.
    const bootT = bootClockRef.current?.t ?? 999
    const wake = bootWindow(bootT, BOOT_START_S, 1.35)
    const baseZ = 3.15 + (CAMERA_Z - 3.15) * wake
    const baseY = -0.18 * (1 - wake)

  const tx = targetRef.current.x * PARALLAX_MAX_X * (REDUCED_MOTION ? 0 : 1)
  const ty = -targetRef.current.y * PARALLAX_MAX_Y * (REDUCED_MOTION ? 0 : 1)

    // Sueño: deriva onírica — la cámara flota en un Lissajous lentísimo.
    const sleepV = sleepRef?.current?.v ?? 0
    const driftX = Math.sin(clock.elapsedTime * 0.07) * 0.07 * sleepV
    const driftY = Math.cos(clock.elapsedTime * 0.05) * 0.05 * sleepV

    cam.position.x += (tx + driftX - cam.position.x) * 0.03
    cam.position.y += (ty + driftY + baseY - cam.position.y) * 0.03
    cam.position.z += (baseZ - cam.position.z) * 0.06
    cam.lookAt(0, 0, 0)
  })

  return null
}

// ── Pilar 5 ligero II: estrella fugaz ocasional ──────────────
// Racha diagonal rápida en idle. Ráfagas de 1-3 estrellas con hueco
// aleatorio de 15-25 s (petición del propietario: «varias de forma
// random sin pasarse»).
const SHOOTING_STAR_POOL = 4
const SHOOTING_STAR_MIN_GAP_S = 15
const SHOOTING_STAR_MAX_GAP_S = 25
const SHOOTING_STAR_LIFE_S = 0.7
const SHOOTING_STAR_SPEED = 6

let streakTextureCache: THREE.CanvasTexture | null = null
function getStreakTexture(): THREE.CanvasTexture {
  if (!streakTextureCache) {
    const w = 128
    const h = 16
    const canvas = document.createElement('canvas')
    canvas.width = w
    canvas.height = h
    const ctx = canvas.getContext('2d')
    if (ctx) {
      const gradient = ctx.createLinearGradient(0, 0, w, 0)
      gradient.addColorStop(0, 'rgba(200,225,255,0)')
      gradient.addColorStop(0.75, 'rgba(210,232,255,0.85)')
      gradient.addColorStop(1, 'rgba(255,255,255,1)')
      ctx.fillStyle = gradient
      ctx.fillRect(0, 0, w, h)
      // Perfil vertical suave (destino-in con gradiente alfa).
      const vGrad = ctx.createLinearGradient(0, 0, 0, h)
      vGrad.addColorStop(0, 'rgba(0,0,0,0)')
      vGrad.addColorStop(0.5, 'rgba(0,0,0,1)')
      vGrad.addColorStop(1, 'rgba(0,0,0,0)')
      ctx.globalCompositeOperation = 'destination-in'
      ctx.fillStyle = vGrad
      ctx.fillRect(0, 0, w, h)
      ctx.globalCompositeOperation = 'source-over'
    }
    streakTextureCache = new THREE.CanvasTexture(canvas)
    streakTextureCache.colorSpace = THREE.SRGBColorSpace
  }
  return streakTextureCache
}

interface ShootingStar {
  mesh: THREE.Mesh
  material: THREE.MeshBasicMaterial
  // t < 0: retardo antes de aparecer; t ∈ [0,1]: vuelo; t ≥ 1: inactiva.
  t: number
  dirX: number
  dirY: number
}

function ShootingStars({
  orbState,
  bootClockRef,
  sleepRef,
}: {
  orbState: OrbState
  bootClockRef: RefObject<BootClock | null>
  sleepRef?: RefObject<{ v: number }>
}) {
  const { scene } = useThree()
  const poolRef = useRef<ShootingStar[] | null>(null)
  // La primera llega pronto pero sin ser instantánea.
  const nextInRef = useRef(6)

  useEffect(() => {
    const texture = getStreakTexture()
    const geometry = new THREE.PlaneGeometry(1, 0.03)
    const pool: ShootingStar[] = []

    for (let i = 0; i < SHOOTING_STAR_POOL; i += 1) {
      const material = new THREE.MeshBasicMaterial({
        map: texture,
        transparent: true,
        opacity: 0,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      })
      const mesh = new THREE.Mesh(geometry, material)
      mesh.visible = false
      mesh.position.z = -1.2 - Math.random() * 0.8
      scene.add(mesh)
      pool.push({ mesh, material, t: 1, dirX: 0, dirY: 0 })
    }

    poolRef.current = pool
    return () => {
      for (const star of pool) {
        scene.remove(star.mesh)
        star.material.dispose()
      }
      geometry.dispose()
      poolRef.current = null
    }
  }, [scene])

  useFrame((_, delta) => {
    const pool = poolRef.current
    if (!pool) return
    const dt = Math.min(delta, MAX_DT)

    // Bucle indexado como LetterStream: el for...of sobre el array del ref
    // dispara react-hooks/immutability.
    for (let i = 0; i < pool.length; i += 1) {
      const star = pool[i]
      if (star.t >= 1) continue
      if (star.t < 0) {
        // Retardo de ráfaga: aún invisible.
        star.t += dt / SHOOTING_STAR_LIFE_S
        if (star.t < 0) continue
      }
      star.t += dt / SHOOTING_STAR_LIFE_S
      if (star.t >= 1) {
        star.mesh.visible = false
        continue
      }
      star.mesh.position.x += star.dirX * SHOOTING_STAR_SPEED * dt
      star.mesh.position.y += star.dirY * SHOOTING_STAR_SPEED * dt
      star.material.opacity = Math.sin(star.t * Math.PI) * 0.8
    }

    // Solo se programan nuevas en idle Y con el boot terminado. Con
    // prefers-reduced-motion no se programan NUNCA; dormido tampoco.
    if (REDUCED_MOTION) return
    if ((sleepRef?.current?.v ?? 0) > 0.3) return
    if (orbState !== 'idle') return
    const bootT = bootClockRef.current?.t ?? 999
    if (bootT < BOOT_TOTAL_S) return
    nextInRef.current -= dt
    if (nextInRef.current > 0) return
    nextInRef.current =
      SHOOTING_STAR_MIN_GAP_S + Math.random() * (SHOOTING_STAR_MAX_GAP_S - SHOOTING_STAR_MIN_GAP_S)

    // Ráfaga: 1 casi siempre, a veces 2, raramente 3.
    let wanted = 1
    if (Math.random() < 0.45) wanted += 1
    if (Math.random() < 0.18) wanted += 1

    const free = pool.filter((star) => star.t >= 1)
    for (let i = 0; i < Math.min(wanted, free.length); i += 1) {
      const star = free[i]
      const side = Math.random() < 0.5 ? -1 : 1
      const angle = (-18 - Math.random() * 14) * (Math.PI / 180)
      star.dirX = Math.cos(angle) * side
      star.dirY = Math.sin(angle)
      star.t = -i * 0.12
      star.mesh.scale.set(1.3 + Math.random() * 0.8, 1, 1)
      star.mesh.rotation.z = Math.atan2(star.dirY, star.dirX)
      star.mesh.position.set(
        -side * (2.6 + Math.random() * 0.6),
        0.9 + Math.random() * 0.7,
        star.mesh.position.z,
      )
      star.mesh.visible = true
    }
  })

  return null
}

interface OrbParams {
  intensity: number
  error: number
  speakingPulse: number
}

function orbParamsForState(state: OrbState, t: number): OrbParams {
  switch (state) {
    case 'thinking':
      return { intensity: 0.75, error: 0, speakingPulse: 0 }
    case 'responding':
      return { intensity: 1.0, error: 0, speakingPulse: 0 }
    case 'speaking':
      return {
        intensity: 0.95,
        error: 0,
        speakingPulse: 0.5 + 0.5 * Math.sin(t * 4.0),
      }
    case 'error':
      return { intensity: 0.35, error: 1.0, speakingPulse: 0 }
    case 'idle':
    default:
      return { intensity: 0.55, error: 0, speakingPulse: 0 }
  }
}

function EnergyCore({
  state,
  orbPulseRef,
  bootClockRef,
  sleepRef,
  levelRef,
}: {
  state: OrbState
  orbPulseRef: RefObject<number>
  bootClockRef: RefObject<BootClock | null>
  sleepRef?: RefObject<{ v: number }>
  levelRef?: RefObject<number>
}) {
  const groupRef = useRef<THREE.Group>(null)
  const wireRef = useRef<THREE.Mesh>(null)
  const wireMatRef = useRef<THREE.MeshBasicMaterial>(null)
  const shellRef = useRef<THREE.Mesh>(null)
  const shellMatRef = useRef<THREE.MeshBasicMaterial>(null)
  const coreRef = useRef<THREE.Mesh>(null)
  const coreMatRef = useRef<THREE.MeshBasicMaterial>(null)
  const simTimeRef = useRef(0)
  const lastFrameRef = useRef(0)
  const errRef = useRef(0)

  const cyan = useMemo(() => new THREE.Color(0.30, 0.80, 1.00), [])
  const red = useMemo(() => new THREE.Color(1.00, 0.16, 0.12), [])
  // Seguidores rápido/lento de la envolvente: el movimiento sale de su
  // diferencia (ripple) para no quedarse clavado en máximo durante vocales.
  // El pre-suavizado quita el escalonado del muestreo (sensación de lag).
  const preRef = useRef(0)
  const fastRef = useRef(0)
  const slowRef = useRef(0)
  // Intensidad del estado suavizada: arranca en idle (0.55) para que el
  // primer frame no dé un escalón de tamaño.
  const intensityRef = useRef(0.55)

  useFrame(() => {
    const now = performance.now()
    const elapsed = lastFrameRef.current > 0 ? (now - lastFrameRef.current) / 1000 : 0
    lastFrameRef.current = now
    const dt = Math.min(elapsed, MAX_DT)
    simTimeRef.current += dt
    const t = simTimeRef.current
    const params = orbParamsForState(state, t)
    // Sueño: el núcleo baja de intensidad y respira LENTO Y HONDO —
    // como un durmiente (ciclo ~7 s, mucho más lento que la vigilia).
    const sleepV = sleepRef?.current?.v ?? 0
    const breathSleep = Math.sin(t * 0.9) * 0.5 + 0.5

    // Único escritor del pulso: LetterStream lo sube al absorber una letra;
    // aquí decae exponencialmente (~0.5 s de vida).
    orbPulseRef.current *= Math.exp(-dt * 6.5)
    const pulse = orbPulseRef.current
    errRef.current += (params.error - errRef.current) * 0.15

    // El gesto visual usa el ripple de la escena.
    // entre un seguidor rápido (sílabas) y uno lento (volumen) — en vocal
    // sostenida vuelve a reposo en vez de quedarse arriba. Solo tamaño,
    // sin deformación.
    const rawLevel = levelRef?.current ?? 0
    const pre = preRef.current
    preRef.current = pre + (rawLevel - pre) * (rawLevel > pre ? 0.85 : 0.7)
    const input = preRef.current
    const fast = fastRef.current
    fastRef.current = fast + (input - fast) * (input > fast ? 0.8 : 0.6)
    const slow = slowRef.current
    slowRef.current = slow + (input - slow) * (input > slow ? 0.25 : 0.15)
    // Intensidad suavizada: el estado salta (idle → responding) pero el
    // tamaño persigue su valor con un follower, así que la transición se ve
    // como un crescendo en vez de un escalón. Se suaviza la TRANSICIÓN, no
    // la curva: un easing sobre el valor comprimiría la zona 0.75-1.0 donde
    // viven los estados activos y los volvería indistinguibles.
    const prevIntensity = intensityRef.current
    intensityRef.current = prevIntensity + (params.intensity - prevIntensity) * Math.min(1, dt * 6)
    const ripple = Math.max(-0.06, Math.min(0.06, (fastRef.current - slowRef.current) * 0.8))
    const vol = Math.min(1, slowRef.current * 1.3)
    const grow = 1 + (intensityRef.current - 0.55) * 0.12 + vol * 0.025 + pulse * 0.05
    if (wireRef.current) {
      wireRef.current.rotation.y += (0.7 + pulse * 1.5) * dt
      wireRef.current.rotation.x = 0.35 + Math.sin(t * 0.2) * 0.06
      wireRef.current.scale.setScalar(grow * (1 + ripple * 0.5))
    }
    if (shellRef.current) {
      shellRef.current.rotation.y -= (0.35 + pulse * 0.8) * dt
      shellRef.current.rotation.x = -0.3 + Math.cos(t * 0.16) * 0.06
      shellRef.current.scale.setScalar(grow * (1 + ripple * 0.4))
    }
    // Plasma ligado a la voz: brillo casi plano en reposo, vivo al hablar.
    const plasma = 0.94 + (0.05 + vol * 0.10) * (0.6 * Math.sin(t * 9.1) + 0.4 * Math.sin(t * 23.7 + 1.3))
    const heat = (0.9 + params.intensity * 0.5 + pulse * 0.9) * plasma
    if (wireMatRef.current) {
      wireMatRef.current.color.copy(cyan).lerp(red, errRef.current).multiplyScalar(heat)
      const target = (0.42 + params.intensity * 0.4 + pulse * 0.5) * (1 - 0.45 * sleepV)
      wireMatRef.current.opacity += (Math.min(1, target) - wireMatRef.current.opacity) * 0.12
    }
    if (shellMatRef.current) {
      shellMatRef.current.color.copy(cyan).lerp(red, errRef.current).multiplyScalar(heat * 0.8)
      const target = (0.22 + params.intensity * 0.25 + pulse * 0.35) * (1 - 0.45 * sleepV)
      shellMatRef.current.opacity += (Math.min(1, target) - shellMatRef.current.opacity) * 0.12
    }
    if (coreMatRef.current) {
      coreMatRef.current.color.copy(cyan).lerp(red, errRef.current)
      const target = (0.75 + params.speakingPulse * 0.25 + pulse * 0.5) * (1 - 0.4 * sleepV)
      coreMatRef.current.opacity += (Math.min(1, target) - coreMatRef.current.opacity) * 0.2
    }
    if (coreRef.current) {
      const s = grow * (1 + ripple * 0.35 + pulse * 0.12)
      coreRef.current.scale.setScalar(s)
    }

    // Boot — implosión inversa del núcleo: nace colapsado (casi punto) y
    // «explota» hacia su tamaño con overshoot elástico. Escala el GRUPO
    // (núcleo + anillos) para que nazcan juntos.
    if (groupRef.current) {
      const bootT = bootClockRef.current?.t ?? 999
      const born = bootWindow(bootT, 0.55, 1.75)
      // En sueño el pecho sube y baja visiblemente con la respiración.
      const scale =
        (0.02 + 0.98 * easeOutBack(born)) *
        (1 - 0.08 * sleepV) *
        (1 + breathSleep * 0.06 * sleepV)
      groupRef.current.scale.setScalar(scale)
    }
  })

  return (
    <group ref={groupRef}>
      {/* Poliedro de alambre: el centro es geometría nítida, no un blob. */}
      <mesh ref={wireRef}>
        <icosahedronGeometry args={[0.55, 2]} />
        <meshBasicMaterial
          ref={wireMatRef}
          wireframe
          color={[0.3, 0.8, 1]}
          transparent
          opacity={0.6}
          depthWrite={false}
          blending={THREE.AdditiveBlending}
        />
      </mesh>
      {/* Jaula exterior gruesa, contrarrotante y tenue: volumen de plasma. */}
      <mesh ref={shellRef}>
        <icosahedronGeometry args={[0.62, 1]} />
        <meshBasicMaterial
          ref={shellMatRef}
          wireframe
          color={[0.3, 0.8, 1]}
          transparent
          opacity={0.3}
          depthWrite={false}
          blending={THREE.AdditiveBlending}
        />
      </mesh>
      {/* Punto reactor: brilla al hablar y con cada letra absorbida. */}
      <mesh ref={coreRef}>
        <sphereGeometry args={[0.10, 32, 32]} />
        <meshBasicMaterial
          ref={coreMatRef}
          color={[0.55, 0.9, 1]}
          transparent
          opacity={0.9}
          depthWrite={false}
          blending={THREE.AdditiveBlending}
        />
      </mesh>
    </group>
  )
}

// Azul eléctrico de las letras de Atlas (fijo, sin tonos).
function LetterStream({
  portRef,
  orbPulseRef,
}: {
  portRef: RefObject<ScenePort | null>
  orbPulseRef: RefObject<number>
}) {
  const { camera, size, scene } = useThree()
  const viewRef = useRef({ camera, size })
  const toneColorRef = useRef(ATLAS_LETTER_TONE.clone())
  const poolRef = useRef<LetterPool | null>(null)

  // El pool se construye en efecto y se añade al scene imperativamente:
  // nada de instancias GL creadas durante el render (regla del compilador).
  useEffect(() => {
    const group = new THREE.Group()
    const sprites: THREE.Sprite[] = []
    const slots: LetterSlot[] = []

    for (let i = 0; i < MAX_LETTERS; i++) {
      const material = new THREE.SpriteMaterial({
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
        opacity: 0,
      })
      const sprite = new THREE.Sprite(material)
      sprite.visible = false
      sprite.scale.setScalar(LETTER_SIZE)
      group.add(sprite)
      sprites.push(sprite)
      slots.push({
        active: false,
        t: 0,
        duration: 1,
        sizeMul: 1,
        start: new THREE.Vector3(),
        control: new THREE.Vector3(),
        spin: 0,
      })
    }

    poolRef.current = { group, sprites, slots }
    scene.add(group)

    return () => {
      scene.remove(group)
      for (const sprite of sprites) sprite.material.dispose()
      poolRef.current = null
    }
  }, [scene])

  const spawnLetter = useCallback((char: string, opts?: LetterSpawnOpts) => {
    if (!char.trim()) return

    const pool = poolRef.current
    if (!pool) return

    // Sin slot libre se roba el más avanzado (el que antes llegará al orb).
    const { slots } = pool
    let slotIndex = slots.findIndex((slot) => !slot.active)
    if (slotIndex === -1) {
      slotIndex = 0
      for (let i = 1; i < slots.length; i++) {
        if (slots[i].t > slots[slotIndex].t) slotIndex = i
      }
    }

    // Origen: barra de input en coords DOM → mundo z=0, con jitter.
    const rect = document.querySelector(INPUT_BAR_SELECTOR)?.getBoundingClientRect()
    const view = viewRef.current
    let px: number
    let py: number
    if (rect && rect.width > 0) {
      px = rect.left + rect.width * (0.15 + Math.random() * 0.7)
      py = rect.top + Math.random() * 12
    } else {
      px = view.size.width * (0.55 + Math.random() * 0.35)
      py = view.size.height * 0.9
    }

    const slot = slots[slotIndex]
    slot.start.copy(domPointToWorld(px, py, view.size.width, view.size.height))
    slot.control.copy(slot.start).lerp(ORB_TARGET, 0.45)
    slot.control.x += (Math.random() - 0.5) * 0.9
    slot.control.y += (Math.random() - 0.5) * 0.4
    slot.t = 0
    slot.sizeMul = opts?.sizeMul ?? 1
    slot.duration =
      (LETTER_LIFETIME_MIN + Math.random() * (LETTER_LIFETIME_MAX - LETTER_LIFETIME_MIN)) *
      (opts?.lifeMul ?? 1)
    slot.spin = (Math.random() - 0.5) * 2.5
    slot.active = true

    const sprite = pool.sprites[slotIndex]
    sprite.material.map = getLetterTexture(char)
    sprite.material.color.copy(toneColorRef.current)
    sprite.material.needsUpdate = true
    sprite.visible = true
  }, [])

  useEffect(() => {
    portRef.current = { spawnLetter }
  }, [portRef, spawnLetter])

  useFrame((_, delta) => {
    viewRef.current.camera = camera
    viewRef.current.size = size

    const pool = poolRef.current
    if (!pool) return

    const dt = Math.min(delta, MAX_DT)

    for (let i = 0; i < MAX_LETTERS; i++) {
      const slot = pool.slots[i]
      const sprite = pool.sprites[i]
      if (!slot.active) continue

      slot.t += dt / slot.duration

      if (slot.t >= 1) {
        slot.active = false
        sprite.visible = false
        orbPulseRef.current = Math.min(1, orbPulseRef.current + ORB_PULSE_PER_LETTER)
        continue
      }

      // Bézier cuadrática start→control→orb con easing suave.
      const e = slot.t * slot.t * (3 - 2 * slot.t)
      tmpVec.copy(slot.start).multiplyScalar((1 - e) * (1 - e))
      tmpVec.addScaledVector(slot.control, 2 * (1 - e) * e)
      tmpVec.addScaledVector(ORB_TARGET, e * e)
      sprite.position.copy(tmpVec)

      const fadeIn = Math.min(1, slot.t / 0.12)
      const fadeOut = 1 - Math.max(0, (slot.t - 0.78) / 0.22)
      sprite.material.opacity = fadeIn * fadeOut * 0.95
      sprite.scale.setScalar(LETTER_SIZE * slot.sizeMul * (0.65 + 0.35 * fadeIn) * (1 - 0.55 * e))
      sprite.material.rotation += slot.spin * dt
    }
  })

  return null
}

// Driver del boot: DEBE montarse primero dentro del <Canvas> para que su
// useFrame avance el reloj antes de que las capas lo lean este frame.
// Además dispara los hitos: pulso del núcleo al terminar la implosión.
const BOOT_CORE_BURST_T = 1.55

/** Modo dormido (#6): único escritor del factor de sueño compartido.
 *  Transición suave de ~2 s hacia el objetivo (0 despierto, 1 dormido). */
function SleepDriver({
  asleep,
  sleepRef,
}: {
  asleep: boolean
  sleepRef: RefObject<{ v: number }>
}) {
  useFrame((_, delta) => {
    const sleep = sleepRef.current
    if (!sleep) return
    const target = asleep ? 1 : 0
    if (sleep.v === target) return
    const dt = Math.min(delta, MAX_DT)
    sleep.v += (target - sleep.v) * Math.min(1, dt * 1.4)
  })
  return null
}

function BootDriver({
  bootClockRef,
  orbPulseRef,
}: {
  bootClockRef: RefObject<BootClock | null>
  orbPulseRef: RefObject<number>
}) {
  const firedRef = useRef({ burst: false })

  useFrame((_, delta) => {
    const clock = bootClockRef.current
    if (!clock || clock.t >= BOOT_TOTAL_S) return
    clock.t = Math.min(BOOT_TOTAL_S, clock.t + Math.min(delta, MAX_DT))

    const fired = firedRef.current
    if (!fired.burst && clock.t >= BOOT_CORE_BURST_T) {
      fired.burst = true
      orbPulseRef.current = Math.min(1, orbPulseRef.current + 0.9)
    }
  })

  return null
}

export function AtlasScene({
  orbState = 'idle',
  visible = true,
  portRef,
  boot = false,
  asleep = false,
  levelRef,
}: {
  orbState?: OrbState
  visible?: boolean
  portRef?: RefObject<ScenePort | null>
  boot?: boolean
  asleep?: boolean
  /** Nivel de voz 0..1 (lo bombea App sin re-renders): mueve el orbe. */
  levelRef?: RefObject<number>
}) {
  const orbPulseRef = useRef(0)
  // Reloj de arranque compartido por todas las capas del canvas.
  const bootClockRef = useRef<BootClock | null>(boot ? { t: BOOT_START_S } : { t: 999 })
  // Factor de sueño compartido (0 despierto → 1 dormido), avanzado por
  // SleepDriver y leído por TODAS las capas para modular su comportamiento.
  const sleepRef = useRef({ v: 0 })
  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        // Hueco del antiguo canvas del Orb: por encima de Órbitas (z1),
        // debajo de chat/floating-controls.
        zIndex: 2,
        pointerEvents: 'none',
      }}
    >
      <Canvas
        camera={{ position: [0, 0, CAMERA_Z], fov: CAMERA_FOV }}
        gl={{ alpha: true, antialias: true }}
        dpr={[1, 2]}
        frameloop={visible ? 'always' : 'never'}
      >
        <SleepDriver asleep={asleep} sleepRef={sleepRef} />
        <BootDriver bootClockRef={bootClockRef} orbPulseRef={orbPulseRef} />
        <CyberGrid orbState={orbState} orbPulseRef={orbPulseRef} bootClockRef={bootClockRef} sleepRef={sleepRef} />
        <CameraRig bootClockRef={bootClockRef} sleepRef={sleepRef} />
        <StarDust orbState={orbState} bootClockRef={bootClockRef} sleepRef={sleepRef} />
        <ShootingStars orbState={orbState} bootClockRef={bootClockRef} sleepRef={sleepRef} />
        <EnergyCore state={orbState} orbPulseRef={orbPulseRef} bootClockRef={bootClockRef} sleepRef={sleepRef} levelRef={levelRef} />
        {portRef && (
          <LetterStream
            portRef={portRef}
            orbPulseRef={orbPulseRef}
          />
        )}
      </Canvas>
    </div>
  )
}
