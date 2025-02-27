import { useState, useMemo } from 'react';
import { Search } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { X } from 'lucide-react';
import { useConnectionsStream, ConnectionInfo, fetcher } from '@/api/v1';
import { useInstance } from '@/contexts/instance';

// Connection type for UI display
interface Connection {
  id: string;
  host: string;
  port: string;
  protocol: string;
  server: string;
  route?: string;
  location?: string;
  timestamp: string;
  duration: number;
  upload?: number;
  download?: number;
  uploadSpeed?: number;
  downloadSpeed?: number;
}

// Connection Item Component
const ConnectionItem = ({ connection, onClose }: { connection: Connection, onClose: (id: string) => void }) => {
  const hasSpeed = (connection.uploadSpeed || 0) > 0 || (connection.downloadSpeed || 0) > 0;

  return (
    <div className="flex items-center justify-between py-2 px-3 border-b border-gray-200 hover:bg-gray-50">
      <div className="flex flex-col flex-grow gap-1">
        <div className="flex items-center gap-2">
          <span className="font-medium">{connection.host}:{connection.port}</span>
        </div>
        <div>
          {connection.route && (
            <Badge variant="secondary"
              className="bg-indigo-100 text-indigo-800 border-indigo-200">
              {connection.route}
            </Badge>
          )}
        </div>
        <div className="flex items-center gap-1 text-sm text-gray-600 flex-wrap">
          <Badge variant="outline" className="bg-amber-100 text-amber-800 border-amber-200">{connection.protocol}</Badge>
          <Badge variant="outline" className="bg-emerald-100 text-emerald-800 border-emerald-200">{connection.server}</Badge>
          <Badge variant="outline" className="bg-violet-100 text-violet-800 border-violet-200">{connection.timestamp}</Badge>

          <Badge variant="outline" className="bg-purple-100 text-purple-800 border-purple-200">
            ↓ {formatBytes(connection.download || 0)} ↑ {formatBytes(connection.upload || 0)}
          </Badge>

          {hasSpeed && (
            <Badge variant="outline" className="bg-cyan-100 text-cyan-800 border-cyan-200">
              ↓ {formatBytes(connection.downloadSpeed || 0)}/s ↑ {formatBytes(connection.uploadSpeed || 0)}/s
            </Badge>
          )}
        </div>
      </div>
      <Button variant="ghost" size="icon" onClick={() => onClose(connection.id)} className="text-gray-500 hover:text-red-500">
        <X size={18} />
      </Button>
    </div>
  );
};

// Convert connection data from API to UI format
const formatConnection = ([id, conn]: [string, ConnectionInfo]): Connection => {
  // Extract host and port from addr
  const parseAddress = (s: string) => {
    const idx = s.lastIndexOf(':');
    if (idx === -1) {
      throw new TypeError('Invalid address');
    }
    const host = s.slice(0, idx);
    const port = s.slice(idx + 1);
    return { host, port };
  }

  const dest = parseAddress(conn.addr);

  // Calculate timestamp
  const timestamp = getRelativeTime(conn.start_time); // Convert to milliseconds
  const duration = Math.floor(Date.now() / 1000) - conn.start_time;

  const server = conn.ctx.net_list[0];
  const route = conn.ctx.net_list.slice(1).join(' / ');

  return {
    id,
    host: dest.host,
    port: dest.port,
    protocol: conn.protocol.toUpperCase(),
    server,
    route,
    timestamp,
    duration,
    upload: conn.upload,
    download: conn.download,
    uploadSpeed: conn.uploadSpeed,
    downloadSpeed: conn.downloadSpeed,
  };
};

// Helper function to format bytes
const formatBytes = (bytes: number): string => {
  if (bytes === 0) return '0 B';

  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));

  return `${parseFloat((bytes / Math.pow(1024, i)).toFixed(2))} ${sizes[i]}`;
};

