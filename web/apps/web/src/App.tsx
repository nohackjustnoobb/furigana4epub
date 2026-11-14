import { BookOpen, Moon, Sun } from "lucide-react"
import { HashRouter, Link, Route, Routes } from "react-router"

import { Button } from "@workspace/ui/components/button"
import { useTheme } from "@/components/theme-provider"
import { ConvertPage } from "@/pages/convert"
import { JobPage } from "@/pages/job"
import "./app.css"

export function App() {
  return (
    <HashRouter>
      <Header />
      <main className="mx-auto w-full max-w-6xl px-5 sm:px-8">
        <Routes>
          <Route index element={<ConvertPage />} />
          <Route path="/jobs/:id" element={<JobPage />} />
          <Route
            path="*"
            element={
              <div className="mx-auto max-w-lg space-y-4 py-20">
                <h1 className="text-3xl font-semibold tracking-tight">
                  Page not found
                </h1>
                <Link
                  to="/"
                  className="text-brand font-medium underline underline-offset-4"
                >
                  Go to the converter
                </Link>
              </div>
            }
          />
        </Routes>
      </main>
    </HashRouter>
  )
}

function Header() {
  const { theme, setTheme } = useTheme()

  function toggleTheme() {
    const isDark =
      theme === "dark" ||
      (theme === "system" &&
        window.matchMedia("(prefers-color-scheme: dark)").matches)
    setTheme(isDark ? "light" : "dark")
  }

  return (
    <header className="border-b border-border bg-card/55">
      <div className="mx-auto flex h-[4.25rem] max-w-6xl items-center justify-between px-5 sm:px-8">
        <Link
          to="/"
          className="focus-visible:ring-brand flex items-center gap-2.5 rounded-lg font-semibold tracking-tight outline-none focus-visible:ring-2"
        >
          <span className="bg-brand text-brand-foreground grid size-8 place-items-center rounded-lg">
            <BookOpen className="size-[17px]" aria-hidden="true" />
          </span>
          furigana4epub
        </Link>
        <Button
          variant="outline"
          size="icon-lg"
          onClick={toggleTheme}
          aria-label="Toggle color theme"
        >
          <Moon className="dark:hidden" aria-hidden="true" />
          <Sun className="hidden dark:block" aria-hidden="true" />
        </Button>
      </div>
    </header>
  )
}
