import { useDeferredValue, useMemo, useState } from 'react'

import type { LogEntry } from '../types'
import { classNames } from '../utils'

interface LogsPageProps {
  logs: LogEntry[]
  onClearLogs: () => void
}

const LEVEL_STYLES: Record<string, string> = {
  ERROR: 'text-rose-600',
  WARN: 'text-amber-600',
  INFO: 'text-sky-600',
  DEBUG: 'text-slate-500',
  TRACE: 'text-violet-600',
  LOG: 'text-slate-500',
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
        if (level !== 'ALL' && entry.level !== level) {
          return false
        }

        if (!deferredQuery) {
          return true
        }

        return `${entry.time ?? ''} ${entry.level} ${entry.message}`
          .toLowerCase()
          .includes(deferredQuery)
      })
  }, [deferredQuery, level, visibleLogs])

  return (
    <div className="space-y-5">
      <section className="surface p-5 md:p-6">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
          <div className="space-y-2">
            <p className="eyebrow">Logs</p>
            <h1 className="font-display text-3xl font-semibold tracking-[-0.04em] text-slate-900 md:text-4xl">
              日志
            </h1>
            <p className="max-w-3xl text-sm leading-6 text-slate-600 md:text-base">
              WebSocket 直接订阅后端日志流。这里保留原始文本，方便观察 selector 切换、
              连接建立失败和真实路由命中情况。
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-3">
            <button
              type="button"
              className={classNames(
                'action-button min-w-28 justify-center',
                paused && 'border-slate-900 bg-slate-900 text-white',
              )}
              onClick={() => {
                if (!paused) {
                  setFrozenLogs(logs)
                  setPaused(true)
                  return
                }

                setPaused(false)
              }}
            >
              {paused ? '继续流式' : '暂停流式'}
            </button>
            <button
              type="button"
              className="action-button action-button-danger"
              onClick={onClearLogs}
            >
              清空视图
            </button>
          </div>
        </div>

        <div className="mt-5 grid gap-3 lg:grid-cols-[14rem_minmax(0,1fr)_auto]">
          <label className="field inline-flex items-center gap-3">
            <span className="text-xs font-semibold uppercase tracking-[0.16em] text-slate-400">
              level
            </span>
            <select
              className="w-full bg-transparent text-sm font-medium text-slate-700 outline-none"
              value={level}
              onChange={(event) => setLevel(event.target.value)}
              aria-label="选择日志级别"
            >
              {levels.map((option) => (
                <option key={option} value={option}>
                  {option}
                </option>
              ))}
            </select>
          </label>

          <input
            className="field"
            placeholder="搜索关键字、span 或错误信息"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            aria-label="过滤日志"
          />

          <div className="metric-panel min-w-40">
            <span className="metric-label">显示条数</span>
            <strong className="metric-value">{filteredLogs.length}</strong>
          </div>
        </div>
      </section>

      <section className="surface overflow-hidden">
        {filteredLogs.length === 0 ? (
          <div className="px-6 py-16 text-center">
            <p className="font-display text-2xl font-semibold tracking-[-0.04em] text-slate-900">
              还没有新的日志
            </p>
            <p className="mt-3 text-sm leading-6 text-slate-500">
              当 mixed server 收到流量，或 selector / 连接状态发生变化时，这里会立刻出现输出。
            </p>
          </div>
        ) : (
          <div className="max-h-[72vh] overflow-auto">
            {filteredLogs.map((entry) => (
              <div
                key={entry.id}
                className="border-b border-slate-200/70 px-5 py-3 font-mono text-[13px] leading-6 text-slate-600"
              >
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                  {entry.time && <span className="text-slate-400">{entry.time}</span>}
                  <span
                    className={classNames(
                      'font-sans text-xs font-semibold uppercase tracking-[0.16em]',
                      LEVEL_STYLES[entry.level] ?? LEVEL_STYLES.LOG,
                    )}
                  >
                    {entry.level}
                  </span>
                </div>
                <pre className="mt-2 whitespace-pre-wrap break-words text-slate-700">
                  {entry.message}
                </pre>
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  )
}
