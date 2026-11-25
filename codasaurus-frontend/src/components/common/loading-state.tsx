import { Skeleton } from "@/components/ui/skeleton"

type LoadingStateProps = {
  rows?: number
}

function LoadingState({ rows = 3 }: LoadingStateProps) {
  return (
    <div className="flex flex-col gap-2">
      {Array.from({ length: rows }, (_, index) => (
        <Skeleton key={index} className="h-4 w-full" />
      ))}
    </div>
  )
}

export { LoadingState }
