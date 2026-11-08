//! Narrow FSData contact-policy port. References (LGPL-2.1-or-later):
//! Folderstorm abbe3cb0, fsdata.cpp::processAgents/canSendToIMContact and
//! fsimcontactpolicy.h. No MOTD, login blocking, tags or unrelated FSData state.
use crate::{EventSender, LoginResponse, LoginSource, NetEvent};
use aurora_llsd::Llsd;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub const SUPPORT: i32 = 1;
pub const NO_SUPPORT: i32 = 16;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_AGENTS: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    Data,
    Agents,
}
impl Feed {
    pub const ALL: [Self; 2] = [Self::Data, Self::Agents];
    pub fn name(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Agents => "agents",
        }
    }
    pub fn index(self) -> usize {
        match self {
            Self::Data => 0,
            Self::Agents => 1,
        }
    }
    fn url(self) -> String {
        format!("https://phoenixviewer.com/app/fsdata/{}.xml", self.name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("contact policy request failed")]
    Transport,
    #[error("contact policy HTTP {0}")]
    Http(u16),
    #[error("contact policy exceeds its size limit")]
    TooLarge,
    #[error("invalid contact policy")]
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub agents: BTreeMap<Uuid, i32>,
    /// Legacy SupportAgents has no NO_SUPPORT information. It can add support
    /// entries but cannot clear an already known modern restriction.
    pub legacy: bool,
}

pub fn parse(bytes: &[u8]) -> Result<Patch, Error> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let data = aurora_llsd::from_xml(bytes).map_err(|_| Error::Invalid)?;
    if !data.is_map() {
        return Err(Error::Invalid);
    }
    let legacy = !data.has("Agents");
    let field = if legacy { "SupportAgents" } else { "Agents" };
    // data.xml is allowed to contain no agent flags at all.
    if !data.has(field) {
        return Ok(Patch {
            agents: BTreeMap::new(),
            legacy,
        });
    }
    let map = data[field].as_map().ok_or(Error::Invalid)?;
    if map.len() > MAX_AGENTS {
        return Err(Error::TooLarge);
    }
    let mut agents = BTreeMap::new();
    for (key, value) in map {
        let id = Uuid::parse_str(key).map_err(|_| Error::Invalid)?;
        if id.is_nil() {
            return Err(Error::Invalid);
        }
        let flags = if legacy {
            if !value.is_map() {
                return Err(Error::Invalid);
            }
            // The legacy format tests key presence, including support=false.
            if value.has("support") { SUPPORT } else { 0 }
        } else {
            match value {
                Llsd::Integer(flags) if *flags >= 0 => *flags & (SUPPORT | NO_SUPPORT),
                _ => return Err(Error::Invalid),
            }
        };
        if agents.insert(id, flags).is_some() {
            return Err(Error::Invalid);
        }
    }
    Ok(Patch { agents, legacy })
}

/// A grid's UUID namespace cannot borrow Second Life's public flags. The
/// reference supplies no OpenSim agents URL; do not invent one from grid input.
pub fn supported_grid(uri: &str) -> bool {
    matches!(uri.trim(), crate::login::SL_MAIN_GRID | crate::login::SL_BETA_GRID)
}

/// Actual login hook only. These GETs contain no account, password or UUID;
/// public TLS verification, HTTPS-only and no redirects avoid simulator TLS
/// exceptions, off-host forwarding and HTTP downgrade.
pub(crate) fn load_for_login(uri: &str, login: &Arc<LoginResponse>, events: EventSender) {
    if !supported_grid(uri) {
        return;
    }
    let Ok(http) = policy_client() else {
        log::warn!("contact policy client unavailable; retaining cached flags");
        return;
    };
    for feed in Feed::ALL {
        let http = http.clone();
        let source = LoginSource::from_login(login);
        let events = events.clone();
        tokio::spawn(async move {
            let result = fetch(&http, &feed.url()).await;
            let _ = events.send(NetEvent::ContactPolicy { source, feed, result });
        });
    }
}

fn policy_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(aurora_version::user_agent())
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
}

async fn fetch(http: &reqwest::Client, url: &str) -> Result<Patch, Error> {
    let mut response = http
        .get(url)
        .header("Accept", "application/llsd+xml")
        .send()
        .await
        .map_err(|_| Error::Transport)?;
    if !response.status().is_success() {
        return Err(Error::Http(response.status().as_u16()));
    }
    if response.content_length().is_some_and(|length| length > MAX_BYTES as u64) {
        return Err(Error::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
        if chunk.len() > MAX_BYTES - bytes.len() {
            return Err(Error::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    parse(&bytes)
}

#[cfg(test)]
mod tests;
