import type { TrafficSample } from '../types'

interface SidebarSparklineProps {
  history: TrafficSample[]
}

function buildPoints(values: number[], height: number, width: number, maxValue: number) {
  if (values.length === 1) {
    return `0,${height / 2} ${width},${height / 2}`
  }

  return values
    .map((value, index) => {
      const x = (index / (values.length - 1)) * width
      const y = height - (value / maxValue) * height
      return `${x},${Number.isFinite(y) ? y : height}`
    })
    .join(' ')
}

export function SidebarSparkline({ history }: SidebarSparklineProps) {
  const width = 216
  const height = 64
  const samples = history.length > 1 ? history : [{ id: 0, uploadRate: 0, downloadRate: 0 }]

  const uploadValues = samples.map((sample) => sample.uploadRate)
  const downloadValues = samples.map((sample) => sample.downloadRate)
  const maxValue = Math.max(...uploadValues, ...downloadValues, 1)

  return (
    <svg
      className="h-16 w-full overflow-visible"
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label="Traffic sparkline"
    >
      <defs>
        <linearGradient id="download-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
          <stop offset="0%" stopColor="rgba(56, 189, 248, 0.75)" />
          <stop offset="100%" stopColor="rgba(59, 130, 246, 1)" />
        </linearGradient>
        <linearGradient id="upload-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
          <stop offset="0%" stopColor="rgba(250, 204, 21, 0.75)" />
          <stop offset="100%" stopColor="rgba(249, 115, 22, 1)" />
        </linearGradient>
      </defs>

      <path
        d={`M0 ${height - 1} H${width}`}
        stroke="rgba(148, 163, 184, 0.2)"
        strokeWidth="1"
      />
      <polyline
        fill="none"
        stroke="url(#download-gradient)"
        strokeWidth="2.2"
        strokeLinejoin="round"
        strokeLinecap="round"
        points={buildPoints(downloadValues, height, width, maxValue)}
      />
      <polyline
        fill="none"
        stroke="url(#upload-gradient)"
        strokeWidth="2.2"
        strokeLinejoin="round"
        strokeLinecap="round"
        points={buildPoints(uploadValues, height, width, maxValue)}
      />
    </svg>
  )
}
