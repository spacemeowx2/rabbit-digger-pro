import useSWR from 'swr'

// 基础接口定义
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

export interface Connection {
  connections: Record<string, unknown>
}

export interface UserDataList {
  keys: string[]
}

export interface ApiError {
  error: string
}

// Define HTTP methods and API endpoints
type HttpMethod = 'get' | 'post' | 'put' | 'delete'

type APIEndpoints = {
  '/config': {
    get: { response: string; params: void }
    post: { response: null; params: ImportSource }
  }
  '/registry': {
    get: { response: Record<string, Record<string, unknown>>; params: void }
  }
  '/connections': {
    get: { response: Connection; params: void }
    delete: { response: Connection; params: void }
  }
  '/state': {
    get: { response: string; params: void }
  }
  '/delay/:netName': {
    get: { response: DelayResponse | null; params: DelayRequest }
  }
  '/select/:netName': {
    post: { response: null; params: PostSelectPayload }
  }
  '/conn/:uuid': {
    delete: { response: boolean; params: void }
  }
  '/userdata': {
    get: { response: UserDataList; params: void }
  }
  '/userdata/:path': {
    get: { response: unknown; params: void }
    put: { response: { copied: number }; params: string }
    delete: { response: { ok: boolean }; params: void }
  }
}

// Helper type to extract path parameters
type ExtractRouteParams<T extends string> = string extends T
  ? Record<string, string>
  : T extends `${string}:${infer Param}/${infer Rest}`
  ? { [K in Param | keyof ExtractRouteParams<Rest>]: string }
  : T extends `${string}:${infer Param}`
  ? { [K in Param]: string }
  : Record<string, never>

type EndpointWithParams<T extends keyof APIEndpoints> = T extends `${string}:${string}` ? T : never

type EndpointMethod<T extends keyof APIEndpoints, M extends keyof APIEndpoints[T]> =
  APIEndpoints[T][M] & { response: unknown; params: unknown }

type FetcherKey<T extends keyof APIEndpoints, M extends keyof APIEndpoints[T] & HttpMethod> =
  T extends EndpointWithParams<T>
  ? [T, M, EndpointMethod<T, M>['params'], ExtractRouteParams<T>, string?]
  : [T, M, EndpointMethod<T, M>['params'], string?]

// Type-safe fetcher function
async function fetcher<T extends keyof APIEndpoints, M extends keyof APIEndpoints[T] & HttpMethod>(
  key: FetcherKey<T, M>
): Promise<EndpointMethod<T, M>['response']> {
  const [path, method, params, paramsOrBaseUrl, maybeBaseUrl] = key
  const baseUrl = typeof paramsOrBaseUrl === 'string' ? paramsOrBaseUrl : (maybeBaseUrl || '')
  const pathParams = typeof paramsOrBaseUrl === 'object' ? paramsOrBaseUrl : undefined

  let url = `${baseUrl}/api${path}`

  if (pathParams) {
    Object.entries(pathParams).forEach(([key, value]) => {
      url = url.replace(`:${key}`, encodeURIComponent(String(value)))
    })
  }

  if (method === 'get' && params && typeof params === 'object') {
    const searchParams = new URLSearchParams(
      Object.entries(params)
        .filter(([, value]) => value !== undefined)
        .map(([key, value]) => [key, String(value)])
    )
    const queryString = searchParams.toString()
    if (queryString) url += `?${queryString}`
  }

  const init: RequestInit = {
    method: method.toUpperCase(),
    ...(method !== 'get' && params !== undefined && {
      headers: { 'Content-Type': 'application/json' },
      body: typeof params === 'string' ? params : JSON.stringify(params)
    })
  }

  const response = await fetch(url, init)
  if (!response.ok) {
    const error: ApiError = await response.json()
    throw new Error(error.error)
  }

  return response.json()
}

// API Hooks
export function useConfig(baseUrl?: string) {
  return useSWR<string>(['/config', 'get', undefined, baseUrl] as const, fetcher)
}

