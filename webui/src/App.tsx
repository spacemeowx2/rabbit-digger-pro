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
import { classNames, formatBytes, formatRate } from './utils'

const navigation = [
  {
    to: '/select-net',
    label: '代理',
    icon: LayersIcon,
  },
  {
    to: '/connections',
    label: '连接',
    icon: LinkIcon,
  },
  {
    to: '/logs',
    label: '日志',
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

  const latestTraffic = trafficHistory[trafficHistory.length - 1]

  return (
    <div className="mx-auto min-h-screen lg:grid lg:grid-cols-[12rem_minmax(0,1fr)]">
      <aside className="border-b border-white/70 bg-white/55 px-3 py-3 backdrop-blur-xl lg:border-b-0 lg:border-r lg:px-3 lg:py-4 lg:flex lg:flex-col">
        <div className="flex items-center justify-between lg:block">
          <div className="flex items-center gap-2.5 px-1">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-slate-900 text-sm font-bold text-white">
              R
            </div>
            <span className="font-display text-sm font-semibold tracking-[-0.02em] text-slate-900">
              RDP
            </span>
            <span
              className={classNames(
                'status-dot ml-auto',
                runtimeState === 'Running' ? 'bg-emerald-500' : 'bg-amber-500',
              )}
            />
          </div>
        </div>

        <nav className="mt-3 grid grid-cols-3 gap-1 lg:mt-4 lg:flex lg:grid-cols-none lg:flex-col lg:gap-0.5">
          {navigation.map((item) => {
            const Icon = item.icon
            return (
              <NavLink
                key={item.to}
                to={item.to}
                className={({ isActive }) =>
                  classNames(
                    'group flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm transition duration-150',
                    isActive
                      ? 'bg-slate-900 text-white'
                      : 'text-slate-600 hover:bg-white/70 hover:text-slate-900',
                  )
                }
              >
                <Icon className="h-4 w-4 shrink-0" />
                <span className="font-medium">{item.label}</span>
              </NavLink>
            )
          })}
        </nav>

        <section className="mt-auto hidden pt-4 lg:block">
          <div className="surface-muted p-3">
            <div className="flex items-center justify-between mb-2">
              <span className="text-xs font-semibold text-slate-500">Traffic</span>
              <PulseIcon className="h-3.5 w-3.5 text-slate-400" />
            </div>

            <SidebarSparkline history={trafficHistory} />

            <div className="mt-2.5 space-y-1.5 text-xs">
              <div className="flex items-center justify-between text-amber-600">
                <span>↑</span>
                <strong>{formatRate(latestTraffic?.uploadRate ?? 0)}</strong>
              </div>
              <div className="flex items-center justify-between text-sky-600">
                <span>↓</span>
                <strong>{formatRate(latestTraffic?.downloadRate ?? 0)}</strong>
              </div>
              <div className="flex items-center justify-between text-slate-400">
                <span>Total</span>
                <strong>{formatBytes(connections.total_download + connections.total_upload)}</strong>
              </div>
            </div>
          </div>
        </section>
      </aside>

      <main className="min-w-0 px-4 py-3 md:px-5 lg:px-6 lg:py-4">
        {error && (
          <div className="mb-3 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-sm text-rose-700">
            {error}
          </div>
        )}

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
