import { motion } from "framer-motion"
import { type ButtonHTMLAttributes, type ReactNode } from "react"

interface GlassButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "ghost" | "primary"
  children: ReactNode
}

export function GlassButton({ variant = "ghost", children, className = "", onClick, disabled, type, form }: GlassButtonProps) {
  const base = "px-4 py-2 rounded-lg text-[12px] font-medium cursor-pointer transition-all duration-150 focus:outline-none disabled:opacity-40 disabled:cursor-not-allowed"

  const variants = {
    ghost: "border border-border text-text-secondary hover:text-text-primary hover:border-border-default hover:bg-bg-hover",
    primary: "bg-accent text-white hover:brightness-110",
  }

  return (
    <motion.button
      whileTap={{ scale: 0.98 }}
      transition={{ type: "spring", stiffness: 500, damping: 30 }}
      className={`${base} ${variants[variant]} ${className}`}
      onClick={onClick}
      disabled={disabled}
      type={type}
      form={form}
    >
      {children}
    </motion.button>
  )
}
