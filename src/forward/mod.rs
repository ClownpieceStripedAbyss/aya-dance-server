mod async_stream;
mod copy_bidirectional;
mod location;
mod sni;
mod tcp;
pub mod tokio_util;

use std::{net::SocketAddr, sync::Arc};

use log::{debug, error, info};
use tcp::TargetData;
use tokio::net::TcpListener;

use crate::forward::{
  location::{Location, NetLocation},
  tcp::TargetLocationData,
};

pub async fn serve_l4_forward(
  listen: String,
  forward_jd: String,
  forward_ud: String,
) -> anyhow::Result<()> {
  let socket = listen
    .parse::<SocketAddr>()
    .expect("Failed to parse listen address");
  let (_, target_jd) = to_location(&forward_jd);
  let (_, target_ud) = to_location(&forward_ud);

  let mut host_mappings = std::collections::HashMap::new();
  host_mappings.insert("jd.pypy.moe".to_string(), (forward_jd, target_jd));
  host_mappings.insert("api.udon.dance".to_string(), (forward_ud, target_ud));
  let sni_map = Arc::new(sni::SniMap { host_mappings });
  
  for (host, (forward, _)) in &sni_map.host_mappings {
    info!(
      "L4 SNI proxy {} {} -> {}",
      socket, host, forward
    );
  }

  loop {
    // Currently no QUIC support, we only support TCP
    if let Err(e) = listen_tcp(socket, sni_map.clone()).await {
      error!("L4 Forward exited with error, restarting\n{:?}", e);
    } else {
      debug!("L4 Forward exited unexpectedly, restarting...");
    }
  }
}

fn to_location(forward_jd: &String) -> (Location, Arc<TargetData>) {
  let location_jd = Location::Address(
    NetLocation::try_from(forward_jd.as_str()).expect("Failed to parse forward address"),
  );
  let target_jd = Arc::new(TargetData {
    location_data: vec![TargetLocationData {
      location: location_jd.clone(),
    }],
    next_address_index: Default::default(),
    tcp_nodelay: false,
  });
  (location_jd, target_jd)
}

async fn listen_tcp(socket: SocketAddr, sni_map: Arc<sni::SniMap>) -> anyhow::Result<()> {
  let listener = TcpListener::bind(socket).await?;

  loop {
    let (stream, client) = match listener.accept().await {
      Ok(v) => v,
      Err(e) => {
        error!("L4 Accept failed: {:?}", e);
        continue;
      }
    };

    let sni_map = sni_map.clone();
    tokio::spawn(async move {
      if let Err(e) = sni::sni_proxy(sni_map, stream, client).await {
        debug!("L4 TCP forward for {:?} exited: {:?}", &client, e);
      }
    });
  }
}
