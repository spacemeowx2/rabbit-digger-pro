import { MainPage } from "@/pages/main"
import { InstanceProvider } from "@/contexts/instance-provider"
import { Toaster } from "@/components/ui/sonner"

function App() {
  return (
    <InstanceProvider>
      <MainPage />
      <Toaster />
    </InstanceProvider>
  )
}

export default App
