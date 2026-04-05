import { create } from 'zustand'
import { applyPatch, type Operation } from 'fast-json-patch'

import { getWebSocketUrl, rdpApi } from './api'
import type { ConnectionSnapshot, EngineStatus, LogEntry, TrafficSample } from './types'
import { parseLogChunk } from './utils'

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

const EMPTY_CONNECTIONS: ConnectionSnapshot = {
  connections: {},
  total_upload: 0,
  total_download: 0,
}

interface RdpStore {
  // Engine status (SSE-driven)
  engineStatus: EngineStatus

  // Real-time data (WebSocket-driven)
  connections: ConnectionSnapshot
  logs: LogEntry[]
  trafficHistory: TrafficSample[]

  // UI state
  error: string | null
  busyNet: string | null
  busyConnectionId: string | null
  closingAll: boolean

  // Actions
  setError: (error: string | null) => void
  setBusyNet: (net: string | null) => void
  setBusyConnectionId: (id: string | null) => void
  setClosingAll: (closing: boolean) => void
  clearLogs: () => void
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

export const useRdpStore = create<RdpStore>((set) => ({
  engineStatus: { status: 'Connecting' as const, servers: [] },
  connections: EMPTY_CONNECTIONS,
  logs: [],
  trafficHistory: [],
  error: null,
  busyNet: null,
  busyConnectionId: null,
  closingAll: false,

  setError: (error) => set({ error }),
  setBusyNet: (busyNet) => set({ busyNet }),
  setBusyConnectionId: (busyConnectionId) => set({ busyConnectionId }),
  setClosingAll: (closingAll) => set({ closingAll }),
  clearLogs: () => set({ logs: [] }),
}))

// ---------------------------------------------------------------------------
// WebSocket streams (module-level, started once)
// ---------------------------------------------------------------------------

let lastTrafficSample: {
  totalUpload: number
  totalDownload: number
  timestamp: number
} | null = null

let lastConnectionSnapshot: ConnectionSnapshot = EMPTY_CONNECTIONS

function applyConnectionSnapshot(snapshot: ConnectionSnapshot) {
  const now = Date.now()
  const previous = lastTrafficSample
  const elapsed = previous ? Math.max(now - previous.timestamp, 1) : 1000
  const uploadRate = previous
    ? Math.max(0, ((snapshot.total_upload - previous.totalUpload) * 1000) / elapsed)
    : 0
  const downloadRate = previous
    ? Math.max(0, ((snapshot.total_download - previous.totalDownload) * 1000) / elapsed)
    : 0

  lastTrafficSample = {
    totalUpload: snapshot.total_upload,
    totalDownload: snapshot.total_download,
    timestamp: now,
  }
  lastConnectionSnapshot = snapshot

  useRdpStore.setState((state) => ({
    connections: snapshot,
    trafficHistory: [
      ...state.trafficHistory.slice(-31),
      { id: now, uploadRate, downloadRate },
    ],
  }))
}

function connectWebSocket(path: string, onMessage: (data: string) => void) {
  let socket: WebSocket | null = null

  const connect = () => {
    socket = new WebSocket(getWebSocketUrl(path))
    socket.onmessage = (event) => onMessage(String(event.data))
    socket.onerror = () => socket?.close()
    socket.onclose = () => {
      window.setTimeout(connect, 1500)
    }
  }

  connect()
}

let streamsStarted = false

interface StreamDeps {
  getRuntimeState: () => string
  invalidateQueries: (keys: string[][]) => void
}

interface ServerEvent {
  event: string
  status?: EngineStatus
}

function connectSSE(deps: StreamDeps) {
  let source: EventSource | null = null

  const connect = () => {
    source = new EventSource('/api/stream/events')

    source.onmessage = (msg) => {
      try {
        const data = JSON.parse(msg.data) as ServerEvent
        switch (data.event) {
          case 'StatusChanged':
            if (data.status) {
              useRdpStore.setState({ engineStatus: data.status })
            }
            break
          case 'ConfigChanged':
            deps.invalidateQueries([['config']])
            break
        }
      } catch {
        // ignore malformed events
      }
    }

    source.onerror = () => {
      source?.close()
      window.setTimeout(connect, 1500)
    }
  }

  connect()
}

export function startStreams(deps: StreamDeps) {
  if (streamsStarted) return
  streamsStarted = true

  // SSE for engine status changes → invalidate queries
  connectSSE(deps)

  // Connection stream
  connectWebSocket('/api/stream/connections?patch=true', (data) => {
    try {
      const payload = JSON.parse(data) as
        | { full?: ConnectionSnapshot; patch?: Operation[] }
        | ConnectionSnapshot

      if ('patch' in payload && Array.isArray(payload.patch)) {
        const nextSnapshot = applyPatch(
          structuredClone(lastConnectionSnapshot),
          payload.patch,
          false,
          true,
        ).newDocument as ConnectionSnapshot
        applyConnectionSnapshot(nextSnapshot)
        return
      }

      const snapshot =
        'full' in payload && payload.full ? payload.full : (payload as ConnectionSnapshot)
      applyConnectionSnapshot(snapshot)
    } catch (caught) {
      if (deps.getRuntimeState() === 'Running') {
        useRdpStore.getState().setError(
          caught instanceof Error ? caught.message : 'Unknown error',
        )
      }
    }
  })

  // Load historical logs from file, then connect WebSocket for live stream
  rdpApi.getLogs(500).then((entries) => {
    const historicalLogs: LogEntry[] = entries.map((entry, i) => {
      const parsed = entry as {
        timestamp?: string
        level?: string
        fields?: Record<string, unknown>
        target?: string
        span?: Record<string, unknown>
        spans?: Array<Record<string, unknown>>
      }
      return {
        id: `hist-${i}`,
        time: parsed.timestamp ?? null,
        level: parsed.level ?? 'LOG',
        message: (parsed.fields?.message as string) ?? JSON.stringify(entry),
        raw: JSON.stringify(entry),
        target: parsed.target ?? null,
        fields: parsed.fields ?? null,
        span: parsed.span ?? null,
        spans: parsed.spans ?? null,
      }
    })
    if (historicalLogs.length > 0) {
      useRdpStore.setState({ logs: historicalLogs })
    }
  }).catch(() => {
    // No log file available (non-daemon mode), that's fine
  })

  // Live log stream (appends after historical)
  connectWebSocket('/api/stream/logs', (data) => {
    const nextLines = parseLogChunk(data)
    if (nextLines.length > 0) {
      useRdpStore.setState((state) => ({
        logs: [...state.logs, ...nextLines].slice(-500),
      }))
    }
  })
}
