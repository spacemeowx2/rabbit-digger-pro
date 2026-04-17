import type { EngineStatus } from '../types'
import { classNames } from '../utils'

const STATUS_LABELS: Record<string, string> = {
  Idle: '空闲',
  Starting: '启动中',
  Running: '运行中',
  Stopping: '停止中',
  Error: '错误',
  Connecting: '连接中',
}

const STATUS_COLORS: Record<string, { dot: string; bg: string; text: string }> = {
  Idle: { dot: 'bg-slate-400', bg: 'bg-slate-50', text: 'text-slate-600' },
  Starting: { dot: 'bg-amber-500 animate-pulse', bg: 'bg-amber-50', text: 'text-amber-700' },
  Running: { dot: 'bg-emerald-500', bg: 'bg-emerald-50', text: 'text-emerald-700' },
  Stopping: { dot: 'bg-amber-500 animate-pulse', bg: 'bg-amber-50', text: 'text-amber-700' },
  Error: { dot: 'bg-rose-500', bg: 'bg-rose-50', text: 'text-rose-700' },
  Connecting: { dot: 'bg-slate-300 animate-pulse', bg: 'bg-slate-50', text: 'text-slate-500' },
}

interface StatusPageProps {
  engineStatus: EngineStatus
}

export function StatusPage({ engineStatus }: StatusPageProps) {
  const { status: runtimeState } = engineStatus
  const colors = STATUS_COLORS[runtimeState] ?? STATUS_COLORS.Connecting
  const servers = engineStatus.servers ?? []

  return (
    <div className="space-y-4">
      <h1 className="font-display text-lg font-semibold text-slate-900">状态</h1>

      {/* Engine status card */}
      <section className="surface overflow-hidden">
        <div className="px-5 py-4 flex items-center gap-4">
          <div className={classNames('h-3 w-3 rounded-full shrink-0', colors.dot)} />
          <div className="flex-1 min-w-0">
            <p className="font-display text-base font-semibold text-slate-900">
              {STATUS_LABELS[runtimeState] ?? runtimeState}
            </p>
            {engineStatus.message && (
              <p className={classNames('text-sm mt-0.5', colors.text)}>
                {engineStatus.message}
              </p>
            )}
          </div>
          <span className={classNames('tag', colors.bg, colors.text)}>
            {runtimeState}
          </span>
        </div>
      </section>

      {/* Servers */}
      <section className="surface overflow-hidden">
        <div className="px-4 py-3 border-b border-slate-200/60">
          <h2 className="font-display text-sm font-semibold text-slate-900">
            Servers
            {servers.length > 0 && (
              <span className="ml-2 text-xs font-normal text-slate-400">
                {servers.length} running
              </span>
            )}
          </h2>
        </div>
        {servers.length > 0 ? (
          <div className="divide-y divide-slate-100">
            {servers.map((server) => (
              <div key={server.name} className="flex items-center gap-3 px-4 py-3">
                <div className="h-2 w-2 rounded-full shrink-0 bg-emerald-500" />
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium text-slate-900 truncate">{server.name}</p>
                </div>
                <span className="tag bg-slate-50 text-slate-500">{server.server_type}</span>
              </div>
            ))}
          </div>
        ) : (
          <div className="px-4 py-8 text-center text-sm text-slate-400">
            {runtimeState === 'Idle' ? '引擎未启动' : '暂无 Server 信息'}
          </div>
        )}
      </section>
    </div>
  )
}
