import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card"

export const ConnectionPage = () => {
    return (
        <div className="container py-4 max-w-[1024px] mx-auto">
            <Card>
                <CardHeader>
                    <CardTitle>连接管理</CardTitle>
                    <CardDescription>查看和管理活动连接</CardDescription>
                </CardHeader>
                <CardContent>
                    {/* TODO: 添加连接管理功能 */}
                </CardContent>
            </Card>
        </div>
    )
}