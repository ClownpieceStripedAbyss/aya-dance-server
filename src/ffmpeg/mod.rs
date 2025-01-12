use std::{ffi::CString, ptr};

use anyhow::anyhow;
use rsmpeg::{
  avcodec::{AVCodec, AVCodecContext, AVCodecParameters, AVPacket},
  avformat::{AVFormatContextInput, AVFormatContextOutput, AVStreamMut, AVStreamRef},
  avutil::{AVDictionary, AVFrame, AVRational},
  error::RsmpegError,
  ffi,
  swresample::SwrContext,
  UnsafeDerefMut,
};

#[derive(Debug, Copy, Clone)]
pub struct AudioCompensationStatistics {
  pub video_copy_secs: f64,
  pub audio_decode_secs: f64,
  pub audio_encode_secs: f64,
  pub audio_resample_secs: f64,
}

// ffmpeg -i %input_file% -ss %audio_offset% -i %input_file% -map 0:v -map 1:a
// -c:v copy -c:a aac -async 1 %output_file%
pub fn ffmpeg_audio_compensation(
  input_file: &str,
  output_file: &str,
  audio_offset: f64,
) -> anyhow::Result<AudioCompensationStatistics> {
  let mut stats = AudioCompensationStatistics {
    video_copy_secs: 0.0,
    audio_decode_secs: 0.0,
    audio_encode_secs: 0.0,
    audio_resample_secs: 0.0,
  };

  let input_file = CString::new(input_file)?;
  let output_file = CString::new(output_file)?;

  // Open input video file
  let mut video_input_ctx = AVFormatContextInput::open(&input_file, None, &mut None)
    .map_err(|e| anyhow!("Could not open input video file: {}", e))?;
  // Open input audio file
  let mut audio_input_ctx = AVFormatContextInput::open(&input_file, None, &mut None)
    .map_err(|e| anyhow!("Could not open input audio file: {}", e))?;

  // Find video and audio streams
  let ((_, video_in_stream_index), (_, audio_in_stream_index)) =
    find_video_audio(&video_input_ctx, &audio_input_ctx)
      .map_err(|e| anyhow!("Could not find video and audio streams: {}", e))?;

  // Create output context with in-memory IO
  let mut output_ctx = AVFormatContextOutput::create(&output_file, None)?;

  // Add video stream to output
  new_stream(
    &video_input_ctx.streams()[video_in_stream_index],
    &mut output_ctx,
    None,
  );

  // Create audio decoder based on input audio stream
  let (_audio_decoder, mut audio_decoder_ctx, audio_in_timebase) = {
    let audio_in_stream = &audio_input_ctx.streams()[audio_in_stream_index];
    let audio_in_codecpar = audio_in_stream.codecpar();
    let audio_decoder = AVCodec::find_decoder(audio_in_codecpar.codec_id)
      .ok_or_else(|| anyhow!("Could not find audio decoder"))?;
    let mut decoder_ctx = AVCodecContext::new(&audio_decoder);
    decoder_ctx
      .apply_codecpar(&audio_in_codecpar)
      .map_err(|e| {
        anyhow!(
          "Could not apply codec parameters to audio decoder context: {}",
          e
        )
      })?;

    // https://stackoverflow.com/questions/25688313/how-to-use-ffmpeg-faststart-flag-programmatically
    if (output_ctx.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32) != 0 {
      decoder_ctx.set_flags(ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    }

    (audio_decoder, decoder_ctx, audio_in_stream.time_base)
  };

  // Create AAC encoder based for output audio stream
  let (_aac_encoder, mut aac_encoder_ctx) = {
    let audio_in_stream = &audio_input_ctx.streams()[audio_in_stream_index];
    let audio_in_codecpar = audio_in_stream.codecpar();
    if audio_in_codecpar.codec_id != ffi::AV_CODEC_ID_AAC {
      return Err(anyhow!("Input audio stream is not in AAC format"));
    }

    let aac_encoder = AVCodec::find_encoder(ffi::AV_CODEC_ID_AAC)
      .ok_or_else(|| anyhow!("Could not find AAC encoder"))?;
    let mut aac_ctx = AVCodecContext::new(&aac_encoder);

    aac_ctx.set_ch_layout(audio_in_codecpar.ch_layout);
    aac_ctx.set_sample_rate(audio_in_codecpar.sample_rate);
    aac_ctx.set_sample_fmt(
      aac_encoder
        .sample_fmts()
        .unwrap_or(&[ffi::AV_SAMPLE_FMT_FLTP])[0],
    );
    aac_ctx.set_bit_rate(audio_in_codecpar.bit_rate);
    // aac_ctx.apply_codecpar(&audio_in_codecpar).map_err(|e| {
    //   anyhow!(
    //     "Could not apply codec parameters to AAC encoder context: {}",
    //     e
    //   )
    // })?;

    // https://stackoverflow.com/questions/25688313/how-to-use-ffmpeg-faststart-flag-programmatically
    if (output_ctx.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32) != 0 {
      log::debug!("Setting global header flag for AAC encoder");
      aac_ctx.set_flags(ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    }

    (aac_encoder, aac_ctx)
  };

  // Open audio decoder
  audio_decoder_ctx
    .open(None)
    .map_err(|e| anyhow!("Could not open audio decoder: {}", e))?;
  let mut dec_audio_ctx = audio_decoder_ctx;

  // Open AAC encoder
  aac_encoder_ctx
    .open(None)
    .map_err(|e| anyhow!("Could not open AAC encoder: {}", e))?;
  let mut enc_audio_ctx = aac_encoder_ctx;

  // Create resampler context when nb_samples > frame_size
  let mut swr_ctx = {
    let in_ch_layout = dec_audio_ctx.ch_layout();
    let in_sample_fmt = dec_audio_ctx.sample_fmt;
    let in_sample_rate = dec_audio_ctx.sample_rate;
    let out_ch_layout = enc_audio_ctx.ch_layout();
    let out_sample_fmt = enc_audio_ctx.sample_fmt;
    let out_sample_rate = enc_audio_ctx.sample_rate;

    let mut swr_ctx = SwrContext::new(
      &out_ch_layout,
      out_sample_fmt,
      out_sample_rate,
      &in_ch_layout,
      in_sample_fmt,
      in_sample_rate,
    )
    .map_err(|e| anyhow!("Could not create SwrContext: {}", e))?;
    swr_ctx
      .init()
      .map_err(|e| anyhow!("Could not initialize SwrContext: {}", e))?;
    swr_ctx
  };

  // Add audio stream to output
  new_stream(
    &audio_input_ctx.streams()[audio_in_stream_index],
    &mut output_ctx,
    Some(enc_audio_ctx.extract_codecpar()),
  );

  // Set faststart flag for HTTP progressive download
  let muxer_opts = AVDictionary::new(&CString::new("movflags")?, &CString::new("+faststart")?, 0);

  // Open output file
  output_ctx
    .write_header(&mut Some(muxer_opts))
    .map_err(|e| anyhow!("Could not write output file header: {}", e))?;

  ///////////////////////////////////
  // VIDEO
  ///////////////////////////////////
  let stat_start = std::time::Instant::now();

  while let Some(mut pkt) = video_input_ctx.read_packet()? {
    if pkt.stream_index as usize != video_in_stream_index {
      continue;
    }
    let in_stream = &video_input_ctx.streams()[pkt.stream_index as usize];
    let out_video_stream = output_ctx
      .streams()
      .iter()
      .find(|s| s.codecpar().codec_type == rsmpeg::ffi::AVMEDIA_TYPE_VIDEO)
      .unwrap();

    pkt.set_stream_index(out_video_stream.index as i32);
    pkt.rescale_ts(in_stream.time_base, out_video_stream.time_base);
    pkt.set_pos(-1);
    output_ctx.interleaved_write_frame(&mut pkt)?;
  }

  stats.video_copy_secs = stat_start.elapsed().as_secs_f64();

  ///////////////////////////////////
  // AUDIO
  ///////////////////////////////////

  unsafe {
    // Seek audio stream to audio_offset
    let ts = audio_offset / ffi::av_q2d(audio_in_timebase);
    ffi::av_seek_frame(
      audio_input_ctx.as_mut_ptr(),
      audio_in_stream_index as i32,
      ts as i64,
      ffi::AVSEEK_FLAG_ANY as i32,
    );
  }

  let (out_audio_steam_index, out_audio_stream_time_base) = {
    let out_audio_stream = output_ctx
      .streams()
      .iter()
      .find(|s| s.codecpar().codec_type == rsmpeg::ffi::AVMEDIA_TYPE_AUDIO)
      .unwrap();
    (out_audio_stream.index, out_audio_stream.time_base)
  };

  let mut start_pts = ffi::AV_NOPTS_VALUE;

  while let Some(pkt) = audio_input_ctx.read_packet()? {
    if pkt.stream_index as usize != audio_in_stream_index {
      continue;
    }

    decode_packet_and_encode_frame_with_offset(
      Some(&pkt),
      &mut output_ctx,
      &mut dec_audio_ctx,
      &mut enc_audio_ctx,
      &mut swr_ctx,
      &mut stats,
      out_audio_steam_index,
      out_audio_stream_time_base,
      &mut start_pts,
    )
    .map_err(|e| anyhow!("Error re-encoding audio packet: {}", e))?;
  }

  // Flush audio decoder
  decode_packet_and_encode_frame_with_offset(
    None,
    &mut output_ctx,
    &mut dec_audio_ctx,
    &mut enc_audio_ctx,
    &mut swr_ctx,
    &mut stats,
    out_audio_steam_index,
    out_audio_stream_time_base,
    &mut start_pts,
  )
  .map_err(|e| anyhow!("Error flushing audio decoder: {}", e))?;

  // Flush audio encoder
  encode_frame_and_write_to_output(
    None,
    &mut output_ctx,
    &mut enc_audio_ctx,
    &mut stats,
    out_audio_steam_index,
    out_audio_stream_time_base,
  )
  .map_err(|e| anyhow!("Error flushing audio encoder: {}", e))?;

  // Ok, we finally finished
  output_ctx.write_trailer()?;

  Ok(stats)
}

fn decode_packet_and_encode_frame_with_offset(
  pkt: Option<&AVPacket>,
  mut output_ctx: &mut AVFormatContextOutput,
  dec_audio_ctx: &mut AVCodecContext,
  mut enc_audio_ctx: &mut AVCodecContext,
  swr_ctx: &mut SwrContext,
  stats: &mut AudioCompensationStatistics,
  out_audio_steam_index: i32,
  out_audio_stream_time_base: AVRational,
  start_pts: &mut i64,
) -> anyhow::Result<()> {
  let decode_start = std::time::Instant::now();
  // Send audio packet to decoder
  dec_audio_ctx
    .send_packet(pkt)
    .map_err(|e| anyhow!("Error sending audio packet to decoder: {}", e))?;
  while let Ok(mut dec_frame) = dec_audio_ctx.receive_frame() {
    stats.audio_decode_secs += decode_start.elapsed().as_secs_f64();

    // Set start_pts if it is the first frame we receive
    if *start_pts == ffi::AV_NOPTS_VALUE {
      *start_pts = dec_frame.pts;
    }

    // Resample audio frame if needed to avoid
    // [aac @ 000001B6FE889140] nb_samples (2048) > frame_size (1024)
    if dec_frame.nb_samples > enc_audio_ctx.frame_size {
      let resample_start = std::time::Instant::now();

      // rsmpeg's convert_frame must be called with an output, but we are converting
      // nb_samples from 2048 to 1024, so we must give a null output.
      let ret = unsafe {
        ffi::swr_convert_frame(
          swr_ctx.as_ptr() as *mut _,
          ptr::null_mut(),
          dec_frame.as_ptr(),
        )
      };
      if ret < 0 {
        return Err(anyhow!(RsmpegError::from(ret)));
      }

      stats.audio_resample_secs += resample_start.elapsed().as_secs_f64();

      let mut last_frame_pts = dec_frame.pts;
      let mut increased_pts = 1;
      loop {
        let resample_start = std::time::Instant::now();

        let mut converted_frame = AVFrame::new();
        converted_frame.set_ch_layout(enc_audio_ctx.ch_layout().clone().into_inner());
        converted_frame.set_format(enc_audio_ctx.sample_fmt);
        converted_frame.set_sample_rate(enc_audio_ctx.sample_rate);
        converted_frame.set_pts(dec_frame.pts);
        converted_frame.set_nb_samples(enc_audio_ctx.frame_size);
        converted_frame
          .alloc_buffer()
          .map_err(|e| anyhow!("Error allocating buffer for resampled audio frame: {}", e))?;

        swr_ctx
          .convert_frame(None, &mut converted_frame)
          .map_err(|e| anyhow!("Error resampling audio frame: {}", e))?;

        // No more samples, break for next decoded frame
        if converted_frame.nb_samples == 0 {
          break;
        }

        // theoretically this should not happen, but just in case
        if converted_frame.nb_samples > enc_audio_ctx.frame_size {
          return Err(anyhow!(
            "Resampled frame still has more samples ({}) than encoder frame size ({})?",
            converted_frame.nb_samples,
            enc_audio_ctx.frame_size
          ));
        }

        // A frame may be resampled to multiple frames, and ffmpeg encoder requires
        // the pts to be monotonically increasing, so we must increase the pts for each
        // resampled frame.
        if converted_frame.pts == last_frame_pts {
          converted_frame.set_pts(converted_frame.pts + increased_pts);
          increased_pts += 1;
        } else {
          last_frame_pts = converted_frame.pts;
          increased_pts = 1;
        }

        // Shift pts
        converted_frame.set_pts(converted_frame.pts - *start_pts);

        stats.audio_resample_secs += resample_start.elapsed().as_secs_f64();

        encode_frame_and_write_to_output(
          Some(&converted_frame),
          &mut output_ctx,
          &mut enc_audio_ctx,
          stats,
          out_audio_steam_index,
          out_audio_stream_time_base,
        )
        .map_err(|e| anyhow!("Error resampling+encoding and writing audio frame: {}", e))?;
      }
    } else {
      // No need to resample, shift pts and encode the frame
      dec_frame.set_pts(dec_frame.pts - *start_pts);
      encode_frame_and_write_to_output(
        Some(&dec_frame),
        &mut output_ctx,
        &mut enc_audio_ctx,
        stats,
        out_audio_steam_index,
        out_audio_stream_time_base,
      )
      .map_err(|e| anyhow!("Error encoding and writing audio frame: {}", e))?;
    }
  }
  Ok(())
}

fn encode_frame_and_write_to_output(
  frame: Option<&AVFrame>,
  output_ctx: &mut AVFormatContextOutput,
  enc_audio_ctx: &mut AVCodecContext,
  stats: &mut AudioCompensationStatistics,
  out_audio_steam_index: i32,
  out_audio_stream_time_base: AVRational,
) -> anyhow::Result<()> {
  let encode_start = std::time::Instant::now();

  enc_audio_ctx
    .send_frame(frame)
    .map_err(|e| anyhow!("Error sending frame to encoder: {}", e))?;
  while let Ok(mut enc_pkt) = enc_audio_ctx.receive_packet() {
    stats.audio_encode_secs += encode_start.elapsed().as_secs_f64();

    enc_pkt.set_stream_index(out_audio_steam_index);
    enc_pkt.rescale_ts(enc_audio_ctx.time_base, out_audio_stream_time_base);
    enc_pkt.set_pos(-1);

    output_ctx
      .interleaved_write_frame(&mut enc_pkt)
      .map_err(|e| {
        anyhow!(
          "Error writing audio packet with interleaved_write_frame: {}",
          e
        )
      })?;
  }
  Ok(())
}

fn find_video_audio<'a>(
  video_input_ctx: &'a AVFormatContextInput,
  audio_input_ctx: &'a AVFormatContextInput,
) -> anyhow::Result<((&'a AVStreamRef<'a>, usize), (&'a AVStreamRef<'a>, usize))> {
  // Find video and audio streams
  let video_in_stream_index = video_input_ctx
    .streams()
    .iter()
    .position(|stream| stream.codecpar().codec_type == rsmpeg::ffi::AVMEDIA_TYPE_VIDEO)
    .ok_or_else(|| anyhow!("No video stream found"))?;
  let audio_in_stream_index = audio_input_ctx
    .streams()
    .iter()
    .position(|stream| stream.codecpar().codec_type == rsmpeg::ffi::AVMEDIA_TYPE_AUDIO)
    .ok_or_else(|| anyhow!("No audio stream found"))?;

  let video_in_stream = &video_input_ctx.streams()[video_in_stream_index];
  let audio_in_stream = &audio_input_ctx.streams()[audio_in_stream_index];
  Ok((
    (video_in_stream, video_in_stream_index),
    (audio_in_stream, audio_in_stream_index),
  ))
}

fn new_stream<'a>(
  in_stream: &AVStreamRef,
  output_ctx: &'a mut AVFormatContextOutput,
  codecpar: Option<AVCodecParameters>,
) -> AVStreamMut<'a> {
  let mut out_stream = output_ctx.new_stream();

  out_stream.set_time_base(in_stream.time_base);
  out_stream.set_codecpar(codecpar.unwrap_or_else(|| in_stream.codecpar().clone()));
  unsafe {
    out_stream.codecpar_mut().deref_mut().codec_tag = 0;
  }
  out_stream
}

