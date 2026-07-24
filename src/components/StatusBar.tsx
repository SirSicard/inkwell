export function StatusBar({ modelName }: { modelName: string }) {
  return (
    <div className="flex items-center justify-between px-5 py-2.5 border-t border-border">
      <span className="text-[11px] font-mono text-text-tertiary tracking-wide flex items-center gap-1.5">
        {modelName === "Loading..." || modelName === "No model loaded" ? (
          <><span className="inline-block w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse" />{modelName}</>
        ) : (
          <><span className="inline-block w-1.5 h-1.5 rounded-full bg-green-400" />{modelName}</>
        )}
      </span>
      <span className="text-[11px] font-mono text-text-tertiary">v0.1.0</span>
    </div>
  )
}
