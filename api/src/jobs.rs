use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::Config;

pub type SharedStore = Arc<Mutex<Store>>;
pub const CUSTOM_WORDLIST_FILENAME: &str = "wordlist.txt";

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    WaitingForUpload,
    Queued,
    Processing,
    Finished,
    Failed,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct JobOptions {
    pub override_existing: bool,
    pub highlight_injected: bool,
    pub highlight_color: Option<String>,
    pub new_identifier: bool,
    pub identifier: Option<String>,
    pub ignore_wordlists: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateJob {
    pub filename: String,
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub options: JobOptions,
}

#[derive(Serialize)]
pub struct Job {
    pub id: String,
    pub filename: String,
    pub status: Status,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wordlist_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'static str>,
    #[serde(skip)]
    pub expected_bytes: Option<u64>,
}

impl Job {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let id: String = row.get("id")?;
        let status = match row.get_ref("status")?.as_str()? {
            "waiting_for_upload" => Status::WaitingForUpload,
            "queued" => Status::Queued,
            "processing" => Status::Processing,
            "finished" => Status::Finished,
            "failed" => Status::Failed,
            _ => return Err(rusqlite::Error::InvalidQuery),
        };
        Ok(Self {
            download_url: (status == Status::Finished).then(|| format!("/jobs/{id}/download")),
            error: (status == Status::Failed).then_some("EPUB conversion failed"),
            id,
            status,
            filename: row.get("filename")?,
            created_at_ms: row.get("created_at")?,
            updated_at_ms: row.get("updated_at")?,
            expected_bytes: row
                .get::<_, Option<i64>>("expected_bytes")?
                .map(|n| n as u64),
            input_size_bytes: row.get::<_, Option<i64>>("input_bytes")?.map(|n| n as u64),
            wordlist_size_bytes: row
                .get::<_, Option<i64>>("wordlist_bytes")?
                .map(|n| n as u64),
            output_size_bytes: row.get::<_, Option<i64>>("output_bytes")?.map(|n| n as u64),
        })
    }
}

pub struct Work {
    pub id: String,
    pub options: String,
    pub directory: PathBuf,
    pub has_custom_wordlist: bool,
}

pub struct Store {
    db: Connection,
    files: PathBuf,
    used_bytes: u64,
    downloads: HashMap<String, ActiveDownload>,
    max_bytes: u64,
    upload_ttl: Duration,
    finished_ttl: Duration,
    _lock: File,
}