// Calculate relative time
const getRelativeTime = (timestamp: number): string => {
  const now = Date.now() / 1000;
  const diffSec = now - timestamp;

  if (diffSec < 60) return 'A few seconds ago';
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)} min ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)} hrs ago`;
  return `${Math.floor(diffSec / 86400)} days ago`;
};

// Sort options
type SortOption = 'time' | 'uploadSpeed' | 'downloadSpeed' | 'uploadTraffic' | 'downloadTraffic';
type SortDirection = 'asc' | 'desc';

interface SortConfig {
  label: string;
  value: SortOption;
}

// Sort options
const sortOptions: SortConfig[] = [
  { label: 'Time', value: 'time' },
  { label: 'Upload Speed', value: 'uploadSpeed' },
  { label: 'Download Speed', value: 'downloadSpeed' },
  { label: 'Upload Traffic', value: 'uploadTraffic' },
  { label: 'Download Traffic', value: 'downloadTraffic' },
];

// Main Connections Component
export const ConnectionsManager = () => {
  const { currentInstance } = useInstance();
  const { state } = useConnectionsStream(currentInstance?.url);
  const [searchQuery, setSearchQuery] = useState('');
  const [sortOption, setSortOption] = useState<SortOption>('time');
  const [sortDirection, setSortDirection] = useState<SortDirection>('asc');

  const { formattedConnections, sortedConnections } = useMemo(() => {
    if (!state) {
      return { formattedConnections: [], filteredConnections: [], sortedConnections: [] };
    }

    const formattedConnections = Object.entries(state.connections).map(formatConnection);

    // Filter connections based on search query
    const filteredConnections = formattedConnections.filter(conn =>
      conn.host.toLowerCase().includes(searchQuery.toLowerCase()) ||
      conn.port.includes(searchQuery)
    );

    const sortedConnections = [...filteredConnections].sort((a, b) => {
      let result = 0;

      switch (sortOption) {
        case 'uploadSpeed':
          result = (b.uploadSpeed || 0) - (a.uploadSpeed || 0);
          break;
        case 'downloadSpeed':
          result = (b.downloadSpeed || 0) - (a.downloadSpeed || 0);
          break;
        case 'uploadTraffic':
          result = (b.upload || 0) - (a.upload || 0);
          break;
        case 'downloadTraffic':
          result = (b.download || 0) - (a.download || 0);
          break;
        case 'time':
        default:
          result = b.duration - a.duration;
      }

      // If direction is ascending, invert the result
      return sortDirection === 'asc' ? -result : result;
    });

    return { formattedConnections, filteredConnections, sortedConnections };
  }, [state, searchQuery, sortOption, sortDirection]);

  // Toggle sort option and direction
  const toggleSort = (option: SortOption) => {
    if (sortOption === option) {
      // Toggle direction if clicking the same option
      setSortDirection(sortDirection === 'asc' ? 'desc' : 'asc');
    } else {
      // Set new sort option with default desc direction
      setSortOption(option);
      setSortDirection('desc');
    }
  };

  // Close a single connection
  const closeConnection = async (id: string) => {
    if (!currentInstance?.url) return;

    try {
      await fetcher(['/conn/:uuid', 'delete', undefined, { uuid: id }, currentInstance.url]);
      // The WebSocket connection will update the state automatically
    } catch (error) {
      console.error('Failed to close connection:', error);
    }
  };

  // Close all connections
  const closeAllConnections = async () => {
    if (!currentInstance?.url) return;

    try {
      await fetcher(['/connections', 'delete', undefined, currentInstance.url]);
      // The WebSocket connection will update the state automatically
    } catch (error) {
      console.error('Failed to close all connections:', error);
    }
  };

  return (
    <div className="container py-4 max-w-[1024px] mx-auto h-[calc(100vh-80px)] flex flex-col">
      <Card className="bg-white shadow-sm flex-grow flex flex-col">
        <CardHeader className="pb-2">
          <div className="flex justify-between items-center">
            <CardTitle className="text-xl text-gray-800">
              Connections <Badge variant="outline" className="ml-2 bg-gray-100">{formattedConnections.length}</Badge>
            </CardTitle>
            <div className="flex gap-2">
              <Button variant="destructive" size="sm" onClick={closeAllConnections}>
                Close All
              </Button>
            </div>
          </div>
          <div className="flex justify-between items-center mt-2">
            <div className="relative w-full max-w-md">
              <Search className="absolute left-2 top-2.5 h-4 w-4 text-gray-500" />
              <Input
                placeholder="Search connections..."
                className="pl-8 bg-white border-gray-300"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
              />
            </div>
            <div className="text-sm text-gray-600">
              Total: ↓ {formatBytes(state?.total_download ?? 0)} | ↑ {formatBytes(state?.total_upload ?? 0)}
            </div>
          </div>
        </CardHeader>
        <div className="px-4 pb-2">
          <div className="flex items-center gap-2">
            <span className="text-sm text-gray-600">Sort by:</span>
            <div className="flex gap-1 flex-wrap">
              {sortOptions.map(option => (
                <Button
                  key={option.value}
                  variant={sortOption === option.value ? "default" : "outline"}
                  size="sm"
                  onClick={() => toggleSort(option.value)}
                  className="gap-1"
                >
                  {option.label}
                  {sortOption === option.value && (
                    <span className="ml-1">{sortDirection === 'asc' ? '↑' : '↓'}</span>
                  )}
                </Button>
              ))}
            </div>
          </div>
        </div>
        <CardContent className="p-0 flex-grow flex flex-col">
          <div className="overflow-y-auto flex-grow">
            {sortedConnections.length > 0 ? (
              sortedConnections.map((connection) => (
                <ConnectionItem
                  key={connection.id}
                  connection={connection}
                  onClose={closeConnection}
                />
              ))
            ) : (
              <div className="py-8 text-center text-gray-500">
                No active connections
              </div>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
};

export const ConnectionPage = () => {
  return <ConnectionsManager />;
};
