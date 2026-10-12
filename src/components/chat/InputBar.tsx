import { useEffect, useRef, useState, type FormEvent } from 'react'

/**
 * Barra de transmisión a Atlas (Fase 2).
 *
 * - Lleva la clase `.input-bar`: es el origen DOM que LetterStream (Scene.tsx)
 *   convierte a mundo para las letras de energía al enviar.
 * - No decide nada sobre sesiones ni IA: recoge texto y lo entrega por
 *   `onSubmit`. Con una petición en curso (`pending`), el botón pasa a
 *   cancelar (`onCancel`). La cadencia de letras la decide App.
 */
interface InputBarProps {
  onSubmit: (content: string) => void
  onCancel: () => void
  pending: boolean
  placeholder?: string
}

export function InputBar({
  onSubmit,
  onCancel,
  pending,
  placeholder = 'Transmite a Atlas…',
}: InputBarProps) {
  const [value, setValue] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)

  const canSubmit = value.trim().length > 0

  // La ventana es una escena sin más foco: al montar, teclear funciona directo.
  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) {
      onCancel()
      return
    }
    const content = value.trim()
    if (!content) return
    onSubmit(content)
    setValue('')
    inputRef.current?.focus()
  }

  return (
    <form className="chat-input-shell" onSubmit={handleSubmit}>
      <input
        ref={inputRef}
        className="input-bar"
        type="text"
        value={value}
        onChange={(event) => setValue(event.target.value)}
        placeholder={pending ? 'Petición en curso — Enter para cancelar' : placeholder}
        enterKeyHint="send"
        autoComplete="off"
        spellCheck={false}
        aria-label="Escribe un mensaje para Atlas"
      />
      <button
        type="submit"
        className="chat-send-btn"
        disabled={!canSubmit && !pending}
        aria-label={pending ? 'Cancelar petición' : 'Enviar mensaje'}
      >
        {pending ? (
          <svg
            viewBox="0 0 24 24"
            width="14"
            height="14"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            aria-hidden="true"
          >
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        ) : (
          <svg
            viewBox="0 0 24 24"
            width="15"
            height="15"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M22 2 11 13" />
            <path d="M22 2 15 22l-4-9-9-4z" />
          </svg>
        )}
      </button>
    </form>
  )
}
