//! Process runtime shared by every pipeline role: role selection, graceful
//! shutdown, operational HTTP endpoints and metrics.

pub mod metrics;
pub mod ops_http;
pub mod shutdown;

pub use ops_http::{ops_router, OpsState};
pub use shutdown::Shutdown;

/// A pipeline role. One binary runs exactly one role per process; roles are
/// scaled independently by running more processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Api,
    Triage,
    Matcher,
    Graph,
    Relay,
}

impl Role {
    pub const ALL: [Role; 5] = [Role::Api, Role::Triage, Role::Matcher, Role::Graph, Role::Relay];

    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Api => "api",
            Role::Triage => "triage",
            Role::Matcher => "matcher",
            Role::Graph => "graph",
            Role::Relay => "relay",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.as_str() == value)
    }

    /// Default port for the role's HTTP listener. The legacy `server` binary
    /// keeps port 3000.
    pub fn default_port(&self) -> u16 {
        match self {
            Role::Api => 3100,
            Role::Triage => 9101,
            Role::Matcher => 9102,
            Role::Graph => 9103,
            Role::Relay => 9104,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Role;

    #[test]
    fn roles_round_trip_and_have_distinct_ports() {
        let mut ports = std::collections::HashSet::new();
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(role));
            assert!(ports.insert(role.default_port()));
        }
        assert_eq!(Role::parse("dispatcher"), None);
    }
}
