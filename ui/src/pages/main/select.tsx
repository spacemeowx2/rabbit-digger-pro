import { useConfig, useSelect } from "@/api/v1";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Button } from "@/components/ui/button";
import { ChevronDown } from "lucide-react";
import { useState } from "react";
import { isSelectNet, SelectNet } from "@/api/rdp";

export const SelectNetPanel: React.FC = () => {
  const { data, error } = useConfig("http://127.0.0.1:8030");
  const [openStates, setOpenStates] = useState<Record<string, boolean>>({});

  if (error) {
    console.error(error);
    return <div className="text-red-500">Failed to load</div>;
  }

  if (!data) {
    return <div>Loading...</div>;
  }

  const selectNets = Object.entries(data.net).flatMap(([key, net]): Array<[string, SelectNet]> =>
    isSelectNet(net) ? [[key, net]] : []
  );

  const toggleOpen = (netName: string) => {
    setOpenStates((prev) => ({
      ...prev,
      [netName]: !prev[netName],
    }));
  };

  const handleSelect = async (netName: string, selected: string) => {
    try {
      await useSelect(netName, selected, "http://127.0.0.1:8030");
    } catch (err) {
      console.error("Failed to select net:", err);
    }
  };

  return (
    <div className="space-y-4">
      {selectNets.map(([netName, net]) => (
        <Collapsible
          key={netName}
          open={openStates[netName]}
          onOpenChange={() => toggleOpen(netName)}
          className="border rounded-lg p-2"
        >
          <CollapsibleTrigger className="flex items-center justify-between w-full">
            <span className="text-lg font-medium">{netName}</span>
            <ChevronDown
              className={`h-4 w-4 transition-transform duration-200 ${openStates[netName] ? "transform rotate-180" : ""
                }`}
            />
          </CollapsibleTrigger>
          <CollapsibleContent className="mt-2 flex flex-wrap gap-2">
            {net.list?.map((item) => (
              <Button
                key={item}
                variant={item === net.selected ? "default" : "outline"}
                onClick={() => handleSelect(netName, item)}
              >
                {item}
              </Button>
            ))}
          </CollapsibleContent>
        </Collapsible>
      ))}
    </div>
  );
};
