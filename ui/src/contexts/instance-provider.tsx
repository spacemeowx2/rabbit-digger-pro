import { ReactNode, useState, useEffect } from "react"
import { InstanceContext, RDPInstance, loadInstances, saveInstances } from "./instance"

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
  }

  // 更新当前实例时同时更新 isActive 状态并保存
  const handleSetCurrentInstance = (instance: RDPInstance) => {
    const updatedInstances = instances.map(item => ({
      ...item,
      isActive: item.id === instance.id
    }))
    setInstances(updatedInstances)
    setCurrentInstance(instance)
  }

  return (
    <InstanceContext.Provider
      value={{
        instances,
        setInstances: handleSetInstances,
        currentInstance,
        setCurrentInstance: handleSetCurrentInstance
      }}
    >
      {children}
    </InstanceContext.Provider>
  )
}
