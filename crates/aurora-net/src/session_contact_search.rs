//! Contact search commands are bound to the original native login, not the
//! current query text or an HTTP callback's mutable viewer/controller fields.
use super::Session;
use crate::{LoginSource, NetCommand, NetEvent};
impl Session<'_> {
    pub(super) fn on_contact_search_command(&mut self, command: &NetCommand) -> bool {
        let NetCommand::ContactSearch { source, request, query } = command else {
            return false;
        };
        if !source.matches_login(&self.login, self.agent_id()) {
            self.sh.events.send(NetEvent::ContactSearch {
                source: source.clone(),
                request: *request,
                result: Err(crate::contact_search::Error::SessionEnded),
            });
            return true;
        }
        crate::contact_search::launch(
            self.sh.caps_http.clone(),
            self.main_cap("AvatarPickerSearch").as_deref(),
            query,
            self.agent_id(),
            LoginSource::from_login(&self.login),
            *request,
            self.sh.events.clone(),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, UdpSocket};
    use uuid::Uuid;
    #[tokio::test]
    async fn contact_search_actual_http_retains_login_and_uuid_and_lease_after_session_drop() {
        let main = "127.0.0.1:13001".parse().unwrap();
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let (events, rx) = crossbeam_channel::unbounded();
        let sh = super::super::event_queue_tests::shared(crate::EventSender(events));
        let mut session = super::super::event_queue_tests::two_regions(&sh, sock, main, "127.0.0.1:13002".parse().unwrap());
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cap = format!("http://{}/picker?auth=opaque%2Ftoken", server.local_addr().unwrap());
        session.sims.get_mut(&main).unwrap().caps = Arc::new([("AvatarPickerSearch".into(), cap)].into());
        let original = session.login.clone();
        let replacement = Arc::new((*original).clone());
        let source = LoginSource::from_login(&original);
        let first = Uuid::new_v4();
        assert!(session.on_contact_search_command(&NetCommand::ContactSearch {
            source: source.clone(),
            request: first,
            query: "Some.Name".into()
        }));
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(2), server.accept())
            .await
            .unwrap()
            .unwrap();
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).await.unwrap();
        let request = std::str::from_utf8(&buffer[..count]).unwrap();
        assert!(request.starts_with("GET /picker/?auth=opaque%2Ftoken&page_size=100&names=Some+Name HTTP/1.1"));
        let second = Uuid::new_v4();
        assert!(session.on_contact_search_command(&NetCommand::ContactSearch {
            source: source.clone(),
            request: second,
            query: "Different Name".into()
        }));
        let (_, busy) = rx.try_recv().unwrap();
        assert!(matches!(busy,NetEvent::ContactSearch{request,result:Err(crate::contact_search::Error::Busy),..} if request==second));
        // Discarding the original session cannot release its outstanding HTTP
        // permit, nor redirect its eventual callback to an identical new login.
        drop(session);
        let body = aurora_llsd::to_xml(
            &aurora_llsd::llsd_map! {"agents"=>vec![aurora_llsd::llsd_map!{"id"=>Uuid::from_u128(77),"display_name"=>"Synthetic peer","username"=>"synthetic.peer"}]},
        );
        stream
            .write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        stream.write_all(&body).await.unwrap();
        drop(stream);
        let (_, result) = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .await
            .unwrap();
        let NetEvent::ContactSearch {
            source: captured,
            request,
            result: Ok(result),
        } = result
        else {
            panic!("actual scoped result")
        };
        assert_eq!(request, first);
        assert_eq!(result.residents.len(), 1);
        assert!(captured.matches_login(&original, original.agent_id));
        assert!(!captured.matches_login(&replacement, replacement.agent_id));
        // Gate release is tested via a real later request, not a test-only
        // permit override or an environment/security setting.
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let (events, rx) = crossbeam_channel::unbounded();
        let sh = super::super::event_queue_tests::shared(crate::EventSender(events));
        let mut next = super::super::event_queue_tests::two_regions(&sh, sock, main, "127.0.0.1:13002".parse().unwrap());
        next.sims.get_mut(&main).unwrap().caps =
            Arc::new([("AvatarPickerSearch".into(), format!("http://{}/next", server.local_addr().unwrap()))].into());
        let third = Uuid::new_v4();
        assert!(next.on_contact_search_command(&NetCommand::ContactSearch {
            source: LoginSource::from_login(&next.login),
            request: third,
            query: "Next Name".into()
        }));
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(2), server.accept())
            .await
            .unwrap()
            .unwrap();
        stream.read(&mut buffer).await.unwrap();
        stream
            .write_all(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let (_, result) = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .await
            .unwrap();
        assert!(matches!(result,NetEvent::ContactSearch{request,result:Err(crate::contact_search::Error::Http(503)),..} if request==third));
    }
}
