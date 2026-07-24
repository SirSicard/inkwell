import { type ReactNode } from "react"

interface GlassSurfaceProps {
  children: ReactNode
  className?: string
}

export function GlassSurface({ children, className = "" }: GlassSurfaceProps) {
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
