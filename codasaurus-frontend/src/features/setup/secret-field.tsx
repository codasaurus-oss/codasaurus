import { useState } from "react"
import { cn } from "cn"
import { EyeIcon, EyeOffIcon } from "lucide-react"

import { FieldLabel } from "@/components/ui/field"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
  InputGroupTextarea,
} from "@/components/ui/input-group"

type SecretFieldProps = {
  id: string
  label: string
  value: string
  onChange: (value: string) => void
  placeholder?: string
  autoComplete?: string
  disabled?: boolean
}

function RevealButton({
  label,
  revealed,
  onToggle,
}: {
  label: string
  revealed: boolean
  onToggle: () => void
}) {
  return (
    <InputGroupButton
      aria-label={`${revealed ? "Hide" : "Show"} ${label}`}
      onClick={onToggle}
      size="icon-xs"
      variant="ghost"
    >
      {revealed ? <EyeOffIcon /> : <EyeIcon />}
    </InputGroupButton>
  )
}

function SecretInput({
  id,
  label,
  value,
  onChange,
  placeholder,
  autoComplete = "off",
  disabled = false,
}: SecretFieldProps) {
  const [revealed, setRevealed] = useState(false)
  return (
    <InputGroup>
      <InputGroupInput
        id={id}
        aria-label={label}
        type={revealed ? "text" : "password"}
        autoComplete={autoComplete}
        placeholder={placeholder}
        disabled={disabled}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      <InputGroupAddon align="inline-end">
        <RevealButton
          label={label}
          revealed={revealed}
          onToggle={() => setRevealed((current) => !current)}
        />
      </InputGroupAddon>
    </InputGroup>
  )
}

function SecretTextarea({
  id,
  label,
  value,
  onChange,
  className,
  rows,
}: SecretFieldProps & { className?: string; rows?: number }) {
  const [revealed, setRevealed] = useState(false)
  return (
    <>
      <div className="flex items-center justify-between gap-2">
        <FieldLabel htmlFor={id}>{label}</FieldLabel>
        <RevealButton
          label={label}
          revealed={revealed}
          onToggle={() => setRevealed((current) => !current)}
        />
      </div>
      <InputGroup className="h-auto items-start">
        <InputGroupTextarea
          id={id}
          aria-label={label}
          className={cn(className, !revealed && "[-webkit-text-security:disc]")}
          rows={rows}
          value={value}
          onChange={(event) => onChange(event.target.value)}
        />
      </InputGroup>
    </>
  )
}

export { SecretInput, SecretTextarea }
