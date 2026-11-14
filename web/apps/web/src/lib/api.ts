const apiBase = (import.meta.env.VITE_API_URL?.trim() || "/api").replace(
  /\/+$/,
  ""
)

type JobStatus =
  "waiting_for_upload" | "queued" | "processing" | "finished" | "failed"

export type Job = {
  id: string
  filename: string
  status: JobStatus
  download_url?: string
  error?: string
}

type ConversionOptions = {
  override_existing: boolean
  highlight_injected: boolean
  highlight_color?: string
  new_identifier: boolean
  ignore_wordlists: string[]
}

export class ApiError extends Error {
  readonly status: number

  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(apiUrl(path), init)
  const isJson = response.headers
    .get("content-type")
    ?.includes("application/json")

  if (!response.ok) {
    const body = isJson ? await response.json().catch(() => null) : null
    const message =
      typeof body?.error === "string"
        ? body.error
        : "The conversion server returned an error (" + response.status + ")."
    throw new ApiError(response.status, message)
  }

  if (!isJson) {
    throw new ApiError(
      response.status,
      "The conversion server returned an unexpected response."
    )
  }

  return response.json()
}

export function getWordlists(signal: AbortSignal) {
  return request<string[]>("/wordlists", { signal })
}

export function getJob(id: string, signal?: AbortSignal) {
  return request<Job>("/jobs/" + encodeURIComponent(id), { signal })
}

export async function convertBook(
  book: File,
  wordlist: File | null,
  options: ConversionOptions
) {
  const job = await request<Job>("/jobs", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      filename: book.name,
      size_bytes: book.size,
      options,
    }),
  })
  const path = "/jobs/" + encodeURIComponent(job.id)

  if (wordlist) {
    await request<Job>(path + "/wordlist", {
      method: "PUT",
      headers: { "Content-Type": "text/plain; charset=utf-8" },
      body: wordlist,
    })
  }

  try {
    return await request<Job>(path + "/upload", {
      method: "PUT",
      headers: { "Content-Type": "application/epub+zip" },
      body: book,
    })
  } catch (error) {
    // The server may have queued the upload before the connection dropped.
    const current = await getJob(job.id).catch(() => null)
    if (current && current.status !== "waiting_for_upload") {
      return current
    }
    throw error
  }
}

export function apiUrl(path: string) {
  return apiBase + path
}

export function errorMessage(error: unknown) {
  return error instanceof ApiError
    ? error.message
    : "Could not reach the conversion server. Check the connection and try again."
}
