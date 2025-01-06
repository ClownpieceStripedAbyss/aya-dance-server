use std::ffi::CString;
use anyhow::anyhow;
use rsmpeg::{avformat::{AVFormatContextInput, AVFormatContextOutput}, avutil, ffi};

pub fn ffmpeg_audio_compensation(input_file: &str, output_file: &str, audio_offset: f64) -> anyhow::Result<()> {
  let input_file = CString::new(input_file)?;
  let output_file = CString::new(output_file)?;

  // Open input file
  let mut input_ctx = AVFormatContextInput::open(&input_file, None, &mut None)?;

  // Find video and audio streams
  let video_in_stream_index = input_ctx
    .streams()
    .iter()
    .position(|stream| stream.codecpar().codec_type == rsmpeg::ffi::AVMEDIA_TYPE_VIDEO)
    .ok_or_else(|| anyhow!("No video stream found"))?;
  let audio_in_stream_index = input_ctx
    .streams()
    .iter()
    .position(|stream| stream.codecpar().codec_type == rsmpeg::ffi::AVMEDIA_TYPE_AUDIO)
    .ok_or_else(|| anyhow!("No audio stream found"))?;

  let video_in_stream = &input_ctx.streams()[video_in_stream_index];
  let audio_in_stream = &input_ctx.streams()[audio_in_stream_index];

  // Create output context with in-memory IO
  let mut output_ctx = AVFormatContextOutput::create(&output_file, None)?;

  // Add video stream to output
  {
    let mut video_out_stream = output_ctx.new_stream();
    video_out_stream.set_time_base(video_in_stream.time_base);
    video_out_stream.set_codecpar(video_in_stream.codecpar().clone());
  }

  // Add audio stream to output
  {
    let mut audio_out_stream = output_ctx.new_stream();
    audio_out_stream.set_time_base(audio_in_stream.time_base);
    audio_out_stream.set_codecpar(audio_in_stream.codecpar().clone());
    
    // audio_out_stream.set_codecpar(unsafe {
    //   // let mut c = AVCodecParameters::from_raw(NonNull::new(audio_in_stream.codecpar).unwrap());
    //   // c.deref_mut().codec_id = ffi::AV_CODEC_ID_AAC;
    //   // c.deref_mut().codec_tag = 0;
    //   // c
    //   AVCodecParameters::from_raw(NonNull::new(audio_in_stream.codecpar).unwrap()).clone()
    // }); 
  }

  // Open output file
  output_ctx.write_header(&mut None)?;

  // Read packets from input and write to output
  while let Some(mut packet) = input_ctx.read_packet()? {
    let stream_index = packet.stream_index as usize;
    let out_stream_time_base;
    let out_stream_index;
    let in_stream = &input_ctx.streams()[stream_index];

    if stream_index == video_in_stream_index {
      let x = output_ctx.streams().iter().find(|s| s.codecpar().codec_type == rsmpeg::ffi::AVMEDIA_TYPE_VIDEO).unwrap();
      out_stream_time_base = x.time_base;
      out_stream_index = x.index;
    } else if stream_index == audio_in_stream_index {
      let x = output_ctx.streams().iter().find(|s| s.codecpar().codec_type == rsmpeg::ffi::AVMEDIA_TYPE_AUDIO).unwrap();
      out_stream_time_base = x.time_base;
      out_stream_index = x.index;
      
      packet.set_pts(packet.pts + avutil::av_rescale_q(
        (audio_offset * ffi::AV_TIME_BASE as f64) as i64,
        ffi::AV_TIME_BASE_Q,
        out_stream_time_base,
      ));
      packet.set_dts(packet.dts + avutil::av_rescale_q(
        (audio_offset * ffi::AV_TIME_BASE as f64) as i64,
        ffi::AV_TIME_BASE_Q,
        out_stream_time_base,
      ));
    } else {
      continue;
    }

    packet.set_pts(avutil::av_rescale_q_rnd(
      packet.pts,
      in_stream.time_base,
      out_stream_time_base,
      ffi::AV_ROUND_NEAR_INF as u32,
    ));
    packet.set_dts(avutil::av_rescale_q_rnd(
      packet.dts,
      in_stream.time_base,
      out_stream_time_base,
      ffi::AV_ROUND_NEAR_INF as u32,
    ));
    packet.set_duration(avutil::av_rescale_q(
      packet.duration,
      in_stream.time_base,
      out_stream_time_base,
    ));
    packet.set_stream_index(out_stream_index as i32);

    output_ctx.interleaved_write_frame(&mut packet)?;
  }

  // Write trailer
  output_ctx.write_trailer()?;

  Ok(())
}
