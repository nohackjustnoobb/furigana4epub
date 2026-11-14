import { ArrowLeft, Download, FileText, LoaderCircle } from "lucide-react"
import { useEffect, useState } from "react"
import { Link, useParams } from "react-router"

import { Alert, AlertDescription } from "@workspace/ui/components/alert"
import { Badge } from "@workspace/ui/components/badge"
import { buttonVariants } from "@workspace/ui/components/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
} from "@workspace/ui/components/card"
import { ApiError, errorMessage, getJob, apiUrl, type Job } from "@/lib/api"

const statusText: Record<
  Job["status"],
  { label: string; description: string }
> = {
  waiting_for_upload: {
    label: "Waiting for upload",
    description:
      "The server is waiting for the EPUB. If the upload was interrupted, return to the converter and choose the book again.",
  },
  queued: {
    label: "Queued",
    description: "Your book is in the queue. This page updates automatically.",
  },
  processing: {
    label: "Adding furigana",
    description:
      "The server is adding readings to your book. This page updates automatically.",
  },
  finished: {
    label: "Ready to download",
    description: "The annotated EPUB is ready.",
  },
  failed: {
    label: "Conversion failed",
    description: "The server could not convert your EPUB.",
  },
}

export function JobPage() {
  const { id } = useParams<"id">()
  return id ? <JobDetails key={id} id={id} /> : null
}

function JobDetails({ id }: { id: string }) {
  const [job, setJob] = useState<Job>()
  const [error, setError] = useState("")
  const [expired, setExpired] = useState(false)

  useEffect(() => {
    const controller = new AbortController()
    let timer: number | undefined

    async function poll() {
      try {
        const current = await getJob(id, controller.signal)
        if (controller.signal.aborted) return
        setJob(current)
        setError("")
        if (current.status === "finished" || current.status === "failed") return
      } catch (error) {
        if (controller.signal.aborted) return
        if (error instanceof ApiError && error.status === 404) {
          setExpired(true)
          setError("")
          return
        }
        setError(errorMessage(error) + " Retrying automatically.")
      }
      timer = window.setTimeout(poll, 2000)
    }

    void poll()
    return () => {
      controller.abort()
      window.clearTimeout(timer)
    }
  }, [id])

  const status = job && statusText[job.status]
  const isActive =
    !job || job.status === "queued" || job.status === "processing"

  return (
    <div className="mx-auto max-w-xl py-12 sm:py-16">
      <Link
        to="/"
        className="text-brand focus-visible:ring-brand inline-flex items-center gap-2 rounded-sm text-sm font-medium hover:underline focus-visible:ring-2"
      >
        <ArrowLeft className="size-4" aria-hidden="true" />
        Back to converter
      </Link>
      <Card className="mt-5 [--card-spacing:--spacing(5)] sm:[--card-spacing:--spacing(7)]">
        <CardHeader>
          <div className="bg-preview text-brand mb-3 grid size-11 place-items-center rounded-xl">
            <FileText className="size-5" aria-hidden="true" />
          </div>
          <h1 className="text-2xl font-semibold tracking-tight">
            Conversion status
          </h1>
          {job && (
            <CardDescription className="wrap-anywhere">
              {job.filename}
            </CardDescription>
          )}
        </CardHeader>
        <CardContent className="space-y-4">
          {expired ? (
            <Alert variant="destructive" role="alert">
              <AlertDescription>
                This job could not be found. It may have expired.
              </AlertDescription>
            </Alert>
          ) : (
            <>
              <div role="status" className="space-y-3">
                <Badge
                  variant={job?.status === "failed" ? "destructive" : "outline"}
                  className="gap-1.5 px-2.5 py-3"
                >
                  {isActive && (
                    <LoaderCircle
                      className="motion-safe:animate-spin"
                      aria-hidden="true"
                    />
                  )}
                  {status?.label ?? "Checking status"}
                </Badge>
                {status && (
                  <p className="text-sm text-muted-foreground">
                    {job?.error || status.description}
                  </p>
                )}
              </div>
              {error && (
                <Alert variant="destructive" role="alert">
                  <AlertDescription>{error}</AlertDescription>
                </Alert>
              )}
              {job?.status === "finished" && job.download_url && (
                <a
                  href={apiUrl(job.download_url)}
                  className={buttonVariants({
                    className:
                      "bg-brand text-brand-foreground hover:bg-brand/90 h-10 gap-2 px-4",
                  })}
                >
                  <Download aria-hidden="true" />
                  Download EPUB
                </a>
              )}
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
