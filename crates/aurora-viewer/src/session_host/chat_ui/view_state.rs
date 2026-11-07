//! View ownership follows MonitorChat::select/draft and selectCycle in the
//! pinned Legacy monitor model (abbe3cb0ca66): selection and reading belong to
//! a surface; text, Undo and operation receipts belong to the exact source.

use super::{Conversation, NativeChatPolicy, ReadDraft, ReadMetadata};
use std::collections::BTreeMap;

/// The complete bounded set: Shared Chat, four fixed monitors and one cycler.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChatSurface {
    #[default]
    Shared,
    Monitor1,
    Monitor2,
    Monitor3,
    Monitor4,
    Cycling,
}

impl ChatSurface {
    pub const ALL: [Self; 6] = [
        Self::Shared,
        Self::Monitor1,
        Self::Monitor2,
        Self::Monitor3,
        Self::Monitor4,
        Self::Cycling,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Shared => 0,
            Self::Monitor1 => 1,
            Self::Monitor2 => 2,
            Self::Monitor3 => 3,
            Self::Monitor4 => 4,
            Self::Cycling => 5,
        }
    }
}

#[derive(Default)]
pub(super) struct ReadingState {
    pub policy: Option<NativeChatPolicy>,
    pub offset: Option<f32>,
    pub tail: bool,
    pub caret: Option<egui::text::CCursorRange>,
    pub draft_revision: u64,
}

pub(super) struct ChatViewState {
    pub conversation: Conversation,
    pub policy: Option<NativeChatPolicy>,
    pub interaction: u64,
    pub changed: bool,
    pub ime_composing: bool,
    pub read_metadata: Option<ReadMetadata>,
    pub read_action: Option<ReadDraft>,
    pub reading: BTreeMap<Conversation, ReadingState>,
    pub incarnation: u64,
    pub owner_generation: u64,
    pub exhausted: bool,
}

impl Default for ChatViewState {
    fn default() -> Self {
        Self {
            conversation: Conversation::Nearby,
            policy: None,
            interaction: 0,
            changed: false,
            ime_composing: false,
            read_metadata: None,
            read_action: None,
            reading: BTreeMap::new(),
            incarnation: 0,
            owner_generation: 0,
            exhausted: false,
        }
    }
}
