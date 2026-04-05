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
    return readJson<null>(`/api/net/${encodeURIComponent(netName)}/select`, {
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
  getLogs(tail = 500) {
    return readJson<Array<Record<string, unknown>>>(`/api/logs?tail=${tail}`)
  },
  applyConfig(source: Record<string, unknown>) {
    return readJson<null>('/api/config', {
      method: 'POST',
      body: JSON.stringify(source),
    })
  },
  engineStop() {
    return readJson<{ ok: boolean }>('/api/engine/stop', {
      method: 'POST',
    })
  },
  getUserdata<T = unknown>(path: string) {
    return readJson<T>(`/api/userdata/${encodeURIComponent(path)}`)
  },
  putUserdata(path: string, value: string) {
    return readJson<{ copied: number }>(`/api/userdata/${encodeURIComponent(path)}`, {
      method: 'PUT',
      headers: { 'content-type': 'text/plain' },
      body: value,
    })
  },
  deleteUserdata(path: string) {
    return readJson<{ ok: boolean }>(`/api/userdata/${encodeURIComponent(path)}`, {
      method: 'DELETE',
    })
  },
  listUserdata() {
    return readJson<{ keys: Array<{ key: string; updated_at: string }> }>('/api/userdata')
  },
}
