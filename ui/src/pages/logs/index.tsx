import { useState, useMemo, useRef, useEffect } from 'react';
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Badge } from '@/components/ui/badge';
import { Search, Play, Pause, Trash2, AlertCircle } from 'lucide-react';
import { useLogsStream, LogContext } from '@/api/v1';
import { useInstance } from '@/contexts/instance';
import clsx from 'clsx';

// 日志级别颜色映射（全部转为小写处理）
const LOG_LEVEL_COLORS: Record<string, string> = {
  error: 'bg-red-100 text-red-800 border-red-200',
  warn: 'bg-amber-100 text-amber-800 border-amber-200',
  warning: 'bg-amber-100 text-amber-800 border-amber-200',
  info: 'bg-emerald-100 text-emerald-800 border-emerald-200',
  debug: 'bg-purple-100 text-purple-800 border-purple-200',
  trace: 'bg-gray-100 text-gray-600 border-gray-200',
} as const;

// 格式化连接信息组件
function ConnectionDetails({ ctx }: { ctx: LogContext }) {
  if (!ctx.dest_socket_addr && !ctx.src_socket_addr && !ctx.dest_domain) {
    return null;
  }

  return (
    <div className="flex flex-wrap gap-2 mt-1">
      {ctx.src_socket_addr && (
        <Badge variant="outline" className="bg-indigo-50 text-indigo-700 border-indigo-200">
          来源: {ctx.src_socket_addr}
        </Badge>
      )}
      {ctx.dest_domain && (
        <Badge variant="outline" className="bg-violet-50 text-violet-700 border-violet-200">
          域名: {ctx.dest_domain}
        </Badge>
      )}
      {ctx.dest_socket_addr && (
        <Badge variant="outline" className="bg-blue-50 text-blue-700 border-blue-200">
          目标: {ctx.dest_socket_addr}
        </Badge>
      )}
      {ctx.net_list && ctx.net_list.length > 0 && (
        <Badge variant="outline" className="bg-emerald-50 text-emerald-700 border-emerald-200">
          路由: {ctx.net_list.join(' → ')}
        </Badge>
      )}
    </div>
  );
}

export const LogsPage = () => {
  const { currentInstance } = useInstance();
  const { logs, isPaused, togglePause, clearLogs, isConnected } = useLogsStream(currentInstance?.url);
  const [searchQuery, setSearchQuery] = useState('');
  const scrollAreaRef = useRef<HTMLDivElement>(null);
  const [shouldAutoScroll, setShouldAutoScroll] = useState(true);

  // 监听滚动事件
  const handleScroll = () => {
    const scrollArea = scrollAreaRef.current;
    if (!scrollArea) return;

    const { scrollTop, scrollHeight, clientHeight } = scrollArea;
    // 判断是否在底部附近（距离底部30px以内）
    const isNearBottom = scrollHeight - scrollTop - clientHeight < 30;
    setShouldAutoScroll(isNearBottom);
  };

  // 自动滚动到底部
  useEffect(() => {
    if (shouldAutoScroll && !isPaused && scrollAreaRef.current) {
      const scrollArea = scrollAreaRef.current;
      scrollArea.scrollTop = scrollArea.scrollHeight;
    }
  }, [logs, shouldAutoScroll, isPaused]);

  const filteredLogs = useMemo(() => {
    if (!searchQuery) return logs;

    const searchLower = searchQuery.toLowerCase();
    return logs.filter(log => {
      const fieldsStr = log.fields?.ctx ? JSON.stringify(log.fields) : '';
      return (
        log.message?.toLowerCase().includes(searchLower) ||
        log.level?.toLowerCase().includes(searchLower) ||
        log.target?.toLowerCase().includes(searchLower) ||
        fieldsStr.toLowerCase().includes(searchLower)
      );
    });
  }, [logs, searchQuery]);

  const formatTime = (timestamp: string) => {
    try {
      const date = new Date(timestamp);
      // 手动格式化毫秒，因为 Intl.DateTimeFormat 不支持毫秒的显示
      const formatter = new Intl.DateTimeFormat('zh-CN', {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
      });
      const ms = date.getMilliseconds().toString().padStart(3, '0');
      return `${formatter.format(date)}.${ms}`;
    } catch {
      return timestamp;
    }
  };

  return (
    <div className="h-[calc(100vh-56px)] p-4">
      <Card className="h-full flex flex-col">
        <CardHeader className="pb-2 shrink-0">
          <div className="flex justify-between items-center">
            <div className="flex items-center gap-2">
              <CardTitle className="text-xl text-gray-800">日志</CardTitle>
              <Badge
                variant="outline"
                className={clsx(
                  'transition-colors',
                  isConnected
                    ? 'bg-emerald-50 text-emerald-700 border-emerald-200'
                    : 'bg-amber-50 text-amber-700 border-amber-200'
                )}
              >
                {isConnected ? '已连接' : '连接中...'}
              </Badge>
            </div>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={togglePause}
                className={clsx(isPaused && 'bg-amber-50 border-amber-200 text-amber-700')}
              >
                {isPaused ? <Play className="h-4 w-4 mr-1" /> : <Pause className="h-4 w-4 mr-1" />}
                {isPaused ? '继续' : '暂停'}
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={clearLogs}
              >
                <Trash2 className="h-4 w-4 mr-1" />
                清空
              </Button>
            </div>
          </div>
          <div className="relative w-full max-w-md mt-2">
            <Search className="absolute left-2 top-2.5 h-4 w-4 text-gray-500" />
            <Input
              placeholder="搜索日志..."
              className="pl-8 bg-white border-gray-300"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </div>
        </CardHeader>
        <CardContent className="p-0 flex-1 overflow-hidden">
          <div
            ref={scrollAreaRef}
            className="h-full overflow-auto"
            onScroll={handleScroll}
          >
            <div className="p-4 space-y-3">
              {filteredLogs.map((log, index) => (
                <div
                  key={index}
                  className={clsx(
                    'text-sm rounded-lg p-2 transition-colors',
                    log.level?.toLowerCase() === 'error' && 'bg-red-50',
                    (log.level?.toLowerCase() === 'warn' || log.level?.toLowerCase() === 'warning') && 'bg-amber-50',
                  )}
                >
                  <div className="flex items-start gap-2">
                    <span className="text-gray-500 shrink-0 font-mono">
                      {formatTime(log.timestamp)}
                    </span>
                    <Badge
                      variant="outline"
                      className={LOG_LEVEL_COLORS[log.level?.toLowerCase()] || LOG_LEVEL_COLORS.trace}
                    >
                      {log.level?.toLowerCase() === 'error' && <AlertCircle className="h-3 w-3 mr-1" />}
                      {log.level}
                    </Badge>
                    {log.target && (
                      <Badge variant="outline" className="bg-gray-50 text-gray-700 border-gray-200">
                        {log.target}
                      </Badge>
                    )}
                  </div>

                  <div className="mt-1 text-gray-700 font-medium pl-[84px]">
                    {log.fields?.message || log.message}
                  </div>

                  {log.fields?.parsedCtx && (
                    <div className="pl-[84px]">
                      <ConnectionDetails ctx={log.fields.parsedCtx} />
                    </div>
                  )}

                  {/* 显示其他字段信息 */}
                  {log.fields && Object.entries(log.fields)
                    .filter(([key]) => !['message', 'ctx', 'parsedCtx'].includes(key))
                    .map(([key, value]) => (
                      <div key={key} className="pl-[84px] mt-1 text-xs text-gray-500">
                        <span className="font-medium">{key}:</span> {JSON.stringify(value)}
                      </div>
                    ))}
                </div>
              ))}
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
};
