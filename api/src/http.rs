use std::fs::{self, File};
use std::io::ErrorKind;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::StreamExt;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::task::spawn_blocking;
use tokio_util::io::ReaderStream;
use tower_http::cors::{Any, CorsLayer};

use crate::State as AppState;
use crate::jobs::{CreateJob, Job, PendingFile, Status};
use crate::wordlists::Wordlists;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/wordlists", get(list_wordlists))
        .route(
            "/jobs",
            post(create).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route("/jobs/{id}", get(status))
        .route("/jobs/{id}/wordlist", put(upload_wordlist))
        .route("/jobs/{id}/upload", put(upload))
        .route("/jobs/{id}/download", get(download))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::PUT])
                .allow_headers([header::CONTENT_TYPE]),
        )
        .layer(middleware::map_response(no_store))
        .with_state(state)
}

async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

struct Error(StatusCode, String);

impl Error {
    fn bad_request(message: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, message.to_owned())
    }

    fn not_found() -> Self {
        Self(StatusCode::NOT_FOUND, "Job not found".into())
    }

    fn conflict(message: &str) -> Self {
        Self(StatusCode::CONFLICT, message.to_owned())
    }

    fn storage_full() -> Self {
        Self(
            StatusCode::INSUFFICIENT_STORAGE,
            "Job storage is full".into(),
        )
    }
}

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(error: E) -> Self {
        eprintln!("API request failed: {:#}", error.into());
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error".into(),
        )
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

async fn create(
    State(state): State<AppState>,
    request: Result<Json<CreateJob>, JsonRejection>,
) -> Result<impl IntoResponse, Error> {
    let Json(request) = request.map_err(|error| Error(error.status(), error.body_text()))?;
    if request.filename.len() > 255
        || request.filename.contains(['/', '\\'])
        || request.filename.chars().any(char::is_control)
        || !request
            .filename
            .rsplit_once('.')
            .is_some_and(|(stem, ext)| !stem.trim().is_empty() && ext.eq_ignore_ascii_case("epub"))
    {
        return Err(Error::bad_request("Expected an EPUB filename"));
    }
    if let Some(bytes) = request.size_bytes {
        if bytes == 0 {
            return Err(Error::bad_request("Input EPUB is empty"));
        }
        if bytes > state.config.max_input_bytes {
            return Err(Upload::Epub.too_large());
        }
    }
    if let Some(color) = &request.options.highlight_color
        && !color.strip_prefix('#').is_some_and(|digits| {
            matches!(digits.len(), 3 | 4 | 6 | 8)
                && digits.bytes().all(|digit| digit.is_ascii_hexdigit())
        })
    {
        return Err(Error::bad_request("Expected a CSS hex highlight color"));
    }
    if let Some(identifier) = &request.options.identifier
        && (identifier.trim().is_empty() || identifier.chars().any(|ch| {
            !matches!(ch, '\t' | '\r' | '\n' | ' '..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
        }))
    {
        return Err(Error::bad_request("Expected a non-empty, XML-compatible EPUB identifier"));
    }
    if !request.options.ignore_wordlists.is_empty() {
        let available = read_wordlists(&state).await?;
        for name in &request.options.ignore_wordlists {
            if available.find(name).is_none() {
                return Err(Error::bad_request(&format!("Unknown wordlist {name:?}")));
            }
        }
    }
    let job = state.store.lock().unwrap().create(request)?;
    let location = format!("/jobs/{}", job.id);
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(job),
    ))
}

async fn list_wordlists(State(state): State<AppState>) -> Result<Json<Vec<String>>, Error> {
    Ok(Json(read_wordlists(&state).await?.names()))
}

async fn read_wordlists(state: &AppState) -> Result<Wordlists, Error> {
    let config = state.config.clone();
    Ok(spawn_blocking(move || Wordlists::read(&config.wordlists_dir)).await??)
}

async fn status(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Job>, Error> {
    let job = state
        .store
        .lock()
        .unwrap()
        .get(&id)?
        .ok_or_else(Error::not_found)?;
    Ok(Json(job))
}

async fn upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Job>), Error> {
    upload_file(state, id, headers, body, Upload::Epub).await
}

async fn upload_wordlist(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Job>), Error> {
    if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
        let media_type = content_type
            .to_str()
            .ok()
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        if !media_type.is_some_and(|value| {
            value.eq_ignore_ascii_case("text/plain")
                || value.eq_ignore_ascii_case("application/octet-stream")
        }) {
            return Err(Error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Send the raw UTF-8 wordlist".into(),
            ));
        }
    }
    upload_file(state, id, headers, body, Upload::Wordlist).await
}

#[derive(Clone, Copy)]
enum Upload {
    Epub,
    Wordlist,
}

impl Upload {
    fn empty(self) -> Error {
        Error::bad_request(match self {
            Self::Epub => "Input EPUB is empty",
            Self::Wordlist => "Wordlist is empty",
        })
    }

    fn too_large(self) -> Error {
        let message = match self {
            Self::Epub => "Input EPUB exceeds the upload limit",
            Self::Wordlist => "Wordlist exceeds the upload limit",
        };
        Error(StatusCode::PAYLOAD_TOO_LARGE, message.into())
    }
}

