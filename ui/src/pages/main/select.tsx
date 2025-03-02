import { useConfig, usePostSelect, useDelay } from '@/api/v1';
import type { DelayResponse } from '@/api/v1';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible';
import { Button } from '@/components/ui/button';
import { ChevronDown, Timer } from 'lucide-react';
import { useState } from 'react';
import { isSelectNet, SelectNet } from '@/api/rdp';
import { useInstance } from '@/contexts/instance';
import { cn } from '@/lib/utils';
import { clsx } from 'clsx';
import pLimit from 'p-limit';
import { useLocalStorage } from '@/hooks/use-local-storage';

// 并发测试数量
const CONCURRENT_TESTS = 5;

// Add helper function to format time with color
const formatLatency = (ms: number | undefined | null) => {
  if (ms === undefined || ms === null) {
    return { text: 'N/A', colorClass: 'text-red-500' };
  }
  const text = ms < 1 ? '<1ms' : ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${Math.round(ms)}ms`;
  const colorClass = ms < 400 ? 'text-green-500' : ms < 800 ? 'text-yellow-500' : 'text-red-500';
  return { text, colorClass };
};

interface SelectNetItemProps {
  netName: string;
  net: SelectNet;
  isOpen: boolean;
  onToggleOpen: () => void;
  onSelect: (selected: string) => void;
  testingStates: Record<string, boolean>;
  latencyResults: Record<string, DelayResponse | null>;
  onBatchSpeedTest: (netList: string[]) => void;
}

const SelectNetItem: React.FC<SelectNetItemProps> = ({
  netName,
  net,
  isOpen,
  onToggleOpen,
  onSelect,
  testingStates,
  latencyResults,
  onBatchSpeedTest,
}) => {
  return (
    <Collapsible
      open={isOpen}
      onOpenChange={onToggleOpen}
      className="border rounded-lg p-2"
    >
      <CollapsibleTrigger className="flex items-center justify-between w-full p-2">
        <span className="text-lg font-medium">{netName}</span>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            disabled={net.list?.some(item => testingStates[item])}
            onClick={(e) => {
              e.preventDefault();
              if (net.list) {
                onBatchSpeedTest(net.list);
              }
            }}
          >
            <Timer className={clsx("h-4 w-4", {
              "animate-spin": net.list?.some(item => testingStates[item])
            })} />
          </Button>
          <ChevronDown
            className={clsx("h-4 w-4 transition-transform duration-200", {
              "transform rotate-180": isOpen
            })}
          />
        </div>
      </CollapsibleTrigger>
      <CollapsibleContent className="mt-2 flex flex-wrap gap-2">
        {net.list?.map((item) => {
          const latency = latencyResults[item]?.response;
          const { text: latencyText, colorClass } = formatLatency(latency);

          return (
            <Button
              key={item}
              variant={item === net.selected ? 'default' : 'outline'}
              onClick={() => onSelect(item)}
              disabled={testingStates[item]}
            >
              {item}
              {item in latencyResults && (
                <span className={cn("ml-2 text-xs", colorClass)}>
                  {latencyText}
                </span>
              )}
            </Button>
          );
        })}
      </CollapsibleContent>
    </Collapsible>
  );
};

export const SelectNetPanel: React.FC = () => {
  const { currentInstance } = useInstance();
  const { data, error, mutate } = useConfig(currentInstance?.url);
  const { trigger } = usePostSelect(currentInstance?.url);
  const { trigger: testDelay } = useDelay(currentInstance?.url);
  const [openStates, setOpenStates] = useLocalStorage<Record<string, boolean>>('selectnet-open-states', {});
  const [testingStates, setTestingStates] = useState<Record<string, boolean>>({});
  const [latencyResults, setLatencyResults] = useState<Record<string, DelayResponse | null>>({});

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
      await trigger({ netName, selected });
      await mutate();
    } catch (err) {
      console.error('Failed to select net:', err);
    }
  };

  const handleBatchSpeedTest = async (netList: string[]) => {
    setTestingStates(prev => {
      const newState = { ...prev };
      netList.forEach(item => {
        newState[item] = true;
      });
      return newState;
    });
    try {
      const limit = pLimit(CONCURRENT_TESTS);
      const promises = netList.map(item =>
        limit(async () => {
          try {
            const latency = await testDelay({
              netName: item,
              url: 'http://www.gstatic.com/generate_204'
            });
            setLatencyResults(prev => ({
              ...prev,
              [item]: latency
            }));
            setTestingStates(prev => ({
              ...prev,
              [item]: false
            }));
            return { item, latency, error: null };
          } catch (error) {
            setLatencyResults(prev => ({
              ...prev,
              [item]: null
            }));
            setTestingStates(prev => ({
              ...prev,
              [item]: false
            }));
            return { item, latency: null, error };
          }
        })
      );
      await Promise.all(promises);
    } catch (error) {
      console.error('Failed to complete speed tests:', error);
    } finally {
      setTestingStates(prev => {
        const newState = { ...prev };
        netList.forEach(item => {
          newState[item] = false;
        });
        return newState;
      });
    }
  };

  return (
    <div className="space-y-4">
      {selectNets.map(([netName, net]) => (
        <SelectNetItem
          key={netName}
          netName={netName}
          net={net}
          isOpen={openStates[netName]}
          onToggleOpen={() => toggleOpen(netName)}
          onSelect={(selected) => handleSelect(netName, selected)}
          testingStates={testingStates}
          latencyResults={latencyResults}
          onBatchSpeedTest={handleBatchSpeedTest}
        />
      ))}
    </div>
  );
};
