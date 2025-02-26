import { ReactNode, useState, useEffect } from "react"
import { loadInstances, saveInstances } from "@/utils/storage"
import { InstanceContext, RDPInstance } from "./instance"

export function InstanceProvider({ children }: { children: ReactNode }) {
    // 从本地存储加载实例
    const [instances, setInstances] = useState<RDPInstance[]>(() => loadInstances())
    const [currentInstance, setCurrentInstance] = useState<RDPInstance>(() => {
        const loadedInstances = loadInstances()
        return loadedInstances.find(instance => instance.isActive) || loadedInstances[0]
    })

    // 当实例列表变化时保存到本地存储
    useEffect(() => {
        saveInstances(instances)
    }, [instances])

    // 更新实例时同时更新持久化存储
    const handleSetInstances = (newInstances: RDPInstance[]) => {
        setInstances(newInstances)
        saveInstances(newInstances)
    }

    return (
        <InstanceContext.Provider
            value={{
                instances,
                setInstances: handleSetInstances,
                currentInstance,
                setCurrentInstance
            }}
        >
            {children}
        </InstanceContext.Provider>
    )
}