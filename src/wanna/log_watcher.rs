use std::{
  env,
  io::SeekFrom,
  path::{Path, PathBuf},
  sync::{mpsc as std_mpsc, Arc},
  thread,
  time::Duration,
};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::{
  fs::File,
  io::{AsyncBufReadExt, AsyncSeekExt, BufReader},
  sync::{mpsc, RwLock},
  time::sleep,
};

use crate::AppService;

type SenderVec = Arc<RwLock<Vec<mpsc::Sender<LogLine>>>>;

#[derive(Debug, Default)]
pub struct WannaLogWatcherImpl {
  senders: SenderVec,
}

pub type WannaLogWatcher = Arc<WannaLogWatcherImpl>;

impl WannaLogWatcherImpl {
  pub async fn register_recipient(&self, sender: mpsc::Sender<LogLine>) {
    let mut senders = self.senders.write().await;
    senders.push(sender);
  }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct QueueItem {
  #[serde(rename = "playerNames")]
  pub player_names: Vec<String>,
  pub title: String,
  #[serde(rename = "playerCount")]
  pub player_count: String,
  #[serde(rename = "songId")]
  pub song_id: i32,
  pub major: String,
  pub duration: i32,
  pub group: String,
  #[serde(rename = "doubleWidth")]
  pub double_width: bool,
}

#[derive(Debug, Clone)]
pub enum LogLine {
  VideoPlay {
    song_info: String,
    song_requester: Option<String>,
  },
  Queue {
    items: Vec<QueueItem>,
  },
}

fn get_vrchat_log_dir() -> PathBuf {
  let appdata = env::var("APPDATA").expect("no APPDATA?");
  let appdata_path = Path::new(&appdata);
  let base = appdata_path.parent().expect("no APPDATA parent?");
  base.join("LocalLow").join("VRChat").join("VRChat")
}

fn is_log_file(path: &Path) -> bool {
  if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
    file_name.starts_with("output_log")
      && path
        .extension()
        .and_then(|s| s.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("txt"))
        .unwrap_or(false)
  } else {
    false
  }
}

async fn tail_file(path: PathBuf, sender: SenderVec) {
  log::info!("Watching log file: {:?}", path);

  let file = match File::open(&path).await {
    Ok(f) => f,
    Err(e) => {
      log::warn!("Failed to open file {:?}: {:?}", path, e);
      return;
    }
  };

  let mut reader = BufReader::new(file);

  if let Err(e) = reader.seek(SeekFrom::End(0)).await {
    log::warn!("Failed to seek to end of file {:?}: {:?}", path, e);
    return;
  }

  loop {
    let mut line = String::new();
    match reader.read_line(&mut line).await {
      Ok(0) => {
        sleep(Duration::from_millis(1000)).await;
      }
      Ok(_) => {
        // [VRCX] VideoPlay(PyPyDance) "https://api.udon.dance/Api/Songs/play?id=222",0,114514,"$https://api.udon.dance/Api/Songs/play?id=222 (imkiva)"
        // [VRCX] VideoPlay(PyPyDance) "http://api.udon.dance/Api/Songs/play?id=1",0,114514,"$1. CH4NGE - Giga | Song^_^ (imkiva)"
        if line.contains("[VRCX] VideoPlay(PyPyDance) ") {
          let parts = line
            .trim()
            .trim_start_matches("[VRCX] VideoPlay(PyPyDance) ")
            .splitn(4, ',')
            .collect::<Vec<&str>>();
          let info = parts[3].trim_start_matches("\"$").trim_end_matches("\"");
          let requester = {
            let vec = info.splitn(2, " (").collect::<Vec<&str>>();
            if vec.len() < 2 {
              ""
            } else {
              vec[1].trim_end_matches(")")
            }
          };
          let song_info = info.splitn(2, " (").next().unwrap_or("");
          for sender in sender.read().await.iter() {
            let _ = sender
              .send(LogLine::VideoPlay {
                song_info: song_info.to_string(),
                song_requester: (requester != "Random").then(|| requester.to_string()),
              })
              .await;
          }
        }

        if line.contains("OnPreSerialization: queue info serialized: ")
          || line.contains("OnDeserialization: syncedQueuedInfoJson = ")
        {
          let json = if line.contains("OnPreSerialization: queue info serialized: ") {
            line
              .splitn(2, "OnPreSerialization: queue info serialized: ")
              .nth(1)
              .unwrap_or("[]")
              .trim()
              .trim_end_matches("</color>")
          } else {
            line
              .splitn(2, "OnDeserialization: syncedQueuedInfoJson = ")
              .nth(1)
              .unwrap_or("[]")
              .trim()
              .trim_end_matches("</color>")
          };
          let queue_item: Vec<QueueItem> = match serde_json::from_str(json) {
            Ok(item) => item,
            Err(e) => {
              log::warn!(
                "Failed to parse queue item from json: {:?}, json: {}",
                e,
                json
              );
              continue;
            }
          };
          for sender in sender.read().await.iter() {
            let _ = sender
              .send(LogLine::Queue {
                items: queue_item.clone(),
              })
              .await;
          }
        }
      }

      Err(e) => {
        log::warn!("Failed to read line from file {:?}: {:?}", path, e);
        sleep(Duration::from_millis(1000)).await;
      }
    }
  }
}

pub async fn serve(app: AppService) -> anyhow::Result<()> {
  // tail each log file in the log directory
  let log_dir = get_vrchat_log_dir();
  log::info!("VRC log folder: {:?}", log_dir);
  if let Ok(entries) = std::fs::read_dir(&log_dir) {
    for entry in entries.filter_map(Result::ok) {
      let path = entry.path();
      if is_log_file(&path) {
        let senders = app.log_watcher.senders.clone();
        tokio::spawn(tail_file(path, senders));
      }
    }
  } else {
    log::warn!("Failed to read VRC log folder: {:?}", log_dir);
  }

  let (new_file_tx, mut new_file_rx) = mpsc::unbounded_channel::<PathBuf>();
  {
    let log_dir_clone = log_dir.clone();
    let new_file_tx_clone = new_file_tx.clone();
    thread::spawn(move || {
      let (tx, rx) = std_mpsc::channel();
      let mut watcher: RecommendedWatcher = match Watcher::new(
        tx,
        notify::Config::default().with_poll_interval(Duration::from_secs(60)),
      ) {
        Ok(w) => w,
        Err(e) => {
          log::warn!("Failed to create VRC log folder notifier: {:?}", e);
          return;
        }
      };
      match watcher.watch(&log_dir_clone, RecursiveMode::NonRecursive) {
        Ok(_) => {}
        Err(e) => {
          log::warn!("Failed to watch VRC log folder: {:?}", e);
          return;
        }
      }

      for res in rx {
        match res {
          Ok(event) => {
            // ignore non-create events
            match event.kind {
              notify::EventKind::Create(_) => (),
              _ => continue,
            }
            // ignore non-log files
            for path in event.paths {
              if is_log_file(&path) {
                log::info!("VRC created new log file, watching it: {:?}", path);
                let _ = new_file_tx_clone.send(path);
              }
            }
          }
          Err(e) => {
            log::warn!("VRC log folder notifier error: {:?}", e);
          }
        }
      }
    });
  }

  while let Some(new_path) = new_file_rx.recv().await {
    let senders = app.log_watcher.senders.clone();
    tokio::spawn(tail_file(new_path, senders));
  }

  Ok(())
}
