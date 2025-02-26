import { MainPage } from "@/pages/main"
import { SettingsPage } from "@/pages/settings"
import { ConnectionPage } from "@/pages/connection"
import { InstanceProvider } from "@/contexts/instance-provider"
import { Toaster } from "@/components/ui/sonner"
import { Navbar } from "@/components/Navbar"
import { BrowserRouter, Routes, Route } from "react-router";

function Layout({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen flex flex-col">
      <Navbar />
      <main className="flex-1">
        {children}
      </main>
    </div>
  )
}

function App() {
  return (
    <InstanceProvider>
      <BrowserRouter>
        <Layout>
          <Routes>
            <Route path="/" element={<MainPage />} />
            <Route path="/settings" element={<SettingsPage />} />
            <Route path="/connection" element={<ConnectionPage />} />
          </Routes>
        </Layout>
      </BrowserRouter>
      <Toaster />
    </InstanceProvider>
  )
}

export default App
