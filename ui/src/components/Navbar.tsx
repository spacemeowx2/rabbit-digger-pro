import { useInstance } from "@/contexts/instance";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { Settings } from "lucide-react";
import { NavLink } from "react-router";

export function Navbar() {
    const { instances, currentInstance, setCurrentInstance } = useInstance();

    return (
        <nav className="border-b">
            <div className="px-4 h-14 flex items-center justify-between">
                <div className="flex items-center gap-4">
                    <h1 className="font-medium">Rabbit Digger Pro</h1>
                    <Select
                        value={currentInstance?.id}
                        onValueChange={(value) => {
                            const instance = instances.find((inst) => inst.id === value);
                            if (instance) {
                                setCurrentInstance(instance);
                            }
                        }}
                    >
                        <SelectTrigger className="w-[200px]">
                            <SelectValue placeholder="Select instance" />
                        </SelectTrigger>
                        <SelectContent>
                            {instances.map((instance) => (
                                <SelectItem key={instance.id} value={instance.id}>
                                    {instance.name}
                                </SelectItem>
                            ))}
                        </SelectContent>
                    </Select>
                </div>
                <div className="flex items-center gap-2">
                    <Button variant="ghost" size="icon" asChild>
                        <NavLink to="/settings">
                            <Settings className="h-4 w-4" />
                        </NavLink>
                    </Button>
                </div>
            </div>
        </nav>
    );
}