export function usePostConfig(source: ImportSource, baseUrl?: string) {
  return useSWR(['/config', 'post', source, baseUrl] as const, fetcher)
}

export function useRegistry(baseUrl?: string) {
  return useSWR<Record<string, Record<string, unknown>>>(
    ['/registry', 'get', undefined, baseUrl] as const,
    fetcher
  )
}

export function useConnections(baseUrl?: string) {
  return useSWR<Connection>(['/connections', 'get', undefined, baseUrl] as const, fetcher)
}

export function useDeleteConnections(baseUrl?: string) {
  return useSWR<Connection>(['/connections', 'delete', undefined, baseUrl] as const, fetcher)
}

export function useState(baseUrl?: string) {
  return useSWR<string>(['/state', 'get', undefined, baseUrl] as const, fetcher)
}

export function useSelect(netName: string, selected: string, baseUrl?: string) {
  return useSWR(
    ['/select/:netName', 'post', { selected }, { netName }, baseUrl] as const,
    fetcher
  )
}

export function useDeleteConn(uuid: string, baseUrl?: string) {
  return useSWR<boolean>(
    ['/conn/:uuid', 'delete', undefined, { uuid }, baseUrl] as const,
    fetcher
  )
}

export function useDelay(netName: string, request: DelayRequest, baseUrl?: string) {
  return useSWR<DelayResponse | null>(
    ['/delay/:netName', 'get', request, { netName }, baseUrl] as const,
    fetcher
  )
}

export function useUserData<T = unknown>(path: string, baseUrl?: string) {
  return useSWR<T>(
    ['/userdata/:path', 'get', undefined, { path }, baseUrl] as const,
    fetcher
  )
}

export function usePutUserData(path: string, data: string, baseUrl?: string) {
  return useSWR<{ copied: number }>(
    ['/userdata/:path', 'put', data, { path }, baseUrl] as const,
    fetcher
  )
}

export function useDeleteUserData(path: string, baseUrl?: string) {
  return useSWR<{ ok: boolean }>(
    ['/userdata/:path', 'delete', undefined, { path }, baseUrl] as const,
    fetcher
  )
}

export function useListUserData(baseUrl?: string) {
  return useSWR<UserDataList>(['/userdata', 'get', undefined, baseUrl] as const, fetcher)
}

// WebSocket helpers
function createWebSocketUrl(path: string, baseUrl = '', params?: URLSearchParams): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const host = baseUrl ? baseUrl.replace(/^https?:\/\//, '') : window.location.host
  const queryString = params?.toString() || ''
  return `${protocol}//${host}/api${path}${queryString ? `?${queryString}` : ''}`
}

export function connectWebSocket(query: ConnectionQuery = {}, baseUrl = ''): WebSocket {
  const params = new URLSearchParams(
    Object.entries(query)
      .filter(([, value]) => value)
      .map(([key]) => [key, 'true'])
  )
  return new WebSocket(createWebSocketUrl('/v1/ws/connection', baseUrl, params))
}

export function connectLogWebSocket(baseUrl = ''): WebSocket {
  return new WebSocket(createWebSocketUrl('/v1/ws/log', baseUrl))
}

export interface UseWebSocketOptions {
  onMessage?: (data: unknown) => void
  onError?: (error: Event) => void
  onClose?: (event: CloseEvent) => void
}

export function useWebSocket(url: string, options: UseWebSocketOptions = {}) {
  const ws = new WebSocket(url)

  ws.onmessage = (event) => {
    try {
      options.onMessage?.(JSON.parse(event.data))
    } catch (e) {
      console.error('Failed to parse WebSocket message:', e)
      options.onError?.(e as Event)
    }
  }

  ws.onclose = (event) => {
    options.onClose?.(event)
  }

  return {
    send: (data: unknown) => ws.send(JSON.stringify(data)),
    close: () => ws.close()
  }
}