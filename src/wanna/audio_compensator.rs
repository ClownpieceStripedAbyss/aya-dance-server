use std::sync::Arc;

use anyhow::anyhow;
use aya_dance_types::SongId;
use log::{info, warn};
use tokio::sync::{mpsc, RwLock};

use crate::{
  wanna::{
    ffmpeg::{ffmpeg_audio_compensation, ffmpeg_copy},
    log_watcher::LogLine,
  },
  AppService,
};

#[derive(Debug, Default)]
pub struct AudioCompensatorServiceImpl {
  running_tasks: RwLock<Vec<CompensatorTask>>,
}

pub type AudioCompensatorService = Arc<AudioCompensatorServiceImpl>;

#[derive(Debug, Clone)]
pub struct CompensatorTask {
  pub(crate) song_id: SongId,
  pub(crate) song_md5: Option<String>,
  pub(crate) input_video_path: String,
  pub(crate) audio_offset: f64,
}

impl CompensatorTask {
  pub fn same_task(&self, other: &Self) -> bool {
    self.song_id == other.song_id
      && self.input_video_path == other.input_video_path
      && self.song_md5 == other.song_md5
      && (self.audio_offset - other.audio_offset).abs() < f64::EPSILON
  }
}

pub async fn serve(app: AppService) -> anyhow::Result<()> {
  loop {
    let _ = serve_audio_compensator(app.clone()).await;
    log::warn!("Audio compensator exited, restarting in 5 seconds");
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
  }
}

pub async fn compute_compensated_file_path(
  app: AppService,
  id: SongId,
  audio_offset: f64,
  md5: Option<String>,
) -> (String, String) {
  let md5 = match md5 {
    Some(m) => m,
    None => app
      .cdn
      .get_video_file_checksum_by_id(id)
      .await
      .unwrap_or_default(),
  };
  let compensated_final = format!(
    "{}/{}-{}-audio-offset-{}.mp4",
    app.cdn.cache_path, id, md5, audio_offset,
  );
  let compensated_stage1 = format!(
    "{}/{}-{}-audio-offset-{}-nocopy.mp4",
    app.cdn.cache_path, id, md5, audio_offset,
  );
  (compensated_final, compensated_stage1)
}

async fn compensate_video_file(
  app: AppService,
  id: SongId,
  video_file: String,
  md5: Option<String>,
  audio_offset: f64,
) -> anyhow::Result<String> {
  let (compensated, compensated_stage1) =
    compute_compensated_file_path(app.clone(), id, audio_offset, md5).await;

  if !std::path::Path::new(compensated.as_str()).exists() {
    std::fs::create_dir_all(app.cdn.cache_path.as_str())
      .map_err(|e| anyhow!("Failed to create cache directory: {}", e))?;

    let start = std::time::Instant::now();
    let stats = ffmpeg_audio_compensation(
      video_file.as_str(),
      compensated_stage1.as_str(),
      audio_offset,
    )
    .map_err(|e| anyhow!("Failed to compensate audio for song {}: {}", id, e))?;

    info!(
      "Compensate {} (ss+aac, {:.2}s, vcopy={:.3}s, adec={:.3}s, ares={:.3}s, aenc={:.3}s)",
      id,
      start.elapsed().as_secs_f64(),
      stats.video_copy_secs,
      stats.audio_decode_secs,
      stats.audio_resample_secs,
      stats.audio_encode_secs,
    );

    let start = std::time::Instant::now();
    ffmpeg_copy(compensated_stage1.as_str(), compensated.as_str())
      .map_err(|e| anyhow!("Failed to copy compensated audio for song {}: {}", id, e))?;

    info!(
      "Compensate {} (copy,   {:.2}s)",
      id,
      start.elapsed().as_secs_f64(),
    );

    if let Err(e) = std::fs::remove_file(compensated_stage1.as_str()) {
      warn!(
        "Failed to remove temporary file {}: {:?}",
        compensated_stage1, e
      );
    }
  }
  Ok(compensated)
}

async fn compensate_one_task(app: AppService, task: CompensatorTask) -> anyhow::Result<()> {
  let CompensatorTask {
    song_id,
    song_md5,
    input_video_path,
    audio_offset,
  } = task;

  compensate_video_file(
    app.clone(),
    song_id,
    input_video_path,
    song_md5,
    audio_offset,
  )
  .await?;
  Ok(())
}

pub async fn submit_new_compensator_task(
  app: AppService,
  task: CompensatorTask,
) -> anyhow::Result<()> {
  log::info!("Received audio compensation task: {}", task.song_id);
  let mut running_tasks = app.audio_compensator.running_tasks.write().await;
  // If the task is already running, skip it
  if running_tasks.iter().any(|t| task.same_task(t)) {
    log::info!(
      "Compensate task for {} already running, don't submit again",
      task.song_id
    );
    return Ok(());
  }

  // Now record we are running this task, don't push the same task again
  running_tasks.push(task.clone());
  let result = compensate_one_task(app.clone(), task.clone()).await;

  // Remove the task from the running tasks
  running_tasks.retain(|t| !t.same_task(&task));

  result
}

async fn serve_audio_compensator(app: AppService) -> anyhow::Result<()> {
  log::info!("Started background audio compensator");
  let (log_tx, mut log_rx) = mpsc::channel::<LogLine>(100);
  app.log_watcher.register_recipient(log_tx).await;

  let (task_tx, mut task_rx) = mpsc::unbounded_channel::<CompensatorTask>();

  tokio::spawn(async move {
    while let Some(task) = task_rx.recv().await {
      if let Err(e) = submit_new_compensator_task(app.clone(), task).await {
        log::warn!("Compensator task failed: {}", e);
      }
    }
  });

  while let Some(line) = log_rx.recv().await {}

  Ok(())
}
