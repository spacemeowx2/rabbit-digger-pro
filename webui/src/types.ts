export interface NetConfig {
  type: string
  selected?: string
  list?: string[]
  server?: string
  net?: unknown
  sni?: string
  url?: string
  interval?: number
  tolerance?: number
  [key: string]: unknown
}

export interface RdpConfig {
  id?: string
  net: Record<string, NetConfig>
  server: Record<string, Record<string, unknown>>
  [key: string]: unknown
}

export interface ConnectionEntry {
  protocol?: string
  addr?: unknown
  start_time?: number
  ctx?: Record<string, unknown>
  upload?: number
  download?: number
}

export interface ConnectionSnapshot {
  connections: Record<string, ConnectionEntry>
  total_upload: number
  total_download: number
}

export interface LogEntry {
  id: string
  time: string | null
  level: string
  message: string
  raw: string
  target: string | null
  fields: Record<string, unknown> | null
  span: Record<string, unknown> | null
  spans: Array<Record<string, unknown>> | null
}

export interface TrafficSample {
  id: number
  uploadRate: number
  downloadRate: number
}

export interface ServerSnapshot {
  name: string
  server_type: string
}

export interface EngineStatus {
  status: 'Idle' | 'Starting' | 'Running' | 'Stopping' | 'Error' | 'Connecting'
  message?: string
  servers?: ServerSnapshot[]
}

export interface DelayResponse {
  connect: number
  response: number
}
