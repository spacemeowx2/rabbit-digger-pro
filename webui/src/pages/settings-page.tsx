import { useCallback, useEffect, useState } from 'react'

import { rdpApi } from '../api'
import { classNames } from '../utils'

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface Subscription {
  name: string
  url: string
  /** Update interval in minutes */
  interval: number
  updatedAt?: string
}

interface DaemonSettings {
  subscriptions: Subscription[]
  port: number
  tunEnabled: boolean
  tunIp: string
  tunMtu: number
}

const DEFAULT_SETTINGS: DaemonSettings = {
  subscriptions: [],
  port: 10800,
  tunEnabled: false,
  tunIp: '192.168.233.1/24',
  tunMtu: 1400,
}

const SETTINGS_KEY = 'daemon/settings'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function extractDomain(url: string): string {
  try {
    return new URL(url).hostname
  } catch {
    return url
  }
}

function formatRelativeTime(dateStr?: string): string {
  if (!dateStr) return ''
  try {
    const diff = Date.now() - new Date(dateStr).getTime()
    const minutes = Math.floor(diff / 60000)
    if (minutes < 1) return '刚刚'
    if (minutes < 60) return `${minutes} 分钟前`
    const hours = Math.floor(minutes / 60)
    if (hours < 24) return `${hours} 小时前`
    const days = Math.floor(hours / 24)
    if (days < 30) return `${days} 天前`
    return `${Math.floor(days / 30)} 个月前`
  } catch {
    return ''
  }
}

function buildConfigText(settings: DaemonSettings): string {
  const net: Record<string, unknown> = { local: { type: 'local' } }
  const server: Record<string, unknown> = {}
  const imports: unknown[] = []

  for (const [i, sub] of settings.subscriptions.entries()) {
    const selectName = i === 0 ? 'proxy' : `proxy-${i}`
    imports.push({
      type: 'clash',
      source: {
        poll: { url: sub.url, interval: sub.interval * 60 },
      },
      select: selectName,
    })
  }

  if (settings.subscriptions.length === 0) {
    net['proxy'] = { type: 'alias', net: 'local' }
  }

  const outboundNet = 'proxy'

  if (settings.tunEnabled) {
    net['raw-gateway'] = {
      type: 'raw',
      device: { tun: 'rdp-tun0' },
      ip_addr: settings.tunIp,
      mtu: settings.tunMtu,
    }
    net['resolve'] = { type: 'resolve', net: outboundNet, resolve_net: 'local', ipv6: false }
    server['tun-forward'] = {
      type: 'forward',
      bind: '0.0.0.0:0',
      listen: 'raw-gateway',
      net: 'resolve',
      tcp: true,
      udp: true,
    }
  }

  server['mixed'] = {
    type: 'http+socks5',
    bind: `127.0.0.1:${settings.port}`,
    net: outboundNet,
    listen: 'local',
  }

  return JSON.stringify({ id: 'daemon', net, server, import: imports })
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

function RefreshIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className={className}>
      <path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" />
      <path d="M21 3v5h-5" />
    </svg>
  )
}

function TrashIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className={className}>
      <path d="M3 6h18" /><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" /><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
    </svg>
  )
}

// ---------------------------------------------------------------------------
// Edit Dialog
// ---------------------------------------------------------------------------

interface EditDialogProps {
  subscription: Subscription
  onSave: (sub: Subscription) => void
  onCancel: () => void
}

