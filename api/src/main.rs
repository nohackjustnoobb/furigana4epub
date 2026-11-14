mod config;
mod http;
mod jobs;
mod wordlists;

use std::fs;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use furigana4epub_core::{Furigana, Options};
use tokio::sync::Notify;
use tokio::task::spawn_blocking;

use crate::config::Config;
use crate::jobs::{CUSTOM_WORDLIST_FILENAME, JobOptions, PendingFile, SharedStore, Store, Work};
use crate::wordlists::Wordlists;

#[derive(Clone)]
struct State {
    config: Arc<Config>,
    store: SharedStore,
    work_ready: Arc<Notify>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;
    let store = Store::open(&config).context("opening the job store")?;
    let converter = Arc::new(
        Furigana::new(&config.dictionary_path)
            .context("loading the dictionary (run furigana4epub update-dict to install it)")?,
    );
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    let state = State {
        config: Arc::new(config),
        store: Arc::new(Mutex::new(store)),
        work_ready: Arc::new(Notify::new()),
    };
    for _ in 0..state.config.workers {
        tokio::spawn(worker(state.clone(), converter.clone()));
    }
    tokio::spawn(cleanup(state.clone()));
    eprintln!("API listening on {}", listener.local_addr()?);
    axum::serve(listener, http::router(state)).await?;
    Ok(())
}

async fn worker(state: State, converter: Arc<Furigana>) {
    loop {
        let notified = state.work_ready.notified();
        let work = state.store.lock().unwrap().claim();
        let work = match work {
            Ok(Some(work)) => work,
            Ok(None) => {
                notified.await;
                continue;
            }
            Err(error) => {
                eprintln!("claiming a queued job: {error:#}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let id = work.id.clone();
        let worker_state = state.clone();
        let converter = converter.clone();
        let result = spawn_blocking(move || convert(&worker_state, &converter, work)).await;
        let error = match result {
            Ok(Ok(())) => continue,
            Ok(Err(error)) => format!("{error:#}"),
            Err(error) => format!("conversion task stopped: {error}"),
        };
        eprintln!("job {id}: {error}");
        if let Err(error) = state.store.lock().unwrap().fail(&id, &error) {
            eprintln!("recording failure for job {id}: {error:#}");
        }
    }
}

fn convert(state: &State, converter: &Furigana, work: Work) -> Result<()> {
    let options: JobOptions = serde_json::from_str(&work.options)?;
    let mut wordlists = Vec::new();
    if !options.ignore_wordlists.is_empty() {
        let available = Wordlists::read(&state.config.wordlists_dir)?;
        for name in &options.ignore_wordlists {
            let path = available
                .find(name)
                .with_context(|| format!("unknown wordlist {name:?}"))?;
            wordlists.push(path.to_path_buf());
        }
    }
    if work.has_custom_wordlist {
        wordlists.push(work.directory.join(CUSTOM_WORDLIST_FILENAME));
    }
    let input = fs::read(work.directory.join("input.epub")).context("reading input EPUB")?;
    let output = converter.convert_epub(
        &input,
        Options {
            override_existing: options.override_existing,
            highlight_injected: options.highlight_injected,
            highlight_color: options.highlight_color.as_deref(),
            new_identifier: options.new_identifier,
            identifier: options.identifier.as_deref(),
            ignore_wordlists: &wordlists,
        },
    )?;
    let (mut pending, mut file) =
        PendingFile::new(state.store.clone(), work.directory.join("output.epub"))
            .context("creating output EPUB")?;
    ensure!(
        pending.reserve(output.len() as u64)?,
        "job storage limit reached while saving output"
    );
    file.write_all(&output).context("writing output EPUB")?;
    file.sync_all().context("syncing output EPUB")?;
    drop(file);
    state.store.lock().unwrap().finish(&work.id, &mut pending)?;
    Ok(())
}

async fn cleanup(state: State) {
    let mut timer = tokio::time::interval(state.config.cleanup_interval);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    timer.tick().await;
    loop {
        timer.tick().await;
        let store = state.store.clone();
        match spawn_blocking(move || store.lock().unwrap().cleanup()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("cleaning up jobs: {error:#}"),
            Err(error) => eprintln!("cleanup task stopped: {error}"),
        }
    }
}
