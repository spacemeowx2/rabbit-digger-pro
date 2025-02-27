import { useState, useEffect } from "react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Loader2, AlertCircle } from "lucide-react"
import type { RDPInstance } from "@/contexts/instance"
import { useInstanceStatuses } from "@/hooks/use-instance-statuses"
import { getStatusTransitionClasses } from "@/lib/status-transition"
import { cn } from "@/lib/utils"

const defaultInstance: RDPInstance = {
  id: '',
  name: '',
  url: '',
  isActive: false
}

interface InstanceDialogProps {
  mode: 'add' | 'edit'
  instance?: RDPInstance
  open: boolean
  onOpenChange: (open: boolean) => void
  onSave: (instance: RDPInstance) => void
}

export function InstanceDialog({
  mode,
  instance,
  open,
  onOpenChange,
  onSave
}: InstanceDialogProps) {
  const [editedInstance, setEditedInstance] = useState(instance || defaultInstance)
  const [urlsToCheck, setUrlsToCheck] = useState<string[]>([])
  const { statuses, refresh, isValidating, statusAnimations } = useInstanceStatuses(urlsToCheck)
  const [urlError, setUrlError] = useState<string | null>(null)

  useEffect(() => {
    if (mode === 'edit' && instance) {
      setEditedInstance(instance)
      if (instance.url) {
        setUrlsToCheck([instance.url])
      }
    } else if (mode === 'add') {
      setEditedInstance(defaultInstance)
      setUrlsToCheck([])
    }
  }, [instance, mode, open])

  const handleRefresh = (e: React.MouseEvent) => {
    e.preventDefault()
    if (editedInstance.url && !urlError) {
      setUrlsToCheck([editedInstance.url])
      refresh()
    }
  }

  const handleUrlChange = (value: string) => {
    try {
      if (value) {
        new URL(value)
        setUrlError(null)
      } else {
        setUrlError(null)
      }
    } catch {
      setUrlError("请输入有效的 URL（例如：http://localhost:8030）")
    }
    setEditedInstance({ ...editedInstance, url: value })
  }

  const canSave = Boolean(
    editedInstance.name &&
    editedInstance.url &&
    !urlError &&
    !isValidating &&
    urlsToCheck.includes(editedInstance.url) &&
    statuses[editedInstance.url]
  )

  const handleSave = () => {
    if (canSave) {
      const finalInstance: RDPInstance = mode === 'add'
        ? {
          ...editedInstance,
          id: Date.now().toString(), // Generate new ID for new instances
        }
        : editedInstance

      onSave(finalInstance)
      if (mode === 'add') {
        setEditedInstance(defaultInstance)
        setUrlsToCheck([])
      }
      onOpenChange(false)
    }
  }

  const title = mode === 'add' ? '添加新实例' : '编辑实例'
  const description = mode === 'add' ? '添加新的 RDP 实例连接' : '修改 RDP 实例的名称和连接地址'
  const buttonText = mode === 'add' ? '添加' : '保存'

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {description}
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-4 py-4">
          <div className="grid gap-2">
            <Input
              placeholder="实例名称"
              value={editedInstance.name}
              onChange={(e) => setEditedInstance({ ...editedInstance, name: e.target.value })}
            />
          </div>
          <div className="grid gap-2">
            <div className="flex gap-2">
              <Input
                placeholder="实例地址（例如：http://localhost:8030）"
                value={editedInstance.url}
                onChange={(e) => handleUrlChange(e.target.value)}
                className={urlError ? "border-red-500" : ""}
              />
              <Button
                variant="outline"
                onClick={handleRefresh}
                disabled={isValidating || !editedInstance.url || !!urlError}
              >
                {isValidating ? (
                  <Loader2 className="w-4 h-4 animate-spin" />
                ) : (
                  "检查连接"
                )}
              </Button>
            </div>
            {urlError ? (
              <p className="text-sm text-red-500">{urlError}</p>
            ) : editedInstance.url && urlsToCheck.includes(editedInstance.url) && (
              <div className="flex items-center gap-2">
                {isValidating ? (
                  <Loader2 className="w-4 h-4 animate-spin text-muted-foreground" />
                ) : (
                  <AlertCircle
                    className={cn(
                      "w-4 h-4",
                      getStatusTransitionClasses({
                        url: editedInstance.url,
                        isOnline: !!statuses[editedInstance.url],
                        statusAnimations,
                        isValidating
                      })
                    )}
                  />
                )}
                <p className={cn(
                  "text-sm",
                  getStatusTransitionClasses({
                    url: editedInstance.url,
                    isOnline: !!statuses[editedInstance.url],
                    statusAnimations,
                    isValidating
                  })
                )}>
                  {statuses[editedInstance.url] ? "连接正常" : "无法连接，请检查地址是否正确且服务是否运行"}
                </p>
              </div>
            )}
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button
            onClick={handleSave}
            disabled={!canSave}
          >
            {isValidating ? (
              <>
                <Loader2 className="w-4 h-4 animate-spin mr-2" />
                检查连接中...
              </>
            ) : (
              buttonText
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
