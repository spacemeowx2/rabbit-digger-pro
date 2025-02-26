import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { SelectNetPanel } from "./select"
import { InstancesPanel } from "./instances"

export const MainPage: React.FC = () => {
  return <>
    <Tabs defaultValue="instances" className="max-w-[1024px] mx-auto p-4">
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
      </TabsContent>
      <TabsContent value="connection">
      </TabsContent>
    </Tabs>
  </>
}
