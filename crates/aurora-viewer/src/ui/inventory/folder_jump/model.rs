//! Bounded personal-Inventory index, Legacy name/path ranking and exact input
//! tickets. The parent must supply current actual login/owner/privacy facts;
//! these local correlation values confer no network or Inventory authority.

use crate::world::inventory::Inventory;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub(super) const MAX_RESULTS: usize = 60;
const MAX_NODES: usize = 50_000;
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_DEPTH: usize = 128;
const MAX_NAME_BYTES: usize = 1_024;
const MAX_PATH_BYTES: usize = 4_096;
const MAX_QUERY_BYTES: usize = 1_024;
const MAX_QUERY_CHARS: usize = 256;
const MAX_TOKENS: usize = 64;
const STEPS_PER_TURN: usize = 256;
const TURN: Duration = Duration::from_millis(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Source {
    pub agent: Uuid,
    pub session: Uuid,
    pub owner: Uuid,
    pub root: Uuid,
    /// Process-selected profile identity, never its personal filesystem path.
    pub profile: u64,
    /// Exact main/additional Inventory owner, not the currently focused window.
    pub window: Uuid,
}

impl Source {
    fn matches(self, inventory: &Inventory) -> bool {
        !self.agent.is_nil()
            && !self.session.is_nil()
            && self.profile != 0
            && self.owner == self.agent
            && self.owner == inventory.owner
            && !self.root.is_nil()
            && self.root == inventory.root
            && self.root != inventory.lib_root
            && inventory
                .folders
                .get(&self.root)
                .is_some_and(|folder| folder.info.id == self.root && !folder.library && folder.info.type_default != 14)
    }
}

/// Whole current owning-view/privacy revision. Only the ordinary full
/// Inventory owner is supported; specialized views must supply allowed=false.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Policy {
    pub revision: u64,
    pub allowed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Unavailable,
    Indexing,
    Matching,
    Ready,
    InvalidQuery,
    Exhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ticket {
    instance: u64,
    epoch: u64,
    input: u64,
    source: Source,
    revision: u64,
    policy: Policy,
    folder: Uuid,
    row: u64,
    selected: Option<Uuid>,
}

impl Ticket {
    pub(super) fn folder(self) -> Uuid {
        self.folder
    }
}

struct Node {
    id: Uuid,
    parent: usize,
    name: String,
    path: String,
    folded_name: String,
    folded_path: String,
    depth: usize,
    serial: u64,
}

#[derive(Clone, Copy)]
struct Match {
    node: usize,
    rank: u8,
}

pub(super) struct Row<'a> {
    pub folder: Uuid,
    pub name: &'a str,
    pub parent_path: &'a str,
    /// The pair never aliases another indexed incarnation, including rebuilds.
    pub widget: (u64, u64),
}

struct Query {
    requested: String,
    folded: String,
    tokens: Vec<String>,
}

impl Query {
    fn parse(value: &str) -> Option<Self> {
        if value.len() > MAX_QUERY_BYTES || value.chars().take(MAX_QUERY_CHARS + 1).count() > MAX_QUERY_CHARS {
            return None;
        }
        let folded = value.trim().to_lowercase();
        let mut tokens = Vec::new();
        for token in folded.split([' ', '/', '\t']).filter(|token| !token.is_empty()) {
            if tokens.len() == MAX_TOKENS {
                return None;
            }
            tokens.push(token.to_owned());
        }
        Some(Self {
            requested: value.to_owned(),
            folded,
            tokens,
        })
    }

    fn rank(&self, node: &Node) -> Option<u8> {
        if self.tokens.is_empty() {
            return (node.depth == 1).then_some(0);
        }
        if node.folded_name == self.folded {
            return Some(0);
        }
        if node.folded_name.starts_with(&self.folded) {
            return Some(1);
        }
        let mut path = false;
        for token in &self.tokens {
            if !node.folded_name.contains(token) {
                if !node.folded_path.contains(token) {
                    return None;
                }
                path = true;
            }
        }
        Some(if path { 3 } else { 2 })
    }
}

#[derive(Clone, Copy)]
struct Limits {
    nodes: usize,
    text: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            nodes: MAX_NODES,
            text: MAX_TEXT_BYTES,
        }
    }
}

