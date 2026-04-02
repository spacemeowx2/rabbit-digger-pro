import { useDeferredValue, useMemo, useState } from 'react'

import type { ConnectionSnapshot } from '../types'
import { classNames, formatAge, formatBytes, summarizeConnection } from '../utils'

interface ConnectionsPageProps {
  connections: ConnectionSnapshot
  busyConnectionId: string | null
  closingAll: boolean
  onCloseConnection: (connectionId: string) => Promise<void>
  onCloseAll: () => Promise<void>
}

export function ConnectionsPage({
  connections,
  busyConnectionId,
  closingAll,
  onCloseConnection,
  onCloseAll,
}: ConnectionsPageProps) {
  const [query, setQuery] = useState('')
  const deferredQuery = useDeferredValue(query.trim().toLowerCase())

  const rows = useMemo(() => {
    return Object.entries(connections.connections)
      .map(([id, connection]) => ({
        id,
        connection,
        summary: summarizeConnection(connection),
      }))
      .filter(({ summary }) => {
        if (!deferredQuery) return true
        const candidate = `${summary.host} ${summary.route} ${summary.source}`.toLowerCase()
        return candidate.includes(deferredQuery)
      })
      .sort((left, right) => (right.connection.start_time ?? 0) - (left.connection.start_time ?? 0))
  }, [connections.connections, deferredQuery])

  return (
    <div className="space-y-3">
      {/* toolbar */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-3">
          <h1 className="font-display text-lg font-semibold text-slate-900">连接</h1>
          <span className="text-xs text-slate-500">
            活跃 {rows.length}
          </span>
          <span className="text-xs text-slate-400">
            ↓ {formatBytes(connections.total_download)} ↑ {formatBytes(connections.total_upload)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <input
            className="field sm:w-64"
            placeholder="过滤域名、路由..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="过滤连接"
          />
          <button
            type="button"
            className="action-button action-button-danger shrink-0"
            onClick={() => void onCloseAll()}
            disabled={closingAll}
          >
            {closingAll ? 'Closing...' : '关闭全部'}
          </button>
        </div>
      </div>

      {/* table */}
      <section className="surface overflow-hidden">
        <div className="hidden lg:grid grid-cols-[minmax(14rem,2fr)_5rem_4rem_5rem_5rem_minmax(8rem,1.5fr)_3.5rem] gap-3 px-4 py-2 text-xs font-semibold uppercase tracking-wider text-slate-400 border-b border-slate-200/60">
          <span>主机</span>
          <span>协议</span>
          <span>时长</span>
          <span>上传</span>
          <span>下载</span>
          <span>链路</span>
          <span></span>
        </div>

        {rows.length === 0 ? (
          <div className="px-4 py-12 text-center text-sm text-slate-500">
            当前没有活跃连接
          </div>
        ) : (
          <div className="divide-y divide-slate-200/50 max-h-[75vh] overflow-auto">
            {rows.map(({ id, connection, summary }) => {
              const isBusy = busyConnectionId === id
              return (
                <div
                  key={id}
                  className="grid gap-3 px-4 py-2 text-sm lg:grid-cols-[minmax(14rem,2fr)_5rem_4rem_5rem_5rem_minmax(8rem,1.5fr)_3.5rem] lg:items-center hover:bg-white/40 transition"
                >
                  <div className="min-w-0">
                    <p className="truncate font-medium text-slate-900 text-sm">{summary.host}</p>
                    <p className="truncate text-xs text-slate-400">{summary.source}</p>
                  </div>
                  <div className="text-xs font-medium uppercase text-slate-500">
                    {connection.protocol ?? 'tcp'}
                  </div>
                  <div className="text-xs text-slate-500">
                    {formatAge(connection.start_time)}
                  </div>
                  <div className="text-xs text-slate-600">
                    {formatBytes(connection.upload ?? 0)}
                  </div>
                  <div className="text-xs text-slate-600">
                    {formatBytes(connection.download ?? 0)}
                  </div>
                  <div className="min-w-0 text-xs text-slate-500 truncate">
                    {summary.route}
                  </div>
                  <div>
                    <button
                      type="button"
                      className={classNames(
                        'text-xs text-rose-500 hover:text-rose-700 font-medium',
                        isBusy && 'cursor-wait opacity-50',
                      )}
                      onClick={() => void onCloseConnection(id)}
                      disabled={isBusy}
                      aria-label={`关闭 ${summary.host}`}
                    >
                      {isBusy ? '...' : '关闭'}
                    </button>
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </section>
    </div>
  )
}
