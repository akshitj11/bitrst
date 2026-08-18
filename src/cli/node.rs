//! `node` subcommand — run a P2P peer manager on an ephemeral chain.

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use bitrst_net::constants::{DEFAULT_MAX_INBOUND, DEFAULT_MAX_OUTBOUND};
use bitrst_net::{PeerManager, PeerManagerConfig, SeedStrategy};
use clap::Args;

use super::args::{resolve_network_time, NetworkArg};
use super::chain::{ephemeral_chain, DEFAULT_BITS, EPHEMERAL_NOTICE, GENESIS_TIME};
use super::error::CliError;
use super::shutdown;

/// Arguments for `bitrst node`.
#[derive(Debug, Args)]
pub struct NodeArgs {
    /// Address to bind for inbound P2P connections (`0` requests an ephemeral port).
    #[arg(long, default_value = "127.0.0.1:8333")]
    pub listen: SocketAddr,

    /// Bitcoin network (magic bytes and seed defaults).
    #[arg(long, value_enum, default_value_t = NetworkArg::Mainnet)]
    pub network: NetworkArg,

    /// Maximum inbound peer connections.
    #[arg(long, default_value_t = DEFAULT_MAX_INBOUND)]
    pub max_inbound: usize,

    /// Maximum outbound peer connections.
    #[arg(long, default_value_t = DEFAULT_MAX_OUTBOUND)]
    pub max_outbound: usize,

    /// Optional explicit seed addresses (repeatable). When omitted, built-in loopback seeds are used.
    #[arg(long = "seed")]
    pub seeds: Vec<SocketAddr>,

    /// Network-adjusted unix time for chain validation (defaults to current time).
    #[arg(long)]
    pub network_time: Option<u32>,

    /// Skip outbound seed connections (useful for isolated local testing).
    #[arg(long)]
    pub no_connect_seeds: bool,
}

/// Configuration assembled from CLI args for dependency injection in tests.
#[derive(Debug, Clone)]
pub struct NodeRunConfig {
    /// Listen address passed to the peer manager (port `0` binds ephemerally).
    pub listen: SocketAddr,
    /// Selected Bitcoin network.
    pub network: NetworkArg,
    /// Inbound peer limit.
    pub max_inbound: usize,
    /// Outbound peer limit.
    pub max_outbound: usize,
    /// Seed selection strategy.
    pub seed_strategy: SeedStrategy,
    /// Network-adjusted unix time for the ephemeral chain.
    pub network_time: u32,
    /// Whether to attempt outbound seed connections on startup.
    pub connect_seeds: bool,
}

impl NodeArgs {
    /// Converts parsed CLI args into a runnable node configuration.
    pub fn into_run_config(self) -> Result<NodeRunConfig, CliError> {
        let network_time = resolve_network_time(self.network_time)?;
        let seed_strategy = if self.seeds.is_empty() {
            SeedStrategy::builtin()
        } else {
            SeedStrategy::Fixed(self.seeds)
        };
        Ok(NodeRunConfig {
            listen: self.listen,
            network: self.network,
            max_inbound: self.max_inbound,
            max_outbound: self.max_outbound,
            seed_strategy,
            network_time,
            connect_seeds: !self.no_connect_seeds,
        })
    }
}

/// Runs the node until `shutdown` completes, then shuts down peers gracefully.
pub async fn run(
    config: NodeRunConfig,
    shutdown: impl Future<Output = ()>,
) -> Result<(), CliError> {
    let chain = ephemeral_chain(config.network_time.max(GENESIS_TIME), DEFAULT_BITS)?;
    let peer_config = PeerManagerConfig {
        network: config.network.p2p_network(),
        listen_addr: config.listen,
        max_inbound: config.max_inbound,
        max_outbound: config.max_outbound,
        seeds: config.seed_strategy,
    };

    let mut manager = PeerManager::new(chain, peer_config);
    manager.start_listener().await?;
    let listen = manager
        .listen_addr()
        .ok_or_else(|| CliError::Io("listener did not report bound address".to_string()))?;
    manager.spawn_acceptor();

    if config.connect_seeds {
        let report = manager.connect_seeds_report().await;
        for (addr, error) in &report.failures {
            eprintln!("seed connection failed for {addr}: {error}");
        }
        if let Some(addr) = report.connected {
            eprintln!("connected to seed {addr}");
        } else if !report.failures.is_empty() {
            return Err(CliError::Net(report.into_result().unwrap_err()));
        }
    }

    eprintln!("listening on {listen} ({})", config.network.label());
    eprintln!("{EPHEMERAL_NOTICE}");

    let poll_result = run_poll_loop(&mut manager, shutdown).await;
    manager.shutdown().await?;
    poll_result
}

/// Polls the peer manager until `shutdown` fires.
pub async fn run_poll_loop(
    manager: &mut PeerManager,
    shutdown: impl Future<Output = ()>,
) -> Result<(), CliError> {
    let mut shutdown = Box::pin(shutdown);
    let mut ticker = tokio::time::interval(Duration::from_millis(100));

    loop {
        tokio::select! {
            () = &mut shutdown => break,
            _ = ticker.tick() => {
                manager.poll().await?;
            }
        }
    }

    Ok(())
}

/// Default shutdown future for the node binary (Ctrl-C / SIGTERM).
pub async fn default_shutdown_signal() {
    shutdown::wait_for_shutdown_signal().await;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{run, NodeArgs, NodeRunConfig};
    use crate::cli::args::NetworkArg;
    use crate::cli::shutdown::ShutdownTrigger;
    use bitrst_net::SeedStrategy;

    #[test]
    fn node_args_into_config_preserves_listen() {
        let args = NodeArgs {
            listen: "127.0.0.1:18333".parse().expect("addr"),
            network: NetworkArg::Testnet,
            max_inbound: 4,
            max_outbound: 2,
            seeds: vec![],
            network_time: Some(1_231_006_505),
            no_connect_seeds: true,
        };
        let config = args.into_run_config().expect("config");
        assert_eq!(config.listen.port(), 18333);
        assert!(!config.connect_seeds);
    }

    #[tokio::test]
    async fn node_starts_and_shuts_down_on_signal() {
        let config = NodeRunConfig {
            listen: "127.0.0.1:0".parse().expect("addr"),
            network: NetworkArg::Testnet,
            max_inbound: 2,
            max_outbound: 0,
            seed_strategy: SeedStrategy::Fixed(vec![]),
            network_time: 1_231_006_505,
            connect_seeds: false,
        };

        let (trigger, wait) = ShutdownTrigger::pair();
        let node = run(config, wait);

        let trigger_for_task = trigger.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            trigger_for_task.signal();
        });

        tokio::time::timeout(Duration::from_secs(5), node)
            .await
            .expect("timeout")
            .expect("node run");
    }

    #[tokio::test]
    async fn node_shutdown_survives_immediate_signal_stress() {
        for _ in 0..16 {
            let config = NodeRunConfig {
                listen: "127.0.0.1:0".parse().expect("addr"),
                network: NetworkArg::Testnet,
                max_inbound: 1,
                max_outbound: 0,
                seed_strategy: SeedStrategy::Fixed(vec![]),
                network_time: 1_231_006_505,
                connect_seeds: false,
            };
            let (trigger, wait) = ShutdownTrigger::pair();
            trigger.signal();
            tokio::time::timeout(Duration::from_secs(5), run(config, wait))
                .await
                .expect("timeout")
                .expect("node run");
        }
    }
}
