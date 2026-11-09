//! Closed character tool catalogue inspected at Folderstorm abbe3cb0ca66ee2484f277219f6a27706fab7fc6.
//! A registered name is not a native capability: callers must inspect the
//! explicit capability list produced by the current native adapter.
use super::permissions::Class;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Tool {
    #[serde(rename = "viewer_status")]
    ViewerStatus,
    #[serde(rename = "viewer_list")]
    ViewerList,
    #[serde(rename = "viewer_select")]
    ViewerSelect,
    #[serde(rename = "inventory_get")]
    InventoryGet,
    #[serde(rename = "inventory_list")]
    InventoryList,
    #[serde(rename = "inventory_search")]
    InventorySearch,
    #[serde(rename = "inventory_system_folder")]
    InventorySystemFolder,
    #[serde(rename = "inventory_create_folder")]
    InventoryCreateFolder,
    #[serde(rename = "inventory_move")]
    InventoryMove,
    #[serde(rename = "inventory_rename")]
    InventoryRename,
    #[serde(rename = "inventory_copy")]
    InventoryCopy,
    #[serde(rename = "inventory_confirm_copy")]
    InventoryConfirmCopy,
    #[serde(rename = "assistant_operation_status")]
    AssistantOperationStatus,
    #[serde(rename = "confirm_action")]
    ConfirmAction,
    #[serde(rename = "inventory_types")]
    InventoryTypes,
    #[serde(rename = "inventory_get_many")]
    InventoryGetMany,
    #[serde(rename = "inventory_resolve_path")]
    InventoryResolvePath,
    #[serde(rename = "inventory_protected_folders")]
    InventoryProtectedFolders,
    #[serde(rename = "inventory_changes")]
    InventoryChanges,
    #[serde(rename = "inventory_read_notecard")]
    InventoryReadNotecard,
    #[serde(rename = "inventory_read_script")]
    InventoryReadScript,
    #[serde(rename = "inventory_landmark")]
    InventoryLandmark,
    #[serde(rename = "inventory_set_description")]
    InventorySetDescription,
    #[serde(rename = "inventory_set_thumbnail")]
    InventorySetThumbnail,
    #[serde(rename = "inventory_snapshot_upload")]
    InventorySnapshotUpload,
    #[serde(rename = "inventory_set_favorite")]
    InventorySetFavorite,
    #[serde(rename = "inventory_link")]
    InventoryLink,
    #[serde(rename = "inventory_create_item")]
    InventoryCreateItem,
    #[serde(rename = "inventory_batch_move")]
    InventoryBatchMove,
    #[serde(rename = "inventory_batch_rename")]
    InventoryBatchRename,
    #[serde(rename = "inventory_batch_copy")]
    InventoryBatchCopy,
    #[serde(rename = "inventory_trash")]
    InventoryTrash,
    #[serde(rename = "inventory_restore")]
    InventoryRestore,
    #[serde(rename = "inventory_replace_links")]
    InventoryReplaceLinks,
    #[serde(rename = "inventory_empty_trash")]
    InventoryEmptyTrash,
    #[serde(rename = "inventory_purge")]
    InventoryPurge,
    #[serde(rename = "inventory_preview_batch_rename")]
    InventoryPreviewBatchRename,
    #[serde(rename = "inventory_preview_batch_move")]
    InventoryPreviewBatchMove,
    #[serde(rename = "inventory_execute_plan")]
    InventoryExecutePlan,
    #[serde(rename = "inventory_history")]
    InventoryHistory,
    #[serde(rename = "inventory_preview_undo")]
    InventoryPreviewUndo,
    #[serde(rename = "appearance_outfits")]
    AppearanceOutfits,
    #[serde(rename = "appearance_outfit_items")]
    AppearanceOutfitItems,
    #[serde(rename = "appearance_worn")]
    AppearanceWorn,
    #[serde(rename = "appearance_wear_outfit")]
    AppearanceWearOutfit,
    #[serde(rename = "appearance_wear_items")]
    AppearanceWearItems,
    #[serde(rename = "appearance_detach")]
    AppearanceDetach,
    #[serde(rename = "camera_get")]
    CameraGet,
    #[serde(rename = "camera_set_pose")]
    CameraSetPose,
    #[serde(rename = "camera_set")]
    CameraSet,
    #[serde(rename = "camera_reset")]
    CameraReset,
    #[serde(rename = "camera_snapshot")]
    CameraSnapshot,
    #[serde(rename = "chat_conversations")]
    ChatConversations,
    #[serde(rename = "chat_history")]
    ChatHistory,
    #[serde(rename = "chat_search")]
    ChatSearch,
    #[serde(rename = "friends_list")]
    FriendsList,
    #[serde(rename = "resident_search")]
    ResidentSearch,
    #[serde(rename = "resident_profile")]
    ResidentProfile,
    #[serde(rename = "chat_open_im")]
    ChatOpenIm,
    #[serde(rename = "chat_draft_get")]
    ChatDraftGet,
    #[serde(rename = "chat_draft_set")]
    ChatDraftSet,
    #[serde(rename = "chat_send")]
    ChatSend,
    #[serde(rename = "chat_mark_read")]
    ChatMarkRead,
    #[serde(rename = "chat_close")]
    ChatClose,
    #[serde(rename = "events_poll")]
    EventsPoll,
    #[serde(rename = "notifications_list")]
    NotificationsList,
    #[serde(rename = "notifications_get")]
    NotificationsGet,
    #[serde(rename = "notification_review")]
    NotificationReview,
    #[serde(rename = "world_location")]
    WorldLocation,
    #[serde(rename = "world_parcel")]
    WorldParcel,
    #[serde(rename = "world_nearby_avatars")]
    WorldNearbyAvatars,
    #[serde(rename = "world_nearby_objects")]
    WorldNearbyObjects,
    #[serde(rename = "world_audio_sources")]
    WorldAudioSources,
    #[serde(rename = "world_media_sources")]
    WorldMediaSources,
    #[serde(rename = "inventory_content_get")]
    InventoryContentGet,
    #[serde(rename = "inventory_content_preview")]
    InventoryContentPreview,
    #[serde(rename = "inventory_update_notecard")]
    InventoryUpdateNotecard,
    #[serde(rename = "inventory_save_compile_script")]
    InventorySaveCompileScript,
    #[serde(rename = "inventory_content_operation_status")]
    InventoryContentOperationStatus,
    #[serde(rename = "gesture_list")]
    GestureList,
    #[serde(rename = "gesture_status")]
    GestureStatus,
    #[serde(rename = "gesture_play")]
    GesturePlay,
    #[serde(rename = "gesture_stop")]
    GestureStop,
    #[serde(rename = "preferences_schema")]
    PreferencesSchema,
    #[serde(rename = "preferences_get")]
    PreferencesGet,
    #[serde(rename = "preferences_status")]
    PreferencesStatus,
    #[serde(rename = "preferences_preview")]
    PreferencesPreview,
    #[serde(rename = "preferences_accept")]
    PreferencesAccept,
    #[serde(rename = "preferences_cancel")]
    PreferencesCancel,
    #[serde(rename = "build_object_get")]
    BuildObjectGet,
    #[serde(rename = "build_object_transform")]
    BuildObjectTransform,
    #[serde(rename = "build_object_surface")]
    BuildObjectSurface,
    #[serde(rename = "build_object_material")]
    BuildObjectMaterial,
    #[serde(rename = "build_rez_primitive")]
    BuildRezPrimitive,
    #[serde(rename = "build_rez_inventory")]
    BuildRezInventory,
    #[serde(rename = "inventory_permissions_get")]
    InventoryPermissionsGet,
    #[serde(rename = "inventory_next_owner_set")]
    InventoryNextOwnerSet,
    #[serde(rename = "navigation_status")]
    NavigationStatus,
    #[serde(rename = "navigation_move_to")]
    NavigationMoveTo,
    #[serde(rename = "navigation_flying_set")]
    NavigationFlyingSet,
    #[serde(rename = "navigation_teleport")]
    NavigationTeleport,
    #[serde(rename = "navigation_stop")]
    NavigationStop,
    #[serde(rename = "inventory_upload_preview")]
    InventoryUploadPreview,
    #[serde(rename = "inventory_upload_execute")]
    InventoryUploadExecute,
    #[serde(rename = "inventory_upload_status")]
    InventoryUploadStatus,
    #[serde(rename = "inventory_upload_cancel")]
    InventoryUploadCancel,
    #[serde(rename = "inventory_upload_mesh_review")]
    InventoryUploadMeshReview,
    #[serde(rename = "pose_status")]
    PoseStatus,
    #[serde(rename = "pose_joints")]
    PoseJoints,
    #[serde(rename = "pose_presets")]
    PosePresets,
    #[serde(rename = "pose_start")]
    PoseStart,
    #[serde(rename = "pose_update")]
    PoseUpdate,
    #[serde(rename = "pose_undo")]
    PoseUndo,
    #[serde(rename = "pose_reset")]
    PoseReset,
    #[serde(rename = "pose_stop")]
    PoseStop,
    #[serde(rename = "pose_load")]
    PoseLoad,
    #[serde(rename = "pose_save")]
    PoseSave,
    #[serde(rename = "pose_shared_sessions")]
    PoseSharedSessions,
    #[serde(rename = "pose_shared_grants")]
    PoseSharedGrants,
    #[serde(rename = "pose_shared_joints")]
    PoseSharedJoints,
    #[serde(rename = "pose_shared_update")]
    PoseSharedUpdate,
    #[serde(rename = "pose_shared_cancel")]
    PoseSharedCancel,
    #[serde(rename = "pose_shared_operation_status")]
    PoseSharedOperationStatus,
    #[serde(rename = "pose_start_sharing")]
    PoseStartSharing,
    #[serde(rename = "pose_stop_sharing")]
    PoseStopSharing,
    #[serde(rename = "workspace_list")]
    WorkspaceList,
    #[serde(rename = "workspace_status")]
    WorkspaceStatus,
    #[serde(rename = "workspace_preview")]
    WorkspacePreview,
    #[serde(rename = "workspace_apply")]
    WorkspaceApply,
    #[serde(rename = "workspace_previous")]
    WorkspacePrevious,
    #[serde(rename = "object_contents_list")]
    ObjectContentsList,
    #[serde(rename = "object_script_get")]
    ObjectScriptGet,
    #[serde(rename = "script_debug_poll")]
    ScriptDebugPoll,
    #[serde(rename = "object_contents_put")]
    ObjectContentsPut,
    #[serde(rename = "object_contents_reconcile")]
    ObjectContentsReconcile,
    #[serde(rename = "object_script_preview")]
    ObjectScriptPreview,
    #[serde(rename = "object_script_save_compile")]
    ObjectScriptSaveCompile,
    #[serde(rename = "object_script_status")]
    ObjectScriptStatus,
    #[serde(rename = "object_script_set_running")]
    ObjectScriptSetRunning,
    #[serde(rename = "object_script_reset")]
    ObjectScriptReset,
    #[serde(rename = "object_contents_rename")]
    ObjectContentsRename,
    #[serde(rename = "object_contents_remove")]
    ObjectContentsRemove,
    #[serde(rename = "object_operation_status")]
    ObjectOperationStatus,
    #[serde(rename = "object_take_copy")]
    ObjectTakeCopy,
    #[serde(rename = "object_take")]
    ObjectTake,
    #[serde(rename = "object_rez_identify")]
    ObjectRezIdentify,
    #[serde(rename = "object_touch")]
    ObjectTouch,
    #[serde(rename = "object_dialog_list")]
    ObjectDialogList,
    #[serde(rename = "object_dialog_reply")]
    ObjectDialogReply,
    #[serde(rename = "object_channel_send")]
    ObjectChannelSend,
    #[serde(rename = "script_test_begin")]
    ScriptTestBegin,
    #[serde(rename = "script_test_status")]
    ScriptTestStatus,
    #[serde(rename = "script_test_stop")]
    ScriptTestStop,
}
pub struct Definition {
    pub tool: Tool,
    pub name: &'static str,
    pub class: Option<Class>,
    pub read_only: bool,
    pub active: bool,
    pub group: &'static str,
}
pub const DEFINITIONS: &[Definition] = &[
    Definition {
        tool: Tool::ViewerStatus,
        name: "viewer_status",
        class: None,
        read_only: true,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::ViewerList,
        name: "viewer_list",
        class: None,
        read_only: true,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::ViewerSelect,
        name: "viewer_select",
        class: None,
        read_only: false,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::InventoryGet,
        name: "inventory_get",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::InventoryList,
        name: "inventory_list",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::InventorySearch,
        name: "inventory_search",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::InventorySystemFolder,
        name: "inventory_system_folder",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::InventoryCreateFolder,
        name: "inventory_create_folder",
        class: Some(Class::Create),
        read_only: false,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::InventoryMove,
        name: "inventory_move",
        class: Some(Class::Move),
        read_only: false,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::InventoryRename,
        name: "inventory_rename",
        class: Some(Class::Edit),
        read_only: false,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::InventoryCopy,
        name: "inventory_copy",
        class: Some(Class::Move),
        read_only: false,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::InventoryConfirmCopy,
        name: "inventory_confirm_copy",
        class: Some(Class::NoCopy),
        read_only: false,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::AssistantOperationStatus,
        name: "assistant_operation_status",
        class: None,
        read_only: true,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::ConfirmAction,
        name: "confirm_action",
        class: None,
        read_only: false,
        active: false,
        group: "base",
    },
    Definition {
        tool: Tool::InventoryTypes,
        name: "inventory_types",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryGetMany,
        name: "inventory_get_many",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryResolvePath,
        name: "inventory_resolve_path",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryProtectedFolders,
        name: "inventory_protected_folders",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryChanges,
        name: "inventory_changes",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryReadNotecard,
        name: "inventory_read_notecard",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryReadScript,
        name: "inventory_read_script",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryLandmark,
        name: "inventory_landmark",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventorySetDescription,
        name: "inventory_set_description",
        class: Some(Class::Edit),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventorySetThumbnail,
        name: "inventory_set_thumbnail",
        class: Some(Class::Edit),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventorySnapshotUpload,
        name: "inventory_snapshot_upload",
        class: Some(Class::Camera),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventorySetFavorite,
        name: "inventory_set_favorite",
        class: Some(Class::Edit),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryLink,
        name: "inventory_link",
        class: Some(Class::Create),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryCreateItem,
        name: "inventory_create_item",
        class: Some(Class::Create),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryBatchMove,
        name: "inventory_batch_move",
        class: Some(Class::Move),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryBatchRename,
        name: "inventory_batch_rename",
        class: Some(Class::Edit),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryBatchCopy,
        name: "inventory_batch_copy",
        class: Some(Class::Move),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryTrash,
        name: "inventory_trash",
        class: Some(Class::Trash),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryRestore,
        name: "inventory_restore",
        class: Some(Class::Move),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryReplaceLinks,
        name: "inventory_replace_links",
        class: Some(Class::Links),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryEmptyTrash,
        name: "inventory_empty_trash",
        class: Some(Class::Read),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryPurge,
        name: "inventory_purge",
        class: Some(Class::Read),
        read_only: false,
        active: false,
        group: "inventory",
    },
    Definition {
        tool: Tool::InventoryPreviewBatchRename,
        name: "inventory_preview_batch_rename",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "bulk",
    },
    Definition {
        tool: Tool::InventoryPreviewBatchMove,
        name: "inventory_preview_batch_move",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "bulk",
    },
    Definition {
        tool: Tool::InventoryExecutePlan,
        name: "inventory_execute_plan",
        class: Some(Class::Move),
        read_only: false,
        active: false,
        group: "bulk",
    },
    Definition {
        tool: Tool::InventoryHistory,
        name: "inventory_history",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "bulk",
    },
    Definition {
        tool: Tool::InventoryPreviewUndo,
        name: "inventory_preview_undo",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "bulk",
    },
    Definition {
        tool: Tool::AppearanceOutfits,
        name: "appearance_outfits",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "appearance",
    },
    Definition {
        tool: Tool::AppearanceOutfitItems,
        name: "appearance_outfit_items",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "appearance",
    },
    Definition {
        tool: Tool::AppearanceWorn,
        name: "appearance_worn",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "appearance",
    },
    Definition {
        tool: Tool::AppearanceWearOutfit,
        name: "appearance_wear_outfit",
        class: Some(Class::Wear),
        read_only: false,
        active: false,
        group: "appearance",
    },
    Definition {
        tool: Tool::AppearanceWearItems,
        name: "appearance_wear_items",
        class: Some(Class::Wear),
        read_only: false,
        active: false,
        group: "appearance",
    },
    Definition {
        tool: Tool::AppearanceDetach,
        name: "appearance_detach",
        class: Some(Class::Wear),
        read_only: false,
        active: false,
        group: "appearance",
    },
    Definition {
        tool: Tool::CameraGet,
        name: "camera_get",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "camera",
    },
    Definition {
        tool: Tool::CameraSetPose,
        name: "camera_set_pose",
        class: Some(Class::Camera),
        read_only: false,
        active: true,
        group: "camera",
    },
    Definition {
        tool: Tool::CameraSet,
        name: "camera_set",
        class: Some(Class::Camera),
        read_only: false,
        active: true,
        group: "camera",
    },
    Definition {
        tool: Tool::CameraReset,
        name: "camera_reset",
        class: Some(Class::Camera),
        read_only: false,
        active: true,
        group: "camera",
    },
    Definition {
        tool: Tool::CameraSnapshot,
        name: "camera_snapshot",
        class: Some(Class::Camera),
        read_only: false,
        active: true,
        group: "camera",
    },
    Definition {
        tool: Tool::ChatConversations,
        name: "chat_conversations",
        class: Some(Class::ChatRead),
        read_only: true,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::ChatHistory,
        name: "chat_history",
        class: Some(Class::ChatRead),
        read_only: true,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::ChatSearch,
        name: "chat_search",
        class: Some(Class::ChatRead),
        read_only: true,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::FriendsList,
        name: "friends_list",
        class: Some(Class::FriendsRead),
        read_only: true,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::ResidentSearch,
        name: "resident_search",
        class: Some(Class::ResidentSearch),
        read_only: true,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::ResidentProfile,
        name: "resident_profile",
        class: Some(Class::ProfileRead),
        read_only: true,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::ChatOpenIm,
        name: "chat_open_im",
        class: Some(Class::ChatState),
        read_only: false,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::ChatDraftGet,
        name: "chat_draft_get",
        class: Some(Class::ChatRead),
        read_only: true,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::ChatDraftSet,
        name: "chat_draft_set",
        class: Some(Class::ChatState),
        read_only: false,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::ChatSend,
        name: "chat_send",
        class: None,
        read_only: false,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::ChatMarkRead,
        name: "chat_mark_read",
        class: Some(Class::ChatState),
        read_only: false,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::ChatClose,
        name: "chat_close",
        class: Some(Class::ChatState),
        read_only: false,
        active: false,
        group: "social",
    },
    Definition {
        tool: Tool::EventsPoll,
        name: "events_poll",
        class: None,
        read_only: true,
        active: false,
        group: "events",
    },
    Definition {
        tool: Tool::NotificationsList,
        name: "notifications_list",
        class: Some(Class::NotificationsRead),
        read_only: true,
        active: false,
        group: "events",
    },
    Definition {
        tool: Tool::NotificationsGet,
        name: "notifications_get",
        class: Some(Class::NotificationsRead),
        read_only: true,
        active: false,
        group: "events",
    },
    Definition {
        tool: Tool::NotificationReview,
        name: "notification_review",
        class: Some(Class::NotificationReview),
        read_only: false,
        active: true,
        group: "events",
    },
    Definition {
        tool: Tool::WorldLocation,
        name: "world_location",
        class: Some(Class::WorldRead),
        read_only: true,
        active: false,
        group: "world",
    },
    Definition {
        tool: Tool::WorldParcel,
        name: "world_parcel",
        class: Some(Class::WorldRead),
        read_only: true,
        active: false,
        group: "world",
    },
    Definition {
        tool: Tool::WorldNearbyAvatars,
        name: "world_nearby_avatars",
        class: Some(Class::WorldRead),
        read_only: true,
        active: false,
        group: "world",
    },
    Definition {
        tool: Tool::WorldNearbyObjects,
        name: "world_nearby_objects",
        class: Some(Class::WorldRead),
        read_only: true,
        active: false,
        group: "world",
    },
    Definition {
        tool: Tool::WorldAudioSources,
        name: "world_audio_sources",
        class: Some(Class::WorldRead),
        read_only: true,
        active: false,
        group: "world",
    },
    Definition {
        tool: Tool::WorldMediaSources,
        name: "world_media_sources",
        class: Some(Class::WorldRead),
        read_only: true,
        active: false,
        group: "world",
    },
    Definition {
        tool: Tool::InventoryContentGet,
        name: "inventory_content_get",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "content",
    },
    Definition {
        tool: Tool::InventoryContentPreview,
        name: "inventory_content_preview",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "content",
    },
    Definition {
        tool: Tool::InventoryUpdateNotecard,
        name: "inventory_update_notecard",
        class: Some(Class::ContentEdit),
        read_only: false,
        active: false,
        group: "content",
    },
    Definition {
        tool: Tool::InventorySaveCompileScript,
        name: "inventory_save_compile_script",
        class: Some(Class::ContentEdit),
        read_only: false,
        active: false,
        group: "content",
    },
    Definition {
        tool: Tool::InventoryContentOperationStatus,
        name: "inventory_content_operation_status",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "content",
    },
    Definition {
        tool: Tool::GestureList,
        name: "gesture_list",
        class: Some(Class::GestureRead),
        read_only: true,
        active: false,
        group: "gestures",
    },
    Definition {
        tool: Tool::GestureStatus,
        name: "gesture_status",
        class: Some(Class::GestureRead),
        read_only: true,
        active: false,
        group: "gestures",
    },
    Definition {
        tool: Tool::GesturePlay,
        name: "gesture_play",
        class: Some(Class::GesturePlay),
        read_only: false,
        active: true,
        group: "gestures",
    },
    Definition {
        tool: Tool::GestureStop,
        name: "gesture_stop",
        class: Some(Class::GesturePlay),
        read_only: false,
        active: true,
        group: "gestures",
    },
    Definition {
        tool: Tool::PreferencesSchema,
        name: "preferences_schema",
        class: Some(Class::PreferencesRead),
        read_only: true,
        active: false,
        group: "preferences",
    },
    Definition {
        tool: Tool::PreferencesGet,
        name: "preferences_get",
        class: Some(Class::PreferencesRead),
        read_only: true,
        active: false,
        group: "preferences",
    },
    Definition {
        tool: Tool::PreferencesStatus,
        name: "preferences_status",
        class: Some(Class::PreferencesRead),
        read_only: true,
        active: false,
        group: "preferences",
    },
    Definition {
        tool: Tool::PreferencesPreview,
        name: "preferences_preview",
        class: Some(Class::PreferencesWrite),
        read_only: false,
        active: false,
        group: "preferences",
    },
    Definition {
        tool: Tool::PreferencesAccept,
        name: "preferences_accept",
        class: Some(Class::PreferencesWrite),
        read_only: false,
        active: false,
        group: "preferences",
    },
    Definition {
        tool: Tool::PreferencesCancel,
        name: "preferences_cancel",
        class: Some(Class::PreferencesWrite),
        read_only: false,
        active: false,
        group: "preferences",
    },
    Definition {
        tool: Tool::BuildObjectGet,
        name: "build_object_get",
        class: Some(Class::WorldRead),
        read_only: true,
        active: false,
        group: "building",
    },
    Definition {
        tool: Tool::BuildObjectTransform,
        name: "build_object_transform",
        class: Some(Class::BuildEdit),
        read_only: false,
        active: true,
        group: "building",
    },
    Definition {
        tool: Tool::BuildObjectSurface,
        name: "build_object_surface",
        class: Some(Class::BuildEdit),
        read_only: false,
        active: true,
        group: "building",
    },
    Definition {
        tool: Tool::BuildObjectMaterial,
        name: "build_object_material",
        class: Some(Class::BuildEdit),
        read_only: false,
        active: true,
        group: "building",
    },
    Definition {
        tool: Tool::BuildRezPrimitive,
        name: "build_rez_primitive",
        class: Some(Class::BuildRez),
        read_only: false,
        active: true,
        group: "building",
    },
    Definition {
        tool: Tool::BuildRezInventory,
        name: "build_rez_inventory",
        class: Some(Class::BuildRez),
        read_only: false,
        active: true,
        group: "building",
    },
    Definition {
        tool: Tool::InventoryPermissionsGet,
        name: "inventory_permissions_get",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "building",
    },
    Definition {
        tool: Tool::InventoryNextOwnerSet,
        name: "inventory_next_owner_set",
        class: Some(Class::InventoryPermissions),
        read_only: false,
        active: false,
        group: "building",
    },
    Definition {
        tool: Tool::NavigationStatus,
        name: "navigation_status",
        class: Some(Class::WorldRead),
        read_only: true,
        active: false,
        group: "navigation",
    },
    Definition {
        tool: Tool::NavigationMoveTo,
        name: "navigation_move_to",
        class: Some(Class::NavigationMove),
        read_only: false,
        active: true,
        group: "navigation",
    },
    Definition {
        tool: Tool::NavigationFlyingSet,
        name: "navigation_flying_set",
        class: Some(Class::NavigationMove),
        read_only: false,
        active: true,
        group: "navigation",
    },
    Definition {
        tool: Tool::NavigationTeleport,
        name: "navigation_teleport",
        class: Some(Class::NavigationTeleport),
        read_only: false,
        active: true,
        group: "navigation",
    },
    Definition {
        tool: Tool::NavigationStop,
        name: "navigation_stop",
        class: Some(Class::WorldRead),
        read_only: false,
        active: false,
        group: "navigation",
    },
    Definition {
        tool: Tool::InventoryUploadPreview,
        name: "inventory_upload_preview",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "uploads",
    },
    Definition {
        tool: Tool::InventoryUploadExecute,
        name: "inventory_upload_execute",
        class: Some(Class::Upload),
        read_only: false,
        active: false,
        group: "uploads",
    },
    Definition {
        tool: Tool::InventoryUploadStatus,
        name: "inventory_upload_status",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "uploads",
    },
    Definition {
        tool: Tool::InventoryUploadCancel,
        name: "inventory_upload_cancel",
        class: Some(Class::Read),
        read_only: false,
        active: false,
        group: "uploads",
    },
    Definition {
        tool: Tool::InventoryUploadMeshReview,
        name: "inventory_upload_mesh_review",
        class: Some(Class::Upload),
        read_only: false,
        active: true,
        group: "uploads",
    },
    Definition {
        tool: Tool::PoseStatus,
        name: "pose_status",
        class: Some(Class::PoseRead),
        read_only: true,
        active: false,
        group: "pose",
    },
    Definition {
        tool: Tool::PoseJoints,
        name: "pose_joints",
        class: Some(Class::PoseRead),
        read_only: true,
        active: false,
        group: "pose",
    },
    Definition {
        tool: Tool::PosePresets,
        name: "pose_presets",
        class: Some(Class::PoseRead),
        read_only: true,
        active: false,
        group: "pose",
    },
    Definition {
        tool: Tool::PoseStart,
        name: "pose_start",
        class: Some(Class::PoseEdit),
        read_only: false,
        active: true,
        group: "pose",
    },
    Definition {
        tool: Tool::PoseUpdate,
        name: "pose_update",
        class: Some(Class::PoseEdit),
        read_only: false,
        active: true,
        group: "pose",
    },
    Definition {
        tool: Tool::PoseUndo,
        name: "pose_undo",
        class: Some(Class::PoseEdit),
        read_only: false,
        active: true,
        group: "pose",
    },
    Definition {
        tool: Tool::PoseReset,
        name: "pose_reset",
        class: Some(Class::PoseEdit),
        read_only: false,
        active: true,
        group: "pose",
    },
    Definition {
        tool: Tool::PoseStop,
        name: "pose_stop",
        class: Some(Class::PoseEdit),
        read_only: false,
        active: true,
        group: "pose",
    },
    Definition {
        tool: Tool::PoseLoad,
        name: "pose_load",
        class: Some(Class::PoseEdit),
        read_only: false,
        active: true,
        group: "pose",
    },
    Definition {
        tool: Tool::PoseSave,
        name: "pose_save",
        class: Some(Class::PoseSave),
        read_only: false,
        active: true,
        group: "pose",
    },
    Definition {
        tool: Tool::PoseSharedSessions,
        name: "pose_shared_sessions",
        class: Some(Class::PoseRead),
        read_only: true,
        active: false,
        group: "shared_pose",
    },
    Definition {
        tool: Tool::PoseSharedGrants,
        name: "pose_shared_grants",
        class: Some(Class::PoseRead),
        read_only: true,
        active: false,
        group: "shared_pose",
    },
    Definition {
        tool: Tool::PoseSharedJoints,
        name: "pose_shared_joints",
        class: Some(Class::PoseRead),
        read_only: true,
        active: false,
        group: "shared_pose",
    },
    Definition {
        tool: Tool::PoseSharedUpdate,
        name: "pose_shared_update",
        class: Some(Class::PoseShare),
        read_only: false,
        active: true,
        group: "shared_pose",
    },
    Definition {
        tool: Tool::PoseSharedCancel,
        name: "pose_shared_cancel",
        class: Some(Class::PoseShare),
        read_only: false,
        active: true,
        group: "shared_pose",
    },
    Definition {
        tool: Tool::PoseSharedOperationStatus,
        name: "pose_shared_operation_status",
        class: Some(Class::PoseRead),
        read_only: true,
        active: false,
        group: "shared_pose",
    },
    Definition {
        tool: Tool::PoseStartSharing,
        name: "pose_start_sharing",
        class: Some(Class::PoseShare),
        read_only: false,
        active: true,
        group: "shared_pose",
    },
    Definition {
        tool: Tool::PoseStopSharing,
        name: "pose_stop_sharing",
        class: Some(Class::PoseShare),
        read_only: false,
        active: true,
        group: "shared_pose",
    },
    Definition {
        tool: Tool::WorkspaceList,
        name: "workspace_list",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "workspace",
    },
    Definition {
        tool: Tool::WorkspaceStatus,
        name: "workspace_status",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "workspace",
    },
    Definition {
        tool: Tool::WorkspacePreview,
        name: "workspace_preview",
        class: Some(Class::Read),
        read_only: true,
        active: false,
        group: "workspace",
    },
    Definition {
        tool: Tool::WorkspaceApply,
        name: "workspace_apply",
        class: Some(Class::Workspace),
        read_only: false,
        active: true,
        group: "workspace",
    },
    Definition {
        tool: Tool::WorkspacePrevious,
        name: "workspace_previous",
        class: Some(Class::Workspace),
        read_only: false,
        active: true,
        group: "workspace",
    },
    Definition {
        tool: Tool::ObjectContentsList,
        name: "object_contents_list",
        class: Some(Class::TaskInventoryRead),
        read_only: true,
        active: false,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectScriptGet,
        name: "object_script_get",
        class: Some(Class::TaskInventoryRead),
        read_only: true,
        active: false,
        group: "objects",
    },
    Definition {
        tool: Tool::ScriptDebugPoll,
        name: "script_debug_poll",
        class: Some(Class::ScriptDebugRead),
        read_only: true,
        active: false,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectContentsPut,
        name: "object_contents_put",
        class: Some(Class::TaskInventoryWrite),
        read_only: false,
        active: true,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectContentsReconcile,
        name: "object_contents_reconcile",
        class: Some(Class::TaskInventoryWrite),
        read_only: false,
        active: true,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectScriptPreview,
        name: "object_script_preview",
        class: Some(Class::TaskInventoryRead),
        read_only: true,
        active: false,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectScriptSaveCompile,
        name: "object_script_save_compile",
        class: Some(Class::ContentEdit),
        read_only: false,
        active: true,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectScriptStatus,
        name: "object_script_status",
        class: Some(Class::TaskInventoryRead),
        read_only: true,
        active: false,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectScriptSetRunning,
        name: "object_script_set_running",
        class: Some(Class::ScriptRun),
        read_only: false,
        active: true,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectScriptReset,
        name: "object_script_reset",
        class: Some(Class::ScriptRun),
        read_only: false,
        active: true,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectContentsRename,
        name: "object_contents_rename",
        class: Some(Class::TaskInventoryWrite),
        read_only: false,
        active: true,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectContentsRemove,
        name: "object_contents_remove",
        class: Some(Class::TaskInventoryRemove),
        read_only: false,
        active: true,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectOperationStatus,
        name: "object_operation_status",
        class: Some(Class::TaskInventoryRead),
        read_only: true,
        active: false,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectTakeCopy,
        name: "object_take_copy",
        class: Some(Class::ObjectTake),
        read_only: false,
        active: true,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectTake,
        name: "object_take",
        class: Some(Class::ObjectTake),
        read_only: false,
        active: true,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectRezIdentify,
        name: "object_rez_identify",
        class: Some(Class::BuildRez),
        read_only: false,
        active: true,
        group: "objects",
    },
    Definition {
        tool: Tool::ObjectTouch,
        name: "object_touch",
        class: Some(Class::ObjectInteract),
        read_only: false,
        active: true,
        group: "testing",
    },
    Definition {
        tool: Tool::ObjectDialogList,
        name: "object_dialog_list",
        class: Some(Class::NotificationsRead),
        read_only: true,
        active: false,
        group: "testing",
    },
    Definition {
        tool: Tool::ObjectDialogReply,
        name: "object_dialog_reply",
        class: Some(Class::ObjectInteract),
        read_only: false,
        active: true,
        group: "testing",
    },
    Definition {
        tool: Tool::ObjectChannelSend,
        name: "object_channel_send",
        class: Some(Class::ObjectInteract),
        read_only: false,
        active: true,
        group: "testing",
    },
    Definition {
        tool: Tool::ScriptTestBegin,
        name: "script_test_begin",
        class: Some(Class::ScriptTest),
        read_only: false,
        active: true,
        group: "testing",
    },
    Definition {
        tool: Tool::ScriptTestStatus,
        name: "script_test_status",
        class: Some(Class::WorldRead),
        read_only: true,
        active: false,
        group: "testing",
    },
    Definition {
        tool: Tool::ScriptTestStop,
        name: "script_test_stop",
        class: None,
        read_only: false,
        active: false,
        group: "testing",
    },
];
impl Tool {
    pub fn definition(self) -> &'static Definition {
        DEFINITIONS.iter().find(|d| d.tool == self).expect("closed tool manifest")
    }
    pub fn parse(name: &str) -> Option<Self> {
        DEFINITIONS.iter().find(|d| d.name == name).map(|d| d.tool)
    }
    pub fn permanently_denied(self) -> bool {
        matches!(self, Self::InventoryPurge | Self::InventoryEmptyTrash)
    }
    pub fn native_confirmation_only(self) -> bool {
        self == Self::ConfirmAction
    }
}
