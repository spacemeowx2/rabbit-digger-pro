import type { ConnectionEntry, LogEntry, NetConfig, RdpConfig } from './types'

const LOG_PATTERN =
  /^(?<time>\S+)\s+(?<level>TRACE|DEBUG|INFO|WARN|ERROR)\s+(?<message>.*)$/u

export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return '0 B'
  }

  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let size = value
  let unitIndex = 0

  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024
    unitIndex += 1
  }

  const digits = size >= 100 || unitIndex === 0 ? 0 : size >= 10 ? 1 : 2
  return `${size.toFixed(digits)} ${units[unitIndex]}`
}

export function formatRate(value: number): string {
  return `${formatBytes(value)}/s`
}

export function formatAge(startTime?: number): string {
  if (!startTime) {
    return 'just now'
  }

  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - startTime)
  if (seconds < 60) {
    return `${seconds}s`
  }
  if (seconds < 3600) {
    return `${Math.floor(seconds / 60)}m`
  }
  if (seconds < 86400) {
    return `${Math.floor(seconds / 3600)}h`
  }
  return `${Math.floor(seconds / 86400)}d`
}

export function formatAddress(value: unknown): string {
  if (typeof value === 'string') {
    return value
  }

  if (Array.isArray(value)) {
    return value.map((item) => formatAddress(item)).join(' / ')
  }

  if (value && typeof value === 'object') {
    const record = value as Record<string, unknown>
    if (typeof record.domain === 'string' && typeof record.port === 'number') {
      return `${record.domain}:${record.port}`
    }
    if (typeof record.ip === 'string' && typeof record.port === 'number') {
      return `${record.ip}:${record.port}`
    }
    if (typeof record.host === 'string' && typeof record.port === 'number') {
      return `${record.host}:${record.port}`
    }
    return Object.entries(record)
      .map(([key, nested]) => `${key}=${formatAddress(nested)}`)
      .join(' ')
  }

  return String(value ?? '')
}

export function summarizeConnection(connection: ConnectionEntry): {
  host: string
  route: string
  source: string
} {
  const ctx = connection.ctx ?? {}
  const host =
    typeof ctx.dest_domain === 'string'
      ? ctx.dest_domain
      : formatAddress(connection.addr) || 'unknown target'

  const routeValue = ctx.net_list
  const route = Array.isArray(routeValue)
    ? routeValue.map((item) => formatAddress(item)).join(' / ')
    : typeof routeValue === 'string'
      ? routeValue
      : 'runtime path unavailable'

  const source =
    typeof ctx.src_socket_addr === 'string' ? ctx.src_socket_addr : 'unknown source'

  return { host, route, source }
}

export function parseLogChunk(chunk: string): LogEntry[] {
  return chunk
    .split('\n')
    .map((rawLine) => rawLine.trimEnd())
    .filter(Boolean)
    .map((rawLine, index) => {
      if (rawLine.startsWith('{')) {
        try {
          const parsed = JSON.parse(rawLine) as {
            timestamp?: string
            level?: string
            message?: string
            fields?: {
              message?: string
            }
          }

          return {
            id: `${Date.now()}-${index}-${rawLine.length}`,
            time: parsed.timestamp ?? null,
            level: parsed.level ?? 'LOG',
            message: parsed.fields?.message ?? parsed.message ?? rawLine,
            raw: rawLine,
          }
        } catch {
          // Fall through to the plain-text parser below.
        }
      }

      const match = rawLine.match(LOG_PATTERN)
      const time = match?.groups?.time ?? null
      const level = match?.groups?.level ?? 'LOG'
      const message = match?.groups?.message ?? rawLine

      return {
        id: `${Date.now()}-${index}-${rawLine.length}`,
        time,
        level,
        message,
        raw: rawLine,
      }
    })
}

export function getSelectGroups(config: RdpConfig | null): Array<[string, NetConfig]> {
  if (!config) {
    return []
  }

  return Object.entries(config.net)
    .filter(([, net]) => net.type === 'select')
    .sort(([leftName], [rightName]) => {
      if (leftName === 'select-net') {
        return 1
      }
      if (rightName === 'select-net') {
        return -1
      }
      return leftName.localeCompare(rightName, 'zh-Hans-CN')
    })
}

export function describeNet(netName: string, net: NetConfig | undefined): string {
  if (!net) {
    return 'No runtime metadata'
  }

  const facts: string[] = []

  if (typeof net.server === 'string') {
    facts.push(net.server)
  }

  if (typeof net.net === 'string') {
    facts.push(`via ${net.net}`)
  }

  if (typeof net.sni === 'string') {
    facts.push(`SNI ${net.sni}`)
  }

  if (Array.isArray(net.list)) {
    facts.push(`${net.list.length} options`)
  }

  return facts[0] ?? `${net.type.toUpperCase()} · ${netName}`
}

export function updateSelectedNet(
  config: RdpConfig | null,
  netName: string,
  selected: string,
): RdpConfig | null {
  if (!config) {
    return config
  }

  const nextNet = config.net[netName]
  if (!nextNet || nextNet.type !== 'select') {
    return config
  }

  return {
    ...config,
    net: {
      ...config.net,
      [netName]: {
        ...nextNet,
        selected,
      },
    },
  }
}

export function classNames(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ')
}
