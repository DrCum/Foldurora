//! Immutable local-metadata namespace from an ACTUAL submitted LoginRequest.
//! This conservative local receipt is not login/session authority. No raw URI,
//! credentials or username survives submission. World owns one pending origin;
//! accepted login consumers copy its digest independently of their IO state.

use sha2::{Digest, Sha256};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GridDigest([u8; 32]);
impl GridDigest {
    fn parse(uri: &str) -> Option<Self> {
        let uri = uri.trim();
        (!uri.is_empty() && uri.len() <= 4096 && !uri.chars().any(char::is_control)).then(|| Self(Sha256::digest(uri.as_bytes()).into()))
    }
    /// Closed native stores use only this digest, never a mutable form value.
    pub(crate) fn namespace(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
    pub(crate) fn bytes(self) -> [u8; 32] {
        self.0
    }
    pub(crate) fn from_uri(uri: &str) -> Option<Self> {
        Self::parse(uri)
    }
}
impl std::fmt::Debug for GridDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GridDigest([redacted])")
    }
}

#[derive(Default)]
enum State {
    #[default]
    Empty,
    Submitted(GridDigest),
    Ambiguous,
}
#[derive(Default)]
pub(crate) struct Pending {
    state: State,
}
impl Pending {
    /// Call immediately before moving the built request to NetCommand::Login.
    /// A differing later request may be ignored by Net; it cannot redirect the
    /// first receipt, nor can another matching request clear ambiguity.
    pub(crate) fn submitted(&mut self, request: &aurora_net::LoginRequest) {
        let Some(grid) = GridDigest::parse(&request.login_uri) else {
            self.state = State::Ambiguous;
            return;
        };
        match &self.state {
            State::Empty => self.state = State::Submitted(grid),
            State::Submitted(original) if *original == grid => {}
            State::Submitted(_) | State::Ambiguous => self.state = State::Ambiguous,
        }
    }
    /// Consume once at actual LoggedIn; account/session come from that event.
    /// A missing/ambiguous result has no mutable-settings fallback.
    pub(crate) fn completed(&mut self) -> Option<GridDigest> {
        match std::mem::take(&mut self.state) {
            State::Submitted(grid) => Some(grid),
            _ => None,
        }
    }
    /// LoginFailed calls this BEFORE any early retry submits its own request.
    pub(crate) fn failed(&mut self) {
        self.state = State::Empty;
    }
}

#[cfg(test)]
pub(crate) fn request(uri: &str) -> aurora_net::LoginRequest {
    aurora_net::LoginRequest {
        login_uri: uri.into(),
        username: String::new(),
        password: String::new(),
        start: aurora_net::StartLocation::Last,
        agree_to_tos: false,
        read_critical: false,
        mfa_token: String::new(),
        mfa_hash: String::new(),
        mac: String::new(),
        id0: String::new(),
        last_exec: aurora_net::login::LastExec::default(),
        platform: aurora_net::login::Platform::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_metadata_origin_copy_keeps_original_digest_and_no_mutable_form_fallback() {
        let mut pending = Pending::default();
        let mut actual = request("  https://one.example.invalid/login  ");
        pending.submitted(&actual);
        actual.login_uri = "https://later.example.invalid/login".into();
        let origin = pending.completed().unwrap();
        let favorites = origin;
        let independent_consumer = origin;
        assert_eq!(favorites, independent_consumer);
        assert_eq!(origin, GridDigest::from_uri("https://one.example.invalid/login").unwrap());
        assert_eq!(
            origin.namespace(),
            "fec5c302f5c05bf242a021da7fb37cb3912922d0b3717fe7bf5e0828fb3b5c9f"
        );
        assert_eq!(origin.namespace().len(), 64);
        assert!(pending.completed().is_none());
        assert!(!format!("{origin:?}").contains("example"));
    }

    #[test]
    fn account_metadata_origin_invalid_submission_stays_ambiguous_until_failure_or_consumption() {
        let mut pending = Pending::default();
        for invalid in ["".to_owned(), "https://bad\n.example.invalid".into(), "x".repeat(4097)] {
            pending.submitted(&request(&invalid));
            pending.submitted(&request("https://valid.example.invalid/login"));
            assert!(pending.completed().is_none());
        }
        pending.submitted(&request("https://failed.example.invalid/login"));
        pending.failed();
        pending.submitted(&request("https://retry.example.invalid/login"));
        assert_eq!(pending.completed(), GridDigest::from_uri("https://retry.example.invalid/login"));
    }

    #[test]
    fn account_metadata_origin_real_world_reset_retires_pending_without_starting_favorites_io() {
        let library = std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = crate::world::World::new(library);
        world.metadata_login.submitted(&request("https://one.example.invalid/login"));
        assert_eq!(world.favorites.group_status(), crate::world::favorites::GroupStatus::Unavailable);
        world.reset();
        assert!(world.metadata_login.completed().is_none());
        assert_eq!(world.favorites.group_status(), crate::world::favorites::GroupStatus::Unavailable);
    }
}
