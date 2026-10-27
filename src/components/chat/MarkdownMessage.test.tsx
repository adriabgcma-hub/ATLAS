import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { MarkdownMessage } from './MarkdownMessage'

const cursor = '<span class="stream-cursor" aria-hidden="true"></span>'

function render(text: string, streaming = true) {
  return renderToStaticMarkup(<MarkdownMessage text={text} streaming={streaming} />)
}

describe('cursor de Markdown', () => {
  it.each([
    ['párrafo', 'Hola mundo', `Hola mundo${cursor}</p>`],
    ['varios párrafos', 'Primero\n\nÚltimo', `Último${cursor}</p>`],
    ['título', '## Título', `Título${cursor}</h2>`],
    ['lista', '- Uno\n- Dos', `Dos${cursor}</li>`],
    ['lista anidada', '- Uno\n  - Dos', `Dos${cursor}</li>`],
    ['cita', '> Cita', `Cita${cursor}</p>`],
    ['negrita', '**Texto**', `<strong>Texto</strong>${cursor}</p>`],
    ['código inline', '`valor`', `<code>valor</code>${cursor}</p>`],
    ['código', '```js\nconst n = 1\n```', `const n = 1${cursor}</code>`],
    ['código incompleto', '```js\nconst n', `const n${cursor}</code>`],
    ['tabla', '| A | B |\n| - | - |\n| Uno | Dos |', `Dos${cursor}</td>`],
    ['salto de línea', 'Uno\nDos', `<br/>\nDos${cursor}</p>`],
    ['lista de tareas', '- [ ] Revisar', `Revisar${cursor}</li>`],
  ])('coloca un único cursor al final de %s', (_name, text, ending) => {
    const html = render(text)
    expect(html).toContain(ending)
    expect(html.split(cursor)).toHaveLength(2)
  })

  it('muestra el cursor antes del primer carácter', () => {
    expect(render('')).toBe(cursor)
  })

  it('no muestra cursor al terminar y conserva el salto final del código', () => {
    const html = render('```js\nconst n = 1\n```', false)
    expect(html).not.toContain('stream-cursor')
    expect(html).toContain('const n = 1\n</code>')
  })

  it('mantiene el HTML del mensaje escapado durante el revelado', () => {
    const html = render('<script>alert(1)</script>')
    expect(html).not.toContain('<script>')
    expect(html).toContain('&lt;script&gt;')
  })
})
