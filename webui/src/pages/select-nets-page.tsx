import { useDeferredValue, useMemo, useState } from 'react'

import { rdpApi } from '../api'
import type { NetConfig, RdpConfig } from '../types'
import { classNames, getSelectGroups } from '../utils'

const DELAY_TEST_URL = 'http://www.gstatic.com/generate_204'
const DELAY_TIMEOUT_MS = 5000

type DelayState =
  | { status: 'testing' }
  | { status: 'done'; response: number; testedAt: number }
  | { status: 'timeout'; testedAt: number }
  | { status: 'error'; testedAt: number }

interface SelectNetsPageProps {
  config: RdpConfig | null
  busyNet: string | null
  onSelect: (netName: string, selected: string) => Promise<void>
}

function getDelayLabel(state?: DelayState): string | null {
  if (!state) return null
  switch (state.status) {
    case 'testing':
      return '...'
    case 'done':
      return `${state.response}ms`
    case 'timeout':
      return 'Timeout'
    case 'error':
      return 'Err'
  }
}

function getDelayColor(state?: DelayState): string {
  if (!state) return 'text-slate-400'
  switch (state.status) {
    case 'testing':
      return 'text-slate-500'
    case 'done':
      if (state.response <= 200) return 'text-emerald-600'
      if (state.response <= 500) return 'text-amber-600'
      return 'text-rose-600'
    case 'timeout':
    case 'error':
      return 'text-rose-500'
  }
}

function getGroupBadge(net: NetConfig): { label: string; className: string } {
  switch (net.type) {
    case 'url-test':
      return { label: 'URL Test', className: 'bg-emerald-50 text-emerald-700' }
    case 'fallback':
      return { label: 'Fallback', className: 'bg-amber-50 text-amber-700' }
    default:
      return { label: 'Selector', className: 'bg-sky-50 text-sky-600' }
  }
}

function getGroupMeta(net: NetConfig): string | null {
  const facts: string[] = []
  if (typeof net.url === 'string' && net.url) facts.push(net.url)
  if (typeof net.interval === 'number') facts.push(`每 ${net.interval}s 检查`)
  if (typeof net.tolerance === 'number' && net.type === 'url-test') {
    facts.push(`容差 ${net.tolerance}ms`)
  }
  return facts.length > 0 ? facts.join(' · ') : null
}

