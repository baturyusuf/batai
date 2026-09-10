use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use super::{
    errors::{Result, RuntimeError},
    events::EventEngine,
    tasks::TaskEngine,
    types::EventType,
};

enum Message {
    Path(PathBuf),
    Stop,
}

pub struct TaskWatcher {
    watcher: Option<RecommendedWatcher>,
    sender: mpsc::Sender<Message>,
    worker: Option<thread::JoinHandle<()>>,
}

impl TaskWatcher {
    pub async fn start(
        tasks_dir: &Path,
        engine: Arc<TaskEngine>,
        events: EventEngine,
        debounce: Duration,
    ) -> Result<Self> {
        std::fs::create_dir_all(tasks_dir)?;
        let mut files = std::fs::read_dir(tasks_dir)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().and_then(|v| v.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        files.sort();
        for file in files {
            if let Err(error) = engine.ingest_file(&file).await {
                emit_file_error(&events, &file, &error);
            }
        }

        let (sender, receiver) = mpsc::channel();
        let notify_sender = sender.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                if let Ok(event) = result {
                    for path in event.paths {
                        if path.extension().and_then(|v| v.to_str()) == Some("json") {
                            let _ = notify_sender.send(Message::Path(path));
                        }
                    }
                }
            })?;
        watcher.watch(tasks_dir, RecursiveMode::NonRecursive)?;
        let handle = tokio::runtime::Handle::current();
        let worker = thread::spawn(move || {
            let mut pending = HashMap::<PathBuf, Instant>::new();
            loop {
                match receiver.recv_timeout(debounce.min(Duration::from_millis(50))) {
                    Ok(Message::Path(path)) => {
                        pending.insert(path, Instant::now());
                    }
                    Ok(Message::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                let ready = pending
                    .iter()
                    .filter(|(_, at)| at.elapsed() >= debounce)
                    .map(|(path, _)| path.clone())
                    .collect::<Vec<_>>();
                for path in ready {
                    pending.remove(&path);
                    if !path.is_file() {
                        continue;
                    }
                    let engine = Arc::clone(&engine);
                    let events = events.clone();
                    handle.spawn(async move {
                        if let Err(error) = engine.ingest_file(&path).await {
                            emit_file_error(&events, &path, &error);
                        }
                    });
                }
            }
        });
        Ok(Self {
            watcher: Some(watcher),
            sender,
            worker: Some(worker),
        })
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.watcher.take();
        let _ = self.sender.send(Message::Stop);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| RuntimeError::Lock("watcher worker"))?;
        }
        Ok(())
    }
}

impl Drop for TaskWatcher {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn emit_file_error(events: &EventEngine, path: &Path, error: &RuntimeError) {
    let _ = events.publish(
        EventType::TaskFailed,
        "watcher",
        None,
        None,
        serde_json::json!({"file":path,"error":error.to_string()}),
    );
}
