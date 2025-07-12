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
      className="border border-slate-200 dark:border-slate-700 rounded-xl bg-white/50 dark:bg-slate-800/50 backdrop-blur-sm shadow-sm hover:shadow-md transition-all duration-300"
    >
      <CollapsibleTrigger className="flex items-center justify-between w-full p-4 hover:bg-slate-50/50 dark:hover:bg-slate-800/50 rounded-xl transition-all duration-200">
        <div className="flex items-center gap-3">
          <div className="w-3 h-3 rounded-full bg-gradient-to-r from-blue-500 to-purple-500" />
          <span className="text-lg font-semibold text-slate-800 dark:text-slate-200">{netName}</span>
          {net.selected && (
            <span className="text-xs px-2 py-1 bg-gradient-to-r from-blue-500 to-purple-500 text-white rounded-full">
              {net.selected}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="icon"
            className="h-9 w-9 hover:bg-slate-100 dark:hover:bg-slate-700 rounded-lg transition-all duration-200"
            disabled={net.list?.some(item => testingStates[item])}
            onClick={(e) => {
              e.preventDefault();
              if (net.list) {
                onBatchSpeedTest(net.list);
              }
            }}
          >
            <Timer className={clsx("h-4 w-4 text-slate-600 dark:text-slate-400", {
              "animate-spin text-blue-500": net.list?.some(item => testingStates[item])
            })} />
          </Button>
          <div className={clsx("h-8 w-8 flex items-center justify-center rounded-lg bg-slate-100 dark:bg-slate-800 transition-all duration-200", {
            "bg-gradient-to-r from-blue-500 to-purple-500 text-white": isOpen
          })}>
            <ChevronDown
              className={clsx("h-4 w-4 text-slate-600 dark:text-slate-400 transition-transform duration-200", {
                "transform rotate-180 text-white": isOpen
              })}
            />
          </div>
        </div>
      </CollapsibleTrigger>
      <CollapsibleContent className="px-4 pb-4">
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-3">
          {net.list?.map((item) => {
            const latency = latencyResults[item]?.response;
            const { text: latencyText, colorClass } = formatLatency(latency);
            const isSelected = item === net.selected;
            const isTesting = testingStates[item];

            return (
              <Button
                key={item}
                variant={isSelected ? 'default' : 'outline'}
                onClick={() => onSelect(item)}
                disabled={isTesting}
                className={cn(
                  "h-auto py-3 px-4 text-left transition-all duration-200",
                  isSelected && "bg-gradient-to-r from-blue-500 to-purple-500 border-0 shadow-md shadow-blue-500/25",
                  !isSelected && "hover:bg-slate-50 dark:hover:bg-slate-800 hover:border-slate-300 dark:hover:border-slate-600",
                  isTesting && "opacity-50 cursor-not-allowed"
                )}
              >
                <div className="flex flex-col items-start gap-1">
                  <span className="font-medium text-sm">{item}</span>
                  {item in latencyResults && (
                    <div className="flex items-center gap-1">
                      <span className={cn("text-xs font-medium", colorClass)}>
                        {latencyText}
                      </span>
                      <span className="text-xs text-slate-500 dark:text-slate-400">
                        {latencyResults[item]?.region && `• ${latencyResults[item]?.region}`}
                      </span>
                    </div>
                  )}
                </div>
                {isTesting && (
                  <div className="absolute top-2 right-2">
                    <div className="w-2 h-2 bg-blue-500 rounded-full animate-pulse" />
                  </div>
                )}
              </Button>
            );
          })}
        </div>
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
      {selectNets.length === 0 ? (
        <div className="text-center py-12">
          <div className="w-16 h-16 mx-auto mb-4 bg-gradient-to-r from-blue-500 to-purple-500 rounded-full flex items-center justify-center">
            <Activity className="h-8 w-8 text-white" />
          </div>
          <h3 className="text-lg font-medium text-slate-800 dark:text-slate-200 mb-2">
            暂无可用节点
          </h3>
          <p className="text-sm text-slate-600 dark:text-slate-400">
            请检查配置文件或添加新的代理节点
          </p>
        </div>
      ) : (
        selectNets.map(([netName, net]) => (
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
        ))
      )}
    </div>
  );
};
