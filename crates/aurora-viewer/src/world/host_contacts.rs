//! Captured Search membership belongs to the actual login, unique local request
//! and full host source/token. Query strings are never response authority.
use super::World;
use aurora_net::contact_search::{Error, MAX_RESULTS, Results};
use aurora_net::{LoginSource, NetCommand};
use aurora_session::chat::{ContactChannel, ContactKey};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub(crate) struct Search {
    key: ContactKey,
    source: LoginSource,
    request: Uuid,
    since: Instant,
    result: Option<Result<Results, Error>>,
}
impl World {
    pub(crate) fn begin_host_contact_search(&mut self, key: ContactKey, query: &str) -> Result<NetCommand, Error> {
        key.validate().map_err(|_| Error::Invalid)?;
        if key.channel != ContactChannel::Search
            || !(2..=127).contains(&query.len())
            || query.chars().any(char::is_control)
            || !self.shared_chat_privacy_allowed()
            || !self.mutes.is_loaded()
        {
            return Err(Error::Invalid);
        }
        let login = self
            .login
            .as_ref()
            .filter(|login| !self.agent_id.is_nil() && self.agent_id == login.agent_id)
            .ok_or(Error::SessionEnded)?;
        if self.host_contact_search.as_ref().is_some_and(|search| {
            search.source.matches_login(login, self.agent_id)
                && (search.key == key || (search.result.is_none() && search.since.elapsed() < Duration::from_secs(20)))
        }) {
            return Err(Error::Busy);
        }
        let source = LoginSource::from_login(login);
        let request = Uuid::new_v4();
        self.host_contact_search = Some(Search {
            key,
            source: source.clone(),
            request,
            since: Instant::now(),
            result: None,
        });
        Ok(NetCommand::ContactSearch {
            source,
            request,
            query: query.into(),
        })
    }
    pub(crate) fn host_contact_search_results(&self, key: ContactKey) -> Result<Option<&Results>, Error> {
        let search = self
            .host_contact_search
            .as_ref()
            .filter(|search| search.key == key)
            .ok_or(Error::SessionEnded)?;
        if !self
            .login
            .as_ref()
            .is_some_and(|login| search.source.matches_login(login, self.agent_id))
        {
            return Err(Error::SessionEnded);
        }
        match &search.result {
            Some(Ok(results)) => Ok(Some(results)),
            Some(Err(error)) => Err(*error),
            None if search.since.elapsed() > Duration::from_secs(21) => Err(Error::Timeout),
            None => Ok(None),
        }
    }
    pub(super) fn complete_host_contact_search(&mut self, source: &LoginSource, request: Uuid, result: Result<Results, Error>) {
        let Some(login) = self.login.as_ref() else {
            return;
        };
        let Some(search) = self.host_contact_search.as_mut().filter(|search| {
            search.request == request
                && search.result.is_none()
                && source.matches_login(login, self.agent_id)
                && search.source.matches_login(login, self.agent_id)
        }) else {
            return;
        };
        let result = result.and_then(|results| {
            let mut seen = BTreeSet::new();
            if results.residents.len() > MAX_RESULTS
                || results.residents.iter().any(|row| {
                    row.id.is_nil()
                        || row.id == self.agent_id
                        || !seen.insert(row.id)
                        || row.display.is_empty()
                        || row.display.len() > 255
                        || row.username.len() > 127
                        || row.display.chars().any(char::is_control)
                        || row.username.chars().any(char::is_control)
                })
            {
                Err(Error::Malformed)
            } else {
                Ok(results)
            }
        });
        search.result = Some(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::{NetEvent, contact_search::Resident};
    use aurora_session::chat::ChatSource;
    use aurora_session::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
    fn key(token: u64) -> ContactKey {
        ContactKey {
            source: ChatSource {
                worker: WorkerIdentity {
                    installation: InstallationId::from_bytes([1; 32]).unwrap(),
                    host: HostId::from_bytes([2; 16]).unwrap(),
                    worker: WorkerId::from_bytes([3; 16]).unwrap(),
                },
                session: SessionIdentity {
                    session: SessionId::from_bytes([4; 16]).unwrap(),
                    account: AccountKey::from_bytes([5; 32]).unwrap(),
                    grid: GridKey::from_bytes([6; 32]).unwrap(),
                    generation: 1,
                },
                lease_epoch: 1,
            },
            channel: ContactChannel::Search,
            token,
        }
    }
    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.bind_session_test_login(Uuid::from_u128(99));
        world.mutes.state = crate::world::mutes::LoadState::Loaded;
        world
    }
    fn result() -> Results {
        Results {
            residents: vec![Resident {
                id: Uuid::from_u128(77),
                display: "Synthetic resident".into(),
                username: "synthetic.resident".into(),
            }],
            limited: false,
        }
    }
    #[test]
    fn contact_search_world_requires_original_login_uuid_and_exact_source_token_without_name_pollution() {
        let mut world = world();
        let command = world.begin_host_contact_search(key(1), "Synthetic Name").unwrap();
        let NetCommand::ContactSearch { source, request, .. } = command else {
            panic!()
        };
        assert!(world.begin_host_contact_search(key(2), "Other Name").is_err());
        world.apply(NetEvent::ContactSearch {
            source: source.clone(),
            request: Uuid::new_v4(),
            result: Ok(result()),
        });
        assert!(world.host_contact_search_results(key(1)).unwrap().is_none());
        world.apply(NetEvent::ContactSearch {
            source: source.clone(),
            request,
            result: Ok(result()),
        });
        assert_eq!(world.host_contact_search_results(key(1)).unwrap().unwrap(), &result());
        assert!(world.host_contact_search_results(key(2)).is_err());
        assert!(world.social.names.is_empty() && world.social.avatar_names.raw(&Uuid::from_u128(77)).is_none());
        let old = world.login.take().unwrap();
        world.login = Some(std::sync::Arc::new((*old).clone()));
        assert!(
            world.host_contact_search_results(key(1)).is_err(),
            "identical identifiers do not rescue old Arc membership"
        );
        let fresh = world.begin_host_contact_search(key(2), "Other Name").unwrap();
        let NetCommand::ContactSearch {
            request: fresh_request, ..
        } = fresh
        else {
            panic!()
        };
        world.apply(NetEvent::ContactSearch {
            source,
            request: fresh_request,
            result: Ok(result()),
        });
        assert!(
            world.host_contact_search_results(key(2)).unwrap().is_none(),
            "old login cannot complete even a known new UUID"
        );
    }
    #[test]
    fn contact_search_world_rejects_invalid_in_process_rows_atomically() {
        let mut world = world();
        let NetCommand::ContactSearch { source, request, .. } = world.begin_host_contact_search(key(1), "Synthetic Name").unwrap() else {
            panic!()
        };
        let mut rows = result();
        rows.residents.push(rows.residents[0].clone());
        world.apply(NetEvent::ContactSearch {
            source,
            request,
            result: Ok(rows),
        });
        assert_eq!(world.host_contact_search_results(key(1)), Err(Error::Malformed));
        assert!(world.social.ims.is_empty());
    }
}
