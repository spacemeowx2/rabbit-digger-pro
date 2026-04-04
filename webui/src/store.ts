import { create } from 'zustand'
import { applyPatch, type Operation } from 'fast-json-patch'

import { getWebSocketUrl } from './api'
import type { ConnectionSnapshot, LogEntry, TrafficSample } from './types'
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

export function startStreams(getRuntimeState: () => string) {
  if (streamsStarted) return
  streamsStarted = true

  // Connection stream
  connectWebSocket('/api/stream/connection?patch=true', (data) => {
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
      // Only surface errors when engine is actually running
      if (getRuntimeState() === 'Running') {
        useRdpStore.getState().setError(
          caught instanceof Error ? caught.message : 'Unknown error',
        )
      }
    }
  })

  // Log stream
  connectWebSocket('/api/stream/logs', (data) => {
    const nextLines = parseLogChunk(data)
    if (nextLines.length > 0) {
      useRdpStore.setState((state) => ({
        logs: [...state.logs, ...nextLines].slice(-500),
      }))
    }
  })
}
