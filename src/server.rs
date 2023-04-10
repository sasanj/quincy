use std::time::Duration;

use crate::config::ServerConfig;
use crate::server::tunnel::QuincyTunnel;
use anyhow::Result;
use dashmap::DashMap;
use tokio::time::sleep;
use tracing::error;

pub mod address_pool;
pub mod connection;
pub mod tunnel;

/// Represents a Quincy server with multiple underlying Quincy tunnels.
pub struct QuincyServer {
    active_tunnels: DashMap<String, QuincyTunnel>,
}

impl QuincyServer {
    /// Creates a new instance of a Quincy server.
    ///
    /// ### Arguments
    /// - `config` - the configuration for the server
    pub async fn new(config: ServerConfig) -> Result<Self> {
        let tunnels = DashMap::new();

        for (name, tunnel_config) in config.tunnels.iter() {
            let tunnel = QuincyTunnel::new(tunnel_config.clone(), &config.connection)?;

            tunnels.insert(name.clone(), tunnel);
        }

        Ok(Self {
            active_tunnels: tunnels,
        })
    }

    /// Starts the Quincy server and all of its underlying tunnels.
    pub async fn run(&self) -> Result<()> {
        for mut entry in self.active_tunnels.iter_mut() {
            let tunnel = entry.value_mut();

            tunnel.start().await?;
        }

        loop {
            for mut entry in self.active_tunnels.iter_mut() {
                let tunnel_name = entry.key().to_owned();
                let tunnel = entry.value_mut();

                if tunnel.is_ok() {
                    continue;
                }

                error!("Tunnel '{tunnel_name}' has crashed. Attempting to restart...");
                tunnel.stop().await?;
                tunnel.start().await?;
            }

            sleep(Duration::from_secs(1)).await;
        }
    }
}
