import { type InputHTMLAttributes } from "react"

interface GlassInputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string
}

export function GlassInput({ label, className = "", ...props }: GlassInputProps) {
  return (
    <div className="space-y-1.5">
      {label && (
        <label className="text-[11px] font-medium text-text-secondary tracking-wide">
          {label}
        </label>
      )}
      <input
        className={`
          w-full px-3 py-2 rounded-lg text-[13px] text-text-primary
          bg-bg-surface border border-border
          focus:border-border-default focus:outline-none
          placeholder:text-text-tertiary
          transition-colors duration-150
          ${className}
        `}
        {...props}
      />
    </div>
  )
}
