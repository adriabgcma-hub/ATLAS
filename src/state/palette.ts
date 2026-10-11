import type { OrbTone } from '../types/visual'

export interface TonePalette {
  label: string
  hex: string
  /** RGB lineal 0–1, exactamente derivado del hex. */
  rgb: [number, number, number]
}

/**
 * ÚNICA fuente de verdad de los colores por agente.
 * La escena WebGL (Scene.tsx) lee de aquí vía el uniforme uToneColor;
 * cualquier UI futura que necesite el color de un agente también.
 * No hardcodear RGB en shaders ni componentes.
 */
export const ORB_PALETTE: Record<OrbTone, TonePalette> = {
  neutral: { label: 'ATLAS · titanio', hex: '#F2F7FF', rgb: [0.949, 0.969, 1.0] },
  gold: { label: 'GENIUS', hex: '#FFC953', rgb: [1.0, 0.788, 0.325] },
  teal: { label: 'JANUS', hex: '#14B8A6', rgb: [0.078, 0.722, 0.651] },
  green: { label: 'ARGOS', hex: '#1AF266', rgb: [0.102, 0.949, 0.4] },
  cyan: { label: 'HERMES', hex: '#00D9FF', rgb: [0.0, 0.851, 1.0] },
  ember: { label: 'THOTH', hex: '#C96F3A', rgb: [0.788, 0.435, 0.227] },
  lava: { label: 'VULCAN', hex: '#FF4D00', rgb: [1.0, 0.302, 0.0] },
  crimson: { label: 'CERBERUS', hex: '#E11D48', rgb: [0.882, 0.114, 0.282] },
  rose: { label: 'APOLO', hex: '#FF6EC7', rgb: [1.0, 0.431, 0.78] },
  orange: { label: 'KRONOS', hex: '#FF7300', rgb: [1.0, 0.451, 0.0] },
  azure: { label: 'IRIS', hex: '#4FC3F7', rgb: [0.31, 0.765, 0.969] },
  lilac: { label: 'PLUTUS', hex: '#A78BFA', rgb: [0.655, 0.545, 0.98] },
  violet: { label: 'ORION', hex: '#991AFF', rgb: [0.6, 0.102, 1.0] },
  coral: { label: 'PROTEUS', hex: '#FF6B6B', rgb: [1.0, 0.42, 0.42] },
}
