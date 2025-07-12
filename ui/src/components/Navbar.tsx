import { useInstance } from "@/contexts/instance";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Settings, AlertCircle, Loader2, Activity, Home, Menu, ScrollText } from "lucide-react";
import { NavLink } from "react-router";
import { useInstanceStatuses } from "@/hooks/use-instance-statuses";
import { cn } from "@/lib/utils";
import { getStatusTransitionClasses } from "@/lib/status-transition";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Button } from "@/components/ui/button";
import { useIsMobile } from "@/hooks/use-mobile";

export function Navbar() {
  const { instances, currentInstance, setCurrentInstance } = useInstance();
  const { statuses, isValidating, statusAnimations } = useInstanceStatuses(
    currentInstance ? [currentInstance.url] : []
  );
  const isMobile = useIsMobile();

  return (
    <nav className="border-b bg-white/95 dark:bg-slate-950/95 backdrop-blur-md border-slate-200/50 dark:border-slate-800/50 shadow-sm">
      <div className="px-4 h-16 flex items-center justify-between max-w-7xl mx-auto">
          {/* 移动端汉堡菜单靠左对齐 */}
          {isMobile && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon">
                  <Menu className="h-5 w-5" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-48">
                <DropdownMenuItem asChild>
                  <NavLink
                    to="/"
                    className="w-full flex items-center gap-2"
                  >
                    <Home className="h-4 w-4" />
                    代理设置
                  </NavLink>
                </DropdownMenuItem>
                <DropdownMenuItem asChild>
                  <NavLink
                    to="/connection"
                    className="w-full flex items-center gap-2"
                  >
                    <Activity className="h-4 w-4" />
                    连接管理
                  </NavLink>
                </DropdownMenuItem>
                <DropdownMenuItem asChild>
                  <NavLink
                    to="/logs"
                    className="w-full flex items-center gap-2"
                  >
                    <ScrollText className="h-4 w-4" />
                    日志管理
                  </NavLink>
                </DropdownMenuItem>
                <DropdownMenuItem asChild>
                  <NavLink
                    to="/settings"
                    className="w-full flex items-center gap-2"
                  >
                    <Settings className="h-4 w-4" />
                    设置
                  </NavLink>
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          )}

          {/* 只在非移动端显示标题 */}
          {!isMobile && (
            <NavLink to="/" className="hover:opacity-80 transition-opacity">
              <h1 className="text-xl font-bold bg-gradient-to-r from-blue-600 to-purple-600 bg-clip-text text-transparent">
                Rabbit Digger Pro
              </h1>
            </NavLink>
          )}

          {/* 桌面导航 */}
          {!isMobile && (
            <div className="flex items-center gap-6">
<NavLink
                to="/"
                className={({ isActive }) => cn(
                  "flex items-center gap-2 px-3 py-2 rounded-lg transition-all duration-200",
                  isActive
                    ? "bg-gradient-to-r from-blue-500 to-purple-500 text-white shadow-md shadow-blue-500/25"
                    : "text-foreground/70 hover:text-foreground hover:bg-slate-100 dark:hover:bg-slate-800"
                )}
              >
                <Home className="h-4 w-4" />
                代理设置
              </NavLink>
              <NavLink
                to="/connection"
                className={({ isActive }) => cn(
                  "flex items-center gap-2 px-3 py-2 rounded-lg transition-all duration-200",
                  isActive
                    ? "bg-gradient-to-r from-blue-500 to-purple-500 text-white shadow-md shadow-blue-500/25"
                    : "text-foreground/70 hover:text-foreground hover:bg-slate-100 dark:hover:bg-slate-800"
                )}
              >
                <Activity className="h-4 w-4" />
                连接管理
              </NavLink>
              <NavLink
                to="/logs"
                className={({ isActive }) => cn(
                  "flex items-center gap-2 px-3 py-2 rounded-lg transition-all duration-200",
                  isActive
                    ? "bg-gradient-to-r from-blue-500 to-purple-500 text-white shadow-md shadow-blue-500/25"
                    : "text-foreground/70 hover:text-foreground hover:bg-slate-100 dark:hover:bg-slate-800"
                )}
              >
                <ScrollText className="h-4 w-4" />
                日志管理
              </NavLink>
            </div>
          )}
        </div>

        <div className="flex items-center gap-4">
          <div className={cn("flex items-center", isMobile ? "gap-1" : "gap-2")}>
            <Select
              value={currentInstance?.id}
              onValueChange={(value) => {
                const instance = instances.find((inst) => inst.id === value);
                if (instance) {
                  setCurrentInstance(instance);
                }
              }}
            >
              <SelectTrigger className={cn(isMobile ? "w-36" : "w-[200px] bg-white/50 dark:bg-slate-800/50 border-slate-200 dark:border-slate-700")}>
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

          {/* 桌面版设置按钮 */}
          {!isMobile && (
            <NavLink
              to="/settings"
              className={({ isActive }) => cn(
                "flex items-center gap-2 px-3 py-2 rounded-lg transition-all duration-200",
                isActive
                  ? "bg-gradient-to-r from-blue-500 to-purple-500 text-white shadow-md shadow-blue-500/25"
                  : "text-foreground/70 hover:text-foreground hover:bg-slate-100 dark:hover:bg-slate-800"
              )}
            >
              <Settings className="h-4 w-4" />
              设置
            </NavLink>
          )}
        </div>
      </div>
    </nav>
  );
}
