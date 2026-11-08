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

const CACHE_SCHEMA: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StoredFlags {
    bits: i32,
    legacy: bool,
}

#[derive(Default)]
pub(crate) struct State {
    source: Option<LoginSource>,
    feeds: [BTreeMap<Uuid, StoredFlags>; 2],
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
        let lower = self.feeds[0].get(&agent).map_or(0, |entry| entry.bits);
        match self.feeds[1].get(&agent) {
            // Legacy primary rows know only SUPPORT. They must not freeze a
            // copied lower-feed NO_SUPPORT bit after an explicit lower update.
            Some(primary) if primary.legacy => lower | primary.bits,
            Some(primary) => primary.bits,
            None => lower,
        }
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
        if patch.agents.len() > MAX_AGENTS {
            self.lose();
            return Err(Error::TooLarge);
        }
        let mask = if patch.legacy { SUPPORT } else { SUPPORT | NO_SUPPORT };
        if patch
            .agents
            .iter()
            .any(|(id, flags)| id.is_nil() || *flags < 0 || *flags & !mask != 0)
        {
            return Err(Error::Invalid);
        }
        let mut updated = self.feeds[feed.index()].clone();
        for (id, bits) in &patch.agents {
            let entry = match updated.get(id) {
                Some(old) if patch.legacy => StoredFlags {
                    bits: old.bits | bits,
                    legacy: old.legacy,
                },
                _ => StoredFlags {
                    bits: *bits,
                    legacy: patch.legacy,
                },
            };
            updated.insert(*id, entry);
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
        let mut legacy = Llsd::new_map();
        for (id, flags) in &self.feeds[feed.index()] {
            if flags.legacy {
                let mut entry = Llsd::new_map();
                if flags.bits & SUPPORT != 0 {
                    entry.insert("support", true);
                }
                legacy.insert(id.to_string(), entry);
            } else {
                agents.insert(id.to_string(), flags.bits);
            }
        }
        aurora_llsd::to_xml(&aurora_llsd::llsd_map! {
            "schema" => CACHE_SCHEMA, "grid" => grid.namespace(), "feed" => feed.name(),
            "Agents" => agents, "SupportAgents" => legacy
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
            let result = read_cache(&path, grid, feed).and_then(|patches| {
                for patch in patches {
                    self.contact_policy.apply(&login, feed, &patch)?;
                }
                Ok(())
            });
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
    format!("contact-policy-v{CACHE_SCHEMA}-{}-{}.xml", grid.namespace(), feed.name())
}

fn read_cache(path: &Path, grid: GridDigest, feed: Feed) -> Result<Vec<Patch>, Error> {
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
    if data["schema"] != Llsd::Integer(CACHE_SCHEMA) || data["grid"].as_str() != grid.namespace() || data["feed"].as_str() != feed.name() {
        return Err(Error::Invalid);
    }
    // Network feeds have whole-document precedence; mixed caches must validate
    // BOTH disjoint maps before installing either. V1 flattened provenance and
    // cannot be repaired by guessing, so its separate filename/schema is ignored.
    if !data["Agents"].is_map() || !data["SupportAgents"].is_map() {
        return Err(Error::Invalid);
    }
    let modern = aurora_net::contact_policy::parse(&bytes)?;
    let legacy = aurora_net::contact_policy::parse(&aurora_llsd::to_xml(&aurora_llsd::llsd_map! {
        "SupportAgents" => data["SupportAgents"].clone()
    }))?;
    if modern.agents.len() + legacy.agents.len() > MAX_AGENTS {
        return Err(Error::TooLarge);
    }
    if modern.agents.keys().any(|id| legacy.agents.contains_key(id)) {
        return Err(Error::Invalid);
    }
    Ok(vec![legacy, modern])
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
