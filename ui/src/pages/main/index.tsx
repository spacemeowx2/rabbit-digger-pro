import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { SelectNetPanel } from "./select"
import { InstancesPanel } from "./instances"

export const MainPage: React.FC = () => {
  return (
    <div className="container py-4 max-w-[1024px] mx-auto">
      <Tabs defaultValue="instances">
        <TabsList className="grid w-full grid-cols-4">
          <TabsTrigger value="instances">实例</TabsTrigger>
          <TabsTrigger value="overview">概览</TabsTrigger>
          <TabsTrigger value="select">选择</TabsTrigger>
          <TabsTrigger value="connection">连接</TabsTrigger>
        </TabsList>
        <TabsContent value="instances">
          <InstancesPanel />
        </TabsContent>
        <TabsContent value="overview">
          <Card>
            <CardHeader>
              <CardTitle>选择网络</CardTitle>
              <CardDescription>你可以在这里更改代理节点</CardDescription>
            </CardHeader>
            <CardContent className="space-y-2">
              <SelectNetPanel />
            </CardContent>
          </Card>
        </TabsContent>
        <TabsContent value="select">
          <Card>
            <CardHeader>
              <CardTitle>网络选择</CardTitle>
              <CardDescription>选择和管理代理网络</CardDescription>
            </CardHeader>
            <CardContent>
              <SelectNetPanel />
            </CardContent>
          </Card>
        </TabsContent>
        <TabsContent value="connection">
          <Card>
            <CardHeader>
              <CardTitle>连接管理</CardTitle>
              <CardDescription>查看和管理活动连接</CardDescription>
            </CardHeader>
            <CardContent>
              {/* TODO: Add connection management panel */}
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  )
}
