//! Shared native/host Direct-contact refusal from FSData. Public cached flags
//! belong to the captured grid, not to mutable Preferences or another login.
use super::{World, account_metadata_origin::GridDigest};
use aurora_llsd::Llsd;
use aurora_net::contact_policy::{Error, Feed, MAX_AGENTS, MAX_BYTES, NO_SUPPORT, Patch, SUPPORT};
use aurora_net::{LoginResponse, LoginSource, NetCommand};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

pub(crate) fn supported_grid(grid: GridDigest) -> bool {
    Some(grid) == GridDigest::from_uri(aurora_net::login::SL_MAIN_GRID)
        || Some(grid) == GridDigest::from_uri(aurora_net::login::SL_BETA_GRID)
}

#[derive(Default)]
pub(crate) struct State {
    source: Option<LoginSource>,
    feeds: [BTreeMap<Uuid, i32>; 2],
    revision: u64,
    lost: bool,
}

impl State {
    pub(super) fn for_login(login: &Arc<LoginResponse>) -> Self {
        Self {
            source: Some(LoginSource::from_login(login)),
            ..Default::default()
        }
    }
    fn current(&self, login: &Arc<LoginResponse>) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.matches_login(login, login.agent_id))
    }
    fn flags(&self, agent: Uuid) -> i32 {
        // Dedicated agents.xml wins regardless of download/cache arrival order.
        // Missing records retain previously known flags, as processAgents does.
        self.feeds[1]
            .get(&agent)
            .or_else(|| self.feeds[0].get(&agent))
            .copied()
            .unwrap_or(0)
    }
    pub(super) fn lose(&mut self) {
        if !self.lost {
            self.lost = true;
            self.revision = self.revision.saturating_add(1);
        }
    }
    pub(super) fn apply(&mut self, login: &Arc<LoginResponse>, feed: Feed, patch: &Patch) -> Result<(), Error> {
        if !self.current(login) {
            return Err(Error::Invalid);
        }
        let mut updated = self.feeds[feed.index()].clone();
        for (id, flags) in &patch.agents {
            let flags = if patch.legacy {
                // Legacy has no information capable of clearing NO_SUPPORT.
                updated.get(id).copied().unwrap_or_else(|| self.flags(*id)) | flags
            } else {
                *flags
            };
            updated.insert(*id, flags);
        }
        if updated.len() > MAX_AGENTS {
            self.lose();
            return Err(Error::TooLarge);
        }
        if updated != self.feeds[feed.index()] {
            let Some(next) = self.revision.checked_add(1) else {
                self.lose();
                return Err(Error::TooLarge);
            };
            self.feeds[feed.index()] = updated;
            self.revision = next;
        }
        Ok(())
    }
    fn allows(&self, login: &Arc<LoginResponse>, peer: Uuid) -> bool {
        if !self.current(login) {
            return true;
        } // no flags for this new source
        !self.lost && !(self.flags(login.agent_id) & NO_SUPPORT != 0 && self.flags(peer) & SUPPORT != 0)
    }
    pub(super) fn cache_xml(&self, grid: GridDigest, feed: Feed) -> Vec<u8> {
        let mut agents = Llsd::new_map();
        for (id, flags) in &self.feeds[feed.index()] {
            agents.insert(id.to_string(), *flags);
        }
        aurora_llsd::to_xml(&aurora_llsd::llsd_map! {
            "schema" => 1, "grid" => grid.namespace(), "feed" => feed.name(), "Agents" => agents
        })
    }
}

impl World {
    /// Independent of RLVa, mute state and any existing-session exemption.
    /// Missing FSData flags allow contact, matching the reference predicate.
    pub(crate) fn allows_direct_contact(&self, peer: Uuid) -> bool {
        self.login
            .as_ref()
            .is_none_or(|login| self.agent_id == login.agent_id && self.contact_policy.allows(login, peer))
    }
    pub(crate) fn contact_policy_revision(&self) -> u64 {
        self.login
            .as_ref()
            .filter(|login| self.contact_policy.current(login))
            .map_or(0, |_| self.contact_policy.revision)
    }
    /// Final common send boundary, including alternate composers and typing.
    /// Receive/Read/Close, system replies, Group and Conference are unchanged.
    pub(crate) fn contact_command_allowed(&self, command: &NetCommand) -> bool {
        match command {
            NetCommand::SendIm { to, .. } | NetCommand::ImReply { to, busy: false, .. } => self.allows_direct_contact(*to),
            NetCommand::SendImDialog {
                to, dialog: 0 | 41 | 42, ..
            } => self.allows_direct_contact(*to),
            _ => true,
        }
    }
    pub(crate) fn load_contact_policy_cache(&mut self, root: &Path, grid: GridDigest) {
        let Some(login) = self.login.clone() else {
            return;
        };
        for feed in Feed::ALL {
            let path = root.join(cache_file(grid, feed));
            let result = read_cache(&path, grid, feed).and_then(|patch| self.contact_policy.apply(&login, feed, &patch));
            if result == Err(Error::TooLarge) {
                self.contact_policy.lose();
            }
            if result.is_err() && path.exists() {
                log::warn!("cached contact policy rejected; retaining known flags");
            }
        }
    }
    pub(crate) fn save_contact_policy_cache(&self, root: &Path, grid: GridDigest, feed: Feed) {
        let Some(login) = &self.login else {
            return;
        };
        if !self.contact_policy.current(login) {
            return;
        }
        if write_cache(root, &cache_file(grid, feed), &self.contact_policy.cache_xml(grid, feed)).is_err() {
            log::warn!("contact policy cache was not saved");
        }
    }
}

fn cache_file(grid: GridDigest, feed: Feed) -> String {
    format!("contact-policy-v1-{}-{}.xml", grid.namespace(), feed.name())
}

fn read_cache(path: &Path, grid: GridDigest, feed: Feed) -> Result<Patch, Error> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| Error::Invalid)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::Invalid);
    }
    if metadata.len() > MAX_BYTES as u64 {
        return Err(Error::TooLarge);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| Error::Invalid)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let data = aurora_llsd::from_xml(&bytes).map_err(|_| Error::Invalid)?;
    if data["schema"] != Llsd::Integer(1) || data["grid"].as_str() != grid.namespace() || data["feed"].as_str() != feed.name() {
        return Err(Error::Invalid);
    }
    aurora_net::contact_policy::parse(&bytes)
}

fn write_cache(root: &Path, file: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let temp = root.join(format!(".contact-policy-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        // Exclusive temporary creation and replacement never truncate/follow
        // an existing cache leaf. These files contain only public flags.
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        output.write_all(bytes)?;
        output.sync_all()?;
        drop(output);
        std::fs::rename(&temp, root.join(file))
    })();
    let _ = std::fs::remove_file(temp);
    result
}

#[cfg(test)]
mod tests;
