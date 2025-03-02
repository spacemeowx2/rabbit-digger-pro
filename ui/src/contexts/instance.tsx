import { createContext, useContext } from "react"

const STORAGE_KEY = "rdp-instances"

// Types
export interface RDPInstance {
  id: string
  name: string
  url: string
  isActive: boolean
}

export interface InstanceContextType {
  instances: RDPInstance[]
  setInstances: (instances: RDPInstance[]) => void
  currentInstance: RDPInstance | null
  setCurrentInstance: (instance: RDPInstance) => void
}

export interface InstanceStatus {
  isOnline: boolean
  lastChecked: number
}

export interface StatusAnimation {
  prevStatus: boolean
  timestamp: number
}

export interface InstanceStatuses {
  [url: string]: boolean
}

export interface UseInstanceStatusesReturn {
  statuses: InstanceStatuses
  error: Error | null
  refresh: () => Promise<InstanceStatuses>
  isValidating: boolean
  statusAnimations: Record<string, StatusAnimation>
}

export interface StatusTransitionOptions {
  url: string
  isOnline: boolean
  statusAnimations: Record<string, StatusAnimation>
  isValidating: boolean
}

// Context
export const InstanceContext = createContext<InstanceContextType | null>(null)

// Hook
export function useInstance() {
  const context = useContext(InstanceContext)
  if (!context) {
    throw new Error("useInstance must be used within an InstanceProvider")
  }
  return context
}

export function saveInstances(instances: RDPInstance[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(instances))
  } catch (error) {
    console.error("Failed to save instances:", error)
  }
}

export function loadInstances(): RDPInstance[] {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (stored) {
      return JSON.parse(stored)
    }
  } catch (error) {
    console.error("Failed to load instances:", error)
  }

  // 如果没有存储的实例或出错，返回默认实例
  return [{
    id: "local",
    name: "本地实例",
    url: window.location.origin,
    isActive: true
  }]
}
