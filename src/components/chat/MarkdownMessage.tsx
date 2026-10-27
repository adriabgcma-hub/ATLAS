import { memo } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkBreaks from 'remark-breaks'
import remarkGfm from 'remark-gfm'

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
export const MarkdownMessage = memo(function MarkdownMessage({ text }: { text: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm, remarkBreaks]}
      components={COMPONENTS}
    >
      {text}
    </ReactMarkdown>
  )
})
