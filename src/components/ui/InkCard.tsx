import { type ReactNode } from "react"
import { InkSurface } from "./InkSurface"

interface InkCardProps {
  children: ReactNode
  className?: string
}

export function InkCard({ children, className = "" }: InkCardProps) {
  return (
    <InkSurface className={`p-4 ${className}`}>
      {children}
    </InkSurface>
  )
}

interface SettingRowProps {
  label: string
  description?: string
  children: ReactNode
}

export function SettingRow({ label, description, children }: SettingRowProps) {
  return (
    <InkCard>
      <div className="flex items-center justify-between">
        <div>
          <p className="text-body font-medium text-text-primary">{label}</p>
          {description && (
            <p className="text-body text-text-tertiary mt-0.5">{description}</p>
          )}
        </div>
        {children}
      </div>
    </InkCard>
  )
}
