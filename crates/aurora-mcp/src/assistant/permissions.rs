//! Per-character permissions, kept separate from controller authority.
//! Behavior reference: fsassistantpermissions.h at abbe3cb0ca66ee2484f277219f6a27706fab7fc6.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Never,
    Ask,
    Allow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Class {
    #[serde(rename = "read")]
    Read,
    #[serde(rename = "camera")]
    Camera,
    #[serde(rename = "create")]
    Create,
    #[serde(rename = "edit")]
    Edit,
    #[serde(rename = "move")]
    Move,
    #[serde(rename = "trash")]
    Trash,
    #[serde(rename = "nocopy")]
    NoCopy,
    #[serde(rename = "wear")]
    Wear,
    #[serde(rename = "workspace")]
    Workspace,
    #[serde(rename = "links")]
    Links,
    #[serde(rename = "chat_read")]
    ChatRead,
    #[serde(rename = "chat_state")]
    ChatState,
    #[serde(rename = "chat_send_im")]
    ChatSendIM,
    #[serde(rename = "chat_send_group")]
    ChatSendGroup,
    #[serde(rename = "chat_send_conference")]
    ChatSendConference,
    #[serde(rename = "chat_send_nearby")]
    ChatSendNearby,
    #[serde(rename = "friends_read")]
    FriendsRead,
    #[serde(rename = "resident_search")]
    ResidentSearch,
    #[serde(rename = "profile_read")]
    ProfileRead,
    #[serde(rename = "notifications_read")]
    NotificationsRead,
    #[serde(rename = "notification_review")]
    NotificationReview,
    #[serde(rename = "world_read")]
    WorldRead,
    #[serde(rename = "gesture_read")]
    GestureRead,
    #[serde(rename = "gesture_play")]
    GesturePlay,
    #[serde(rename = "pose_read")]
    PoseRead,
    #[serde(rename = "pose_edit")]
    PoseEdit,
    #[serde(rename = "pose_save")]
    PoseSave,
    #[serde(rename = "pose_share")]
    PoseShare,
    #[serde(rename = "content_edit")]
    ContentEdit,
    #[serde(rename = "preferences_read")]
    PreferencesRead,
    #[serde(rename = "preferences_write")]
    PreferencesWrite,
    #[serde(rename = "build_edit")]
    BuildEdit,
    #[serde(rename = "build_rez")]
    BuildRez,
    #[serde(rename = "inventory_permissions")]
    InventoryPermissions,
    #[serde(rename = "navigation_move")]
    NavigationMove,
    #[serde(rename = "navigation_teleport")]
    NavigationTeleport,
    #[serde(rename = "upload")]
    Upload,
    #[serde(rename = "task_inventory_read")]
    TaskInventoryRead,
    #[serde(rename = "script_debug_read")]
    ScriptDebugRead,
    #[serde(rename = "task_inventory_write")]
    TaskInventoryWrite,
    #[serde(rename = "task_inventory_remove")]
    TaskInventoryRemove,
    #[serde(rename = "script_run")]
    ScriptRun,
    #[serde(rename = "object_take")]
    ObjectTake,
    #[serde(rename = "object_interact")]
    ObjectInteract,
    #[serde(rename = "script_test")]
    ScriptTest,
}
pub struct Definition {
    pub class: Class,
    pub id: &'static str,
    pub title: &'static str,
    pub ask: bool,
    pub private_read: bool,
}
pub const DEFINITIONS: &[Definition] = &[
    Definition {
        class: Class::Read,
        id: "read",
        title: "Read inventory and outfits",
        ask: false,
        private_read: false,
    },
    Definition {
        class: Class::Camera,
        id: "camera",
        title: "Move camera and take pictures",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::Create,
        id: "create",
        title: "Create folders and items",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::Edit,
        id: "edit",
        title: "Rename and edit details",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::Move,
        id: "move",
        title: "Move and copy",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::Trash,
        id: "trash",
        title: "Trash (can be restored)",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::NoCopy,
        id: "nocopy",
        title: "Move no-copy items during a copy",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::Wear,
        id: "wear",
        title: "Wear and detach",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::Workspace,
        id: "workspace",
        title: "Switch workspaces and layouts",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::Links,
        id: "links",
        title: "Replace links",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::ChatRead,
        id: "chat_read",
        title: "Read Chat",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::ChatState,
        id: "chat_state",
        title: "Manage Chat drafts and read state",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::ChatSendIM,
        id: "chat_send_im",
        title: "Send direct IMs",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::ChatSendGroup,
        id: "chat_send_group",
        title: "Send group Chat",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::ChatSendConference,
        id: "chat_send_conference",
        title: "Send conference Chat",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::ChatSendNearby,
        id: "chat_send_nearby",
        title: "Speak in Nearby Chat",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::FriendsRead,
        id: "friends_read",
        title: "Read friends and online status",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::ResidentSearch,
        id: "resident_search",
        title: "Search residents",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::ProfileRead,
        id: "profile_read",
        title: "Read resident profiles",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::NotificationsRead,
        id: "notifications_read",
        title: "Read viewer notifications",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::NotificationReview,
        id: "notification_review",
        title: "Review native notifications",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::WorldRead,
        id: "world_read",
        title: "Read nearby world and media observations",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::GestureRead,
        id: "gesture_read",
        title: "Read gestures and playback state",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::GesturePlay,
        id: "gesture_play",
        title: "Play and stop gestures",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::PoseRead,
        id: "pose_read",
        title: "Read poses and joint state",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::PoseEdit,
        id: "pose_edit",
        title: "Edit your own pose",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::PoseSave,
        id: "pose_save",
        title: "Save pose presets",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::PoseShare,
        id: "pose_share",
        title: "Use owner-approved shared posing",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::ContentEdit,
        id: "content_edit",
        title: "Edit notecard and script contents",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::PreferencesRead,
        id: "preferences_read",
        title: "Read curated viewer preferences",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::PreferencesWrite,
        id: "preferences_write",
        title: "Change curated viewer preferences",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::BuildEdit,
        id: "build_edit",
        title: "Edit owned world objects",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::BuildRez,
        id: "build_rez",
        title: "Rez world objects",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::InventoryPermissions,
        id: "inventory_permissions",
        title: "Change inventory next-owner permissions",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::NavigationMove,
        id: "navigation_move",
        title: "Move your avatar and change flight",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::NavigationTeleport,
        id: "navigation_teleport",
        title: "Teleport your avatar",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::Upload,
        id: "upload",
        title: "Upload local files",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::TaskInventoryRead,
        id: "task_inventory_read",
        title: "Read owned object Contents",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::ScriptDebugRead,
        id: "script_debug_read",
        title: "Read script errors and owner output",
        ask: false,
        private_read: true,
    },
    Definition {
        class: Class::TaskInventoryWrite,
        id: "task_inventory_write",
        title: "Change owned object Contents",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::TaskInventoryRemove,
        id: "task_inventory_remove",
        title: "Permanently remove object Contents",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::ScriptRun,
        id: "script_run",
        title: "Start stop and reset object scripts",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::ObjectTake,
        id: "object_take",
        title: "Take owned objects into inventory",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::ObjectInteract,
        id: "object_interact",
        title: "Touch objects and answer game dialogs",
        ask: true,
        private_read: false,
    },
    Definition {
        class: Class::ScriptTest,
        id: "script_test",
        title: "Run scoped object test recipes",
        ask: true,
        private_read: false,
    },
];
impl Class {
    pub fn definition(self) -> &'static Definition {
        DEFINITIONS.iter().find(|d| d.class == self).expect("closed permission manifest")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    ReadOnly,
    AskBeforeChanges,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    levels: BTreeMap<Class, Level>,
}
impl Default for Permissions {
    fn default() -> Self {
        let mut value = Self { levels: BTreeMap::new() };
        value.preset(Preset::ReadOnly);
        value
    }
}
impl Permissions {
    pub fn level(&self, class: Class) -> Level {
        self.levels.get(&class).copied().unwrap_or(Level::Never)
    }
    pub fn set(&mut self, class: Class, level: Level) -> Result<(), &'static str> {
        if level == Level::Ask && !class.definition().ask {
            return Err("read approval is not supported for this class");
        }
        self.levels.insert(class, level);
        Ok(())
    }
    pub fn preset(&mut self, preset: Preset) {
        if preset == Preset::Custom {
            return;
        }
        for def in DEFINITIONS {
            let level = if def.private_read {
                Level::Never
            } else if def.class == Class::Read {
                Level::Allow
            } else if preset == Preset::ReadOnly {
                Level::Never
            } else if def.ask {
                Level::Ask
            } else {
                Level::Never
            };
            self.levels.insert(def.class, level);
        }
    }
    pub fn validate(&self) -> bool {
        self.levels.len() <= DEFINITIONS.len() && self.levels.iter().all(|(c, l)| *l != Level::Ask || c.definition().ask)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Configuration {
    pub permissions: Permissions,
}
