//! Hardcoded seed addresses for deterministic local/test networking.

use std::net::SocketAddr;

use crate::constants::Network;

/// Strategy for choosing initial peer addresses without live DNS lookups.
#[derive(Debug, Clone)]
pub enum SeedStrategy {
    /// Use only explicit addresses supplied by the caller.
    Fixed(Vec<SocketAddr>),
    /// Use localhost seeds on adjacent ports (for integration tests).
    Localhost {
        /// Base port for the first seed.
        base_port: u16,
        /// Number of sequential ports to try.
        count: u16,
    },
}

impl SeedStrategy {
    /// Returns seed addresses for `network`.
    ///
    /// The hardcoded lists are intentionally offline-friendly for tests.
    #[must_use]
    pub fn addresses(&self, _network: Network) -> Vec<SocketAddr> {
        match self {
            Self::Fixed(addresses) => addresses.clone(),
            Self::Localhost { base_port, count } => (0..*count)
                .map(|offset| SocketAddr::from(([127, 0, 0, 1], base_port + offset)))
                .collect(),
        }
    }

    /// Default localhost strategy for tests.
    #[must_use]
    pub const fn localhost(base_port: u16) -> Self {
        Self::Localhost {
            base_port,
            count: 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SeedStrategy;
    use crate::constants::Network;

    #[test]
    fn localhost_strategy_never_requires_dns() {
        let seeds = SeedStrategy::localhost(18_333).addresses(Network::Testnet);
        assert_eq!(seeds.len(), 4);
        assert!(seeds.iter().all(|addr| addr.ip().is_loopback()));
    }
}
