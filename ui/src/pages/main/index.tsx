import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { SelectNetPanel } from "./select"

export const MainPage: React.FC = () => {
  return <>
    <Tabs defaultValue="overview" className="max-w-[1024px] mx-auto">
      <TabsList className="grid w-full grid-cols-3">
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="select">Select</TabsTrigger>
        <TabsTrigger value="connection">Connection</TabsTrigger>
      </TabsList>
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
