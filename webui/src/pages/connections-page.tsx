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
        if (!deferredQuery) {
          return true
        }

        const candidate = `${summary.host} ${summary.route} ${summary.source}`.toLowerCase()
        return candidate.includes(deferredQuery)
      })
      .sort((left, right) => (right.connection.start_time ?? 0) - (left.connection.start_time ?? 0))
  }, [connections.connections, deferredQuery])
  const hasHistoricalTraffic =
    connections.total_download > 0 || connections.total_upload > 0

  return (
    <div className="space-y-5">
      <section className="surface p-5 md:p-6">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
          <div className="space-y-2">
            <p className="eyebrow">Connections</p>
            <h1 className="font-display text-3xl font-semibold tracking-[-0.04em] text-slate-900 md:text-4xl">
              连接
            </h1>
            <p className="max-w-3xl text-sm leading-6 text-slate-600 md:text-base">
              查看当前 TCP / UDP 会话、累计流量和实际命中的链路。
            </p>
          </div>

          <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
            <input
              className="field sm:min-w-[16rem] xl:min-w-[20rem]"
              placeholder="按目标域名、路由或源地址过滤"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              aria-label="过滤连接"
            />
            <button
              type="button"
              className="action-button action-button-danger shrink-0 whitespace-nowrap"
              onClick={() => void onCloseAll()}
              disabled={closingAll}
            >
              {closingAll ? 'Closing...' : '关闭全部'}
            </button>
          </div>
        </div>

        <div className="mt-5 grid gap-3 md:grid-cols-3">
          <div className="metric-panel">
            <span className="metric-label">活跃会话</span>
            <strong className="metric-value">{rows.length}</strong>
          </div>
          <div className="metric-panel">
            <span className="metric-label">累计上传</span>
            <strong className="metric-value">{formatBytes(connections.total_upload)}</strong>
          </div>
          <div className="metric-panel">
            <span className="metric-label">累计下载</span>
            <strong className="metric-value">{formatBytes(connections.total_download)}</strong>
          </div>
        </div>

        {rows.length === 0 && hasHistoricalTraffic && (
          <p className="mt-4 text-sm text-slate-500">
            当前没有活跃连接，累计流量保留本次运行期间的历史总量。
          </p>
        )}
      </section>

      <section className="surface overflow-hidden">
        <div className="hidden grid-cols-[minmax(16rem,2.4fr)_0.8fr_0.8fr_0.9fr_1.2fr_1.2fr_auto] gap-4 border-b border-slate-200/70 px-5 py-4 text-xs font-semibold uppercase tracking-[0.18em] text-slate-400 lg:grid">
          <span>目标</span>
          <span>协议</span>
          <span>Age</span>
          <span>上传</span>
          <span>下载</span>
          <span>链路</span>
          <span></span>
        </div>

        {rows.length === 0 ? (
          <div className="px-6 py-16 text-center">
            <p className="font-display text-2xl font-semibold tracking-[-0.04em] text-slate-900">
              当前没有活跃连接
            </p>
            <p className="mt-3 text-sm leading-6 text-slate-500">
              让客户端通过 mixed 代理发起一次请求，这里就会出现目标域名、路由链路和流量统计。
            </p>
          </div>
        ) : (
          <div className="divide-y divide-slate-200/70">
            {rows.map(({ id, connection, summary }) => {
              const isBusy = busyConnectionId === id

              return (
                <div
                  key={id}
                  className="grid gap-4 px-5 py-4 lg:grid-cols-[minmax(16rem,2.4fr)_0.8fr_0.8fr_0.9fr_1.2fr_1.2fr_auto] lg:items-center"
                >
                  <div className="min-w-0">
                    <p className="truncate font-semibold text-slate-900">{summary.host}</p>
                    <p className="mt-1 truncate text-sm text-slate-500">
                      {summary.source}
                    </p>
                  </div>
                  <div className="text-sm font-medium uppercase text-slate-500">
                    {connection.protocol ?? 'tcp'}
                  </div>
                  <div className="text-sm text-slate-500">
                    {formatAge(connection.start_time)}
                  </div>
                  <div className="text-sm text-slate-600">
                    {formatBytes(connection.upload ?? 0)}
                  </div>
                  <div className="text-sm text-slate-600">
                    {formatBytes(connection.download ?? 0)}
                  </div>
                  <div className="min-w-0 text-sm text-slate-500">
                    <p className="truncate">{summary.route}</p>
                  </div>
                  <div>
                    <button
                      type="button"
                      className={classNames(
                        'action-button w-full lg:w-auto',
                        isBusy && 'cursor-wait opacity-70',
                      )}
                      onClick={() => void onCloseConnection(id)}
                      disabled={isBusy}
                      aria-label={`关闭连接 ${summary.host}`}
                    >
                      {isBusy ? 'Closing...' : '关闭'}
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
