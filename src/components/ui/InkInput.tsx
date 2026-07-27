import { type InputHTMLAttributes } from "react"

interface InkInputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string
}

export function InkInput({ label, className = "", ...props }: InkInputProps) {
  return (
    <div className="space-y-1.5">
      {label && (
        <label className="text-body font-medium text-text-secondary tracking-wide">
          {label}
        </label>
      )}
      <input
        className={`
          w-full px-3 py-2 rounded-lg text-body text-text-primary
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
