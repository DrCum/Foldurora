//! Bounded, closed selective-profile data projection. Behavior follows the explicit
//! native preference groups/default filling in fssessionprofileseed.h and
//! tools/session-host/main.cpp::selectProfileCopy (pinned abbe3, LGPL2.1).
//! No profile path/writer, login, or Settings load/save effect is available.
//! Native default construction may lazily query the keyboard layout; the real
//! adapter must acquire defaults off the GUI/control pump under its worker lease.
//! Unknown unselected JSON VALUES survive successful writes; refusals emit no
//! replacement bytes. This prerequisite is test-only until its real IO consumer.
use super::{ChatStyle, ColorSettings, FavoritesAccess, NotificationsStyle, Rgba, Settings, WorldMapStyle, presentation};
use crate::keybinds::{Action, Binding, Input};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::io::Write;

mod json;
#[cfg(test)]
mod tests;

pub(crate) const VERSION: u32 = super::SETTINGS_VERSION;
pub(crate) const MAX_FILE: usize = 4 * 1024 * 1024;
pub(crate) const MAX_PAYLOAD: usize = 256 * 1024;
const MAX_DEPTH: usize = 32;
const MAX_NODES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Families,
    Bound,
    Json,
    Version,
    Selected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    Graphics,
    Interface,
    Colors,
    Bindings,
}
impl Family {
    pub(crate) const ALL: [Self; 4] = [Self::Graphics, Self::Interface, Self::Colors, Self::Bindings];
    fn bit(self) -> u8 {
        1 << self as u8
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Families(u8);
impl Families {
    pub(crate) fn new(bits: u8) -> Result<Self, Error> {
        if bits == 0 || bits & !15 != 0 {
            Err(Error::Families)
        } else {
            Ok(Self(bits))
        }
    }
    pub(crate) fn contains(self, family: Family) -> bool {
        self.0 & family.bit() != 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    pub family: Family,
    pub fields: usize,
    pub defaulted: usize,
    pub changed: usize,
}

// Explicit field lists intentionally do not grow when Settings gains fields.
// Defaults come from the actual native defaults; no whole Settings file is
// deserialized/serialized or retained by this projection.
macro_rules! fields {
    ($name:ident, $native:ty, $defaults:expr, {$($field:ident: $ty:ty),+ $(,)?}) => {
        #[derive(Serialize, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct $name { $($field: $ty),+ }
        impl Default for $name {
            fn default() -> Self {
                let native: $native = $defaults;
                Self { $($field: native.$field),+ }
            }
        }
        impl $name { const FIELDS: &'static [&'static str] = &[$(stringify!($field)),+]; }
    };
}
fields!(Graphics, Settings, Settings::default(), {
    draw_distance: f32,
    vsync: bool,
    shadows: bool,
    exposure: f32,
    sharpen: f32,
    glow: bool,
    bloom: bool,
    bloom_strength: f32,
    occlusion_culling: bool,
    lod_factor: f32,
    texture_budget_auto: bool,
    texture_budget_mb: u32,
    max_texture_resolution: u32,
    shadow_quality: u8,
    antialiasing: u8,
    render_scale_percent: u32,
});
fields!(Interface, Settings, Settings::default(), {
    ui_scale: f32,
    font: String,
    font_scale: f32,
    chat_font_scale: f32,
    use_pie_menu: bool,
    snap_windows: bool,
    classic_bars: bool,
    place_bar_stretch: bool,
    place_bar_rights_text: bool,
    favorites_access: FavoritesAccess,
    favorites_row_folded: bool,
    classic_profiles: bool,
    classic_people: bool,
    classic_inventory: bool,
    classic_places: bool,
    chat_style: ChatStyle,
    notifications_style: NotificationsStyle,
    world_map_style: WorldMapStyle,
});
fields!(Colors, ColorSettings, ColorSettings::default(), {
    relation_names: bool,
    tag_me: Rgba,
    tag_friend: Rgba,
    tag_muted: Rgba,
    tag_linden: Rgba,
    tag_match: Rgba,
    tag_mismatch: Rgba,
    map_me: Rgba,
    map_other: Rgba,
    map_friend: Rgba,
    map_linden: Rgba,
    map_muted: Rgba,
    map_ranges: bool,
    map_whisper_on: bool,
    map_say_on: bool,
    map_shout_on: bool,
    map_whisper: Rgba,
    map_say: Rgba,
    map_shout: Rgba,
    chat_mine: Rgba,
    chat_others: Rgba,
    chat_objects: Rgba,
    chat_friends: Rgba,
    chat_linden: Rgba,
    chat_muted: Rgba,
    chat_system: Rgba,
    chat_errors: Rgba,
    chat_object_im: Rgba,
    chat_owner: Rgba,
    chat_urls: Rgba,
    chat_slurl: Rgba,
    chat_direct: Rgba,
    sender_avatar: Rgba,
    sender_object: Rgba,
    mention_text: Rgba,
    mention_residents: Rgba,
    mention_me: Rgba,
});

impl Graphics {
    fn valid(&self) -> bool {
        let same = |value: f32, normalized: f32| value.is_finite() && value == normalized;
        same(self.draw_distance, presentation::draw_distance(self.draw_distance))
            && same(self.exposure, presentation::exposure(self.exposure))
            && same(self.sharpen, presentation::sharpen(self.sharpen))
            && same(self.bloom_strength, presentation::bloom_strength(self.bloom_strength))
            && same(self.lod_factor, presentation::lod_factor(self.lod_factor))
            && self.texture_budget_mb == presentation::texture_budget_mb(self.texture_budget_mb)
            && self.max_texture_resolution == super::max_texture_resolution_choice(self.max_texture_resolution)
            && self.shadow_quality == presentation::shadow_quality(self.shadow_quality)
            && self.antialiasing == presentation::antialiasing(self.antialiasing)
            && self.render_scale_percent == presentation::render_scale_percent(self.render_scale_percent)
    }
}
impl Interface {
    fn valid(&self) -> bool {
        self.ui_scale.is_finite()
            && self.ui_scale == presentation::ui_scale(self.ui_scale)
            && self.font_scale.is_finite()
            && self.font_scale == presentation::font_scale(self.font_scale)
            && self.chat_font_scale.is_finite()
            && self.chat_font_scale == presentation::chat_font_scale(self.chat_font_scale)
            && self.font.len() <= 128
            && !self.font.chars().any(char::is_control)
    }
}

#[derive(PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedBinding {
    input: Input,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    shift: bool,
    #[serde(default)]
    alt: bool,
}
impl From<Binding> for SeedBinding {
    fn from(binding: Binding) -> Self {
        Self {
            input: binding.input,
            ctrl: binding.ctrl,
            shift: binding.shift,
            alt: binding.alt,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Bindings {
    map: Vec<(Action, [Option<SeedBinding>; 2])>,
}
impl Default for Bindings {
    fn default() -> Self {
        Self {
            map: Settings::default()
                .keybinds
                .map
                .into_iter()
                .map(|(action, bindings)| (action, bindings.map(|b| b.map(Into::into))))
                .collect(),
        }
    }
}
impl Bindings {
    fn finish(&mut self) -> Result<(), Error> {
        let known: HashSet<_> = crate::keybinds::sections()
            .iter()
            .flat_map(|(_, actions)| actions.iter().map(|(action, _)| *action))
            .collect();
        if self.map.len() > known.len() {
            return Err(Error::Selected);
        }
        let mut seen = HashSet::new();
        for (action, bindings) in &self.map {
            if !known.contains(action) || !seen.insert(*action) {
                return Err(Error::Selected);
            }
            for binding in bindings.iter().flatten() {
                let valid = match &binding.input {
                    Input::Key(key) => known_key(key),
                    Input::Mouse(token) => {
                        matches!(token.as_str(), "Middle" | "Back" | "Forward")
                            || token
                                .strip_prefix("Other")
                                .and_then(|n| n.parse::<u16>().ok())
                                .is_some_and(|n| *token == format!("Other{n}"))
                    }
                };
                if !valid {
                    return Err(Error::Selected);
                }
            }
        }
        for (action, bindings) in Self::default().map {
            if seen.insert(action) {
                self.map.push((action, bindings));
            }
        }
        Ok(())
    }
}

// Current native physical-key names, pinned to winit0.30.13 KeyCode.
// The ordinary reader keeps unknown strings; selected profile copying refuses
// unsupported input instead of silently making a different native binding.
fn known_key(key: &str) -> bool {
    matches!(
        key,
        "Backquote"
            | "Backslash"
            | "BracketLeft"
            | "BracketRight"
            | "Comma"
            | "Digit0"
            | "Digit1"
            | "Digit2"
            | "Digit3"
            | "Digit4"
            | "Digit5"
            | "Digit6"
            | "Digit7"
            | "Digit8"
            | "Digit9"
            | "Equal"
            | "IntlBackslash"
            | "IntlRo"
            | "IntlYen"
            | "KeyA"
            | "KeyB"
            | "KeyC"
            | "KeyD"
            | "KeyE"
            | "KeyF"
            | "KeyG"
            | "KeyH"
            | "KeyI"
            | "KeyJ"
            | "KeyK"
            | "KeyL"
            | "KeyM"
            | "KeyN"
            | "KeyO"
            | "KeyP"
            | "KeyQ"
            | "KeyR"
            | "KeyS"
            | "KeyT"
            | "KeyU"
            | "KeyV"
            | "KeyW"
            | "KeyX"
            | "KeyY"
            | "KeyZ"
            | "Minus"
            | "Period"
            | "Quote"
            | "Semicolon"
            | "Slash"
            | "AltLeft"
            | "AltRight"
            | "Backspace"
            | "CapsLock"
            | "ContextMenu"
            | "ControlLeft"
            | "ControlRight"
            | "Enter"
            | "SuperLeft"
            | "SuperRight"
            | "ShiftLeft"
            | "ShiftRight"
            | "Space"
            | "Tab"
            | "Convert"
            | "KanaMode"
            | "Lang1"
            | "Lang2"
            | "Lang3"
            | "Lang4"
            | "Lang5"
            | "NonConvert"
            | "Delete"
            | "End"
            | "Help"
            | "Home"
            | "Insert"
            | "PageDown"
            | "PageUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "ArrowUp"
            | "NumLock"
            | "Numpad0"
            | "Numpad1"
            | "Numpad2"
            | "Numpad3"
            | "Numpad4"
            | "Numpad5"
            | "Numpad6"
            | "Numpad7"
            | "Numpad8"
            | "Numpad9"
            | "NumpadAdd"
            | "NumpadBackspace"
            | "NumpadClear"
            | "NumpadClearEntry"
            | "NumpadComma"
            | "NumpadDecimal"
            | "NumpadDivide"
            | "NumpadEnter"
            | "NumpadEqual"
            | "NumpadHash"
            | "NumpadMemoryAdd"
            | "NumpadMemoryClear"
            | "NumpadMemoryRecall"
            | "NumpadMemoryStore"
            | "NumpadMemorySubtract"
            | "NumpadMultiply"
            | "NumpadParenLeft"
            | "NumpadParenRight"
            | "NumpadStar"
            | "NumpadSubtract"
            | "Escape"
            | "Fn"
            | "FnLock"
            | "PrintScreen"
            | "ScrollLock"
            | "Pause"
            | "BrowserBack"
            | "BrowserFavorites"
            | "BrowserForward"
            | "BrowserHome"
            | "BrowserRefresh"
            | "BrowserSearch"
            | "BrowserStop"
            | "Eject"
            | "LaunchApp1"
            | "LaunchApp2"
            | "LaunchMail"
            | "MediaPlayPause"
            | "MediaSelect"
            | "MediaStop"
            | "MediaTrackNext"
            | "MediaTrackPrevious"
            | "Power"
            | "Sleep"
            | "AudioVolumeDown"
            | "AudioVolumeMute"
            | "AudioVolumeUp"
            | "WakeUp"
            | "Meta"
            | "Hyper"
            | "Turbo"
            | "Abort"
            | "Resume"
            | "Suspend"
            | "Again"
            | "Copy"
            | "Cut"
            | "Find"
            | "Open"
            | "Paste"
            | "Props"
            | "Select"
            | "Undo"
            | "Hiragana"
            | "Katakana"
            | "F1"
            | "F2"
            | "F3"
            | "F4"
            | "F5"
            | "F6"
            | "F7"
            | "F8"
            | "F9"
            | "F10"
            | "F11"
            | "F12"
            | "F13"
            | "F14"
            | "F15"
            | "F16"
            | "F17"
            | "F18"
            | "F19"
            | "F20"
            | "F21"
            | "F22"
            | "F23"
            | "F24"
            | "F25"
            | "F26"
            | "F27"
            | "F28"
            | "F29"
            | "F30"
            | "F31"
            | "F32"
            | "F33"
            | "F34"
            | "F35"
    )
}

struct Selection {
    graphics: Option<Graphics>,
    interface: Option<Interface>,
    colors: Option<Colors>,
    bindings: Option<Bindings>,
}
impl Selection {
    fn from_map(map: &json::Document, families: Families) -> Result<Self, Error> {
        fn top<T: serde::de::DeserializeOwned>(map: &json::Document, fields: &[&str]) -> Result<T, Error> {
            let selected = fields.iter().filter_map(|key| map.get(*key).map(|raw| (*key, raw.as_ref())));
            T::deserialize(serde::de::value::MapDeserializer::<_, serde_json::Error>::new(selected)).map_err(|_| Error::Selected)
        }
        fn nested<T: serde::de::DeserializeOwned + Default>(map: &json::Document, key: &str) -> Result<T, Error> {
            map.get(key).map_or_else(
                || Ok(T::default()),
                |raw| serde_json::from_str(raw.get()).map_err(|_| Error::Selected),
            )
        }
        let graphics: Option<Graphics> = families
            .contains(Family::Graphics)
            .then(|| top(map, Graphics::FIELDS))
            .transpose()?;
        let interface: Option<Interface> = families
            .contains(Family::Interface)
            .then(|| top(map, Interface::FIELDS))
            .transpose()?;
        let colors = families.contains(Family::Colors).then(|| nested(map, "colors")).transpose()?;
        let mut bindings: Option<Bindings> = families.contains(Family::Bindings).then(|| nested(map, "keybinds")).transpose()?;
        if graphics.as_ref().is_some_and(|g| !g.valid()) || interface.as_ref().is_some_and(|i| !i.valid()) {
            return Err(Error::Selected);
        }
        if let Some(bindings) = &mut bindings {
            bindings.finish()?;
        }
        Ok(Self {
            graphics,
            interface,
            colors,
            bindings,
        })
    }
    fn values(&self) -> Result<Map<String, Value>, Error> {
        let mut result = Map::new();
        for value in [
            self.graphics.as_ref().map(serde_json::to_value),
            self.interface.as_ref().map(serde_json::to_value),
        ]
        .into_iter()
        .flatten()
        {
            let Value::Object(fields) = value.map_err(|_| Error::Selected)? else {
                return Err(Error::Selected);
            };
            result.extend(fields);
        }
        if let Some(colors) = &self.colors {
            result.insert("colors".into(), serde_json::to_value(colors).map_err(|_| Error::Selected)?);
        }
        if let Some(bindings) = &self.bindings {
            result.insert("keybinds".into(), serde_json::to_value(bindings).map_err(|_| Error::Selected)?);
        }
        Ok(result)
    }
}
/// Contains only validated allow-listed typed values, never raw source JSON,
/// profile paths, account identity, credentials, unknown fields or authority.
pub(crate) struct ProfileSeed {
    families: Families,
    selected: Selection,
    summaries: Vec<Summary>,
}
impl std::fmt::Debug for ProfileSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileSeed")
            .field("families", &self.families)
            .field("summary", &self.summaries)
            .finish_non_exhaustive()
    }
}
impl ProfileSeed {
    pub(crate) fn read(source: &[u8], families: Families) -> Result<Self, Error> {
        let map = json::object(source)?;
        let selected = Selection::from_map(&map, families)?;
        let payload = selected.values()?;
        drop(bounded_bytes(&Value::Object(payload), MAX_PAYLOAD)?);
        let mut summaries = Vec::new();
        for family in Family::ALL.into_iter().filter(|family| families.contains(*family)) {
            let (fields, defaulted) = match family {
                Family::Graphics => (
                    Graphics::FIELDS.len(),
                    Graphics::FIELDS.iter().filter(|key| !map.contains_key(**key)).count(),
                ),
                Family::Interface => (
                    Interface::FIELDS.len(),
                    Interface::FIELDS.iter().filter(|key| !map.contains_key(**key)).count(),
                ),
                Family::Colors => {
                    let members = map.get("colors").map(|raw| json::members(raw)).transpose()?;
                    (
                        Colors::FIELDS.len(),
                        Colors::FIELDS
                            .iter()
                            .filter(|key| members.as_ref().is_none_or(|m| !m.contains_key(**key)))
                            .count(),
                    )
                }
                Family::Bindings => {
                    let defaults = Bindings::default().map.len();
                    let members = map.get("keybinds").map(|raw| json::members(raw)).transpose()?;
                    let present = members
                        .as_ref()
                        .and_then(|m| m.get("map"))
                        .map(|raw| serde_json::from_str::<Vec<&serde_json::value::RawValue>>(raw.get()).map_err(|_| Error::Selected))
                        .transpose()?
                        .map_or(0, |rows| rows.len());
                    (defaults, defaults.saturating_sub(present))
                }
            };
            summaries.push(Summary {
                family,
                fields,
                defaulted,
                changed: 0,
            });
        }
        Ok(Self {
            families,
            selected,
            summaries,
        })
    }
    pub(crate) fn summary(&self) -> &[Summary] {
        &self.summaries
    }
    /// Data preparation only: native defaults may query the keyboard layout.
    /// The future adapter must acquire defaults off its GUI/control pump and own profile
    /// locks, original bytes, cancellation and exact Apply transaction guards.
    /// A refusal returns no output and never modifies either input byte slice.
    pub(crate) fn patch(&self, target: Option<&[u8]>) -> Result<(Vec<u8>, Vec<Summary>), Error> {
        let mut map = match target {
            Some(bytes) => json::object(bytes)?,
            None => json::Document::from_iter([(
                "settings_version".into(),
                serde_json::value::to_raw_value(&VERSION).map_err(|_| Error::Version)?,
            )]),
        };
        let previous = Selection::from_map(&map, self.families)?.values()?;
        let next = self.selected.values()?;
        let mut summaries = self.summaries.clone();
        for summary in &mut summaries {
            summary.changed = match summary.family {
                Family::Graphics => Graphics::FIELDS.iter().filter(|key| previous.get(**key) != next.get(**key)).count(),
                Family::Interface => Interface::FIELDS
                    .iter()
                    .filter(|key| previous.get(**key) != next.get(**key))
                    .count(),
                Family::Colors => Colors::FIELDS
                    .iter()
                    .filter(|key| previous.get("colors").and_then(|v| v.get(**key)) != next.get("colors").and_then(|v| v.get(**key)))
                    .count(),
                Family::Bindings => {
                    let old: Bindings = serde_json::from_value(previous["keybinds"].clone()).map_err(|_| Error::Selected)?;
                    let new: Bindings = serde_json::from_value(next["keybinds"].clone()).map_err(|_| Error::Selected)?;
                    new.map
                        .iter()
                        .filter(|(action, bindings)| old.map.iter().find(|(a, _)| a == action).is_none_or(|(_, prior)| prior != bindings))
                        .count()
                }
            };
        }
        for (key, value) in next {
            map.insert(key, serde_json::value::to_raw_value(&value).map_err(|_| Error::Selected)?);
        }
        Ok((bounded_bytes(&map, MAX_FILE)?, summaries))
    }
}
struct BoundedBytes {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedBytes {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        if input.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        let required = self.bytes.len() + input.len();
        if required > self.bytes.capacity() {
            // Bound Vec capacity as well as logical payload, even when one
            // large unselected string is followed by small serialized fields.
            let capacity = required.max(self.bytes.capacity().saturating_mul(2)).max(64).min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::OutOfMemory))?;
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bounded_bytes<T: Serialize + ?Sized>(value: &T, limit: usize) -> Result<Vec<u8>, Error> {
    let mut writer = BoundedBytes { bytes: Vec::new(), limit };
    serde_json::to_writer(&mut writer, value).map_err(|_| Error::Bound)?;
    Ok(writer.bytes)
}
