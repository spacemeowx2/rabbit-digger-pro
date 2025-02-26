import type { StatusTransitionOptions } from "@/contexts/instance-types"

export function getStatusTransitionClasses({
    url,
    isOnline,
    statusAnimations,
    isValidating
}: StatusTransitionOptions): string {
    if (isValidating) return "text-muted-foreground"

    const animation = statusAnimations[url]
    const now = Date.now()
    const isAnimating = animation && (now - animation.timestamp < 2000)

    if (!isAnimating) {
        return isOnline ? "text-green-500" : "text-red-500"
    }

    // 从离线到在线的转换
    if (!animation.prevStatus && isOnline) {
        return "text-green-500 animate-bounce"
    }

    // 从在线到离线的转换
    if (animation.prevStatus && !isOnline) {
        return "text-red-500 animate-pulse"
    }

    return isOnline ? "text-green-500" : "text-red-500"
}