pub(super) struct Model {
    instance: u64,
    epoch: u64,
    input: u64,
    serial: u64,
    source: Option<Source>,
    revision: u64,
    policy: Policy,
    phase: Phase,
    nodes: Vec<Node>,
    seen: HashMap<Uuid, usize>,
    parent: usize,
    child: usize,
    matching: usize,
    best: Vec<Match>,
    matches: usize,
    query: Option<Query>,
    selected: Option<Uuid>,
    select_first: bool,
    limited: bool,
    text: usize,
    limits: Limits,
}

impl Default for Model {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let instance = NEXT
            .fetch_update(AtomicOrdering::Relaxed, AtomicOrdering::Relaxed, |value| value.checked_add(1))
            .unwrap_or(0);
        Self {
            instance,
            epoch: 0,
            input: 0,
            serial: 0,
            source: None,
            revision: 0,
            policy: Policy {
                revision: 0,
                allowed: false,
            },
            phase: if instance == 0 { Phase::Exhausted } else { Phase::Unavailable },
            nodes: Vec::new(),
            seen: HashMap::new(),
            parent: 0,
            child: 0,
            matching: 1,
            best: Vec::new(),
            matches: 0,
            query: Query::parse(""),
            selected: None,
            select_first: true,
            limited: false,
            text: 0,
            limits: Limits::default(),
        }
    }
}

impl Model {
    fn discard(&mut self) {
        self.nodes.clear();
        self.seen.clear();
        self.best.clear();
        self.matches = 0;
        self.text = 0;
        self.parent = 0;
        self.child = 0;
        self.matching = 1;
    }

    fn exhaust(&mut self) {
        self.discard();
        self.source = None;
        self.selected = None;
        self.phase = Phase::Exhausted;
    }

    fn invalidate(&mut self) -> bool {
        let Some(next) = self.epoch.checked_add(1) else {
            self.exhaust();
            return false;
        };
        self.epoch = next;
        true
    }

    /// Called after actual source/model events, before input and dispatch.
    /// A false/unknown source purges private folder text immediately.
    pub(super) fn sync(&mut self, inventory: &Inventory, source: Option<Source>, policy: Policy) {
        if self.phase == Phase::Exhausted {
            return;
        }
        let source = source.filter(|source| policy.allowed && source.matches(inventory));
        if source.is_none() {
            if self.source.is_some() || self.phase != Phase::Unavailable {
                self.invalidate();
            }
            self.discard();
            self.source = None;
            self.selected = None;
            self.query = Query::parse("");
            self.select_first = true;
            if self.phase != Phase::Exhausted {
                self.phase = Phase::Unavailable;
            }
            return;
        }
        if self.source == source && self.revision == inventory.generation && self.policy == policy {
            return;
        }
        let changed_source = self.source != source;
        if !self.invalidate() {
            return;
        }
        self.discard();
        self.source = source;
        self.revision = inventory.generation;
        self.policy = policy;
        self.limited = false;
        if changed_source {
            self.query = Query::parse("");
            self.selected = None;
            self.select_first = true;
        }
        let Some(source) = source else { return };
        self.nodes.push(Node {
            id: source.root,
            parent: 0,
            name: String::new(),
            path: String::new(),
            folded_name: String::new(),
            folded_path: String::new(),
            depth: 0,
            serial: 0,
        });
        self.seen.insert(source.root, 0);
        self.phase = Phase::Indexing;
    }

    pub(super) fn set_query(&mut self, value: &str) {
        if self.phase == Phase::Exhausted {
            return;
        }
        let query = Query::parse(value);
        if self.query.as_ref().map(|query| &query.requested) == query.as_ref().map(|query| &query.requested) {
            return;
        }
        if !self.invalidate() {
            return;
        }
        self.query = query;
        self.selected = None;
        self.select_first = true;
        self.best.clear();
        self.matches = 0;
        self.matching = 1;
        if self.source.is_some() && self.phase != Phase::Indexing {
            self.phase = if self.query.is_some() {
                Phase::Matching
            } else {
                Phase::InvalidQuery
            };
        }
    }

    pub(super) fn phase(&self) -> Phase {
        self.phase
    }

