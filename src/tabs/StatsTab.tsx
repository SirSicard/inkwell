import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import type { Stats } from "../types"

/**
 * Usage statistics, asked for by the first week of real users ("purely for
 * vanity, but it would be nice to know how much I am using it").
 *
 * Everything is derived from the transcript history at render time; there is
 * no separate counter anywhere. That keeps the delete button honest: erase
 * your history and this page genuinely forgets it, rather than a total
 * remembering what the rows used to say.
 */

/** "1h 23m" / "4m 12s" / "38s". Milliseconds of key-held speaking time. */
function fmtDuration(ms: number): string {
  const s = Math.round(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m ${s % 60}s`
  return `${Math.floor(m / 60)}h ${m % 60}m`
}

/** Weekday initial for the activity strip ("M", "T"...). */
function dayInitial(date: string): string {
  return new Date(`${date}T12:00:00`).toLocaleDateString(undefined, { weekday: "narrow" })
}

function fmtDay(date: string): string {
  return new Date(`${date}T12:00:00`).toLocaleDateString(undefined, {
    day: "numeric", month: "short",
  })
}

function StatTile({ value, label, caption }: { value: string; label: string; caption?: string }) {
  return (
    <div className="rounded-lg border border-border bg-bg-surface p-3.5">
      <div className="text-xl font-semibold text-text-primary tabular-nums">{value}</div>
      <div className="mt-0.5 text-sm text-text-secondary">{label}</div>
      {caption && <div className="mt-0.5 text-meta text-text-tertiary">{caption}</div>}
    </div>
  )
}

export function StatsTab() {
  const [stats, setStats] = useState<Stats | null>(null)
  const [error, setError] = useState("")

  useEffect(() => {
    const load = () =>
      invoke<Stats>("get_stats").then(setStats).catch((e) => setError(String(e)))
    load()
    // A dictation finishing while this page is open should count immediately.
    const unlisten = listen("transcription", load)
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  if (error) {
    return (
      <div className="space-y-3">
        <h2 className="text-heading font-sans font-semibold text-text-primary">Stats</h2>
        <p className="text-body text-text-tertiary">{error}</p>
      </div>
    )
  }
  if (!stats) return null

  if (stats.total_count === 0) {
    return (
      <div className="space-y-3">
        <h2 className="text-heading font-sans font-semibold text-text-primary">Stats</h2>
        <p className="text-body text-text-secondary">
          Nothing to count yet. Dictate something and this page starts keeping score.
        </p>
      </div>
    )
  }

  // The one estimated number on the page, and the assumption is printed with
  // it. Everything else is measured.
  const TYPING_WPM = 40
  const typingMs = (stats.total_words / TYPING_WPM) * 60_000
  const savedMs = typingMs - stats.total_speaking_ms

  const maxCount = Math.max(...stats.recent_days.map(([, c]) => c), 1)
  const recentTotal = stats.recent_days.reduce((sum, [, c]) => sum + c, 0)
  const modelMax = Math.max(...stats.per_model.map(([, c]) => c), 1)

  return (
    <div className="space-y-3">
      <h2 className="text-heading font-sans font-semibold text-text-primary">Stats</h2>

      <div className="grid grid-cols-2 gap-3">
        <StatTile
          value={stats.total_words.toLocaleString()}
          label="Words dictated"
        />
        <StatTile
          value={stats.total_count.toLocaleString()}
          label="Dictations"
        />
        <StatTile
          value={fmtDuration(stats.total_speaking_ms)}
          label="Time speaking"
        />
        {savedMs > 0 ? (
          <StatTile
            value={fmtDuration(savedMs)}
            label="Saved over typing"
            caption={`assuming ${TYPING_WPM} words a minute typed`}
          />
        ) : (
          <StatTile
            value={String(stats.days_active)}
            label="Days used"
          />
        )}
      </div>

      <div className="rounded-lg border border-border bg-bg-surface p-3.5 space-y-2">
        <div className="flex items-baseline justify-between">
          <span className="text-sm text-text-primary">Last 14 days</span>
          <span className="text-meta text-text-tertiary">
            {recentTotal.toLocaleString()} dictation{recentTotal === 1 ? "" : "s"}
          </span>
        </div>
        <div className="flex items-end gap-[3px] h-20" role="img"
          aria-label={`Dictations per day over the last 14 days, up to ${maxCount} in one day`}>
          {stats.recent_days.map(([date, count, words]) => (
            <div
              key={date}
              className="flex-1 flex flex-col items-center gap-1 min-w-0"
              title={`${fmtDay(date)}: ${count} dictation${count === 1 ? "" : "s"}, ${words} words`}
            >
              {/* Labels selectively: only the busiest day carries its number. */}
              <span className="text-[10px] leading-none text-text-tertiary tabular-nums h-3">
                {count === maxCount && count > 0 ? count : ""}
              </span>
              <div className="w-full flex-1 flex items-end">
                <div
                  className={`w-full rounded-t ${count > 0 ? "bg-accent" : "bg-border"}`}
                  style={{ height: count > 0 ? `${Math.max((count / maxCount) * 100, 8)}%` : "2px" }}
                />
              </div>
              <span className="text-[10px] leading-none text-text-tertiary">{dayInitial(date)}</span>
            </div>
          ))}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <StatTile
          value={String(stats.streak_days)}
          label={stats.streak_days === 1 ? "Day streak" : "Day streak"}
        />
        <StatTile
          value={stats.best_day ? String(stats.best_day[1]) : "0"}
          label="Best day"
          caption={stats.best_day ? fmtDay(stats.best_day[0]) : undefined}
        />
      </div>

      {stats.per_model.length > 0 && (
        <div className="rounded-lg border border-border bg-bg-surface p-3.5 space-y-2">
          <span className="text-sm text-text-primary">By model</span>
          <div className="space-y-1.5">
            {stats.per_model.map(([model, count]) => (
              <div key={model} className="flex items-center gap-2">
                <span className="w-28 truncate text-meta text-text-secondary">{model}</span>
                <div className="flex-1 h-2 rounded-full bg-border overflow-hidden">
                  <div
                    className="h-full rounded-full bg-accent"
                    style={{ width: `${(count / modelMax) * 100}%` }}
                  />
                </div>
                <span className="w-10 text-right text-meta font-mono text-text-tertiary tabular-nums">
                  {count}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      <p className="text-meta text-text-tertiary leading-relaxed">
        Counted from your transcript history, nothing else. Delete history and
        these numbers forget it too.
      </p>
    </div>
  )
}
