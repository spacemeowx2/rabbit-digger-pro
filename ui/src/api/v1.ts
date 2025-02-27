import useSWR from 'swr'
import { useState, useMemo, useRef } from 'react'
import { applyPatch } from 'fast-json-patch'
import useWebSocket, { ReadyState } from 'react-use-websocket';

// 基础接口定义
export interface ImportStorage {
  folder: string
  key: string
}

export interface ImportUrl {
  url: string
  interval?: number
}

export interface ImportSource {
  path?: string
  poll?: ImportUrl
  storage?: ImportStorage
  text?: string
}

export interface Import {
  name?: string
  type: string
  source: ImportSource
  [key: string]: unknown
}

export interface Net {
  type: string
  [key: string]: unknown
}

export interface ServerMetadata {
  [key: string]: unknown
}

export interface Server {
  metadata?: ServerMetadata
  type: string
  [key: string]: unknown
}

export interface Config {
  id?: string
  net: Record<string, Net>
  server: Record<string, Server>
}

export interface ConfigExt extends Config {
  import: Import[]
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
    get: { response: ConfigExt; params: void }
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
  '/net/:netName/delay': {
    get: { response: DelayResponse | null; params: DelayRequest }
  }
  '/net/:netName': {
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
export async function fetcher<T extends keyof APIEndpoints, M extends keyof APIEndpoints[T] & HttpMethod>(
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
  return useSWR<ConfigExt>(['/config', 'get', undefined, baseUrl] as const, fetcher)
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

export function useSelect(baseUrl?: string) {
  return {
    select: async (netName: string, selected: string) => {
      const key: FetcherKey<'/net/:netName', 'post'> = [
        '/net/:netName',
        'post',
        { selected },
        { netName },
        baseUrl
      ];
      return fetcher(key);
    }
  };
}

export function useDeleteConn(uuid: string, baseUrl?: string) {
  return useSWR<boolean>(
    ['/conn/:uuid', 'delete', undefined, { uuid }, baseUrl] as const,
    fetcher
  )
}

export function useDelay(netName: string, request: DelayRequest, baseUrl?: string) {
  return useSWR<DelayResponse | null>(
    ['/net/:netName/delay', 'get', request, { netName }, baseUrl] as const,
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
  return new WebSocket(createWebSocketUrl('/stream/connection', baseUrl, params))
}

export function connectLogWebSocket(baseUrl = ''): WebSocket {
  return new WebSocket(createWebSocketUrl('/stream/log', baseUrl))
}

export interface UseWebSocketOptions {
  onMessage?: (data: unknown) => void
  onError?: (error: Event) => void
  onClose?: (event: CloseEvent) => void
}

// MaybePatch represents the two possible message formats from the connection stream
export type MaybePatch =
  | { full: Record<string, unknown> }
  | { patch: Array<{ op: string; path: string; value?: unknown }> }

export interface ConnectionData {
  connections: Record<string, ConnectionInfo>
  total_upload: number;
  total_download: number;
}

export interface ConnectionContext {
  src_socket_addr: string
  dest_domain?: string
  net_list: string[]
  [key: string]: unknown
}

export interface ConnectionInfo {
  protocol: string
  addr: string
  upload: number
  download: number
  uploadSpeed?: number
  downloadSpeed?: number
  start_time: number
  ctx: ConnectionContext
  [key: string]: unknown
}

/**
 * Hook to track connections using WebSocket
 * @param baseUrl Base URL for the WebSocket connection
 * @returns connections as arrays of ConnectionInfo
 */
export function useConnectionsStream(baseUrl?: string) {
  const [state, setState] = useState<ConnectionData>();
  const lastData = useRef<ConnectionData>();
  const lastTraffic = useRef(new Map<string, { upload: number; download: number }>());

  const wsUrl = useMemo(() => {
    const params = new URLSearchParams([['patch', 'true']]);
    const url = new URL('/api/stream/connection', baseUrl || window.location.origin);
    url.protocol = url.protocol.replace('http', 'ws');
    url.search = params.toString();
    return url.toString();
  }, [baseUrl]);

  const { readyState } = useWebSocket(wsUrl, {
    shouldReconnect: () => true, // 总是尝试重连
    reconnectAttempts: 10,
    reconnectInterval: 3000,
    retryOnError: true,
    onMessage: (event) => {
      try {
        const data = JSON.parse(event.data as string);
        if (data.full) {
          setState(data.full);
          lastData.current = data.full;
        } else if (data.patch && lastData.current) {
          try {
            const newData = applyPatch(lastData.current, data.patch).newDocument;

            const dataWithSpeeds = {
              ...newData,
              connections: Object.fromEntries(
                Object.entries(newData.connections).map(([id, conn]) => {
                  const prevConn = lastTraffic.current.get(id);
                  let uploadSpeed = 0;
                  let downloadSpeed = 0;

                  if (prevConn) {
                    uploadSpeed = (conn.upload - prevConn.upload);
                    downloadSpeed = (conn.download - prevConn.download);
                  }

                  lastTraffic.current.set(id, {
                    upload: conn.upload,
                    download: conn.download,
                  });

                  return [
                    id,
                    {
                      ...conn,
                      uploadSpeed,
                      downloadSpeed,
                    },
                  ];
                })
              ),
            };

            setState(dataWithSpeeds);
            lastData.current = dataWithSpeeds;
          } catch (e) {
            console.error('Failed to apply patch:', e);
          }
        }
      } catch (e) {
        console.error('Failed to parse message:', e);
      }
    },
  });

  return {
    state,
    readyState,
    isConnected: readyState === ReadyState.OPEN,
  };
}
