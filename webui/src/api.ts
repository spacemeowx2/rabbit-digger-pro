import type { DelayResponse, RdpConfig } from './types'

async function readJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      'content-type': 'application/json',
      ...(init?.headers ?? {}),
    },
  })

  if (!response.ok) {
    const text = await response.text()
    throw new Error(text || `${response.status} ${response.statusText}`)
  }

  const text = await response.text()
  return (text ? JSON.parse(text) : null) as T
}

export function getWebSocketUrl(path: string): string {
  const url = new URL(path, window.location.origin)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  return url.toString()
}

export const rdpApi = {
  getConfig() {
    return readJson<RdpConfig>('/api/config')
  },
  getState() {
    return readJson<string>('/api/state')
  },
  getNetDelay(netName: string, url: string, timeout = 5000) {
    const params = new URLSearchParams({
      url,
      timeout: String(timeout),
    })
    return readJson<DelayResponse | null>(
      `/api/net/${encodeURIComponent(netName)}/delay?${params.toString()}`,
    )
  },
  selectNet(netName: string, selected: string) {
    return readJson<null>(`/api/net/${encodeURIComponent(netName)}`, {
      method: 'POST',
      body: JSON.stringify({ selected }),
    })
  },
  closeConnection(uuid: string) {
    return readJson<boolean>(`/api/connection/${uuid}`, {
      method: 'DELETE',
    })
  },
  closeAllConnections() {
    return readJson<number>('/api/connection', {
      method: 'DELETE',
    })
  },
}
