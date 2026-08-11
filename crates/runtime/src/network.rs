use std::collections::HashSet;

use aoe_domain::ArenaManifest;
use thiserror::Error;

/// Network coordinates assigned to one guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAssignment {
    pub territory: String,
    pub arena_ip: String,
    pub mac_address: String,
    pub ssh_port: u16,
    pub service_port: u16,
    pub guest_service_port: u16,
    pub multicast_port: u16,
}

impl NetworkAssignment {
    /// QEMU networking arguments for controller management and isolated peers.
    #[must_use]
    pub fn qemu_net_opts(&self) -> String {
        format!(
            "-netdev user,id=mgmt,restrict=on,hostfwd=tcp:127.0.0.1:{}-:22,hostfwd=tcp:127.0.0.1:{}-:{} \
             -device virtio-net-pci,netdev=mgmt \
             -netdev socket,id=arena,mcast=230.77.0.1:{} \
             -device virtio-net-pci,netdev=arena,mac={}",
            self.ssh_port,
            self.service_port,
            self.guest_service_port,
            self.multicast_port,
            self.mac_address
        )
    }
}

/// Deterministic network plan for every territory in one match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkPlan {
    pub assignments: Vec<NetworkAssignment>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NetworkPlanError {
    #[error("public internet access is forbidden")]
    PublicInternet,
    #[error("arena supports at most 240 territories")]
    TooManyTerritories,
    #[error("port range exceeds 65535")]
    PortRange,
    #[error("duplicate host port {0}")]
    DuplicatePort(u16),
    #[error("unsupported arena CIDR {0}; expected a /24 private IPv4 network")]
    UnsupportedCidr(String),
}

impl NetworkPlan {
    /// Assign isolated guest addresses and controller-only forwarded ports.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe networking, unsupported address space, or
    /// overlapping host ports.
    pub fn from_manifest(
        manifest: &ArenaManifest,
        base_port: u16,
        multicast_port: u16,
    ) -> Result<Self, NetworkPlanError> {
        if manifest.network.allow_public_internet {
            return Err(NetworkPlanError::PublicInternet);
        }
        if manifest.territories.len() > 240 {
            return Err(NetworkPlanError::TooManyTerritories);
        }
        let prefix = manifest
            .network
            .cidr
            .strip_suffix(".0/24")
            .ok_or_else(|| NetworkPlanError::UnsupportedCidr(manifest.network.cidr.clone()))?;

        let required_ports = manifest
            .territories
            .len()
            .checked_mul(2)
            .and_then(|count| u16::try_from(count).ok())
            .ok_or(NetworkPlanError::PortRange)?;
        base_port
            .checked_add(required_ports)
            .ok_or(NetworkPlanError::PortRange)?;

        let mut ports = HashSet::new();
        let assignments = manifest
            .territories
            .iter()
            .enumerate()
            .map(|(index, territory)| {
                let offset = u16::try_from(index * 2).map_err(|_| NetworkPlanError::PortRange)?;
                let ssh_port = base_port
                    .checked_add(offset)
                    .ok_or(NetworkPlanError::PortRange)?;
                let service_port = ssh_port.checked_add(1).ok_or(NetworkPlanError::PortRange)?;
                for port in [ssh_port, service_port] {
                    if !ports.insert(port) {
                        return Err(NetworkPlanError::DuplicatePort(port));
                    }
                }
                let host = index + 10;
                Ok(NetworkAssignment {
                    territory: territory.id.clone(),
                    arena_ip: format!("{prefix}.{host}"),
                    mac_address: format!("52:54:00:77:00:{host:02x}"),
                    ssh_port,
                    service_port,
                    guest_service_port: territory.service.port,
                    multicast_port,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { assignments })
    }
}
