import { type ReactNode } from "react"

interface InkSurfaceProps {
  children: ReactNode
  className?: string
}

export function InkSurface({ children, className = "" }: InkSurfaceProps) {
  return (
    <div
      className={`
        rounded-lg
        bg-bg-surface
        border border-border
        ${className}
      `}
    >
      {children}
    </div>
  )
}
