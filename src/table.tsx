import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { TableWindow } from './components/table/TableWindow'
import './table.css'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <TableWindow />
  </StrictMode>,
)