async fn upload_file(
    state: AppState,
    id: String,
    headers: HeaderMap,
    body: Body,
    upload: Upload,
) -> Result<(StatusCode, Json<Job>), Error> {
    let length = headers
        .get(header::CONTENT_LENGTH)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|raw| raw.parse::<u64>().ok())
                .ok_or_else(|| Error::bad_request("Invalid Content-Length"))
        })
        .transpose()?;
    let limit = match upload {
        Upload::Epub => state.config.max_input_bytes,
        Upload::Wordlist => state.config.max_wordlist_bytes,
    };
    let (mut pending, file, remaining, expected) =
        start_upload(&state, &id, length, upload, limit)?;
    tokio::time::timeout(
        remaining,
        receive_upload(&mut pending, file, body, limit, expected, upload),
    )
    .await
    .map_err(|_| Error(StatusCode::REQUEST_TIMEOUT, "Upload window expired".into()))??;

    match upload {
        Upload::Epub => {
            let job = state.store.lock().unwrap().queue(&id, &mut pending)?;
            state.work_ready.notify_waiters();
            Ok((StatusCode::ACCEPTED, Json(job)))
        }
        Upload::Wordlist => {
            let job = state
                .store
                .lock()
                .unwrap()
                .save_wordlist(&id, &mut pending)?;
            Ok((StatusCode::CREATED, Json(job)))
        }
    }
}

fn start_upload(
    state: &AppState,
    id: &str,
    length: Option<u64>,
    upload: Upload,
    limit: u64,
) -> Result<(PendingFile, File, Duration, Option<u64>), Error> {
    let store = state.store.lock().unwrap();
    let job = store.get(id)?.ok_or_else(Error::not_found)?;
    if job.status != Status::WaitingForUpload {
        return Err(Error::conflict("Job already has an EPUB upload"));
    }
    if matches!(upload, Upload::Wordlist) && job.wordlist_size_bytes.is_some() {
        return Err(Error::conflict("Job already has a custom wordlist"));
    }
    let remaining = store.upload_remaining(&job);
    if remaining.is_zero() {
        return Err(Error::not_found());
    }
    if length == Some(0) {
        return Err(upload.empty());
    }
    if length.is_some_and(|bytes| bytes > limit) {
        return Err(upload.too_large());
    }
    let expected_bytes = match upload {
        Upload::Epub => job.expected_bytes,
        Upload::Wordlist => None,
    };
    if let (Some(length), Some(expected)) = (length, expected_bytes)
        && length != expected
    {
        return Err(Error::bad_request(
            "Upload size does not match job metadata",
        ));
    }
    let expected = expected_bytes.or(length);
    if expected.is_some_and(|bytes| bytes > state.config.max_storage_bytes) {
        return Err(Error::storage_full());
    }
    let directory = store.directory(&job.id);
    fs::create_dir_all(&directory)?;
    // Both endpoints share the same staging file so the EPUB cannot queue mid-upload.
    let (pending, file) = PendingFile::new(state.store.clone(), directory.join("upload.part"))
        .map_err(|error| match error.kind() {
            ErrorKind::AlreadyExists => Error::conflict("An upload is already in progress"),
            _ => error.into(),
        })?;
    Ok((pending, file, remaining, expected))
}

async fn receive_upload(
    pending: &mut PendingFile,
    file: File,
    body: Body,
    limit: u64,
    expected: Option<u64>,
    upload: Upload,
) -> Result<(), Error> {
    let mut file = tokio::fs::File::from_std(file);
    let mut stream = body.into_data_stream();
    let mut received = 0;
    if let Some(bytes) = expected
        && !pending.reserve(bytes)?
    {
        return Err(Error::storage_full());
    }
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::bad_request("Incomplete upload"))?;
        let bytes = chunk.len() as u64;
        received += bytes;
        if received > limit {
            return Err(upload.too_large());
        }
        if expected.is_some_and(|expected| received > expected) {
            return Err(Error::bad_request(
                "Upload size does not match job metadata",
            ));
        }
        if expected.is_none() && !pending.reserve(bytes)? {
            return Err(Error::storage_full());
        }
        file.write_all(&chunk).await?;
    }
    if received == 0 {
        return Err(upload.empty());
    }
    if expected.is_some_and(|expected| received != expected) {
        return Err(Error::bad_request(
            "Upload size does not match job metadata",
        ));
    }
    file.sync_all().await?;
    if matches!(upload, Upload::Wordlist) {
        match tokio::fs::read_to_string(pending.path()).await {
            Ok(text) if text.trim_start_matches('\u{feff}').trim().is_empty() => {
                return Err(upload.empty());
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::InvalidData => {
                return Err(Error::bad_request("Expected a UTF-8 text wordlist"));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

async fn download(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let (job, file, download) = {
        let mut store = state.store.lock().unwrap();
        let job = store.get(&id)?.ok_or_else(Error::not_found)?;
        if job.status != Status::Finished {
            return Err(Error::conflict("Job has no downloadable result"));
        }
        let file = File::open(store.directory(&job.id).join("output.epub"))?;
        let download = store.track_download(state.store.clone(), &job.id);
        (job, file, download)
    };
    let (stem, _) = job.filename.rsplit_once('.').unwrap();
    let filename = format!("{stem}.furigana.epub");
    let disposition = format!(
        "attachment; filename=converted.epub; filename*=UTF-8''{}",
        utf8_percent_encode(&filename, NON_ALPHANUMERIC),
    );
    let stream = ReaderStream::new(tokio::fs::File::from_std(file)).map(move |chunk| {
        let _download = &download;
        chunk
    });
    let body = Body::from_stream(stream);
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/epub+zip"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition)?,
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from(job.output_size_bytes.unwrap()),
    );
    Ok(response)
}
