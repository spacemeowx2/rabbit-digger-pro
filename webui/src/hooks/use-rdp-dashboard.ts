import {
  startTransition,
  useCallback,
  useEffect,
  useEffectEvent,
  useRef,
  useState,
} from 'react'
import { applyPatch, type Operation } from 'fast-json-patch'

import { getWebSocketUrl, rdpApi } from '../api'
import type { ConnectionSnapshot, LogEntry, RdpConfig, TrafficSample } from '../types'
import { parseLogChunk, updateSelectedNet } from '../utils'

const EMPTY_CONNECTIONS: ConnectionSnapshot = {
  connections: {},
  total_upload: 0,
  total_download: 0,
}

function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : 'Unknown error'
}

type ConnectionStreamStats = {
  mode: 'ws-patch'
  fullFrames: number
  patchFrames: number
  lastFrameType: 'full' | 'patch' | null
  lastEventAt: number | null
}

declare global {
  interface Window {
    __RDP_DEBUG?: {
      connectionStream?: ConnectionStreamStats
    }
  }
}

export function useRdpDashboard() {
  const [config, setConfig] = useState<RdpConfig | null>(null)
  const [runtimeState, setRuntimeState] = useState('Connecting')
  const [connections, setConnections] = useState<ConnectionSnapshot>(EMPTY_CONNECTIONS)
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [trafficHistory, setTrafficHistory] = useState<TrafficSample[]>([])
  const [error, setError] = useState<string | null>(null)
  const [busyNet, setBusyNet] = useState<string | null>(null)
  const [busyConnectionId, setBusyConnectionId] = useState<string | null>(null)
  const [closingAll, setClosingAll] = useState(false)

  const lastTrafficSample = useRef<{
    totalUpload: number
    totalDownload: number
    timestamp: number
  } | null>(null)
  const lastConnectionSnapshot = useRef<ConnectionSnapshot>(EMPTY_CONNECTIONS)
  const connectionStreamStats = useRef<ConnectionStreamStats>({
    mode: 'ws-patch',
    fullFrames: 0,
    patchFrames: 0,
    lastFrameType: null,
    lastEventAt: null,
  })

  const refreshConfig = useCallback(async () => {
    try {
      const nextConfig = await rdpApi.getConfig()
      setConfig(nextConfig)
    } catch (caught) {
      setError(getErrorMessage(caught))
    }
  }, [])

  const refreshState = useCallback(async () => {
    try {
      const nextState = await rdpApi.getState()
      setRuntimeState(nextState)
    } catch (caught) {
      setError(getErrorMessage(caught))
    }
  }, [])

  const applyConnectionSnapshot = useEffectEvent((snapshot: ConnectionSnapshot) => {
    startTransition(() => {
      setConnections(snapshot)
      lastConnectionSnapshot.current = snapshot

      const now = Date.now()
      const previous = lastTrafficSample.current
      const elapsed = previous ? Math.max(now - previous.timestamp, 1) : 1000
      const uploadRate = previous
        ? Math.max(0, ((snapshot.total_upload - previous.totalUpload) * 1000) / elapsed)
        : 0
      const downloadRate = previous
        ? Math.max(0, ((snapshot.total_download - previous.totalDownload) * 1000) / elapsed)
        : 0

      lastTrafficSample.current = {
        totalUpload: snapshot.total_upload,
        totalDownload: snapshot.total_download,
        timestamp: now,
      }

      setTrafficHistory((current) => [
        ...current.slice(-31),
        {
          id: now,
          uploadRate,
          downloadRate,
        },
      ])
    })
  })

  const updateConnectionStreamStats = useEffectEvent((frameType: 'full' | 'patch') => {
    const current = connectionStreamStats.current
    const next = {
      ...current,
      fullFrames: current.fullFrames + Number(frameType === 'full'),
      patchFrames: current.patchFrames + Number(frameType === 'patch'),
      lastFrameType: frameType,
      lastEventAt: Date.now(),
    }

    connectionStreamStats.current = next
    window.__RDP_DEBUG = {
      ...window.__RDP_DEBUG,
      connectionStream: next,
    }
  })

  const appendLogs = useEffectEvent((chunk: string) => {
    const nextLines = parseLogChunk(chunk)
    if (nextLines.length === 0) {
      return
    }

    startTransition(() => {
      setLogs((current) => [...current, ...nextLines].slice(-500))
    })
  })

  useEffect(() => {
    void refreshConfig()
    void refreshState()

    const stateTimer = window.setInterval(() => {
      void refreshState()
    }, 5000)

    return () => {
      window.clearInterval(stateTimer)
    }
  }, [refreshConfig, refreshState])

  useEffect(() => {
    let closed = false
    let socket: WebSocket | null = null
    let retryTimer = 0

    const connect = () => {
      socket = new WebSocket(getWebSocketUrl('/api/stream/connection?patch=true'))

      socket.onmessage = (event) => {
        try {
          const payload = JSON.parse(String(event.data)) as
            | { full?: ConnectionSnapshot; patch?: Operation[] }
            | ConnectionSnapshot

          if ('patch' in payload && Array.isArray(payload.patch)) {
            const nextSnapshot = applyPatch(
              structuredClone(lastConnectionSnapshot.current),
              payload.patch,
              false,
              true,
            ).newDocument as ConnectionSnapshot

            updateConnectionStreamStats('patch')
            applyConnectionSnapshot(nextSnapshot)
            return
          }

          const snapshot =
            'full' in payload && payload.full ? payload.full : (payload as ConnectionSnapshot)
          updateConnectionStreamStats('full')
          applyConnectionSnapshot(snapshot)
        } catch (caught) {
          setError(getErrorMessage(caught))
        }
      }

      socket.onerror = () => {
        socket?.close()
      }

      socket.onclose = () => {
        if (!closed) {
          retryTimer = window.setTimeout(connect, 1500)
        }
      }
    }

    connect()

    return () => {
      closed = true
      window.clearTimeout(retryTimer)
      socket?.close()
    }
  }, [])

  useEffect(() => {
    let closed = false
    let socket: WebSocket | null = null
    let retryTimer = 0

    const connect = () => {
      socket = new WebSocket(getWebSocketUrl('/api/stream/logs'))

      socket.onmessage = (event) => {
        appendLogs(String(event.data))
      }

      socket.onerror = () => {
        socket?.close()
      }

      socket.onclose = () => {
        if (!closed) {
          retryTimer = window.setTimeout(connect, 1500)
        }
      }
    }

    connect()

    return () => {
      closed = true
      window.clearTimeout(retryTimer)
      socket?.close()
    }
  }, [])

  const selectNet = useCallback(
    async (netName: string, selected: string) => {
      const previousSelected = config?.net[netName]?.selected

      setBusyNet(netName)
      setError(null)
      setConfig((current) => updateSelectedNet(current, netName, selected))

      try {
        await rdpApi.selectNet(netName, selected)
      } catch (caught) {
        setConfig((current) =>
          previousSelected ? updateSelectedNet(current, netName, previousSelected) : current,
        )
        setError(getErrorMessage(caught))
      } finally {
        setBusyNet(null)
      }
    },
    [config],
  )

  const closeConnection = useCallback(async (connectionId: string) => {
    setBusyConnectionId(connectionId)
    setError(null)

    try {
      await rdpApi.closeConnection(connectionId)
    } catch (caught) {
      setError(getErrorMessage(caught))
    } finally {
      setBusyConnectionId(null)
    }
  }, [])

  const closeAllConnections = useCallback(async () => {
    setClosingAll(true)
    setError(null)

    try {
      await rdpApi.closeAllConnections()
    } catch (caught) {
      setError(getErrorMessage(caught))
    } finally {
      setClosingAll(false)
    }
  }, [])

  const clearLogs = useCallback(() => {
    setLogs([])
  }, [])

  return {
    config,
    runtimeState,
    connections,
    logs,
    trafficHistory,
    error,
    busyNet,
    busyConnectionId,
    closingAll,
    refreshConfig,
    selectNet,
    closeConnection,
    closeAllConnections,
    clearLogs,
  }
}