impl Store {
    pub fn open(config: &Config) -> Result<Self> {
        fs::create_dir_all(&config.data_dir)?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(config.data_dir.join("api.lock"))?;
        lock.try_lock().context("locking the API data directory")?;
        let files = config.data_dir.join("files");
        fs::create_dir_all(&files)?;
        let db = Connection::open(config.data_dir.join("jobs.sqlite3"))?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS jobs (
                 id TEXT PRIMARY KEY,
                 filename TEXT NOT NULL,
                 options TEXT NOT NULL,
                 status TEXT NOT NULL CHECK (status IN (
                     'waiting_for_upload', 'queued', 'processing', 'finished', 'failed', 'deleting'
                 )),
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 queued_at INTEGER,
                 expected_bytes INTEGER,
                 input_bytes INTEGER,
                 wordlist_bytes INTEGER,
                 output_bytes INTEGER,
                 error TEXT
             );
             CREATE INDEX IF NOT EXISTS jobs_cleanup ON jobs(status, updated_at);
             CREATE INDEX IF NOT EXISTS jobs_queue ON jobs(status, queued_at);",
        )?;
        let used_bytes: i64 = db.query_row(
            "SELECT COALESCE(SUM(
                 COALESCE(input_bytes, 0) + COALESCE(wordlist_bytes, 0) + COALESCE(output_bytes, 0)
             ), 0)
             FROM jobs",
            [],
            |row| row.get(0),
        )?;
        let mut store = Self {
            db,
            files,
            used_bytes: used_bytes as u64,
            downloads: HashMap::new(),
            max_bytes: config.max_storage_bytes,
            upload_ttl: config.upload_ttl,
            finished_ttl: config.finished_ttl,
            _lock: lock,
        };
        store.recover()?;
        store.cleanup()?;
        Ok(store)
    }

    fn recover(&self) -> Result<()> {
        for entry in fs::read_dir(&self.files)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(id) = name.to_str().filter(|name| Uuid::parse_str(name).is_ok()) else {
                continue;
            };
            let job: Option<(String, bool)> = self
                .db
                .query_row(
                    "SELECT status, wordlist_bytes IS NOT NULL FROM jobs WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let directory = entry.path();
            match job
                .as_ref()
                .map(|(status, has_wordlist)| (status.as_str(), *has_wordlist))
            {
                None | Some(("waiting_for_upload", false)) => remove_directory(&directory)?,
                Some(("waiting_for_upload", true)) => {
                    remove_file(&directory.join("upload.part"))?;
                    remove_file(&directory.join("input.epub"))?;
                }
                Some(("processing" | "queued" | "failed", _)) => {
                    remove_file(&directory.join("output.epub"))?;
                }
                _ => {}
            }
        }
        self.db.execute(
            "UPDATE jobs SET status = 'queued', updated_at = ?1 WHERE status = 'processing'",
            [now_ms()],
        )?;
        Ok(())
    }

    pub fn create(&self, request: CreateJob) -> Result<Job> {
        let id = Uuid::new_v4().to_string();
        let options = serde_json::to_string(&request.options)?;
        let job = self.db.query_one(
            "INSERT INTO jobs (id, filename, options, status, expected_bytes, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'waiting_for_upload', ?4, ?5, ?5) RETURNING *",
            params![id, request.filename, options, request.size_bytes.map(|bytes| bytes as i64), now_ms()],
            Job::from_row,
        )?;
        Ok(job)
    }

    pub fn get(&self, id: &str) -> Result<Option<Job>> {
        Ok(self
            .db
            .query_row(
                "SELECT * FROM jobs WHERE id = ?1 AND status != 'deleting'",
                [id],
                Job::from_row,
            )
            .optional()?)
    }

    pub fn upload_remaining(&self, job: &Job) -> Duration {
        let age = now_ms().saturating_sub(job.created_at_ms).max(0) as u64;
        self.upload_ttl.saturating_sub(Duration::from_millis(age))
    }

    pub fn directory(&self, id: &str) -> PathBuf {
        self.files.join(id)
    }

    pub fn track_download(&mut self, store: SharedStore, id: &str) -> Download {
        self.downloads.entry(id.to_owned()).or_default().count += 1;
        Download {
            store,
            id: id.to_owned(),
        }
    }

    pub fn queue(&self, id: &str, pending: &mut PendingFile) -> Result<Job> {
        pending.move_to(self.directory(id).join("input.epub"))?;
        let job = self.db.query_one(
            "UPDATE jobs SET status = 'queued', input_bytes = ?2, queued_at = ?3, updated_at = ?3
             WHERE id = ?1 AND status = 'waiting_for_upload' RETURNING *",
            params![id, pending.bytes as i64, now_ms()],
            Job::from_row,
        )?;
        pending.keep();
        Ok(job)
    }

    pub fn save_wordlist(&self, id: &str, pending: &mut PendingFile) -> Result<Job> {
        pending.move_to(self.directory(id).join(CUSTOM_WORDLIST_FILENAME))?;
        let job = self.db.query_one(
            "UPDATE jobs SET wordlist_bytes = ?2, updated_at = ?3
             WHERE id = ?1 AND status = 'waiting_for_upload' AND wordlist_bytes IS NULL RETURNING *",
            params![id, pending.bytes as i64, now_ms()],
            Job::from_row,
        )?;
        pending.keep();
        Ok(job)
    }

    pub fn claim(&self) -> Result<Option<Work>> {
        Ok(self
            .db
            .query_one(
                "UPDATE jobs SET status = 'processing', updated_at = ?1 WHERE id = (
                SELECT id FROM jobs WHERE status = 'queued' ORDER BY queued_at, rowid LIMIT 1
             ) RETURNING id, options, wordlist_bytes IS NOT NULL",
                [now_ms()],
                |row| {
                    let id: String = row.get(0)?;
                    Ok(Work {
                        directory: self.directory(&id),
                        id,
                        options: row.get(1)?,
                        has_custom_wordlist: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn finish(&self, id: &str, pending: &mut PendingFile) -> Result<()> {
        let changed = self.db.execute(
            "UPDATE jobs SET status = 'finished', output_bytes = ?2, updated_at = ?3
             WHERE id = ?1 AND status = 'processing'",
            params![id, pending.bytes as i64, now_ms()],
        )?;
        ensure!(changed == 1, "job {id} is no longer processing");
        pending.keep();
        Ok(())
    }

    pub fn fail(&self, id: &str, error: &str) -> Result<()> {
        self.db.execute(
            "UPDATE jobs SET status = 'failed', error = ?2, updated_at = ?3
             WHERE id = ?1 AND status = 'processing'",
            params![id, error, now_ms()],
        )?;
        Ok(())
    }

    pub fn cleanup(&mut self) -> Result<()> {
        let now = now_ms();
        let mut query = self.db.prepare(
            "SELECT id, status FROM jobs WHERE status = 'deleting'
             OR (status = 'waiting_for_upload' AND created_at <= ?1)
             OR (status = 'finished' AND updated_at <= ?2)",
        )?;
        let jobs = query
            .query_map(
                [
                    now - self.upload_ttl.as_millis() as i64,
                    now - self.finished_ttl.as_millis() as i64,
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(query);
        for (id, status) in jobs {
            if status != "waiting_for_upload"
                || !self.directory(&id).join("upload.part").try_exists()?
            {
                self.delete(&id)?;
            }
        }
        self.make_room(0)?;
        Ok(())
    }

    fn make_room(&mut self, bytes: u64) -> Result<bool> {
        let Some(target) = self.max_bytes.checked_sub(bytes) else {
            return Ok(false);
        };
        if self.used_bytes <= target {
            return Ok(true);
        }

        let mut remaining = self.used_bytes - target;
        let mut oldest = Vec::new();
        let mut query = self.db.prepare(
            "SELECT id, input_bytes + COALESCE(wordlist_bytes, 0), output_bytes FROM jobs
             WHERE status = 'finished' ORDER BY updated_at, rowid",
        )?;
        let mut rows = query.query([])?;
        while remaining > 0 {
            let Some(row) = rows.next()? else {
                break;
            };
            let id: String = row.get(0)?;
            let input: i64 = row.get(1)?;
            let output: i64 = row.get(2)?;
            let reclaimable = if self.downloads.contains_key(&id) {
                input
            } else {
                input + output
            };
            remaining = remaining.saturating_sub(reclaimable as u64);
            oldest.push(id);
        }
        drop(rows);
        drop(query);
        // A lower quota may leave the store over budget on startup. In that case,
        // free what we can. For a new write, evict only if it can proceed.
        if remaining > 0 && bytes > 0 {
            return Ok(false);
        }
        for id in oldest {
            self.delete(&id)?;
        }
        Ok(remaining == 0)
    }

    fn delete(&mut self, id: &str) -> Result<()> {
        self.db
            .execute("UPDATE jobs SET status = 'deleting' WHERE id = ?1", [id])?;
        remove_directory(&self.directory(id))?;
        let (input, output): (i64, i64) = self.db.query_one(
            "DELETE FROM jobs WHERE id = ?1
             RETURNING COALESCE(input_bytes, 0) + COALESCE(wordlist_bytes, 0), COALESCE(output_bytes, 0)",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let retained = if let Some(download) = self.downloads.get_mut(id) {
            download.retained_bytes = output as u64;
            download.retained_bytes
        } else {
            0
        };
        self.used_bytes -= (input + output) as u64 - retained;
        Ok(())
    }
}

#[derive(Default)]
struct ActiveDownload {
    count: usize,
    retained_bytes: u64,
}

pub struct Download {
    store: SharedStore,
    id: String,
}

impl Drop for Download {
    fn drop(&mut self) {
        let mut store = self.store.lock().unwrap();
        let download = store.downloads.get_mut(&self.id).unwrap();
        download.count -= 1;
        if download.count == 0 {
            // A file unlinked by cleanup occupies space until the last reader closes it.
            store.used_bytes -= download.retained_bytes;
            store.downloads.remove(&self.id);
        }
    }
}

pub struct PendingFile {
    store: SharedStore,
    path: Option<PathBuf>,
    pub bytes: u64,
}

impl PendingFile {
    pub fn new(store: SharedStore, path: PathBuf) -> std::io::Result<(Self, File)> {
        let file = File::create_new(&path)?;
        Ok((
            Self {
                store,
                path: Some(path),
                bytes: 0,
            },
            file,
        ))
    }

    pub fn reserve(&mut self, bytes: u64) -> Result<bool> {
        let mut store = self.store.lock().unwrap();
        if !store.make_room(bytes)? {
            return Ok(false);
        }
        store.used_bytes += bytes;
        self.bytes += bytes;
        Ok(true)
    }

    pub fn path(&self) -> &Path {
        self.path.as_deref().unwrap()
    }

    fn move_to(&mut self, path: PathBuf) -> std::io::Result<()> {
        fs::rename(self.path(), &path)?;
        self.path = Some(path);
        Ok(())
    }

    fn keep(&mut self) {
        self.path = None;
    }
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let mut store = self.store.lock().unwrap();
            match remove_file(&path) {
                Ok(()) => store.used_bytes -= self.bytes,
                Err(error) => eprintln!("removing {}: {error}", path.display()),
            }
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn remove_directory(path: &Path) -> std::io::Result<()> {
    ignore_missing(fs::remove_dir_all(path))
}

fn remove_file(path: &Path) -> std::io::Result<()> {
    ignore_missing(fs::remove_file(path))
}

fn ignore_missing(result: std::io::Result<()>) -> std::io::Result<()> {
    result.or_else(|error| match error.kind() {
        ErrorKind::NotFound => Ok(()),
        _ => Err(error),
    })
}
