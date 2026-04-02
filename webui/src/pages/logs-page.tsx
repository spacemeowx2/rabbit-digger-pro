import { useDeferredValue, useMemo, useState } from 'react'

import type { LogEntry } from '../types'
import { classNames, extractLogContext } from '../utils'

interface LogsPageProps {
  logs: LogEntry[]
  onClearLogs: () => void
}

const LEVEL_COLORS: Record<string, string> = {
  ERROR: 'bg-rose-100 text-rose-700',
  WARN: 'bg-amber-100 text-amber-700',
  INFO: 'bg-sky-100 text-sky-700',
  DEBUG: 'bg-slate-100 text-slate-500',
  TRACE: 'bg-violet-100 text-violet-600',
  LOG: 'bg-slate-100 text-slate-500',
}

function formatTime(time: string | null): string {
  if (!time) return ''
  // Extract HH:MM:SS from ISO timestamp
  const match = time.match(/T(\d{2}:\d{2}:\d{2})/)
  return match ? match[1] : time
}

export function LogsPage({ logs, onClearLogs }: LogsPageProps) {
  const [query, setQuery] = useState('')
  const [level, setLevel] = useState('ALL')
  const [paused, setPaused] = useState(false)
  const [frozenLogs, setFrozenLogs] = useState<LogEntry[]>(logs)
  const deferredQuery = useDeferredValue(query.trim().toLowerCase())

  const visibleLogs = paused ? frozenLogs : logs

  const levels = useMemo(() => {
    return ['ALL', ...new Set(visibleLogs.map((entry) => entry.level))]
  }, [visibleLogs])

  const filteredLogs = useMemo(() => {
    return [...visibleLogs]
      .reverse()
      .filter((entry) => {
        if (level !== 'ALL' && entry.level !== level) return false
        if (!deferredQuery) return true
        return `${entry.time ?? ''} ${entry.level} ${entry.target ?? ''} ${entry.message}`
          .toLowerCase()
          .includes(deferredQuery)
      })
  }, [deferredQuery, level, visibleLogs])

  return (
    <div className="space-y-3">
      {/* toolbar */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-3">
          <h1 className="font-display text-lg font-semibold text-slate-900">日志</h1>
          <span className="text-xs text-slate-400">{filteredLogs.length} 条</span>
        </div>
        <div className="flex items-center gap-2">
          <select
            className="field w-24 text-xs"
            value={level}
            onChange={(e) => setLevel(e.target.value)}
          >
            {levels.map((opt) => (
              <option key={opt} value={opt}>{opt}</option>
            ))}
          </select>
          <input
            className="field sm:w-56"
            placeholder="过滤..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <button
            type="button"
            className={classNames(
              'action-button text-xs',
              paused && 'bg-slate-900 text-white border-slate-900',
            )}
            onClick={() => {
              if (!paused) {
                setFrozenLogs(logs)
                setPaused(true)
              } else {
                setPaused(false)
              }
            }}
          >
            {paused ? '继续' : '暂停'}
          </button>
          <button
            type="button"
            className="action-button action-button-danger text-xs"
            onClick={onClearLogs}
          >
            清空
          </button>
        </div>
      </div>

      {/* log entries */}
      <section className="surface overflow-hidden">
        {filteredLogs.length === 0 ? (
          <div className="px-4 py-12 text-center text-sm text-slate-500">
            还没有新的日志
          </div>
        ) : (
          <div className="max-h-[80vh] overflow-auto divide-y divide-slate-200/40">
            {filteredLogs.map((entry) => {
              const ctx = extractLogContext(entry)
              const hasContext = ctx.dest || ctx.netList

              return (
                <div
                  key={entry.id}
                  className="px-4 py-2 hover:bg-white/40 transition text-sm"
                >
                  <div className="flex items-start gap-2">
                    {/* time */}
                    <span className="shrink-0 w-16 text-xs font-mono text-slate-400">
                      {formatTime(entry.time)}
                    </span>

                    {/* level badge */}
                    <span
                      className={classNames(
                        'shrink-0 w-12 text-center rounded px-1.5 py-0.5 text-[10px] font-bold uppercase',
                        LEVEL_COLORS[entry.level] ?? LEVEL_COLORS.LOG,
                      )}
                    >
                      {entry.level}
                    </span>

                    {/* target */}
                    {entry.target && (
                      <span className="shrink-0 text-xs text-slate-400 font-mono truncate max-w-40">
                        {entry.target}
                      </span>
                    )}

                    {/* message */}
                    <span className="min-w-0 text-slate-700 break-words">
                      {entry.message}
                    </span>
                  </div>

                  {/* structured context for connection logs */}
                  {hasContext && (
                    <div className="mt-1 ml-[7.5rem] flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
                      {ctx.dest && (
                        <span className="text-slate-600">
                          <span className="text-slate-400">dest:</span> {ctx.dest}
                        </span>
                      )}
                      {ctx.src && (
                        <span className="text-slate-600">
                          <span className="text-slate-400">src:</span> {ctx.src}
                        </span>
                      )}
                      {ctx.netList && ctx.netList.length > 0 && (
                        <span className="text-slate-600">
                          <span className="text-slate-400">route:</span>{' '}
                          {ctx.netList.join(' → ')}
                        </span>
                      )}
                      {ctx.process && (
                        <span className="text-slate-600">
                          <span className="text-slate-400">proc:</span> {ctx.process}
                        </span>
                      )}
                    </div>
                  )}

                  {/* extra fields */}
                  {Object.keys(ctx.extraFields).length > 0 && (
                    <div className="mt-1 ml-[7.5rem] flex flex-wrap gap-x-3 gap-y-1 text-xs">
                      {Object.entries(ctx.extraFields).map(([key, value]) => (
                        <span key={key} className="text-slate-500">
                          <span className="text-slate-400">{key}:</span>{' '}
                          {typeof value === 'string' ? value : JSON.stringify(value)}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        )}
      </section>
    </div>
  )
}
