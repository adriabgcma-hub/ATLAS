export const dotClass = (active?: string | null, load?: string | null): string => {
  if (load === 'masked' || active === 'failed') return 'err'
  if (active === 'activating') return 'warn'
  if (active === 'active') return 'ok'
  return 'dim'
}

export const unitLabel = (u: string) => u.replace(/\.service$/, '')