function EditDialog({ subscription, onSave, onCancel }: EditDialogProps) {
  const [draft, setDraft] = useState<Subscription>({ ...subscription })

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm" onClick={onCancel}>
      <div className="surface w-full max-w-md mx-4 p-5 space-y-4" onClick={(e) => e.stopPropagation()}>
        <h2 className="font-display text-base font-semibold text-slate-900">编辑订阅</h2>

        <label className="block">
          <span className="text-xs font-medium text-slate-500">名称</span>
          <input
            className="field mt-1"
            value={draft.name}
            onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
          />
        </label>

        <label className="block">
          <span className="text-xs font-medium text-slate-500">订阅链接</span>
          <textarea
            className="field mt-1 resize-none"
            rows={3}
            value={draft.url}
            onChange={(e) => setDraft((d) => ({ ...d, url: e.target.value }))}
          />
        </label>

        <label className="block">
          <span className="text-xs font-medium text-slate-500">更新间隔</span>
          <div className="flex items-center gap-2 mt-1">
            <input
              className="field w-28"
              type="number"
              min={1}
              value={draft.interval}
              onChange={(e) => {
                const v = parseInt(e.target.value, 10)
                if (v > 0) setDraft((d) => ({ ...d, interval: v }))
              }}
            />
            <span className="text-sm text-slate-500">分钟</span>
          </div>
        </label>

        <div className="flex justify-end gap-2 pt-2">
          <button type="button" className="action-button" onClick={onCancel}>
            取消
          </button>
          <button
            type="button"
            className="action-button bg-sky-600 text-white border-sky-600 hover:bg-sky-700"
            onClick={() => onSave({ ...draft, url: draft.url.trim() })}
            disabled={!draft.url.trim()}
          >
            保存
          </button>
        </div>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Settings Page
// ---------------------------------------------------------------------------

interface SettingsPageProps {
  runtimeState: string
  onRefreshConfig: () => void
}

export function SettingsPage({ runtimeState, onRefreshConfig }: SettingsPageProps) {
  const [settings, setSettings] = useState<DaemonSettings>(DEFAULT_SETTINGS)
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)
  const [starting, setStarting] = useState(false)
  const [stopping, setStopping] = useState(false)
  const [message, setMessage] = useState<{ type: 'ok' | 'err'; text: string } | null>(null)
  const [newSubUrl, setNewSubUrl] = useState('')
  const [refreshingIndex, setRefreshingIndex] = useState<number | null>(null)
  const [editingIndex, setEditingIndex] = useState<number | null>(null)

  const isRunning = runtimeState === 'Running'

  useEffect(() => {
    rdpApi
      .getUserdata<{ content: string }>(SETTINGS_KEY)
      .then((data) => {
        try {
          const parsed = JSON.parse(data.content) as DaemonSettings
          setSettings({ ...DEFAULT_SETTINGS, ...parsed })
        } catch { /* use defaults */ }
      })
      .catch(() => {})
      .finally(() => setLoaded(true))
  }, [])

  const saveSettings = useCallback(async (next: DaemonSettings) => {
    setSaving(true)
    try {
      await rdpApi.putUserdata(SETTINGS_KEY, JSON.stringify(next))
      setSettings(next)
    } catch (e) {
      setMessage({ type: 'err', text: `保存失败: ${e instanceof Error ? e.message : e}` })
    } finally {
      setSaving(false)
    }
  }, [])

  const applyAndStart = useCallback(async () => {
    setStarting(true)
    setMessage(null)
    try {
      await rdpApi.putUserdata(SETTINGS_KEY, JSON.stringify(settings))
      await rdpApi.applyConfig({ text: buildConfigText(settings) })
      onRefreshConfig()
      setMessage({ type: 'ok', text: '已启动' })
    } catch (e) {
      setMessage({ type: 'err', text: `启动失败: ${e instanceof Error ? e.message : e}` })
    } finally {
      setStarting(false)
    }
  }, [settings, onRefreshConfig])

  const stopEngine = useCallback(async () => {
    setStopping(true)
    setMessage(null)
    try {
      await rdpApi.engineStop()
      setMessage({ type: 'ok', text: '已停止' })
    } catch (e) {
      setMessage({ type: 'err', text: `停止失败: ${e instanceof Error ? e.message : e}` })
    } finally {
      setStopping(false)
    }
  }, [])

  const addSubscription = useCallback(() => {
    const url = newSubUrl.trim()
    if (!url) return
    const next: DaemonSettings = {
      ...settings,
      subscriptions: [
        ...settings.subscriptions,
        { name: extractDomain(url), url, interval: 60, updatedAt: new Date().toISOString() },
      ],
    }
    setNewSubUrl('')
    void saveSettings(next)
  }, [newSubUrl, settings, saveSettings])

  const updateSubscription = useCallback(
    (index: number, sub: Subscription) => {
      const next = {
        ...settings,
        subscriptions: settings.subscriptions.map((s, i) => (i === index ? sub : s)),
      }
      setEditingIndex(null)
      void saveSettings(next)
    },
    [settings, saveSettings],
  )

  const removeSubscription = useCallback(
    (index: number) => {
      const next = {
        ...settings,
        subscriptions: settings.subscriptions.filter((_, i) => i !== index),
      }
      void saveSettings(next)
    },
    [settings, saveSettings],
  )

  const refreshSubscription = useCallback(
    async (index: number) => {
      setRefreshingIndex(index)
      try {
        const next = {
          ...settings,
          subscriptions: settings.subscriptions.map((sub, i) =>
            i === index ? { ...sub, updatedAt: new Date().toISOString() } : sub,
          ),
        }
        await saveSettings(next)
        if (isRunning) {
          await rdpApi.applyConfig({ text: buildConfigText(next) })
          onRefreshConfig()
        }
      } finally {
        setRefreshingIndex(null)
      }
    },
    [settings, saveSettings, isRunning, onRefreshConfig],
  )

  if (!loaded) {
    return (
      <div className="flex items-center justify-center py-20">
        <span className="text-sm text-slate-500">Loading...</span>
      </div>
    )
  }

  const isFirstTime = settings.subscriptions.length === 0 && !isRunning

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h1 className="font-display text-lg font-semibold text-slate-900">
          {isFirstTime ? '欢迎使用 RDP' : '设置'}
        </h1>
        <div className="flex items-center gap-2">
          <span className={classNames('status-dot', isRunning ? 'bg-emerald-500' : 'bg-amber-500')} />
          <span className="text-sm text-slate-600">{isRunning ? '运行中' : runtimeState}</span>
        </div>
      </div>

      {message && (
        <div className={classNames(
          'rounded-lg border px-3 py-2 text-sm',
          message.type === 'ok'
            ? 'border-emerald-200 bg-emerald-50 text-emerald-700'
            : 'border-rose-200 bg-rose-50 text-rose-700',
        )}>
          {message.text}
        </div>
      )}

      {isFirstTime && (
        <div className="surface px-5 py-4">
          <p className="text-sm text-slate-600">添加你的第一个代理订阅开始使用。</p>
        </div>
      )}

      {/* ── Subscriptions ── */}
      <section className="surface overflow-hidden">
        <div className="px-4 py-3 border-b border-slate-200/60">
          <h2 className="font-display text-sm font-semibold text-slate-900">订阅管理</h2>
        </div>
        <div className="px-4 py-3 space-y-3">
          {/* URL input */}
          <div className="flex gap-2">
            <input
              className="field flex-1"
              placeholder="订阅文件链接"
              value={newSubUrl}
              onChange={(e) => setNewSubUrl(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') addSubscription() }}
            />
            <button
              type="button"
              className="action-button shrink-0"
              onClick={addSubscription}
              disabled={!newSubUrl.trim() || saving}
            >
              导入
            </button>
          </div>

          {/* Cards */}
          {settings.subscriptions.length > 0 && (
            <div className="grid gap-3 grid-cols-1 sm:grid-cols-2 lg:grid-cols-3">
              {settings.subscriptions.map((sub, index) => (
                <div
                  key={index}
                  className="group relative rounded-xl border border-slate-200/80 bg-white/70 p-3.5 transition hover:border-slate-300 hover:bg-white/90 cursor-pointer"
                  onClick={() => setEditingIndex(index)}
                >
                  {/* Header */}
                  <div className="flex items-start justify-between gap-2 mb-2">
                    <h3 className="text-sm font-semibold text-slate-900 truncate">{sub.name}</h3>
                    <div
                      className="flex items-center gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity"
                      onClick={(e) => e.stopPropagation()}
                    >
                      <button
                        type="button"
                        className="rounded-md p-1 text-slate-400 hover:text-sky-600 hover:bg-sky-50 transition"
                        onClick={() => void refreshSubscription(index)}
                        disabled={refreshingIndex === index}
                        title="刷新"
                      >
                        <RefreshIcon className={classNames('h-3.5 w-3.5', refreshingIndex === index && 'animate-spin')} />
                      </button>
                      <button
                        type="button"
                        className="rounded-md p-1 text-slate-400 hover:text-rose-600 hover:bg-rose-50 transition"
                        onClick={() => removeSubscription(index)}
                        title="删除"
                      >
                        <TrashIcon className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  </div>

                  {/* Domain */}
                  <p className="text-xs text-slate-500 truncate mb-2">{extractDomain(sub.url)}</p>

                  {/* Footer */}
                  <div className="flex items-center justify-between text-xs text-slate-400">
                    <span>{formatRelativeTime(sub.updatedAt)}</span>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </section>

      {/* ── Proxy settings ── */}
      <section className="surface overflow-hidden">
        <div className="px-4 py-3 border-b border-slate-200/60">
          <h2 className="font-display text-sm font-semibold text-slate-900">代理设置</h2>
        </div>
        <div className="px-4 py-3 space-y-3">
          <div className="flex items-center gap-3">
            <label className="text-sm text-slate-600 w-32 shrink-0">HTTP/SOCKS5 端口</label>
            <input
              className="field w-32"
              type="number"
              min={1}
              max={65535}
              value={settings.port}
              onChange={(e) => {
                const port = parseInt(e.target.value, 10)
                if (port > 0 && port <= 65535) setSettings((s) => ({ ...s, port }))
              }}
              onBlur={() => void saveSettings(settings)}
            />
            <span className="text-xs text-slate-400">127.0.0.1:{settings.port}</span>
          </div>
        </div>
      </section>

      {/* ── TUN mode ── */}
      <section className="surface overflow-hidden">
        <div className="px-4 py-3 border-b border-slate-200/60">
          <h2 className="font-display text-sm font-semibold text-slate-900">TUN 模式</h2>
        </div>
        <div className="px-4 py-3 space-y-3">
          <div className="flex items-center gap-3">
            <label className="text-sm text-slate-600 w-32 shrink-0">启用 TUN</label>
            <button
              type="button"
              className={classNames(
                'relative inline-flex h-6 w-11 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200',
                settings.tunEnabled ? 'bg-sky-600' : 'bg-slate-300',
              )}
              onClick={() => {
                const next = { ...settings, tunEnabled: !settings.tunEnabled }
                setSettings(next)
                void saveSettings(next)
              }}
            >
              <span className={classNames(
                'pointer-events-none inline-block h-5 w-5 rounded-full bg-white shadow-lg transition-transform duration-200',
                settings.tunEnabled ? 'translate-x-5' : 'translate-x-0',
              )} />
            </button>
          </div>
          {settings.tunEnabled && (
            <>
              <div className="flex items-center gap-3">
                <label className="text-sm text-slate-600 w-32 shrink-0">IP 地址</label>
                <input
                  className="field w-48"
                  value={settings.tunIp}
                  onChange={(e) => setSettings((s) => ({ ...s, tunIp: e.target.value }))}
                  onBlur={() => void saveSettings(settings)}
                  placeholder="192.168.233.1/24"
                />
              </div>
              <div className="flex items-center gap-3">
                <label className="text-sm text-slate-600 w-32 shrink-0">MTU</label>
                <input
                  className="field w-32"
                  type="number"
                  value={settings.tunMtu}
                  onChange={(e) => {
                    const mtu = parseInt(e.target.value, 10)
                    if (mtu > 0) setSettings((s) => ({ ...s, tunMtu: mtu }))
                  }}
                  onBlur={() => void saveSettings(settings)}
                />
              </div>
            </>
          )}
        </div>
      </section>

      {/* ── Engine control ── */}
      <section className="surface overflow-hidden">
        <div className="px-4 py-3 border-b border-slate-200/60">
          <h2 className="font-display text-sm font-semibold text-slate-900">引擎控制</h2>
        </div>
        <div className="px-4 py-3">
          <div className="flex items-center gap-2">
            <button
              type="button"
              className={classNames('action-button', !isRunning && 'bg-emerald-50 text-emerald-700 border-emerald-200')}
              onClick={applyAndStart}
              disabled={starting || saving}
            >
              {starting ? '启动中...' : isRunning ? '重新应用配置' : '启动'}
            </button>
            {isRunning && (
              <button
                type="button"
                className="action-button action-button-danger"
                onClick={stopEngine}
                disabled={stopping}
              >
                {stopping ? '停止中...' : '停止'}
              </button>
            )}
          </div>
        </div>
      </section>

      {/* ── Edit dialog ── */}
      {editingIndex !== null && settings.subscriptions[editingIndex] && (
        <EditDialog
          subscription={settings.subscriptions[editingIndex]}
          onSave={(sub) => updateSubscription(editingIndex, sub)}
          onCancel={() => setEditingIndex(null)}
        />
      )}
    </div>
  )
}
