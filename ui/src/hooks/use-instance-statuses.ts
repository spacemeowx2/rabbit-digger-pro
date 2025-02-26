import useSWR from "swr"
import { toast } from "sonner"
import { useState } from "react"
import type { InstanceStatuses } from "@/contexts/instance"

async function checkInstancesStatus(urls: string[]): Promise<InstanceStatuses> {
    const validUrls = urls.filter(url => {
        try {
            new URL(url)
            return true
        } catch {
            return false
        }
    })

    if (validUrls.length === 0) {
        return {}
    }

    const statuses = await Promise.all(
        validUrls.map(async (url) => {
            try {
                const response = await fetch(`${url}/api/config`)
                return response.ok
            } catch {
                return false
            }
        })
    )
    return Object.fromEntries(validUrls.map((url, i) => [url, statuses[i]]))
}

interface StatusAnimation {
    [url: string]: {
        prevStatus: boolean
        timestamp: number
    }
}

export function useInstanceStatuses(urls: string[]) {
    const [statusAnimations, setStatusAnimations] = useState<StatusAnimation>({})
    const { data = {}, error, mutate, isValidating } = useSWR(
        urls.length > 0 ? ['instance-statuses', urls] : null,
        () => checkInstancesStatus(urls),
        {
            refreshInterval: 30000,
            revalidateOnFocus: true,
            dedupingInterval: 2000,
            errorRetryCount: 3,
            onSuccess: (newData) => {
                // 状态变化时更新动画状态
                const now = Date.now()
                urls.forEach(url => {
                    if (!url) return
                    const prevStatus = data[url]
                    const newStatus = newData[url]
                    if (prevStatus !== undefined && prevStatus !== newStatus) {
                        setStatusAnimations(prev => ({
                            ...prev,
                            [url]: {
                                prevStatus,
                                timestamp: now
                            }
                        }))

                        // 显示通知
                        toast(newStatus ? "连接恢复" : "连接断开", {
                            description: `实例 ${url} ${newStatus ? "现在可以访问" : "无法访问"}`,
                            position: "top-right",
                            duration: 3000,
                        })
                    }
                })

                // 清理旧的动画状态
                const OLD_ANIMATION_THRESHOLD = 2000 // 2秒
                setStatusAnimations(prev => {
                    const filtered: StatusAnimation = {}
                    Object.entries(prev).forEach(([url, animation]) => {
                        if (now - animation.timestamp < OLD_ANIMATION_THRESHOLD) {
                            filtered[url] = animation
                        }
                    })
                    return filtered
                })
            }
        }
    )

    return {
        statuses: data,
        error,
        refresh: mutate,
        isValidating,
        statusAnimations
    }
}