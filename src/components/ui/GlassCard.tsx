import { type ReactNode } from "react"
import { GlassSurface } from "./GlassSurface"

interface GlassCardProps {
  children: ReactNode
  className?: string
}

export function GlassCard({ children, className = "" }: GlassCardProps) {
  return (
    <GlassSurface className={`p-4 ${className}`}>
      {children}
    </GlassSurface>
  )
}

interface SettingRowProps {
  label: string
  description?: string
  children: ReactNode
}

export function SettingRow({ label, description, children }: SettingRowProps) {
  return (
    <GlassCard>
      <div className="flex items-center justify-between">
        <div>
          <p className="text-[13px] font-medium text-text-primary">{label}</p>
          {description && (
            <p className="text-[11px] text-text-tertiary mt-0.5">{description}</p>
          )}
        </div>
        {children}
      </div>
    </GlassCard>
  )
}
