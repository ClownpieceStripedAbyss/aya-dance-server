use std::{
  env,
  io::SeekFrom,
  path::{Path, PathBuf},
  sync::mpsc as std_mpsc,
  thread,
};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use obws;
use serde_json::json;
use tokio::{
  fs::File,
  io::{AsyncBufReadExt, AsyncSeekExt, BufReader},
  sync::mpsc,
  time::{sleep, Duration},
};

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

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct QueueItem {
  #[serde(rename = "playerNames")]
  player_names: Vec<String>,
  title: String,
  #[serde(rename = "playerCount")]
  player_count: String,
  #[serde(rename = "songId")]
  song_id: u32,
  major: String,
  duration: u32,
  group: String,
  #[serde(rename = "doubleWidth")]
  double_width: bool,
}

#[derive(Debug, Clone)]
enum LogLine {
  VideoPlay {
    song_info: String,
    song_requester: Option<String>,
  },
  Queue {
    items: Vec<QueueItem>,
  },
}

async fn tail_file(path: PathBuf, sender: mpsc::Sender<LogLine>) {
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
        sleep(Duration::from_millis(200)).await;
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
          let _ = sender
            .send(LogLine::VideoPlay {
              song_info: song_info.to_string(),
              song_requester: (requester != "Random").then(|| requester.to_string()),
            })
            .await;
        }

        if line.contains("OnPreSerialization: queue info serialized: ")
          || line.contains("OnDeserialization: syncedQueuedInfoJson = ")
        {
          let json = if line.contains("OnPreSerialization: queue info serialized: ") {
            line.splitn(2, "OnPreSerialization: queue info serialized: ")
              .nth(1)
              .unwrap_or("[]")
          } else {
            line.splitn(2, "OnDeserialization: syncedQueuedInfoJson = ")
              .nth(1)
              .unwrap_or("[]")
          };
          let queue_item: Vec<QueueItem> = match serde_json::from_str(json) {
            Ok(item) => item,
            Err(e) => {
              log::warn!("Failed to parse queue item: {:?}", e);
              continue;
            }
          };
          let _ = sender.send(LogLine::Queue { items: queue_item }).await;
        }
      }

      Err(e) => {
        log::warn!("Failed to read line from file {:?}: {:?}", path, e);
        sleep(Duration::from_millis(200)).await;
      }
    }
  }
}

pub async fn serve_obws(obs_host: String, obs_port: u16) -> anyhow::Result<()> {
  log::info!("Connecting to OBS WebSocket {}:{}", obs_host, obs_port);
  let obs_client = match obws::Client::connect(obs_host, obs_port, None as Option<&str>).await {
    Ok(client) => client,
    Err(e) => {
      log::warn!("Failed to connect to OBS WebSocket: {:?}", e);
      loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
      }
    }
  };

  serve_obws_impl(obs_client).await
}

async fn serve_obws_impl(obs_client: obws::Client) -> anyhow::Result<()> {
  log::info!("OBS WebSocket Connnected");
  let (log_tx, mut log_rx) = mpsc::channel::<LogLine>(100);

  tokio::spawn(async move {
    while let Some(line) = log_rx.recv().await {
      let (input_name, text) = match line {
        LogLine::VideoPlay {
          song_info,
          song_requester,
        } => (
          "WDNow",
          match song_requester {
            None => format!("当前播放: {}", song_info),
            Some(song_requester) => format!("当前播放: {} ({})", song_info, song_requester),
          },
        ),
        LogLine::Queue { items } => ("WDQueue", {
          match items.first() {
            Some(item) => {
              let song_info = format!("{} - {}", item.title, item.group);
              let song_requester = item.player_names.join(", ");
              format!("下一首: {} ({})", song_info, song_requester)
            }
            None => "".to_string(),
          }
        }),
      };
      
      log::info!("Updating OBS text source: {} = {}", input_name, text);

      if let Err(e) = obs_client
        .inputs()
        .set_settings(obws::requests::inputs::SetSettings {
          input: obws::requests::inputs::InputId::Name(input_name),
          settings: &json!({
              "text": text,
          }),
          overlay: Some(true),
        })
        .await
      {
        log::warn!("Failed to update OBS text source: {:?}", e);
      }
    }
  });

  // tail each log file in the log directory
  let log_dir = get_vrchat_log_dir();
  log::info!("VRC log folder: {:?}", log_dir);
  if let Ok(entries) = std::fs::read_dir(&log_dir) {
    for entry in entries.filter_map(Result::ok) {
      let path = entry.path();
      if is_log_file(&path) {
        let tx = log_tx.clone();
        tokio::spawn(tail_file(path, tx));
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
        notify::Config::default().with_poll_interval(Duration::from_secs(30)),
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
          Ok(event) if event.kind == notify::EventKind::Create(notify::event::CreateKind::File) => {
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
          _ => (),
        }
      }
    });
  }

  // 在异步任务中接收新文件事件，并启动 tail 任务
  while let Some(new_path) = new_file_rx.recv().await {
    let tx = log_tx.clone();
    tokio::spawn(tail_file(new_path, tx));
  }

  Ok(())
}