export function SelectNetsPage({
  config,
  busyNet,
  onSelect,
}: SelectNetsPageProps) {
  const [query, setQuery] = useState('')
  const [expandedGroups, setExpandedGroups] = useState<Record<string, boolean>>({})
  const [delayResults, setDelayResults] = useState<Record<string, DelayState>>({})
  const [activeDelayScope, setActiveDelayScope] = useState<string | null>(null)
  const deferredQuery = useDeferredValue(query.trim().toLowerCase())

  const groups = useMemo(() => {
    const allGroups = getSelectGroups(config)
    if (!deferredQuery) return allGroups

    return allGroups
      .map(([groupName, net]) => {
        const list = net.list ?? []
        const nextList = list.filter((option) =>
          `${groupName} ${option}`.toLowerCase().includes(deferredQuery),
        )
        return [groupName, { ...net, list: nextList }] as const
      })
      .filter(([, net]) => (net.list?.length ?? 0) > 0)
  }, [config, deferredQuery])

  const visibleOptions = useMemo(() => {
    return [...new Set(groups.flatMap(([, net]) => net.list ?? []))]
  }, [groups])

  async function runDelayTest(targets: string[], scope: string) {
    if (activeDelayScope) return
    const uniqueTargets = [...new Set(targets)]
    if (uniqueTargets.length === 0) return

    setActiveDelayScope(scope)
    setDelayResults((current) => {
      const next = { ...current }
      uniqueTargets.forEach((t) => { next[t] = { status: 'testing' } })
      return next
    })

    try {
      const queue = [...uniqueTargets]
      const workerCount = Math.min(6, queue.length)
      await Promise.all(
        Array.from({ length: workerCount }, async () => {
          while (queue.length > 0) {
            const target = queue.shift()
            if (!target) return
            try {
              const result = await rdpApi.getNetDelay(target, DELAY_TEST_URL, DELAY_TIMEOUT_MS)
              setDelayResults((c) => ({
                ...c,
                [target]: result
                  ? { status: 'done', response: result.response, testedAt: Date.now() }
                  : { status: 'timeout', testedAt: Date.now() },
              }))
            } catch {
              setDelayResults((c) => ({
                ...c,
                [target]: { status: 'error', testedAt: Date.now() },
              }))
            }
          }
        }),
      )
    } finally {
      setActiveDelayScope((c) => (c === scope ? null : c))
    }
  }

  return (
    <div className="space-y-3">
      {/* toolbar */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <h1 className="font-display text-lg font-semibold text-slate-900">代理组</h1>
        <div className="flex items-center gap-2">
          <input
            className="field sm:w-64"
            placeholder="搜索节点..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <button
            type="button"
            className="action-button shrink-0"
            onClick={() => void runDelayTest(visibleOptions, 'all')}
            disabled={Boolean(activeDelayScope)}
          >
            {activeDelayScope === 'all' ? '测试中...' : '测速全部'}
          </button>
        </div>
      </div>

      {/* groups */}
      {groups.map(([groupName, net], index) => {
        const selected = net.selected
        const options = net.list ?? []
        const isExpanded = deferredQuery ? true : expandedGroups[groupName] ?? index === 0
        const badge = getGroupBadge(net)
        const groupMeta = getGroupMeta(net)
        const isManualGroup = net.type === 'select'

        return (
          <section key={groupName} className="surface overflow-hidden">
            {/* group header */}
            <button
              type="button"
              className="flex w-full items-center gap-3 px-4 py-2.5 text-left hover:bg-white/40 transition"
              onClick={() =>
                setExpandedGroups((c) => ({ ...c, [groupName]: !isExpanded }))
              }
            >
              <h2 className="font-display text-sm font-semibold text-slate-900 truncate">
                {groupName}
              </h2>
              <span className={classNames('tag', badge.className)}>{badge.label}</span>
              <span className="text-xs text-slate-500 truncate">
                {selected ?? '未选择'}
              </span>
              {groupMeta && <span className="hidden text-xs text-slate-400 xl:block truncate">{groupMeta}</span>}
              <span className="ml-auto text-xs font-medium text-slate-400">{options.length}</span>
              <svg
                className={classNames(
                  'h-4 w-4 text-slate-400 transition-transform',
                  isExpanded && 'rotate-180',
                )}
                viewBox="0 0 20 20"
                fill="currentColor"
              >
                <path
                  fillRule="evenodd"
                  d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z"
                  clipRule="evenodd"
                />
              </svg>
            </button>

            {isExpanded && (
              <div className="border-t border-slate-200/60 px-4 py-3">
                <div className="flex items-center gap-2 mb-3">
                  <button
                    type="button"
                    className="action-button text-xs"
                    onClick={() => void runDelayTest(options, groupName)}
                    disabled={Boolean(activeDelayScope)}
                  >
                    {activeDelayScope === groupName ? '测试中...' : '测速本组'}
                  </button>
                  {!isManualGroup && (
                    <span className="text-xs text-slate-500">
                      自动组，当前展示的是运行时选中结果
                    </span>
                  )}
                </div>

                <div className="grid gap-2 grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
                  {options.map((optionName) => {
                    const isSelected = selected === optionName
                    const isBusy = busyNet === groupName
                    const delayState = delayResults[optionName]
                    const delayLabel = getDelayLabel(delayState)
                    const canSelect = isManualGroup

                    return (
                      <button
                        key={optionName}
                        type="button"
                        className={classNames(
                          'rounded-lg border px-3 py-2 text-left transition duration-150',
                          isSelected
                            ? 'border-sky-400 bg-sky-600 text-white shadow-sm'
                            : 'border-slate-200 bg-white/60 hover:bg-white hover:border-slate-300',
                          (isBusy || !canSelect) && 'cursor-not-allowed',
                          isBusy && 'opacity-60',
                          !canSelect && 'opacity-80',
                        )}
                        onClick={() => {
                          if (canSelect) void onSelect(groupName, optionName)
                        }}
                        disabled={isBusy || !canSelect}
                      >
                        <p className="truncate text-sm font-medium">{optionName}</p>
                        <div className="mt-1 flex items-center justify-between">
                          <span
                            className={classNames(
                              'text-xs',
                              isSelected ? 'text-sky-100' : getDelayColor(delayState),
                            )}
                          >
                            {delayLabel ?? '\u00A0'}
                          </span>
                        </div>
                      </button>
                    )
                  })}
                </div>
              </div>
            )}
          </section>
        )
      })}

      {groups.length === 0 && (
        <div className="surface px-6 py-10 text-center">
          <p className="text-sm font-medium text-slate-500">
            没有匹配到可切换的 selector。
          </p>
        </div>
      )}
    </div>
  )
}
