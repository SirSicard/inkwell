import * as Select from "@radix-ui/react-select"
import { motion, AnimatePresence } from "framer-motion"
import { useState } from "react"

interface GlassSelectProps {
  value: string
  onChange: (value: string) => void
  options: { label: string; value: string }[]
  placeholder?: string
}

export function GlassSelect({ value, onChange, options, placeholder }: GlassSelectProps) {
  const [open, setOpen] = useState(false)
  const selectedLabel = options.find((o) => o.value === value)?.label

  return (
    <Select.Root value={value} onValueChange={onChange} open={open} onOpenChange={setOpen}>
      <Select.Trigger
        className="
          inline-flex items-center gap-2 px-3 py-1.5 rounded-md text-[12px] font-mono
          bg-bg-surface border border-border text-text-primary
          hover:border-border-default focus:outline-none
          cursor-pointer transition-colors duration-150
        "
      >
        <Select.Value placeholder={placeholder}>{selectedLabel}</Select.Value>
        <Select.Icon>
          <svg width="10" height="6" viewBox="0 0 10 6" fill="none" className="text-text-tertiary">
            <path d="M1 1L5 5L9 1" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </Select.Icon>
      </Select.Trigger>

      <AnimatePresence>
        {open && (
          <Select.Portal>
            <Select.Content position="popper" sideOffset={4} asChild>
              <motion.div
                initial={{ opacity: 0, y: -4, scale: 0.98 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: -4, scale: 0.98 }}
                transition={{ type: "spring", stiffness: 400, damping: 30 }}
                className="
                  z-50 min-w-[120px] rounded-lg p-1
                  bg-bg-overlay border border-border-default
                  shadow-xl
                "
              >
                <Select.Viewport>
                  {options.map((option) => (
                    <Select.Item
                      key={option.value}
                      value={option.value}
                      className="
                        px-3 py-1.5 text-[12px] font-mono text-text-secondary rounded-md
                        hover:bg-bg-hover hover:text-text-primary
                        focus:outline-none focus:bg-bg-hover focus:text-text-primary
                        cursor-pointer transition-colors duration-100
                        data-[state=checked]:text-accent
                      "
                    >
                      <Select.ItemText>{option.label}</Select.ItemText>
                    </Select.Item>
                  ))}
                </Select.Viewport>
              </motion.div>
            </Select.Content>
          </Select.Portal>
        )}
      </AnimatePresence>
    </Select.Root>
  )
}
