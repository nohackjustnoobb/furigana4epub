import {
  ArrowRight,
  ChevronDown,
  FileText,
  LoaderCircle,
  Upload,
} from "lucide-react"
import { useEffect, useState, type FormEvent } from "react"
import { useNavigate } from "react-router"

import { Alert, AlertDescription } from "@workspace/ui/components/alert"
import { Button } from "@workspace/ui/components/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
} from "@workspace/ui/components/card"
import { Checkbox } from "@workspace/ui/components/checkbox"
import { Input } from "@workspace/ui/components/input"
import { Label } from "@workspace/ui/components/label"
import { convertBook, errorMessage, getWordlists } from "@/lib/api"

export function ConvertPage() {
  const navigate = useNavigate()
  const [book, setBook] = useState<File | null>(null)
  const [wordlist, setWordlist] = useState<File | null>(null)
  const [wordlists, setWordlists] = useState<string[] | undefined>()
  const [wordlistError, setWordlistError] = useState(false)
  const [ignored, setIgnored] = useState<string[]>([])
  const [replaceExisting, setReplaceExisting] = useState(false)
  const [highlight, setHighlight] = useState(false)
  const [color, setColor] = useState("#c2410c")
  const [newIdentifier, setNewIdentifier] = useState(false)
  const [uploading, setUploading] = useState(false)
  const [error, setError] = useState("")

  useEffect(() => {
    const controller = new AbortController()
    getWordlists(controller.signal).then(
      (available) => {
        if (!controller.signal.aborted) setWordlists(available)
      },
      () => {
        if (!controller.signal.aborted) setWordlistError(true)
      }
    )
    return () => controller.abort()
  }, [])

  function chooseBook(file: File | undefined) {
    if (!file) {
      setBook(null)
      return
    }
    if (!/^.+\.epub$/i.test(file.name) || file.size === 0) {
      setBook(null)
      setError("Choose a non-empty EPUB file.")
      return
    }
    setBook(file)
    setError("")
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!book || uploading) return

    setUploading(true)
    setError("")
    try {
      const job = await convertBook(book, wordlist, {
        override_existing: replaceExisting,
        highlight_injected: highlight,
        highlight_color: highlight ? color : undefined,
        new_identifier: newIdentifier,
        ignore_wordlists: ignored,
      })
      navigate("/jobs/" + encodeURIComponent(job.id))
    } catch (error) {
      setError(errorMessage(error))
    } finally {
      setUploading(false)
    }
  }

  return (
    <div className="pt-12 pb-16 sm:pt-16">
      <h1 className="max-w-3xl text-4xl leading-[1.05] font-semibold tracking-[-0.055em] sm:text-[3.75rem]">
        Add furigana to your EPUB
      </h1>
      <p className="mt-4 max-w-xl text-base leading-relaxed text-muted-foreground sm:text-lg">
        Upload a Japanese ebook and download a copy with small kana readings
        above the kanji.
      </p>

      <div className="mt-9 grid items-start gap-5 lg:grid-cols-[1.3fr_1fr] lg:gap-6">
        <Card className="[--card-spacing:--spacing(5)] sm:[--card-spacing:--spacing(7)]">
          <CardHeader>
            <h2 className="text-xl font-semibold tracking-tight">
              Convert an EPUB
            </h2>
          </CardHeader>
          <CardContent>
            <form onSubmit={handleSubmit} aria-busy={uploading}>
              <fieldset disabled={uploading} className="min-w-0 space-y-5">
                <Label
                  htmlFor="book"
                  onDragOver={(event) => event.preventDefault()}
                  onDrop={(event) => {
                    event.preventDefault()
                    if (!uploading) chooseBook(event.dataTransfer.files[0])
                  }}
                  className="hover:border-brand hover:bg-preview/50 focus-within:ring-brand flex min-h-45 cursor-pointer flex-col justify-center gap-3 rounded-xl border border-dashed border-input bg-background px-4 py-6 text-center leading-normal transition-colors focus-within:ring-2 has-[:disabled]:cursor-wait"
                >
                  <input
                    id="book"
                    type="file"
                    accept=".epub,application/epub+zip"
                    onChange={(event) => {
                      chooseBook(event.currentTarget.files?.[0])
                      event.currentTarget.value = ""
                    }}
                    className="sr-only"
                  />
                  <span className="bg-preview text-brand grid size-11 place-items-center rounded-xl">
                    {book ? (
                      <FileText className="size-5" aria-hidden="true" />
                    ) : (
                      <Upload className="size-5" aria-hidden="true" />
                    )}
                  </span>
                  <span className="flex w-full flex-col gap-1">
                    <span className="text-brand font-semibold break-all">
                      {book ? book.name : "Choose an EPUB"}
                    </span>
                    <span className="text-xs font-normal text-muted-foreground">
                      {book
                        ? "Choose a different file"
                        : "or drag and drop a file here"}
                    </span>
                  </span>
                </Label>

                <details className="group/options border-y border-border">
                  <summary className="focus-visible:ring-brand flex cursor-pointer list-none items-center justify-between rounded-sm py-3.5 text-sm font-medium outline-none focus-visible:ring-2 [&::-webkit-details-marker]:hidden">
                    Conversion options
                    <ChevronDown
                      className="size-4 text-muted-foreground group-open/options:rotate-180"
                      aria-hidden="true"
                    />
                  </summary>
                  <div className="space-y-4 pb-5">
                    <div className="flex items-center gap-2.5">
                      <Checkbox
                        id="replace-existing"
                        checked={replaceExisting}
                        onCheckedChange={setReplaceExisting}
                      />
                      <Label htmlFor="replace-existing">
                        Replace existing furigana
                      </Label>
                    </div>
                    <div className="flex flex-wrap items-center gap-2.5">
                      <Checkbox
                        id="highlight"
                        checked={highlight}
                        onCheckedChange={setHighlight}
                      />
                      <Label htmlFor="highlight">
                        Highlight added furigana
                      </Label>
                      {highlight && (
                        <Input
                          type="color"
                          aria-label="Furigana highlight color"
                          value={color}
                          onChange={(event) => setColor(event.target.value)}
                          className="h-7 w-10 cursor-pointer p-1"
                        />
                      )}
                    </div>
                    <div className="space-y-2">
                      <div className="flex items-center gap-2.5">
                        <Checkbox
                          id="new-identifier"
                          checked={newIdentifier}
                          onCheckedChange={setNewIdentifier}
                          aria-describedby={
                            newIdentifier ? "new-identifier-help" : undefined
                          }
                        />
                        <Label htmlFor="new-identifier">
                          Generate a new EPUB identifier
                        </Label>
                      </div>
                      {newIdentifier && (
                        <p
                          id="new-identifier-help"
                          className="pl-6.5 text-xs text-muted-foreground"
                        >
                          Available for EPUBs without encrypted or obfuscated
                          resources.
                        </p>
                      )}
                    </div>
                    <fieldset className="space-y-2">
                      <legend className="mb-2 text-sm font-medium">
                        Skip words in installed lists
                      </legend>
                      {wordlists?.map((name, index) => (
                        <div key={name} className="flex items-center gap-2.5">
                          <Checkbox
                            id={"wordlist-" + index}
                            checked={ignored.includes(name)}
                            onCheckedChange={(checked) =>
                              setIgnored((selected) =>
                                checked
                                  ? [...selected, name]
                                  : selected.filter((item) => item !== name)
                              )
                            }
                          />
                          <Label
                            htmlFor={"wordlist-" + index}
                            className="font-normal"
                          >
                            {name.replace(/^jlpt\/(N[1-5])\.txt$/, "JLPT $1")}
                          </Label>
                        </div>
                      ))}
                      <p
                        className="text-xs text-muted-foreground"
                        role="status"
                      >
                        {wordlistError
                          ? "Installed wordlists could not be loaded."
                          : wordlists?.length === 0
                            ? "No installed wordlists are available."
                            : !wordlists
                              ? "Loading wordlists…"
                              : null}
                      </p>
                    </fieldset>
                    <div className="space-y-2">
                      <Label htmlFor="custom-wordlist">Custom wordlist</Label>
                      <Input
                        id="custom-wordlist"
                        type="file"
                        accept=".txt,text/plain"
                        aria-describedby="wordlist-help"
                        onChange={(event) =>
                          setWordlist(event.currentTarget.files?.[0] ?? null)
                        }
                      />
                      <p
                        id="wordlist-help"
                        className="text-xs text-muted-foreground"
                      >
                        Optional UTF-8 text file with one word to skip per line.
                      </p>
                    </div>
                  </div>
                </details>

                {error && (
                  <Alert variant="destructive" role="alert">
                    <AlertDescription>{error}</AlertDescription>
                  </Alert>
                )}

                <Button
                  className="bg-brand text-brand-foreground hover:bg-brand/90 h-11 w-full gap-2 rounded-lg"
                  disabled={!book || uploading}
                  type="submit"
                >
                  {uploading ? "Uploading EPUB…" : "Add furigana"}
                  {uploading ? (
                    <LoaderCircle
                      className="motion-safe:animate-spin"
                      aria-hidden="true"
                    />
                  ) : (
                    <ArrowRight aria-hidden="true" />
                  )}
                </Button>
              </fieldset>
            </form>
          </CardContent>
        </Card>
        <FuriganaExample />
      </div>
    </div>
  )
}

