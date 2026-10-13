//! HTTPS, with the operating system verifying certificates.

use std::io::Read;
use std::time::Duration;

const USER_AGENT: &str = concat!("Craft-Proton-Sync/", env!("CARGO_PKG_VERSION"));

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(1800)))
        .user_agent(USER_AGENT)
        .build()
        .into()
}

/// A small text file, refused when it is larger than `max` bytes.
pub(crate) fn text(url: &str, max: u64) -> Result<String, String> {
    let response = agent().get(url).call().map_err(|e| e.to_string())?;
    let mut body = String::new();
    let read = response
        .into_body()
        .into_reader()
        .take(max.saturating_add(1))
        .read_to_string(&mut body)
        .map_err(|e| e.to_string())?;
    if read as u64 > max {
        return Err("the answer is far larger than it should be".into());
    }
    Ok(body)
}
