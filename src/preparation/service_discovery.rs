use crate::RepositoryAnalysis;

use super::ServiceExecutionProfile;

pub fn discover_services(analysis: &RepositoryAnalysis) -> Vec<ServiceExecutionProfile> {
    let mut services = analysis
        .runtime_spec
        .services
        .iter()
        .map(|service| ServiceExecutionProfile {
            name: service.id.clone(),
            runtime: service.runtime.clone(),
            port: service.ports.first().copied(),
            mode: "real".to_string(),
        })
        .collect::<Vec<_>>();
    services.sort_by(|left, right| left.name.cmp(&right.name));
    services
}

pub fn discover_ports(analysis: &RepositoryAnalysis) -> Vec<u16> {
    let mut ports = analysis.runtime_spec.ports.clone();
    ports.sort();
    ports.dedup();
    ports
}
