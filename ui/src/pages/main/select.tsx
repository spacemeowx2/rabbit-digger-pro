import { useConfig } from '@/api/v1'

export const SelectNet: React.FC = () => {
  const { data, error } = useConfig('http://127.0.0.1:8030')

  if (error) {
    console.error(error)
    return <div>Failed to load</div>
  }

  return <>
    {JSON.stringify(data, null, 2)}

  </>
}
