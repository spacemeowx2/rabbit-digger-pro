import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card"
import { SelectNetPanel } from "./select"

export const MainPage: React.FC = () => {
  return (
    <div className="min-h-screen bg-gradient-to-br from-slate-50 via-white to-slate-100 dark:from-slate-900 dark:via-slate-950 dark:to-slate-900">
      <div className="container py-8 max-w-6xl mx-auto px-4">
        <div className="text-center mb-8">
          <h1 className="text-4xl font-bold bg-gradient-to-r from-blue-600 to-purple-600 bg-clip-text text-transparent mb-2">
            网络代理管理
          </h1>
          <p className="text-lg text-muted-foreground">
            智能选择最优代理节点，提升网络体验
          </p>
        </div>
        
        <Card className="border-0 shadow-xl bg-white/80 dark:bg-slate-900/80 backdrop-blur-sm">
          <CardHeader className="border-b border-slate-200/50 dark:border-slate-800/50">
            <div className="flex items-center justify-between">
              <div>
                <CardTitle className="text-2xl font-semibold flex items-center gap-2">
                  <div className="w-2 h-8 bg-gradient-to-b from-blue-500 to-purple-500 rounded-full"></div>
                  节点选择
                </CardTitle>
                <CardDescription className="text-base mt-1">
                  选择最适合您的代理节点，支持延迟测试和批量管理
                </CardDescription>
              </div>
            </div>
          </CardHeader>
          <CardContent className="p-6">
            <SelectNetPanel />
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
