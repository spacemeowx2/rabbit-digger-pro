import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card"

export const SettingsPage = () => {
    return (
        <div className="container py-4 max-w-[1024px] mx-auto">
            <Card>
                <CardHeader>
                    <CardTitle>设置</CardTitle>
                    <CardDescription>配置 Rabbit Digger Pro</CardDescription>
                </CardHeader>
                <CardContent>
                    {/* TODO: Add settings content */}
                </CardContent>
            </Card>
        </div>
    )
}