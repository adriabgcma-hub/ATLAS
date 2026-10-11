import { useRef } from 'react'
import { Orbits } from './components/orb/Orbits'
import { AtlasScene } from './components/scene/Scene'
import './App.css'

export default function App() {
  const levelRef = useRef(0)

  return (
    <main className="app-shell">
      <Orbits asleep={false} />
      <AtlasScene agentTone="neutral" orbState="idle" levelRef={levelRef} />
      <div className="app-label" aria-label="Atlas">ATLAS</div>
    </main>
  )
}
