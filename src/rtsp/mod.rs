use std::{net::SocketAddr, sync::Arc};

use anyhow::bail;
use log::{debug, error, info};
use rtsp_types::{Empty, Message, Method, Response};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter}, join, net::{TcpListener, TcpStream}};

#[derive(Debug)]
struct WorkerCtxImpl {}

type WorkerCtx = Arc<WorkerCtxImpl>;

pub async fn serve_rtsp_typewriter(listen: String) -> anyhow::Result<()> {
  let socket = listen
    .parse::<SocketAddr>()
    .expect("Failed to parse listen address");

  info!("RTSP typewriter listening on rtsp://{}", socket);

  let ctx = Arc::new(WorkerCtxImpl {});

  loop {
    if let Err(e) = listen_tcp(socket, ctx.clone()).await {
      error!("RTSP typewriter exited with error, restarting\n{:?}", e);
    } else {
      debug!("RTSP typewriter exited unexpectedly, restarting...");
    }
  }
}

async fn listen_tcp(socket: SocketAddr, ctx: WorkerCtx) -> anyhow::Result<()> {
  let listener = TcpListener::bind(socket).await?;

  loop {
    let (stream, client) = match listener.accept().await {
      Ok(v) => v,
      Err(e) => {
        error!("RTSP Accept failed: {:?}", e);
        continue;
      }
    };

    debug!("RTSP Connection from: {}", &client);

    let ctx = ctx.clone();
    tokio::spawn(async move {
      if let Err(e) = handle_client(stream, client, ctx).await {
        debug!("RTSP Client {} exited: {:?}", client, e);
      }
    });
  }
}

async fn handle_client(
  mut stream: TcpStream,
  client: SocketAddr,
  ctx: WorkerCtx,
) -> anyhow::Result<()> {
  let (rx, tx) = stream.split();
  let mut reader = BufReader::new(rx);
  let mut writer = BufWriter::new(tx);
  let mut buf = String::new();

  loop {
    match reader.read_line(&mut buf).await {
      Ok(0) => {
        debug!("RTSP Client {} disconnected", client);
        return Ok(());
      }
      Err(e) => {
        bail!("read error: {:?}", e);
      }
      // If the buffer contains a double CRLF, then we have a complete message
      Ok(_) if buf.contains("\r\n\r\n") => {
        let response = handle_rtsp_message(ctx.clone(), client, &buf).await?;
        let mut reply = Vec::new();
        response
          .write(&mut reply)
          .map_err(|e| anyhow::anyhow!("failed to serialize response: {:?}", e))?;
        writer
          .write_all(&reply)
          .await
          .map_err(|e| anyhow::anyhow!("failed to write response: {:?}", e))?;
        buf.clear();
      }
      // Not enough data to parse a message, continue reading
      Ok(_) => (),
    }
  }
}

async fn handle_rtsp_message(
  ctx: WorkerCtx,
  client: SocketAddr,
  raw: &String,
) -> anyhow::Result<Response<Empty>> {
  let (message, consumed): (Message<Vec<u8>>, _) = Message::parse(raw.as_bytes())?;
  if consumed != raw.len() {
    bail!("failed to consume entire buffer {}", raw);
  }

  match message {
    Message::Request(request) => {
      let method = request.method();
      let url = request
        .request_uri()
        .ok_or_else(|| anyhow::anyhow!("missing request uri"))?;
      let path = url
        .path_segments()
        .ok_or_else(|| anyhow::anyhow!("missing path segments"))?
        .collect::<Vec<&str>>();
      let cseq = request
        .header(&rtsp_types::headers::CSEQ)
        .ok_or_else(|| anyhow::anyhow!("missing CSeq"))?;

      match (method, path.as_slice()) {
        (Method::Describe, ["typewriter", letter]) => {
          info!("RTSP Client {} typewriter: {}", client, letter);
        }
        _ => (),
      }

      Ok(
        rtsp_types::Response::builder(rtsp_types::Version::V2_0, rtsp_types::StatusCode::Ok)
          .header(rtsp_types::headers::CSEQ, cseq.clone())
          .empty(),
      )
    }

    Message::Response(_) => bail!("client sent a response, funny"),
    Message::Data(_) => bail!("client sent some data, funny"),
  }
}
