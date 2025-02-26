import { RDPInstance } from "@/contexts/instance-context"

const STORAGE_KEY = "rdp-instances"

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
        url: "http://127.0.0.1:8030",
        isActive: true
    }]
}