    pub(super) fn limited(&self) -> bool {
        self.limited
    }

    /// Count only for the retained subset. The parent must qualify it when
    /// limited(), and distinguish the best-60 display cap from index limits.
    pub(super) fn matches(&self) -> usize {
        self.matches
    }

    pub(super) fn selected(&self) -> Option<Uuid> {
        self.selected
    }

    pub(super) fn rows(&self) -> impl ExactSizeIterator<Item = Row<'_>> {
        let best = if self.phase == Phase::Ready { self.best.as_slice() } else { &[] };
        best.iter().map(|matched| {
            let node = &self.nodes[matched.node];
            Row {
                folder: node.id,
                name: &node.name,
                parent_path: &node.path,
                widget: (self.instance, node.serial),
            }
        })
    }

    fn compare(&self, left: Match, right: Match) -> Ordering {
        let left_node = &self.nodes[left.node];
        let right_node = &self.nodes[right.node];
        (left.rank, &left_node.folded_name, &left_node.folded_path, left_node.id).cmp(&(
            right.rank,
            &right_node.folded_name,
            &right_node.folded_path,
            right_node.id,
        ))
    }

    fn finish_index(&mut self) {
        self.parent = self.nodes.len();
        self.child = 0;
        self.phase = if self.query.is_some() {
            Phase::Matching
        } else {
            Phase::InvalidQuery
        };
    }

    fn index_step(&mut self, inventory: &Inventory) {
        let Some(parent) = self.nodes.get(self.parent) else {
            self.finish_index();
            return;
        };
        let Some(folder) = inventory.folders.get(&parent.id) else {
            self.parent += 1;
            self.child = 0;
            return;
        };
        let Some(id) = folder.children.get(self.child).copied() else {
            self.parent += 1;
            self.child = 0;
            return;
        };
        self.child += 1;
        if id.is_nil() || id == inventory.lib_root || self.seen.contains_key(&id) {
            return;
        }
        let Some(folder) = inventory.folders.get(&id) else { return };
        if folder.info.id != id || folder.info.parent != parent.id || folder.library || folder.info.type_default == 14 {
            return;
        }
        let depth = parent.depth + 1;
        let name = &folder.info.name;
        if depth > MAX_DEPTH || name.len() > MAX_NAME_BYTES {
            self.limited = true;
            return;
        }
        let path = if parent.name.is_empty() {
            parent.path.clone()
        } else if parent.path.is_empty() {
            parent.name.clone()
        } else {
            format!("{} / {}", parent.path, parent.name)
        };
        let folded_name = name.to_lowercase();
        let folded_path = path.to_lowercase();
        if path.len() > MAX_PATH_BYTES || folded_path.len() > MAX_PATH_BYTES || folded_name.len() > MAX_NAME_BYTES * 3 {
            self.limited = true;
            return;
        }
        let bytes = name.len() + path.len() + folded_name.len() + folded_path.len();
        if self.nodes.len() >= self.limits.nodes || bytes > self.limits.text.saturating_sub(self.text) {
            self.limited = true;
            self.finish_index();
            return;
        }
        let Some(serial) = self.serial.checked_add(1) else {
            self.exhaust();
            return;
        };
        self.serial = serial;
        self.text += bytes;
        self.seen.insert(id, self.nodes.len());
        self.nodes.push(Node {
            id,
            parent: self.parent,
            name: name.clone(),
            path,
            folded_name,
            folded_path,
            depth,
            serial,
        });
    }

    fn match_step(&mut self) {
        let Some(node) = self.nodes.get(self.matching) else {
            if self.select_first {
                self.selected = self.best.first().map(|matched| self.nodes[matched.node].id);
            } else {
                self.selected = self
                    .selected
                    .filter(|id| self.best.iter().any(|matched| self.nodes[matched.node].id == *id));
            }
            self.select_first = false;
            self.phase = Phase::Ready;
            return;
        };
        let matched = self
            .query
            .as_ref()
            .and_then(|query| query.rank(node))
            .map(|rank| Match { node: self.matching, rank });
        self.matching += 1;
        if let Some(matched) = matched {
            self.matches += 1;
            let position = self
                .best
                .partition_point(|other| self.compare(*other, matched) != Ordering::Greater);
            if position < MAX_RESULTS {
                self.best.insert(position, matched);
                self.best.truncate(MAX_RESULTS);
            }
        }
    }

    fn advance_with(&mut self, inventory: &Inventory, steps: usize, mut expired: impl FnMut() -> bool) -> usize {
        if inventory.generation != self.revision || !self.source.is_some_and(|source| source.matches(inventory)) {
            return 0;
        }
        let mut used = 0;
        while used < steps && matches!(self.phase, Phase::Indexing | Phase::Matching) {
            if used % 32 == 0 && expired() {
                break;
            }
            match self.phase {
                Phase::Indexing => self.index_step(inventory),
                Phase::Matching => self.match_step(),
                _ => break,
            }
            used += 1;
        }
        used
    }

    /// At most 256 bounded edge/parent/match steps and a 2 ms soft deadline.
    pub(super) fn advance(&mut self, inventory: &Inventory) {
        let started = Instant::now();
        self.advance_with(inventory, STEPS_PER_TURN, || started.elapsed() >= TURN);
    }

    pub(super) fn capture(&self, folder: Uuid) -> Option<Ticket> {
        if self.phase != Phase::Ready || !self.best.iter().any(|matched| self.nodes[matched.node].id == folder) {
            return None;
        }
        let node = &self.nodes[*self.seen.get(&folder)?];
        Some(Ticket {
            instance: self.instance,
            epoch: self.epoch,
            input: self.input,
            source: self.source?,
            revision: self.revision,
            policy: self.policy,
            folder,
            row: node.serial,
            selected: self.selected,
        })
    }

    /// Current original parent chain, not a new search or a replacement row.
    pub(super) fn validate(&self, ticket: Ticket, inventory: &Inventory, source: Option<Source>, policy: Policy) -> bool {
        if self.capture(ticket.folder) != Some(ticket)
            || source != Some(ticket.source)
            || !policy.allowed
            || policy != ticket.policy
            || inventory.generation != ticket.revision
            || !ticket.source.matches(inventory)
        {
            return false;
        }
        let Some(mut index) = self.seen.get(&ticket.folder).copied() else {
            return false;
        };
        if index == 0 {
            return false;
        }
        for _ in 0..=MAX_DEPTH {
            let node = &self.nodes[index];
            let Some(folder) = inventory.folders.get(&node.id) else {
                return false;
            };
            if folder.info.id != node.id || node.id == inventory.lib_root || folder.library || folder.info.type_default == 14 {
                return false;
            }
            if index == 0 {
                return node.id == ticket.source.root;
            }
            if folder.info.name != node.name || folder.info.parent != self.nodes[node.parent].id {
                return false;
            }
            index = node.parent;
        }
        false
    }

    pub(super) fn select(&mut self, ticket: Ticket, inventory: &Inventory, source: Option<Source>, policy: Policy) -> bool {
        if !self.validate(ticket, inventory, source, policy) {
            return false;
        }
        if self.selected != Some(ticket.folder) {
            let Some(input) = self.input.checked_add(1) else {
                self.exhaust();
                return false;
            };
            self.input = input;
            self.selected = Some(ticket.folder);
        }
        true
    }

    pub(super) fn capture_selected(&self) -> Option<Ticket> {
        self.capture(self.selected?)
    }

    pub(super) fn capture_navigation(&self, previous: bool) -> Option<Ticket> {
        if self.phase != Phase::Ready || self.best.is_empty() {
            return None;
        }
        let current = self
            .selected
            .and_then(|id| self.best.iter().position(|matched| self.nodes[matched.node].id == id));
        let next = match (current, previous) {
            (Some(current), true) => current.saturating_sub(1),
            (Some(current), false) => (current + 1).min(self.best.len() - 1),
            (None, true) => self.best.len() - 1,
            (None, false) => 0,
        };
        self.capture(self.nodes[self.best[next].node].id)
    }

    /// Cancels old press/paint correlation without discarding a valid index.
    pub(super) fn cancel_input(&mut self) {
        let Some(input) = self.input.checked_add(1) else {
            self.exhaust();
            return;
        };
        self.input = input;
    }
}

#[cfg(test)]
mod tests;
