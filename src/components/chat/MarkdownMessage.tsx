import { memo, useState, type ReactNode } from 'react'
import type { Element, Root } from 'hast'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkBreaks from 'remark-breaks'
import remarkGfm from 'remark-gfm'
import { invoke } from '@tauri-apps/api/core'

const BLOCK_TAGS = new Set([
  'p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote',
  'ul', 'ol', 'li', 'pre', 'table', 'thead', 'tbody', 'tr', 'th', 'td',
])

function rehypeStreamingCursor() {
  return (tree: Root) => {
    let target: Root | Element = tree
    while (true) {
      const last: Root['children'][number] | undefined = target.children.findLast(
        (child) => child.type !== 'text' || child.value.trim() !== '',
      )
      if (last?.type !== 'element') break
      const isCodeBlock = target.type === 'element' && target.tagName === 'pre' && last.tagName === 'code'
      if (!BLOCK_TAGS.has(last.tagName) && !isCodeBlock) break
      target = last
    }

    const last = target.children.at(-1)
    // Markdown añade un salto final al código; el cursor va antes de ese salto.
    if (target.type === 'element' && target.tagName === 'code' && last?.type === 'text') {
      last.value = last.value.replace(/\n$/, '')
    }
    target.children.push({
      type: 'element',
      tagName: 'span',
      properties: { className: ['stream-cursor'], ariaHidden: 'true' },
      children: [],
    })
  }
}

function textOf(node: Element): string {
  return node.children
    .map((child) => {
      if (child.type === 'text') return child.value
      if (child.type === 'element') return textOf(child)
      return ''
    })
    .join('')
}

function tableData(node?: Element): { headers: string[]; rows: string[][] } | null {
  if (!node) return null
  const rows = node.children
    .filter(
      (child): child is Element =>
        child.type === 'element' && (child.tagName === 'thead' || child.tagName === 'tbody'),
    )
    .flatMap((section) =>
      section.children.filter(
        (child): child is Element => child.type === 'element' && child.tagName === 'tr',
      ),
    )
  const cells = rows.map((row) =>
    row.children
      .filter(
        (child): child is Element =>
          child.type === 'element' && (child.tagName === 'th' || child.tagName === 'td'),
      )
      .map(textOf),
  )
  const headers = cells[0]
  if (!headers || headers.length === 0) return null
  return { headers, rows: cells.slice(1) }
}

/** Tabla GFM con acceso a la ventana independiente de Atlas. */
function TableBlock({ node, children }: { node?: Element; children?: ReactNode }) {
  const [error, setError] = useState('')
  const data = tableData(node)
  const canOpen = typeof window !== 'undefined' && '__TAURI__' in window

  async function open() {
    if (!data) return
    setError('')
    try {
      await invoke('open_table_window', {
        payload: { title: 'Tabla', headers: data.headers, rows: data.rows },
      })
    } catch {
      setError('No se pudo abrir la ventana de tabla.')
    }
  }

  return (
    <div className="md-table">
      <div className="md-table-bar">
        <span>Tabla</span>
        {canOpen ? (
          <button type="button" onClick={open}>
            Abrir en ventana
          </button>
        ) : null}
        {error ? <span className="md-table-error">{error}</span> : null}
      </div>
      <table>{children}</table>
    </div>
  )
}

/** HTML crudo desactivado (react-markdown lo escapa): el LLM no inyecta DOM.
 *  Enlaces en pestaña nueva y tablas con scroll horizontal en el panel. */
const COMPONENTS: Components = {
  a: ({ href, children }) => (
    <a href={href} target="_blank" rel="noreferrer noopener">
      {children}
    </a>
  ),
  table: ({ node, children }) => <TableBlock node={node}>{children}</TableBlock>,
}

/** Respuestas de ATLAS con markdown real (GFM: tablas, tachado, tareas). */
export const MarkdownMessage = memo(function MarkdownMessage({
  text,
  streaming = false,
}: {
  text: string
  streaming?: boolean
}) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm, remarkBreaks]}
      rehypePlugins={streaming ? [rehypeStreamingCursor] : []}
      components={COMPONENTS}
    >
      {text}
    </ReactMarkdown>
  )
})
