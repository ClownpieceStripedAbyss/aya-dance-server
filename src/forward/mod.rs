mod async_stream;
mod copy_bidirectional;
mod location;
mod sni;
mod tcp;
pub mod tokio_util;

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use log::{debug, error, info};
use tcp::TargetData;
use tokio::net::TcpListener;

use crate::forward::{
  location::{Location, NetLocation},
  tcp::TargetLocationData,
};

pub async fn serve_sni_proxy(
  listen: String,
  proxy_targets: HashMap<String, String>,
) -> anyhow::Result<()> {
  let socket = listen
    .parse::<SocketAddr>()
    .expect("Failed to parse listen address");

  let mut host_mappings = HashMap::new();
  for (host, forward_target) in proxy_targets {
    let (_, target_location) = to_location(&forward_target);
    host_mappings.insert(host, (forward_target, target_location));
  }
  let sni_map = Arc::new(sni::SniMap { host_mappings });

  for (host, (forward, _)) in &sni_map.host_mappings {
    info!("L4 SNI proxy {} {} -> {}", socket, host, forward);
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

fn to_location(forward_target: &String) -> (Location, Arc<TargetData>) {
  let location_jd = Location::Address(
    NetLocation::try_from(forward_target.as_str()).expect("Failed to parse forward address"),
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
