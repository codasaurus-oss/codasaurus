import { useId, useState, type ReactNode } from "react"

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Input } from "@/components/ui/input"

type ConfirmDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description: string
  onConfirm: () => void
  destructive?: boolean
  confirmLabel?: string
  confirmationText?: string
  children?: ReactNode
}

function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  onConfirm,
  destructive = false,
  confirmLabel,
  confirmationText,
  children,
}: ConfirmDialogProps) {
  const [typed, setTyped] = useState("")
  const instructionsId = useId()

  function handleOpenChange(value: boolean) {
    if (!value) setTyped("")
    onOpenChange(value)
  }

  const confirmed = confirmationText === undefined || typed === confirmationText

  function confirm() {
    if (!confirmed) {
      return
    }
    onConfirm()
    handleOpenChange(false)
  }

  return (
    <AlertDialog open={open} onOpenChange={handleOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>{description}</AlertDialogDescription>
        </AlertDialogHeader>
        {confirmationText !== undefined ? (
          <div className="flex flex-col gap-2">
            <p id={instructionsId} className="text-sm text-muted-foreground">
              Type{" "}
              <span className="font-mono text-foreground">{confirmationText}</span>{" "}
              to confirm.
            </p>
            <Input
              aria-label="Confirmation"
              aria-describedby={instructionsId}
              autoComplete="off"
              value={typed}
              onChange={(event) => setTyped(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault()
                  confirm()
                }
              }}
            />
          </div>
        ) : null}
        {children}
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant={destructive ? "destructive" : "default"}
            disabled={!confirmed}
            onClick={confirm}
          >
            {confirmLabel ?? (destructive ? "Delete" : "Confirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

export { ConfirmDialog }
