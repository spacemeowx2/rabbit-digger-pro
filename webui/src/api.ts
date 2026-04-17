import type { DelayResponse, RdpConfig } from './types'

type JsonRpcId = number

type PendingRequest = {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
}

type SubscriptionHandler = (payload: unknown) => void

type SubscriptionRecord = {
  key: string
  topic: string
  params: unknown
  handlers: Set<SubscriptionHandler>
  serverId: string | null
  subscribePromise: Promise<void> | null
}

type JsonRpcResponse = {
  id?: JsonRpcId
  result?: unknown
  error?: { code?: number; message?: string; data?: unknown }
}

type JsonRpcSubscriptionMessage = {
  method?: string
  params?: {
    subscription?: string
    topic?: string
    payload?: unknown
  }
}

export function getWebSocketUrl(path: string): string {
  const url = new URL(path, window.location.origin)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  return url.toString()
}

class RpcClient {
  private socket: WebSocket | null = null
  private connectPromise: Promise<void> | null = null
  private reconnectTimer: number | null = null
  private nextId = 1
  private pending = new Map<JsonRpcId, PendingRequest>()
  private subscriptions = new Map<string, SubscriptionRecord>()

  async request<T>(method: string, params: unknown = {}): Promise<T> {
    await this.ensureConnected()
    const id = this.nextId++
    const payload = { jsonrpc: '2.0', id, method, params }

    return await new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (value: unknown) => void, reject })
      try {
        this.socket?.send(JSON.stringify(payload))
      } catch (error) {
        this.pending.delete(id)
        reject(error instanceof Error ? error : new Error(String(error)))
      }
    })
  }

  subscribe(topic: string, params: unknown, handler: SubscriptionHandler): () => void {
    const key = JSON.stringify({ topic, params })
    let record = this.subscriptions.get(key)
    if (!record) {
      record = {
        key,
        topic,
        params,
        handlers: new Set(),
        serverId: null,
        subscribePromise: null,
      }
      this.subscriptions.set(key, record)
    }
    record.handlers.add(handler)
    void this.ensureSubscription(record)

    return () => {
      const current = this.subscriptions.get(key)
      if (!current) return
      current.handlers.delete(handler)
      if (current.handlers.size === 0) {
        this.subscriptions.delete(key)
        const serverId = current.serverId
        current.serverId = null
        if (serverId) {
          void this.request('rpc.unsubscribe', { subscription: serverId }).catch(() => undefined)
        }
      }
    }
  }

  private async ensureConnected(): Promise<void> {
    if (this.socket?.readyState === WebSocket.OPEN) return
    if (this.connectPromise) return await this.connectPromise

    this.connectPromise = new Promise<void>((resolve, reject) => {
      const socket = new WebSocket(getWebSocketUrl('/api/rpc'))
      this.socket = socket

      socket.onopen = () => {
        this.connectPromise = null
        for (const subscription of this.subscriptions.values()) {
          subscription.serverId = null
          subscription.subscribePromise = null
          void this.ensureSubscription(subscription)
        }
        resolve()
      }

      socket.onmessage = (event) => this.handleMessage(String(event.data))

      socket.onerror = () => {
        if (socket.readyState !== WebSocket.OPEN) {
          this.connectPromise = null
          reject(new Error('RPC connection failed'))
        }
      }

      socket.onclose = () => {
        if (this.socket === socket) {
          this.socket = null
        }
        if (this.connectPromise) {
          this.connectPromise = null
          reject(new Error('RPC connection closed'))
        }
        this.rejectPending('RPC connection closed')
        this.scheduleReconnect()
      }
    })

    return await this.connectPromise
  }

  private scheduleReconnect() {
    if (this.reconnectTimer !== null) return
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null
      void this.ensureConnected().catch(() => undefined)
    }, 1500)
  }

  private rejectPending(message: string) {
    for (const [id, pending] of this.pending) {
      pending.reject(new Error(message))
      this.pending.delete(id)
    }
  }

  private async ensureSubscription(record: SubscriptionRecord): Promise<void> {
    if (record.serverId || record.subscribePromise || record.handlers.size === 0) return
    record.subscribePromise = (async () => {
      try {
        const result = await this.request<{ subscription: string }>('rpc.subscribe', {
          topic: record.topic,
          params: record.params,
        })
        if (!this.subscriptions.has(record.key)) {
          await this.request('rpc.unsubscribe', { subscription: result.subscription }).catch(() => undefined)
          return
        }
        record.serverId = result.subscription
      } finally {
        record.subscribePromise = null
      }
    })()
    await record.subscribePromise
  }

  private handleMessage(raw: string) {
    const parsed = JSON.parse(raw) as JsonRpcResponse | JsonRpcSubscriptionMessage
    if ('method' in parsed && parsed.method === 'rpc.subscription') {
      const serverId = parsed.params?.subscription ?? null
      if (!serverId) return
      for (const subscription of this.subscriptions.values()) {
        if (subscription.serverId === serverId) {
          for (const handler of subscription.handlers) {
            handler(parsed.params?.payload)
          }
          break
        }
      }
      return
    }

    if ('id' in parsed && typeof parsed.id === 'number') {
      const pending = this.pending.get(parsed.id)
      if (!pending) return
      this.pending.delete(parsed.id)
      if (parsed.error) {
        const suffix = parsed.error.data ? `: ${JSON.stringify(parsed.error.data)}` : ''
        pending.reject(new Error(`${parsed.error.message ?? 'RPC error'}${suffix}`))
      } else {
        pending.resolve(parsed.result)
      }
    }
  }
}

const rpcClient = new RpcClient()

export const rdpApi = {
  getConfig() {
    return rpcClient.request<RdpConfig | null>('config.get')
  },
  getNetDelay(netName: string, url: string, timeout = 5000) {
    return rpcClient.request<DelayResponse | null>('net.delay', {
      net_name: netName,
      url,
      timeout,
    })
  },
  selectNet(netName: string, selected: string) {
    return rpcClient.request<null>('net.select', { net_name: netName, selected })
  },
  closeConnection(uuid: string) {
    return rpcClient.request<boolean>('connection.close', { uuid })
  },
  closeAllConnections() {
    return rpcClient.request<number>('connection.closeAll')
  },
  getLogs(tail = 500) {
    return rpcClient.request<Array<Record<string, unknown>>>('logs.tail', { tail })
  },
  applyConfig(source: Record<string, unknown>) {
    return rpcClient.request<null>('config.apply', source)
  },
  engineStop() {
    return rpcClient.request<{ ok: boolean }>('engine.stop')
  },
  getUserdata<T = unknown>(path: string) {
    return rpcClient.request<T>('userdata.get', { path })
  },
  putUserdata(path: string, value: string) {
    return rpcClient.request<{ copied: number }>('userdata.put', { path, value })
  },
  deleteUserdata(path: string) {
    return rpcClient.request<{ ok: boolean }>('userdata.delete', { path })
  },
  listUserdata() {
    return rpcClient.request<{ keys: Array<{ key: string; updated_at: string }> }>('userdata.list')
  },
  suggestTunIp() {
    return rpcClient.request<{ ip: string }>('tun.suggestIp')
  },
  subscribe(topic: string, params: unknown, handler: SubscriptionHandler) {
    return rpcClient.subscribe(topic, params, handler)
  },
}
