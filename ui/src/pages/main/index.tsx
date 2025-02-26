import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card"
import { SelectNetPanel } from "./select"

export const MainPage: React.FC = () => {
  return (
    <div className="container py-4 max-w-[1024px] mx-auto">
      <Card>
        <CardHeader>
          <CardTitle>选择网络</CardTitle>
          <CardDescription>你可以在这里更改代理节点</CardDescription>
        </CardHeader>
        <CardContent className="space-y-2">
          <SelectNetPanel />
        </CardContent>
      </Card>
    </div>
  )
}
