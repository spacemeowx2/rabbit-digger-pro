import { useDeferredValue, useMemo, useState } from 'react'

import { rdpApi } from '../api'
import type { RdpConfig } from '../types'
import { classNames, describeNet, getSelectGroups } from '../utils'

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
  if (!state) {
    return null
  }

  switch (state.status) {
    case 'testing':
      return 'Testing...'
    case 'done':
      return `${state.response} ms`
    case 'timeout':
      return 'Timeout'
    case 'error':
      return 'Failed'
  }
}

function getDelayTone(state: DelayState | undefined, isSelected: boolean): string {
  if (isSelected) {
    return 'bg-white/12 text-white'
  }

  if (!state) {
    return 'bg-slate-100 text-slate-400'
  }

  switch (state.status) {
    case 'testing':
      return 'bg-slate-900 text-white'
    case 'done':
      if (state.response <= 200) {
        return 'bg-emerald-50 text-emerald-700'
      }
      if (state.response <= 500) {
        return 'bg-amber-50 text-amber-700'
      }
      return 'bg-rose-50 text-rose-700'
    case 'timeout':
    case 'error':
      return 'bg-rose-50 text-rose-700'
  }
}

function getBestDelay(options: string[], delayResults: Record<string, DelayState>): number | null {
  const latencies = options.flatMap((optionName) => {
    const result = delayResults[optionName]
    return result?.status === 'done' ? [result.response] : []
  })

  return latencies.length > 0 ? Math.min(...latencies) : null
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
    if (!deferredQuery) {
      return allGroups
    }

    return allGroups
      .map(([groupName, net]) => {
        const list = net.list ?? []
        const nextList = list.filter((option) => {
          const candidate = `${groupName} ${option} ${describeNet(option, config?.net[option])}`
          return candidate.toLowerCase().includes(deferredQuery)
        })

        return [groupName, { ...net, list: nextList }] as const
      })
      .filter(([, net]) => (net.list?.length ?? 0) > 0)
  }, [config, deferredQuery])

  const visibleOptions = useMemo(() => {
    return [...new Set(groups.flatMap(([, net]) => net.list ?? []))]
  }, [groups])

  const allExpanded =
    !deferredQuery &&
    groups.length > 0 &&
    groups.every(([groupName], index) => expandedGroups[groupName] ?? index === 0)

  async function runDelayTest(targets: string[], scope: string) {
    if (activeDelayScope) {
      return
    }

    const uniqueTargets = [...new Set(targets)]
    if (uniqueTargets.length === 0) {
      return
    }

    setActiveDelayScope(scope)
    setDelayResults((current) => {
      const next = { ...current }
      uniqueTargets.forEach((target) => {
        next[target] = { status: 'testing' }
      })
      return next
    })

    try {
      const queue = [...uniqueTargets]
      const workerCount = Math.min(6, queue.length)

      await Promise.all(
        Array.from({ length: workerCount }, async () => {
          while (queue.length > 0) {
            const target = queue.shift()
            if (!target) {
              return
            }

            try {
              const result = await rdpApi.getNetDelay(target, DELAY_TEST_URL, DELAY_TIMEOUT_MS)
              setDelayResults((current) => ({
                ...current,
                [target]: result
                  ? {
                      status: 'done',
                      response: result.response,
                      testedAt: Date.now(),
                    }
                  : {
                      status: 'timeout',
                      testedAt: Date.now(),
                    },
              }))
            } catch {
              setDelayResults((current) => ({
                ...current,
                [target]: {
                  status: 'error',
                  testedAt: Date.now(),
                },
              }))
            }
          }
        }),
      )
    } finally {
      setActiveDelayScope((current) => (current === scope ? null : current))
    }
  }

  return (
    <div className="space-y-5">
      <section className="surface p-5 md:p-6">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
          <div className="space-y-2">
            <p className="eyebrow">Select Net</p>
            <h1 className="font-display text-3xl font-semibold tracking-[-0.04em] text-slate-900 md:text-4xl">
              代理组
            </h1>
            <p className="max-w-3xl text-sm leading-6 text-slate-600 md:text-base">
              查看 selector net，并直接切换当前出口。
            </p>
          </div>

          <div className="flex w-full max-w-2xl flex-col gap-3">
            <label className="sr-only" htmlFor="select-net-search">
              搜索策略组
            </label>
            <input
              id="select-net-search"
              className="field"
              placeholder="搜索代理组、节点名称或目标地址"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />

            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                className="action-button shrink-0"
                onClick={() => void runDelayTest(visibleOptions, 'all')}
                disabled={Boolean(activeDelayScope)}
              >
                {activeDelayScope === 'all' ? '测试中...' : '测试全部延迟'}
              </button>
              {!deferredQuery && groups.length > 1 && (
                <button
                  type="button"
                  className="action-button shrink-0"
                  onClick={() => {
                    setExpandedGroups(
                      Object.fromEntries(
                        groups.map(([groupName]) => [groupName, !allExpanded]),
                      ),
                    )
                  }}
                >
                  {allExpanded ? '收起全部' : '展开全部'}
                </button>
              )}
            </div>
          </div>
        </div>
      </section>

      {groups.map(([groupName, net], index) => {
        const selected = net.selected
        const options = net.list ?? []
        const isExpanded = deferredQuery ? true : expandedGroups[groupName] ?? index === 0
        const bestDelay = getBestDelay(options, delayResults)
        const preview = options.slice(0, 3).join(' · ')

        return (
          <section key={groupName} className="surface p-5 md:p-6">
            <div
              className={classNames(
                'flex flex-col gap-4',
                isExpanded && 'border-b border-slate-200/70 pb-5',
              )}
            >
              <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
                <div className="min-w-0 space-y-3">
                  <div className="flex flex-wrap items-center gap-2">
                    <h2 className="font-display text-2xl font-semibold tracking-[-0.04em] text-slate-900">
                      {groupName}
                    </h2>
                    <span className="tag text-slate-500">selector</span>
                    <span className="tag text-slate-500">{options.length} options</span>
                    {bestDelay !== null && (
                      <span className="tag bg-emerald-50 text-emerald-700">
                        fastest {bestDelay} ms
                      </span>
                    )}
                  </div>

                  <div className="flex flex-wrap items-center gap-2 text-sm leading-6 text-slate-600">
                    <span>当前出口</span>
                    <span className="rounded-full bg-slate-900 px-3 py-1 text-xs font-semibold text-white">
                      {selected ?? '未选择'}
                    </span>
                    {!isExpanded && preview && (
                      <span className="text-slate-400">
                        候选: {preview}
                        {options.length > 3 ? ` 等 ${options.length} 项` : ''}
                      </span>
                    )}
                  </div>
                </div>

                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    className="action-button shrink-0"
                    onClick={() => void runDelayTest(options, groupName)}
                    disabled={Boolean(activeDelayScope)}
                  >
                    {activeDelayScope === groupName ? '测试中...' : '测试本组'}
                  </button>
                  <button
                    type="button"
                    className="action-button shrink-0"
                    onClick={() => {
                      setExpandedGroups((current) => ({
                        ...current,
                        [groupName]: !isExpanded,
                      }))
                    }}
                    aria-expanded={isExpanded}
                  >
                    {isExpanded ? '收起' : '展开'}
                  </button>
                </div>
              </div>

              {isExpanded && (
                <div className="mt-1 grid gap-3 md:grid-cols-2 2xl:grid-cols-3">
                  {options.map((optionName) => {
                    const optionNet = config?.net[optionName]
                    const isSelected = selected === optionName
                    const isBusy = busyNet === groupName
                    const delayState = delayResults[optionName]
                    const delayLabel = getDelayLabel(delayState)

                    return (
                      <button
                        key={optionName}
                        type="button"
                        className={classNames(
                          'group rounded-[24px] border p-4 text-left transition duration-200',
                          isSelected
                            ? 'border-sky-400/70 bg-sky-600 text-white shadow-[0_24px_60px_-36px_rgba(14,116,144,0.65)]'
                            : 'border-slate-200 bg-white/75 hover:-translate-y-0.5 hover:border-slate-300 hover:bg-white',
                          isBusy && 'cursor-wait opacity-70',
                        )}
                        onClick={() => void onSelect(groupName, optionName)}
                        disabled={isBusy}
                        aria-label={`切换 ${groupName} 到 ${optionName}`}
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0">
                            <p className="truncate text-base font-semibold">{optionName}</p>
                            <p
                              className={classNames(
                                'mt-2 text-sm leading-5',
                                isSelected ? 'text-sky-100' : 'text-slate-500',
                              )}
                            >
                              {describeNet(optionName, optionNet)}
                            </p>
                          </div>
                          <span
                            className={classNames(
                              'mt-1 inline-flex h-6 min-w-6 items-center justify-center rounded-full border px-2 text-xs font-semibold',
                              isSelected
                                ? 'border-white/25 bg-white/10 text-white'
                                : 'border-slate-200 bg-slate-100 text-slate-500',
                            )}
                          >
                            {optionNet?.type?.toUpperCase() ?? 'NET'}
                          </span>
                        </div>

                        <div className="mt-4 flex items-center justify-between gap-3 text-xs font-medium">
                          <span
                            className={classNames(
                              'rounded-full px-3 py-1',
                              isSelected
                                ? 'bg-white/12 text-white'
                                : 'bg-slate-100 text-slate-500',
                            )}
                          >
                            {isSelected ? '当前已生效' : '点击切换'}
                          </span>
                          <span
                            className={classNames(
                              'rounded-full px-3 py-1',
                              getDelayTone(delayState, isSelected),
                            )}
                          >
                            {isBusy
                              ? 'Updating...'
                              : delayLabel ?? (isSelected ? 'Selected' : 'Use')}
                          </span>
                        </div>
                      </button>
                    )
                  })}
                </div>
              )}
            </div>
          </section>
        )
      })}

      {groups.length === 0 && (
        <section className="surface p-8 text-center">
          <p className="font-display text-xl font-semibold text-slate-900">
            没有匹配到可切换的 selector。
          </p>
          <p className="mt-2 text-sm text-slate-500">
            试着缩短搜索词，或者确认当前配置已经成功加载。
          </p>
        </section>
      )}
    </div>
  )
}
