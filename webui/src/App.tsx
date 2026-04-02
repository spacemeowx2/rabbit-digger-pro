import { useMemo } from 'react'
import {
  BrowserRouter,
  Navigate,
  NavLink,
  Route,
  Routes,
} from 'react-router-dom'

import {
  LayersIcon,
  LinkIcon,
  LogIcon,
  PulseIcon,
} from './components/icons'
import { SidebarSparkline } from './components/sidebar-sparkline'
import { useRdpDashboard } from './hooks/use-rdp-dashboard'
import { ConnectionsPage } from './pages/connections-page'
import { LogsPage } from './pages/logs-page'
import { SelectNetsPage } from './pages/select-nets-page'
import { classNames, formatBytes, formatRate, getSelectGroups } from './utils'

const navigation = [
  {
    to: '/select-net',
    label: '代理组',
    caption: 'Selectors',
    icon: LayersIcon,
  },
  {
    to: '/connections',
    label: '连接',
    caption: 'Runtime',
    icon: LinkIcon,
  },
  {
    to: '/logs',
    label: '日志',
    caption: 'Stream',
    icon: LogIcon,
  },
]

function Shell() {
  const {
    config,
    runtimeState,
    connections,
    logs,
    trafficHistory,
    error,
    busyNet,
    busyConnectionId,
    closingAll,
    selectNet,
    closeConnection,
    closeAllConnections,
    clearLogs,
  } = useRdpDashboard()

  const selectGroups = useMemo(() => getSelectGroups(config), [config])
  const activeConnections = Object.keys(connections.connections).length
  const latestTraffic = trafficHistory[trafficHistory.length - 1]
  const instanceLabel = useMemo(() => {
    const raw = config?.id ?? 'unknown'
    const matched = raw.match(/^path:"(.+)"$/)
    if (!matched) {
      return raw
    }

    const segments = matched[1].split(/[\\/]/)
    return segments[segments.length - 1] || matched[1]
  }, [config])

  return (
    <div className="mx-auto min-h-screen max-w-[1720px] lg:grid lg:grid-cols-[18.5rem_minmax(0,1fr)]">
      <aside className="border-b border-white/70 bg-white/55 px-4 py-4 backdrop-blur-xl lg:border-b-0 lg:border-r lg:px-5 lg:py-6">
        <div className="flex items-center justify-between lg:block">
          <div>
            <p className="eyebrow">Rabbit Digger Pro</p>
            <div className="mt-2 flex items-center gap-3">
              <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-slate-900 text-lg font-bold text-white shadow-[0_18px_45px_-28px_rgba(15,23,42,0.7)]">
                R
              </div>
              <div>
                <p className="font-display text-xl font-semibold tracking-[-0.04em] text-slate-900">
                  RDP WebUI
                </p>
                <p className="text-sm text-slate-500">
                  calm tooling for deep proxy work
                </p>
              </div>
            </div>
          </div>

          <div className="hidden items-center gap-2 rounded-full bg-white/80 px-3 py-2 text-xs font-semibold text-slate-600 shadow-[0_18px_45px_-35px_rgba(15,23,42,0.35)] sm:flex lg:mt-6">
            <span
              className={classNames(
                'status-dot',
                runtimeState === 'Running' ? 'bg-emerald-500' : 'bg-amber-500',
              )}
            />
            {runtimeState}
          </div>
        </div>

        <nav className="mt-5 grid grid-cols-3 gap-2 lg:mt-8 lg:flex lg:grid-cols-none lg:flex-col">
          {navigation.map((item) => {
            const Icon = item.icon
            return (
              <NavLink
                key={item.to}
                to={item.to}
                className={({ isActive }) =>
                  classNames(
                    'group flex min-w-0 items-center gap-3 rounded-[22px] px-3 py-3 transition duration-200 sm:px-4 lg:min-w-0',
                    isActive
                      ? 'bg-slate-900 text-white shadow-[0_26px_60px_-36px_rgba(15,23,42,0.75)]'
                      : 'bg-white/70 text-slate-600 hover:-translate-y-0.5 hover:bg-white hover:text-slate-900',
                  )
                }
              >
                <span className="flex h-11 w-11 items-center justify-center rounded-2xl border border-current/10 bg-current/5">
                  <Icon className="h-5 w-5" />
                </span>
                <span className="min-w-0 overflow-hidden">
                  <span className="block truncate font-semibold">{item.label}</span>
                  <span className="hidden truncate text-xs uppercase tracking-[0.18em] opacity-65 sm:block">
                    {item.caption}
                  </span>
                </span>
              </NavLink>
            )
          })}
        </nav>

        <section className="surface-muted mt-6 hidden p-4 lg:block">
          <div className="flex items-center justify-between">
            <div>
              <p className="eyebrow">Live Traffic</p>
              <p className="mt-2 font-display text-lg font-semibold tracking-[-0.04em] text-slate-900">
                mixed server
              </p>
            </div>
            <PulseIcon className="h-5 w-5 text-slate-400" />
          </div>

          <div className="mt-4">
            <SidebarSparkline history={trafficHistory} />
          </div>

          <div className="mt-4 space-y-3 text-sm">
            <div className="flex items-center justify-between text-amber-600">
              <span>↑ upload</span>
              <strong>{formatRate(latestTraffic?.uploadRate ?? 0)}</strong>
            </div>
            <div className="flex items-center justify-between text-sky-600">
              <span>↓ download</span>
              <strong>{formatRate(latestTraffic?.downloadRate ?? 0)}</strong>
            </div>
            <div className="flex items-center justify-between text-slate-500">
              <span>accumulated</span>
              <strong>{formatBytes(connections.total_download + connections.total_upload)}</strong>
            </div>
          </div>
        </section>
      </aside>

      <main className="min-w-0 px-4 py-4 md:px-6 lg:px-7 lg:py-6">
        <div className="mb-5 lg:sticky lg:top-0 lg:z-20">
          <section className="surface px-4 py-4 md:px-6">
            <div className="flex flex-col gap-4 xl:flex-row xl:items-center xl:justify-between">
              <div className="flex flex-wrap items-center gap-2.5">
                <span className="tag bg-white text-slate-500">
                  instance {instanceLabel}
                </span>
                <span className="tag bg-white text-slate-500">
                  {selectGroups.length} selectors
                </span>
                <span className="tag bg-white text-slate-500">
                  {logs.length} log lines buffered
                </span>
              </div>

              <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
                <div className="metric-panel px-4 py-3">
                  <span className="metric-label">运行状态</span>
                  <strong className="metric-value text-[1.1rem]">
                    {runtimeState}
                  </strong>
                </div>
                <div className="metric-panel px-4 py-3">
                  <span className="metric-label">活跃连接</span>
                  <strong className="metric-value text-[1.1rem]">
                    {activeConnections}
                  </strong>
                </div>
                <div className="metric-panel px-4 py-3">
                  <span className="metric-label">累计下载</span>
                  <strong className="metric-value text-[1.1rem]">
                    {formatBytes(connections.total_download)}
                  </strong>
                </div>
                <div className="metric-panel px-4 py-3">
                  <span className="metric-label">累计上传</span>
                  <strong className="metric-value text-[1.1rem]">
                    {formatBytes(connections.total_upload)}
                  </strong>
                </div>
              </div>
            </div>

            {error && (
              <div className="mt-4 rounded-[20px] border border-rose-200 bg-rose-50 px-4 py-3 text-sm text-rose-700">
                {error}
              </div>
            )}
          </section>
        </div>

        <Routes>
          <Route path="/" element={<Navigate to="/select-net" replace />} />
          <Route
            path="/select-net"
            element={
              <SelectNetsPage config={config} busyNet={busyNet} onSelect={selectNet} />
            }
          />
          <Route
            path="/connections"
            element={
              <ConnectionsPage
                connections={connections}
                busyConnectionId={busyConnectionId}
                closingAll={closingAll}
                onCloseConnection={closeConnection}
                onCloseAll={closeAllConnections}
              />
            }
          />
          <Route
            path="/logs"
            element={<LogsPage logs={logs} onClearLogs={clearLogs} />}
          />
          <Route path="*" element={<Navigate to="/select-net" replace />} />
        </Routes>
      </main>
    </div>
  )
}

function App() {
  return (
    <BrowserRouter>
      <Shell />
    </BrowserRouter>
  )
}

export default App
