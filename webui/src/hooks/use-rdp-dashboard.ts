import { useCallback, useEffect } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { rdpApi } from '../api'
import type { RdpConfig } from '../types'
import { updateSelectedNet } from '../utils'
import { startStreams, useRdpStore } from '../store'

function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : 'Unknown error'
}

export function useRdpDashboard() {
  const queryClient = useQueryClient()

  // Config via TanStack Query — SSE ConfigChanged invalidates this
  const { data: config = null } = useQuery<RdpConfig | null>({
    queryKey: ['config'],
    queryFn: async () => {
      try {
        return await rdpApi.getConfig()
      } catch {
        return null
      }
    },
  })

  // Engine status from Zustand (SSE-driven, no polling)
  const engineStatus = useRdpStore((s) => s.engineStatus)
  const runtimeState = engineStatus.status

  // Real-time data + UI state from Zustand
  const connections = useRdpStore((s) => s.connections)
  const logs = useRdpStore((s) => s.logs)
  const trafficHistory = useRdpStore((s) => s.trafficHistory)
  const error = useRdpStore((s) => s.error)
  const busyNet = useRdpStore((s) => s.busyNet)
  const busyConnectionId = useRdpStore((s) => s.busyConnectionId)
  const closingAll = useRdpStore((s) => s.closingAll)
  const setError = useRdpStore((s) => s.setError)
  const setBusyNet = useRdpStore((s) => s.setBusyNet)
  const setBusyConnectionId = useRdpStore((s) => s.setBusyConnectionId)
  const setClosingAll = useRdpStore((s) => s.setClosingAll)
  const clearLogs = useRdpStore((s) => s.clearLogs)

  // Start streams once
  useEffect(() => {
    startStreams({
      getRuntimeState: () => useRdpStore.getState().engineStatus.status,
      invalidateQueries: (keys) => {
        for (const key of keys) {
          void queryClient.invalidateQueries({ queryKey: key })
        }
      },
    })
  }, [queryClient])

  const selectNet = useCallback(
    async (netName: string, selected: string) => {
      const previousSelected = config?.net[netName]?.selected

      setBusyNet(netName)
      setError(null)
      queryClient.setQueryData<RdpConfig | null>(
        ['config'],
        (current) => updateSelectedNet(current ?? null, netName, selected),
      )

      try {
        await rdpApi.selectNet(netName, selected)
      } catch (caught) {
        queryClient.setQueryData<RdpConfig | null>(
          ['config'],
          (current) =>
            previousSelected
              ? updateSelectedNet(current ?? null, netName, previousSelected)
              : current ?? null,
        )
        setError(getErrorMessage(caught))
      } finally {
        setBusyNet(null)
      }
    },
    [config, queryClient, setBusyNet, setError],
  )

  const closeConnection = useCallback(
    async (connectionId: string) => {
      setBusyConnectionId(connectionId)
      setError(null)
      try {
        await rdpApi.closeConnection(connectionId)
      } catch (caught) {
        setError(getErrorMessage(caught))
      } finally {
        setBusyConnectionId(null)
      }
    },
    [setBusyConnectionId, setError],
  )

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
  }, [setClosingAll, setError])

  return {
    config,
    engineStatus,
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
  }
}
