;(() => {
  let stored = null
  try {
    stored = localStorage.getItem("vite-ui-theme")
  } catch {
    // Storage is blocked; fall through to the system preference.
  }
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches
  const dark = stored === "dark" || (stored !== "light" && prefersDark)
  document.documentElement.classList.add(dark ? "dark" : "light")
})()