pub fn ffmpeg_copy(input_file: &str, output_file: &str) -> anyhow::Result<()> {
  let input_file = CString::new(input_file)?;
  let output_file = CString::new(output_file)?;

  // Open input file
  let mut input_ctx = AVFormatContextInput::open(&input_file, None, &mut None)?;

  // Find video and audio streams
  let ((video_in_stream, video_in_stream_index), (audio_in_stream, audio_in_stream_index)) =
    find_video_audio(&input_ctx, &input_ctx)
      .map_err(|e| anyhow!("Could not find video and audio streams: {}", e))?;

  // Create output context with in-memory IO
  let mut output_ctx = AVFormatContextOutput::create(&output_file, None)?;

  // Add video stream to output
  new_stream(video_in_stream, &mut output_ctx, None);
  // Add audio stream to output
  new_stream(audio_in_stream, &mut output_ctx, None);

  // Open output file
  output_ctx.write_header(&mut None)?;

  // Read packets from input and write to output
  while let Some(mut packet) = input_ctx.read_packet()? {
    let stream_index = packet.stream_index as usize;
    let out_stream_time_base;
    let out_stream_index;
    let in_stream = &input_ctx.streams()[stream_index];

    if stream_index == video_in_stream_index {
      let x = output_ctx
        .streams()
        .iter()
        .find(|s| s.codecpar().codec_type == rsmpeg::ffi::AVMEDIA_TYPE_VIDEO)
        .unwrap();
      out_stream_time_base = x.time_base;
      out_stream_index = x.index;
    } else if stream_index == audio_in_stream_index {
      let x = output_ctx
        .streams()
        .iter()
        .find(|s| s.codecpar().codec_type == rsmpeg::ffi::AVMEDIA_TYPE_AUDIO)
        .unwrap();
      out_stream_time_base = x.time_base;
      out_stream_index = x.index;
    } else {
      continue;
    }

    packet.set_stream_index(out_stream_index as i32);
    packet.rescale_ts(in_stream.time_base, out_stream_time_base);
    packet.set_pos(-1);
    output_ctx.interleaved_write_frame(&mut packet)?;
  }

  // Write trailer
  output_ctx.write_trailer()?;

  Ok(())
}
