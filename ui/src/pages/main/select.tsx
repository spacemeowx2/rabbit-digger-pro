import { useConfig, useSelect, fetcher } from '@/api/v1';
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
import pLimit from 'p-limit';

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

export const SelectNetPanel: React.FC = () => {
  const { currentInstance } = useInstance();
  const { data, error, mutate } = useConfig(currentInstance?.url);
  const { select } = useSelect(currentInstance?.url);
  const [openStates, setOpenStates] = useState<Record<string, boolean>>({});
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
      await select(netName, selected);
      await mutate();
    } catch (err) {
      console.error("Failed to select net:", err);
    }
  };

  const handleBatchSpeedTest = async (netList: string[]) => {
    // Mark all nodes in this group as testing
    setTestingStates(prev => {
      const newState = { ...prev };
      netList.forEach(item => {
        newState[item] = true;
      });
      return newState;
    });

    try {
      // Create a limit function
      const limit = pLimit(CONCURRENT_TESTS);

      // Create an array of limited promises
      const promises = netList.map(item =>
        limit(async () => {
          try {
            const latency = await fetcher<'/net/:netName/delay', 'get'>([
              '/net/:netName/delay',
              'get',
              { url: 'http://www.gstatic.com/generate_204' },
              { netName: item },
              currentInstance?.url
            ]);

            // Update result immediately when each test completes
            setLatencyResults(prev => ({
              ...prev,
              [item]: latency
            }));

            // Clear testing state for this item
            setTestingStates(prev => ({
              ...prev,
              [item]: false
            }));

            return { item, latency, error: null };
          } catch (error) {
            // Update result and clear testing state even on error
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

      // Wait for all tests to complete
      await Promise.all(promises);
    } catch (error) {
      console.error('Failed to complete speed tests:', error);
    } finally {
      // Ensure all testing states are cleared in case of unexpected errors
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
        <Collapsible
          key={netName}
          open={openStates[netName]}
          onOpenChange={() => toggleOpen(netName)}
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
                  e.preventDefault(); // Prevent triggering Collapsible
                  if (net.list) {
                    handleBatchSpeedTest(net.list);
                  }
                }}
              >
                <Timer className={`h-4 w-4 ${net.list?.some(item => testingStates[item]) ? 'animate-spin' : ''}`} />
              </Button>
              <ChevronDown
                className={`h-4 w-4 transition-transform duration-200 ${openStates[netName] ? "transform rotate-180" : ""
                  }`}
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
                  onClick={() => handleSelect(netName, item)}
                  disabled={testingStates[item]}
                >
                  {item}
                  {item in latencyResults && (
                    <span className={cn('ml-2 text-xs', colorClass)}>
                      {latencyText}
                    </span>
                  )}
                </Button>
              );
            })}
          </CollapsibleContent>
        </Collapsible>
      ))}
    </div>
  );
};
