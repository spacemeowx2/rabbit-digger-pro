export interface ImportSource {
  type: string
  data: Record<string, unknown>
}

export interface ConnectionQuery {
  patch?: boolean
  without_connections?: boolean
}

export interface DelayRequest {
  url: string
  timeout?: number
}

export interface DelayResponse {
  connect: number
  response: number
}

export interface PostSelectPayload {
  selected: string
}

// API 响应类型
export type ApiResponse<T = unknown> = T

// API 错误响应
export interface ApiError {
  error: string
}

// 创建类型安全的 fetch 包装函数
async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api${path}`, init)
  if (!response.ok) {
    const error: ApiError = await response.json()
    throw new Error(error.error)
  }
  return response.json()
}

export async function getConfig(): Promise<string> {
  return apiFetch('/config')
}

export async function postConfig(source: ImportSource): Promise<null> {
  return apiFetch('/config', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(source),
  })
}

export type RegistryData = Record<string, Record<string, unknown>>
export async function getRegistry(): Promise<RegistryData> {
  return apiFetch('/registry')
}

export interface Connection {
  connections: Record<string, unknown>
}
export async function getConnections(): Promise<Connection> {
  return apiFetch('/connections')
}

export async function deleteConnections(): Promise<Connection> {
  return apiFetch('/connections', {
    method: 'DELETE',
  })
}

export async function getState(): Promise<string> {
  return apiFetch('/state')
}

export async function postSelect(netName: string, selected: string): Promise<null> {
  return apiFetch(`/select/${encodeURIComponent(netName)}`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ selected }),
  })
}

export async function deleteConn(uuid: string): Promise<boolean> {
  return apiFetch(`/conn/${encodeURIComponent(uuid)}`, {
    method: 'DELETE',
  })
}

export async function getDelay(netName: string, request: DelayRequest): Promise<DelayResponse | null> {
  const params = new URLSearchParams()
  params.set('url', request.url)
  if (request.timeout !== undefined) {
    params.set('timeout', request.timeout.toString())
  }
  return apiFetch(`/delay/${encodeURIComponent(netName)}?${params.toString()}`)
}

// Userdata 相关 API
export async function getUserData<T = unknown>(path: string): Promise<T> {
  return apiFetch(`/userdata/${encodeURIComponent(path)}`)
}

export async function putUserData(path: string, data: string): Promise<{ copied: number }> {
  const response = await fetch(`/api/v1/userdata/${encodeURIComponent(path)}`, {
    method: 'PUT',
    body: data,
  })
  if (!response.ok) {
    const error: ApiError = await response.json()
    throw new Error(error.error)
  }
  return response.json()
}

export async function deleteUserData(path: string): Promise<{ ok: boolean }> {
  return apiFetch(`/userdata/${encodeURIComponent(path)}`, {
    method: 'DELETE',
  })
}

export interface UserDataList {
  keys: string[]
}
export async function listUserData(): Promise<UserDataList> {
  return apiFetch('/userdata')
}

// WebSocket 相关函数
export function connectWebSocket(query: ConnectionQuery = {}): WebSocket {
  const params = new URLSearchParams()
  if (query.patch) params.set('patch', 'true')
  if (query.without_connections) params.set('without_connections', 'true')

  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const ws = new WebSocket(
    `${protocol}//${window.location.host}/api/v1/ws/connection?${params.toString()}`
  )
  return ws
}

export function connectLogWebSocket(): WebSocket {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const ws = new WebSocket(`${protocol}//${window.location.host}/api/v1/ws/log`)
  return ws
}

interface UseWebSocketOptions {
  onMessage?: (data: unknown) => void
  onError?: (error: Event) => void
  onClose?: (event: CloseEvent) => void
}

export function useWebSocket(url: string, options: UseWebSocketOptions = {}) {
  const ws = new WebSocket(url)

  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data)
      options.onMessage?.(data)
    } catch (e) {
      console.error('Failed to parse WebSocket message:', e)
      options.onError?.(e as Event)
    }
  }

  ws.onclose = (event) => {
    options.onClose?.(event)
  }

  return {
    send: (data: unknown) => {
      ws.send(JSON.stringify(data))
    },
    close: () => {
      ws.close()
    },
  }
}