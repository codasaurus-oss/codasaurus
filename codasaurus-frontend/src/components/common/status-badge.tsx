import { cn } from "cn"

import { Badge } from "@/components/ui/badge"

type StatusTone = "default" | "success" | "warning" | "destructive"

const toneClasses: Record<StatusTone, string> = {
  default: "",
  success: "border-success/30 bg-success/10 text-success",
  warning: "border-warning/30 bg-warning/10 text-warning",
  destructive: "",
}

type StatusBadgeProps = {
  label: string
  tone?: StatusTone
}

function StatusBadge({ label, tone = "default" }: StatusBadgeProps) {
  return (
    <Badge
      variant={tone === "destructive" ? "destructive" : "outline"}
      className={cn(toneClasses[tone])}
    >
      {label}
    </Badge>
  )
}

function reviewStatusTone(status: string): StatusTone {
  if (status === "passed") {
    return "success"
  }
  if (status === "failed" || status === "error") {
    return "destructive"
  }
  if (status === "running") {
    return "warning"
  }
  return "default"
}

export { reviewStatusTone, StatusBadge }
