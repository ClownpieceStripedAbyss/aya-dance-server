use serde_json::json;
use tokio::sync::mpsc;

use crate::{wanna::log_watcher::LogLine, AppService};

pub async fn serve(app: AppService, obs_host: String, obs_port: u16) -> anyhow::Result<()> {
  loop {
    log::info!("Connecting to OBS WebSocket {}:{}", obs_host, obs_port);
    match obws::Client::connect(obs_host.clone(), obs_port, None as Option<&str>).await {
      Ok(client) => serve_obws_impl(app.clone(), client)
        .await
        .unwrap_or_else(|e| {
          log::warn!("OBS WebSocket disconnected: {:?}", e);
        }),
      Err(e) => {
        log::warn!("Failed to connect to OBS WebSocket: {:?}, retry in 60s", e);
      }
    }
    tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
  }
}

async fn serve_obws_impl(app: AppService, obs_client: obws::Client) -> anyhow::Result<()> {
  log::info!("OBS WebSocket connected");
  let (log_tx, mut log_rx) = mpsc::channel::<LogLine>(100);
  app.log_watcher.register_recipient(log_tx).await;

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

    obs_client
      .inputs()
      .set_settings(obws::requests::inputs::SetSettings {
        input: obws::requests::inputs::InputId::Name(input_name),
        settings: &json!({
            "text": text,
        }),
        overlay: Some(true),
      })
      .await?;
    // ^ If fails, return error and try reconnecting to OBS
  }

  Ok(())
}
