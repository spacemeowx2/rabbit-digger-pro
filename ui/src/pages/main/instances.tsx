import { useState, useMemo } from "react"
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card"
import { Button } from "@/components/ui/button"
import { Check, Settings2, Trash2, AlertCircle, Loader2, Plus } from "lucide-react"
import { useInstance } from "@/contexts/instance"
import type { RDPInstance } from "@/contexts/instance"
import { InstanceDialog } from "@/components/instance-dialog"
import { DeleteInstanceDialog } from "@/components/delete-instance-dialog"
import { useInstanceStatuses } from "@/hooks/use-instance-statuses"
import { getStatusTransitionClasses } from "@/lib/status-transition"
import { cn } from "@/lib/utils"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Separator } from "@/components/ui/separator"

export const InstancesPanel: React.FC = () => {
    const { instances, setInstances, currentInstance, setCurrentInstance } = useInstance()
    const [editingInstance, setEditingInstance] = useState<RDPInstance | null>(null)
    const [deletingInstance, setDeletingInstance] = useState<RDPInstance | null>(null)
    const [addDialogOpen, setAddDialogOpen] = useState(false)

    const urls = instances.map(i => i.url).filter(Boolean)
    const { statuses, refresh, isValidating, statusAnimations } = useInstanceStatuses(urls)

    // 对实例列表进行排序，活跃实例在前
    const sortedInstances = useMemo(() => {
        return [...instances].sort((a, b) => {
            if (a.id === currentInstance?.id) return -1
            if (b.id === currentInstance?.id) return 1
            if (a.isActive && !b.isActive) return -1
            if (!a.isActive && b.isActive) return 1
            return 0
        })
    }, [instances, currentInstance])

    // 包装 refresh 函数以正确处理点击事件
    const handleRefresh = (e: React.MouseEvent) => {
        e.preventDefault()
        refresh()
    }

    const handleSelectInstance = (instance: RDPInstance) => {
        const updatedInstances = instances.map(i => ({
            ...i,
            isActive: i.id === instance.id
        }))
        setInstances(updatedInstances)
        setCurrentInstance(instance)
    }

    const handleEditInstance = (instance: RDPInstance) => {
        const updatedInstances = instances.map(i =>
            i.id === instance.id ? instance : i
        )
        setInstances(updatedInstances)
        if (currentInstance?.id === instance.id) {
            setCurrentInstance(instance)
        }
    }

    const handleDeleteInstance = (instanceId: string) => {
        const updatedInstances = instances.filter(i => i.id !== instanceId)
        setInstances(updatedInstances)
        if (currentInstance?.id === instanceId && updatedInstances.length > 0) {
            setCurrentInstance(updatedInstances[0])
        }
        setDeletingInstance(null)
    }

    const handleAddInstance = (instance: RDPInstance) => {
        setInstances([...instances, instance])
    }

    // 添加新的实例项组件来减少重复代码
    const InstanceStatus = ({ url }: { url: string }) => {
        if (isValidating) {
            return <Loader2 className="w-4 h-4 animate-spin text-muted-foreground" />
        }

        const transitionClasses = getStatusTransitionClasses({
            url,
            isOnline: !!statuses[url],
            statusAnimations,
            isValidating
        })

        return (
            <AlertCircle className={cn("w-4 h-4", transitionClasses)} />
        )
    }

    return (
        <Card className="flex flex-col max-h-[80vh]">
            <CardHeader className="flex-none">
                <div className="flex items-center justify-between">
                    <div>
                        <CardTitle>RDP 实例</CardTitle>
                        <CardDescription>管理多个 RDP 实例，快速切换不同环境</CardDescription>
                    </div>
                    <Button
                        variant="outline"
                        size="sm"
                        onClick={handleRefresh}
                        disabled={isValidating}
                    >
                        {isValidating ? (
                            <Loader2 className="w-4 h-4 animate-spin" />
                        ) : (
                            <AlertCircle className="w-4 h-4" />
                        )}
                        <span className="ml-2">
                            {isValidating ? "检查中..." : "检查连接"}
                        </span>
                    </Button>
                </div>
            </CardHeader>

            <ScrollArea className="flex-1">
                <CardContent className="space-y-4">
                    <div className="grid gap-4">
                        {sortedInstances.map((instance) => (
                            <div
                                key={instance.id}
                                className={cn(
                                    "flex items-center justify-between p-2 border rounded-lg",
                                    "transition-colors duration-300",
                                    instance.id === currentInstance?.id
                                        ? "bg-accent/50"
                                        : "hover:bg-accent/30"
                                )}
                            >
                                <div className="flex items-center gap-2">
                                    <Button
                                        variant={instance.id === currentInstance?.id ? "default" : "ghost"}
                                        size="sm"
                                        onClick={() => handleSelectInstance(instance)}
                                    >
                                        {instance.id === currentInstance?.id && <Check className="w-4 h-4 mr-1" />}
                                        {instance.name}
                                    </Button>
                                    <span className="text-sm text-muted-foreground">
                                        {instance.url}
                                    </span>
                                    <InstanceStatus url={instance.url} />
                                </div>
                                <div className="flex gap-2">
                                    <Button
                                        variant="ghost"
                                        size="icon"
                                        onClick={() => setEditingInstance(instance)}
                                    >
                                        <Settings2 className="w-4 h-4" />
                                    </Button>
                                    {instances.length > 1 && (
                                        <Button
                                            variant="ghost"
                                            size="icon"
                                            onClick={() => setDeletingInstance(instance)}
                                            className="text-destructive hover:text-destructive"
                                        >
                                            <Trash2 className="w-4 h-4" />
                                        </Button>
                                    )}
                                </div>
                            </div>
                        ))}
                    </div>
                </CardContent>
            </ScrollArea>

            <Separator className="my-4" />

            <CardContent className="flex-none pt-0">
                <Button
                    onClick={() => setAddDialogOpen(true)}
                    className="w-full"
                >
                    <Plus className="w-4 h-4 mr-2" />
                    添加新实例
                </Button>
            </CardContent>

            {editingInstance && (
                <InstanceDialog
                    mode="edit"
                    instance={editingInstance}
                    open={!!editingInstance}
                    onOpenChange={(open) => !open && setEditingInstance(null)}
                    onSave={handleEditInstance}
                />
            )}

            {deletingInstance && (
                <DeleteInstanceDialog
                    instanceName={deletingInstance.name}
                    open={!!deletingInstance}
                    onOpenChange={(open) => !open && setDeletingInstance(null)}
                    onConfirm={() => handleDeleteInstance(deletingInstance.id)}
                />
            )}

            <InstanceDialog
                mode="add"
                open={addDialogOpen}
                onOpenChange={setAddDialogOpen}
                onSave={handleAddInstance}
            />
        </Card>
    )
}