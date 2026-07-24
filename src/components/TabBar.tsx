import { useState, useEffect, useRef } from "react"
import { motion, AnimatePresence } from "framer-motion"
import type { Tab } from "../types"

export function TabBar({ tabs, activeTab, onTabChange }: { tabs: readonly string[]; activeTab: string; onTabChange: (t: Tab) => void }) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [showMenu, setShowMenu] = useState(false)
  const [containerWidth, setContainerWidth] = useState(999)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const observer = new ResizeObserver((entries) => {
      setContainerWidth(entries[0].contentRect.width)
    })
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  // ~80px per tab on average. Reserve 40px for hamburger.
  const maxVisible = Math.max(1, Math.floor((containerWidth - 40) / 80))
  const needsOverflow = tabs.length > maxVisible
  const visibleTabs = needsOverflow ? tabs.slice(0, maxVisible) : tabs
  const overflowTabs = needsOverflow ? tabs.slice(maxVisible) : []

  return (
    <div ref={containerRef} role="tablist" className="relative flex items-center gap-0.5 px-5 pt-3 pb-0 border-b border-border"
      onKeyDown={(e) => {
        const allTabs = [...tabs]
        const idx = allTabs.indexOf(activeTab)
        if (e.key === "ArrowRight" && idx < allTabs.length - 1) { e.preventDefault(); onTabChange(allTabs[idx + 1] as Tab) }
        if (e.key === "ArrowLeft" && idx > 0) { e.preventDefault(); onTabChange(allTabs[idx - 1] as Tab) }
        if (e.key === "Home") { e.preventDefault(); onTabChange(allTabs[0] as Tab) }
        if (e.key === "End") { e.preventDefault(); onTabChange(allTabs[allTabs.length - 1] as Tab) }
      }}
    >
      {visibleTabs.map((tab) => (
        <button
          key={tab}
          role="tab"
          aria-selected={activeTab === tab}
          tabIndex={activeTab === tab ? 0 : -1}
          onClick={() => { onTabChange(tab as Tab); setShowMenu(false) }}
          className={`relative px-3 py-2.5 text-[11px] font-medium tracking-wide transition-colors whitespace-nowrap focus:outline-none ${
            activeTab === tab
              ? "text-text-primary"
              : "text-text-tertiary hover:text-text-secondary"
          }`}
        >
          {tab}
          {activeTab === tab && (
            <motion.div
              layoutId="tab-underline"
              className="absolute bottom-0 left-3 right-3 h-[2px] bg-accent rounded-full"
              transition={{ type: "spring", stiffness: 400, damping: 35 }}
            />
          )}
        </button>
      ))}

      {needsOverflow && (
        <div className="ml-auto relative pb-2">
          <button
            onClick={() => setShowMenu(!showMenu)}
            className={`flex items-center justify-center w-7 h-7 rounded-md transition-colors ${
              showMenu || overflowTabs.includes(activeTab)
                ? "text-text-primary bg-bg-hover"
                : "text-text-tertiary hover:text-text-secondary"
            }`}
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
              <circle cx="3" cy="8" r="1.2" fill="currentColor" />
              <circle cx="8" cy="8" r="1.2" fill="currentColor" />
              <circle cx="13" cy="8" r="1.2" fill="currentColor" />
            </svg>
          </button>
          <AnimatePresence>
            {showMenu && (
              <motion.div
                initial={{ opacity: 0, y: -4, scale: 0.97 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: -4, scale: 0.97 }}
                transition={{ type: "spring", stiffness: 400, damping: 30 }}
                className="absolute right-0 top-full mt-1 z-50 min-w-[140px] bg-bg-overlay border border-border-default rounded-lg shadow-xl py-1"
              >
                {overflowTabs.map((tab) => (
                  <button
                    key={tab}
                    onClick={() => { onTabChange(tab as Tab); setShowMenu(false) }}
                    className={`w-full text-left px-3 py-2 text-[11px] font-medium tracking-wide transition-colors ${
                      activeTab === tab
                        ? "text-text-primary bg-bg-hover"
                        : "text-text-tertiary hover:text-text-secondary hover:bg-bg-hover"
                    }`}
                  >
                    {tab}
                  </button>
                ))}
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      )}
    </div>
  )
}
