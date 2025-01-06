use std::{ffi::CString, ptr, ptr::NonNull};
use std::mem::ManuallyDrop;
use anyhow::anyhow;
use rsmpeg::{
  avcodec::AVCodecParameters,
  avformat::{AVFormatContextInput, AVFormatContextOutput, AVStreamMut},
  avutil, ffi,
};

pub fn ffmpeg_audio_test() -> anyhow::Result<()> {
  let input_file = CString::new("input.mp4")?;
  let output_file = CString::new("output.mp4")?;
  let audio_offset = -0.16667; // Audio delay in seconds

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

  let video_in_stream = input_ctx.streams()[video_in_stream_index].clone();
  let audio_in_stream = input_ctx.streams()[audio_in_stream_index].clone();

  // Create output context
  let mut output_ctx = AVFormatContextOutput::create(&output_file, None)?;

  // Add video stream to output
  let mut video_out_stream = unsafe {
    let new_stream = NonNull::new(ffi::avformat_new_stream(
      output_ctx.as_mut_ptr(),
      ptr::null(),
    ))
    .ok_or_else(|| anyhow!("Failed to call avformat_new_stream"))?;
    AVStreamMut::from_raw(new_stream)
  };
  video_out_stream.set_time_base(video_in_stream.time_base);
  video_out_stream.set_codecpar(unsafe {
    AVCodecParameters::from_raw(NonNull::new(video_in_stream.codecpar).unwrap())
  });

  // Add video stream to output, NOTE: wrap it in ManuallyDrop to prevent drop,
  // since the video_out_stream will drop the same pointer, which will cause double free.
  let mut audio_out_stream = ManuallyDrop::new(unsafe {
    let new_stream = NonNull::new(ffi::avformat_new_stream(
      output_ctx.as_mut_ptr(),
      ptr::null(),
    ))
    .ok_or_else(|| anyhow!("Failed to call avformat_new_stream"))?;
    AVStreamMut::from_raw(new_stream)
  });
  audio_out_stream.set_time_base(audio_in_stream.time_base);
  audio_out_stream.set_codecpar(unsafe {
    AVCodecParameters::from_raw(NonNull::new(audio_in_stream.codecpar).unwrap())
  });

  // Open output file
  output_ctx.write_header(&mut None)?;

  // Read packets from input and write to output
  while let Some(mut packet) = input_ctx.read_packet()? {
    let stream_index = packet.stream_index as usize;

    if stream_index == video_in_stream_index {
      // Video stream, copy packet as is
      packet.set_stream_index(video_out_stream.index as i32);
      output_ctx.write_frame(&mut packet)?;
    } else if stream_index == audio_in_stream_index {
      // Audio stream, adjust timestamp for offset
      packet.set_pts(avutil::av_rescale_q(
        packet.pts + (audio_offset * time_base_to_double(audio_in_stream.time_base)) as i64,
        audio_in_stream.time_base,
        audio_out_stream.time_base,
      ));
      packet.set_dts(avutil::av_rescale_q(
        packet.dts + (audio_offset * time_base_to_double(audio_in_stream.time_base)) as i64,
        audio_in_stream.time_base,
        audio_out_stream.time_base,
      ));
      
      packet.set_stream_index(audio_out_stream.index as i32);
      output_ctx.write_frame(&mut packet)?;
    }
  }

  // Write trailer
  output_ctx.write_trailer()?;

  Ok(())
}

fn time_base_to_double(time_base: avutil::AVRational) -> f64 {
  time_base.num as f64 / time_base.den as f64
}