function FuriganaExample() {
  return (
    <Card className="bg-preview ring-preview-line [--card-spacing:--spacing(5)] sm:[--card-spacing:--spacing(7)]">
      <CardHeader>
        <h2 className="text-lg font-semibold tracking-tight">
          Furigana example
        </h2>
        <CardDescription>
          Readings appear above the corresponding kanji.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <figure className="border-preview-line grid gap-4 rounded-xl border bg-card px-5 py-6 sm:px-6">
          <div className="space-y-2 pb-4">
            <p className="text-xs font-medium text-muted-foreground">
              Original text
            </p>
            <p
              lang="ja"
              className="font-serif text-3xl leading-snug tracking-wider"
            >
              日本語の本
            </p>
          </div>
          <div className="border-preview-line space-y-3 border-t pt-5">
            <p className="text-xs font-medium text-muted-foreground">
              With furigana
            </p>
            <p
              lang="ja"
              className="font-serif text-3xl leading-snug tracking-wider"
            >
              <ruby>
                日本語
                <rt className="text-brand font-sans text-xs font-medium tracking-normal">
                  にほんご
                </rt>
              </ruby>
              の
              <ruby>
                本
                <rt className="text-brand font-sans text-xs font-medium tracking-normal">
                  ほん
                </rt>
              </ruby>
            </p>
          </div>
        </figure>
      </CardContent>
    </Card>
  )
}
