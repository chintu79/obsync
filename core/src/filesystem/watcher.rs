use std::path::Path;
use std::time::Duration;

use notify::event::ModifyKind;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as NotifyWatcher};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum WatchEvent {
    Created(std::path::PathBuf),
    Modified(std::path::PathBuf),
    Removed(std::path::PathBuf),
    Renamed(std::path::PathBuf, std::path::PathBuf),
}

pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<WatchEvent>,
}

impl FileWatcher {
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel(1024);

        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    let tx = tx.clone();
                    let events = translate_event(event);
                    for evt in events {
                        let _ = tx.try_send(evt);
                    }
                }
            },
            Config::default()
                .with_poll_interval(Duration::from_secs(2))
                .with_compare_contents(false),
        )
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        Ok(Self { _watcher: watcher, rx })
    }

    pub async fn next_event(&mut self) -> Option<WatchEvent> {
        self.rx.recv().await
    }

    pub fn event_stream(&mut self) -> &mut mpsc::Receiver<WatchEvent> {
        &mut self.rx
    }
}

fn translate_event(event: Event) -> Vec<WatchEvent> {
    let mut events = Vec::new();
    match event.kind {
        EventKind::Create(_) => {
            for path in event.paths {
                events.push(WatchEvent::Created(path));
            }
        }
        EventKind::Modify(kind) => {
            if matches!(
                kind,
                ModifyKind::Data(_) | ModifyKind::Name(_) | ModifyKind::Any
            ) {
                for path in event.paths {
                    events.push(WatchEvent::Modified(path));
                }
            }
        }
        EventKind::Remove(_) => {
            for path in event.paths {
                events.push(WatchEvent::Removed(path));
            }
        }
        _ => {}
    }
    events
}

pub struct Debouncer {
    interval: Duration,
}

impl Debouncer {
    pub fn new(interval_ms: u64) -> Self {
        Self {
            interval: Duration::from_millis(interval_ms),
        }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }
}

impl Default for Debouncer {
    fn default() -> Self {
        Self::new(500)
    }
}
