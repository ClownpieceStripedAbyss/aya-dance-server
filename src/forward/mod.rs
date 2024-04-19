mod async_stream;
mod copy_bidirectional;
mod location;
mod tcp;
mod tokio_util;

use std::{net::SocketAddr, sync::Arc};

use log::{debug, error, info};
use tcp::TargetData;
use tokio::net::TcpListener;

use crate::forward::{
    location::{Location, NetLocation},
    tcp::TargetLocationData,
};

pub async fn serve_l3_forward(listen: String, forward: String) -> anyhow::Result<()> {
    let socket = listen
        .parse::<SocketAddr>()
        .expect("Failed to parse listen address");
    let location = Location::Address(
        NetLocation::try_from(forward.as_str()).expect("Failed to parse forward address"),
    );
    let target = Arc::new(TargetData {
        location_data: vec![TargetLocationData {
            location: location.clone(),
        }],
        next_address_index: Default::default(),
        tcp_nodelay: false,
    });

    info!("L3 forward {} -> {}", socket, forward);

    loop {
        // Currently no QUIC support, we only support TCP
        if let Err(e) = listen_tcp(socket, target.clone(), location.clone()).await {
            error!("L3 Forward exited with error, restarting\n{:?}", e);
        } else {
            debug!("L3 Forward exited unexpectedly, restarting...");
        }
    }
}

async fn listen_tcp(
    socket: SocketAddr,
    forward: Arc<TargetData>,
    location: Location,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(socket).await?;

    loop {
        let (stream, client) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                error!("L3 Accept failed: {:?}", e);
                continue;
            }
        };

        debug!("L3 {:?} -> {}", &client, &location);

        let forward = forward.clone();

        tokio::spawn(async move {
            if let Err(e) = tcp::process_generic_stream(Box::new(stream), &client, forward).await {
                debug!("L3 TCP forward for {:?} exited: {:?}", &client, e);
            }
        });
    }
}
