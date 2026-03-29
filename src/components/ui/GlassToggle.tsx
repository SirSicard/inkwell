import { motion } from "framer-motion"

interface GlassToggleProps {
  checked: boolean
  onChange: (checked: boolean) => void
}

export function GlassToggle({ checked, onChange }: GlassToggleProps) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={`
        relative w-9 h-[22px] rounded-full transition-colors duration-200 cursor-pointer shrink-0
        ${checked ? "bg-accent" : "bg-bg-hover border border-border"}
      `}
    >
      <motion.div
        className={`absolute top-[3px] left-[3px] w-4 h-4 rounded-full ${checked ? "bg-white" : "bg-text-tertiary"}`}
        animate={{ x: checked ? 14 : 0 }}
        transition={{ type: "spring", stiffness: 500, damping: 30 }}
      />
    </button>
  )
}
