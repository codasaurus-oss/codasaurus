type PickerOption = { value: string; label: string }

function withCurrent(options: PickerOption[], current: string) {
  if (!current || options.some((option) => option.value === current)) {
    return options
  }
  return [...options, { value: current, label: current }]
}

export { withCurrent, type PickerOption }
