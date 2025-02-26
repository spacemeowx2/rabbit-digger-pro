import { useInstance } from "@/contexts/instance";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { Settings, AlertCircle, Loader2, Activity, Home } from "lucide-react";
import { NavLink } from "react-router";
import { useInstanceStatuses } from "@/hooks/use-instance-statuses";
import { cn } from "@/lib/utils";
import { getStatusTransitionClasses } from "@/lib/status-transition";

export function Navbar() {
    const { instances, currentInstance, setCurrentInstance } = useInstance();
    const { statuses, isValidating, statusAnimations } = useInstanceStatuses(
        currentInstance ? [currentInstance.url] : []
    );

    return (
        <nav className="border-b">
            <div className="px-4 h-14 flex items-center justify-between">
                <div className="flex items-center gap-6">
                    <NavLink to="/" className="hover:opacity-80 transition-opacity">
                        <h1 className="font-medium">Rabbit Digger Pro</h1>
                    </NavLink>
                    <div className="flex items-center gap-6">
                        <NavLink
                            to="/"
                            className={({ isActive }) => cn(
                                "flex items-center gap-2 px-2 py-1 rounded-md transition-colors",
                                isActive
                                    ? "text-primary font-medium"
                                    : "text-foreground/70 hover:text-foreground"
                            )}
                        >
                            <Home className="h-4 w-4" />
                            代理设置
                        </NavLink>
                        <NavLink
                            to="/connection"
                            className={({ isActive }) => cn(
                                "flex items-center gap-2 px-2 py-1 rounded-md transition-colors",
                                isActive
                                    ? "text-primary font-medium"
                                    : "text-foreground/70 hover:text-foreground"
                            )}
                        >
                            <Activity className="h-4 w-4" />
                            连接管理
                        </NavLink>
                    </div>
                </div>
                <div className="flex items-center gap-4">
                    <div className="flex items-center gap-2">
                        <Select
                            value={currentInstance?.id}
                            onValueChange={(value) => {
                                const instance = instances.find((inst) => inst.id === value);
                                if (instance) {
                                    setCurrentInstance(instance);
                                }
                            }}
                        >
                            <SelectTrigger className="w-[200px]">
                                <SelectValue placeholder="选择实例" />
                            </SelectTrigger>
                            <SelectContent>
                                {instances.map((instance) => (
                                    <SelectItem key={instance.id} value={instance.id}>
                                        {instance.name}
                                    </SelectItem>
                                ))}
                            </SelectContent>
                        </Select>
                        {currentInstance && (
                            <div className="flex items-center">
                                {isValidating ? (
                                    <Loader2 className="w-4 h-4 animate-spin text-muted-foreground" />
                                ) : (
                                    <AlertCircle
                                        className={cn(
                                            "w-4 h-4",
                                            getStatusTransitionClasses({
                                                url: currentInstance.url,
                                                isOnline: !!statuses[currentInstance.url],
                                                statusAnimations,
                                                isValidating
                                            })
                                        )}
                                    />
                                )}
                            </div>
                        )}
                    </div>
                    <NavLink
                        to="/settings"
                        className={({ isActive }) => cn(
                            "flex items-center gap-2 px-2 py-1 rounded-md transition-colors",
                            isActive
                                ? "text-primary font-medium"
                                : "text-foreground/70 hover:text-foreground"
                        )}
                    >
                        <Settings className="h-4 w-4" />
                        设置
                    </NavLink>
                </div>
            </div>
        </nav>
    );
}