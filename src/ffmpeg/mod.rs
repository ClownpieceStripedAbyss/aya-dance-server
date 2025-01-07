use std::ffi::CString;

use anyhow::anyhow;
use rsmpeg::{
  avcodec::{AVCodec, AVCodecContext},
  avformat::{AVFormatContextInput, AVFormatContextOutput},
  ffi, UnsafeDerefMut,
};

// ffmpeg -i %input_file% -ss %audio_offset% -i %input_file% -map 0:v -map 1:a
// -c:v copy -c:a aac -async 1 %output_file%
pub fn ffmpeg_audio_compensation(
  input_file: &str,
  output_file: &str,
  audio_offset: f64,
) -> anyhow::Result<()> {
  let input_file = CString::new(input_file)?;
  let output_file = CString::new(output_file)?;

  // Open input video file
  let mut video_input_ctx = AVFormatContextInput::open(&input_file, None, &mut None)
    .map_err(|e| anyhow!("Could not open input video file: {}", e))?;
  // Open input audio file
  let mut audio_input_ctx = AVFormatContextInput::open(&input_file, None, &mut None)
    .map_err(|e| anyhow!("Could not open input audio file: {}", e))?;

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

  // Create output context with in-memory IO
  let mut output_ctx = AVFormatContextOutput::create(&output_file, None)?;

  // Add video stream to output
  {
    let video_in_stream = &video_input_ctx.streams()[video_in_stream_index];

    let mut video_out_stream = output_ctx.new_stream();
    video_out_stream.set_time_base(video_in_stream.time_base);
    video_out_stream.set_codecpar(video_in_stream.codecpar().clone());
    unsafe {
      video_out_stream.codecpar_mut().deref_mut().codec_tag = 0;
    }
  }

  // Add audio stream to output
  {
    let audio_in_stream = &audio_input_ctx.streams()[audio_in_stream_index];

    let mut audio_out_stream = output_ctx.new_stream();
    audio_out_stream.set_time_base(audio_in_stream.time_base);
    audio_out_stream.set_codecpar(audio_in_stream.codecpar().clone());
    unsafe {
      audio_out_stream.codecpar_mut().deref_mut().codec_id = ffi::AV_CODEC_ID_AAC;
      audio_out_stream.codecpar_mut().deref_mut().codec_tag = 0;
    }
  }

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

  // Open output file
  output_ctx
    .write_header(&mut None)
    .map_err(|e| anyhow!("Could not write output file header: {}", e))?;

  ///////////////////////////////////
  // VIDEO
  ///////////////////////////////////
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

  while let Some(mut pkt) = audio_input_ctx.read_packet()? {
    if pkt.stream_index as usize != audio_in_stream_index {
      continue;
    }

    // Send audio packet to decoder
    dec_audio_ctx
      .send_packet(Some(&mut pkt))
      .map_err(|e| anyhow!("Error sending audio packet to decoder: {}", e))?;
    while let Ok(mut dec_frame) = dec_audio_ctx.receive_frame() {
      // Set start_pts if it is the first frame we receive
      if start_pts == ffi::AV_NOPTS_VALUE {
        start_pts = dec_frame.pts;
      }
      // Shift pts
      dec_frame.set_pts(dec_frame.pts - start_pts);

      // Now encode it
      enc_audio_ctx
        .send_frame(Some(&mut dec_frame))
        .map_err(|e| anyhow!("Error sending audio frame to encoder: {}", e))?;
      while let Ok(mut enc_pkt) = enc_audio_ctx.receive_packet() {
        enc_pkt.set_stream_index(out_audio_steam_index);
        enc_pkt.rescale_ts(enc_audio_ctx.time_base, out_audio_stream_time_base);

        output_ctx.interleaved_write_frame(&mut enc_pkt)?;
      }
    }
  }

  // Flush audio decoder
  dec_audio_ctx
    .send_packet(None)
    .map_err(|e| anyhow!("Error flushing audio decoder: {}", e))?;
  while let Ok(mut dec_frame) = dec_audio_ctx.receive_frame() {
    // Set start_pts if it is the first frame we receive
    if start_pts == ffi::AV_NOPTS_VALUE {
      start_pts = dec_frame.pts;
    }
    // Shift pts
    dec_frame.set_pts(dec_frame.pts - start_pts);

    // Now encode it
    enc_audio_ctx
      .send_frame(Some(&mut dec_frame))
      .map_err(|e| anyhow!("Error sending audio frame to encoder: {}", e))?;
    while let Ok(mut enc_pkt) = enc_audio_ctx.receive_packet() {
      enc_pkt.set_stream_index(out_audio_steam_index);
      enc_pkt.rescale_ts(enc_audio_ctx.time_base, out_audio_stream_time_base);

      output_ctx.interleaved_write_frame(&mut enc_pkt)?;
    }
  }

  // Flush audio encoder
  enc_audio_ctx
    .send_frame(None)
    .map_err(|e| anyhow!("Error flushing audio encoder: {}", e))?;
  while let Ok(mut enc_pkt) = enc_audio_ctx.receive_packet() {
    enc_pkt.set_stream_index(out_audio_steam_index);
    enc_pkt.rescale_ts(enc_audio_ctx.time_base, out_audio_stream_time_base);

    output_ctx.interleaved_write_frame(&mut enc_pkt)?;
  }

  // Ok, we finally finished
  output_ctx.write_trailer()?;

  Ok(())
}
