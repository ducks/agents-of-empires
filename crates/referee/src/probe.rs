use std::time::{Duration, Instant};

use async_trait::async_trait;

/// Controller-visible address and expected response for one territory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeTarget {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub expected_status: u16,
    pub expected_body: String,
}

/// One external observation. Transport failures are unhealthy observations,
/// not errors in the referee itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthObservation {
    pub healthy: bool,
    pub status: Option<u16>,
    pub latency_ms: u64,
    pub detail: String,
}

#[async_trait]
pub trait HealthProbe: Send + Sync {
    async fn observe(&self, target: &ProbeTarget) -> HealthObservation;
}

/// HTTP probe with a hard per-request timeout.
pub struct HttpProbe {
    client: reqwest::Client,
}

impl HttpProbe {
    /// Construct a probe.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be constructed.
    pub fn new(timeout: Duration) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder().timeout(timeout).build()?,
        })
    }
}

#[async_trait]
impl HealthProbe for HttpProbe {
    async fn observe(&self, target: &ProbeTarget) -> HealthObservation {
        let start = Instant::now();
        let url = format!("http://{}:{}{}", target.host, target.port, target.path);
        match self.client.get(url).send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                match response.text().await {
                    Ok(body) => HealthObservation {
                        healthy: status == target.expected_status && body == target.expected_body,
                        status: Some(status),
                        latency_ms: millis(start.elapsed()),
                        detail: body,
                    },
                    Err(error) => HealthObservation {
                        healthy: false,
                        status: Some(status),
                        latency_ms: millis(start.elapsed()),
                        detail: error.to_string(),
                    },
                }
            }
            Err(error) => HealthObservation {
                healthy: false,
                status: None,
                latency_ms: millis(start.elapsed()),
                detail: error.to_string(),
            },
        }
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
