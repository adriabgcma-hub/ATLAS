import { memo } from 'react'
import type { Element, Root } from 'hast'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkBreaks from 'remark-breaks'
import remarkGfm from 'remark-gfm'

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

/** HTML crudo desactivado (react-markdown lo escapa): el LLM no inyecta DOM.
 *  Enlaces en pestaña nueva y tablas con scroll horizontal en el panel. */
const COMPONENTS: Components = {
  a: ({ href, children }) => (
    <a href={href} target="_blank" rel="noreferrer noopener">
      {children}
    </a>
  ),
  table: ({ children }) => (
    <div className="md-table">
      <table>{children}</table>
    </div>
  ),
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
