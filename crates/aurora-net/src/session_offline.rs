//! Session side of the offline instant messages: the request made once
//! after login (LLIMProcessing::requestOfflineMessages: the ReadOfflineMsgs
//! capability when it and the capabilities that answer offline offers are
//! there, else the RetrieveInstantMessages message) and the answers to a
//! friendship offer or a group invitation that came that way
//! (accept_friendship_coro, decline_friendship_coro,
//! response_group_invitation_coro in llviewermessage.cpp).

use super::Session;
use crate::offline_im;
use crate::types::{LoginSource, NetCommand, NetEvent};
use aurora_llsd::llsd_map;
use aurora_msg::msgs::*;

impl Session<'_> {
    /// Commands of this module; false when `c` is not one of them.
    pub(super) fn on_offline_command(&mut self, c: &NetCommand) -> bool {
        match c {
            NetCommand::RequestOfflineMessages => self.request_offline_messages(),
            NetCommand::RequestOfflineMessagesLegacy => self.request_offline_messages_legacy(),
            NetCommand::AnswerOfflineFriendship {
                from,
                transaction,
                folder,
                accept,
                agent_name,
            } => self.answer_offline_friendship(*from, *transaction, *folder, *accept, agent_name),
            NetCommand::AnswerOfflineGroupInvite { group, accept } => self.answer_offline_group_invite(*group, *accept),
            _ => return false,
        }
        true
    }

    /// LLIMProcessing::requestOfflineMessages.
    fn request_offline_messages(&mut self) {
        let caps: Vec<Option<String>> = offline_im::REQUIRED_CAPS.iter().map(|c| self.main_cap(c)).collect();
        let Some(url) = caps.iter().all(Option::is_some).then(|| caps[0].clone()).flatten() else {
            // the offline messages capability gives no session id for
            // friendship offers and group invitations: without the
            // capabilities that answer those, the legacy message
            self.request_offline_messages_legacy();
            return;
        };
        log::info!("requesting offline messages (capability)");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let source = LoginSource::from_login(&self.login);
        tokio::spawn(async move {
            let contents = match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                    Ok(bytes) if bytes.is_empty() => Ok(aurora_llsd::Llsd::Undef),
                    Ok(bytes) => aurora_llsd::from_xml(&bytes).map_err(|e| e.to_string()),
                    Err(e) => Err(e.without_url().to_string()),
                },
                Ok(resp) => Err(format!("HTTP {}", resp.status())),
                Err(e) => Err(e.without_url().to_string()),
            };
            match contents {
                // "Error requesting offline messages via capability...
                // Falling back to legacy method."
                Err(e) => {
                    log::warn!("offline messages: capability failed ({e}); falling back to the legacy request");
                    let _ = events.send(NetEvent::OfflineMessagesFailed { source });
                }
                Ok(contents) => match offline_im::parse(&contents) {
                    Ok(messages) => {
                        log::info!("offline messages: {}", messages.len());
                        for m in messages {
                            let _ = events.send(NetEvent::SavedInstantMessage {
                                source: source.clone(),
                                message: m,
                            });
                        }
                    }
                    // Firestorm stops there too, without the legacy request
                    Err(e) => log::warn!("offline messages: {e}"),
                },
            }
        });
    }

    /// LLIMProcessing::requestOfflineMessagesLegacy: the simulator then
    /// sends each message as an ImprovedInstantMessage.
    fn request_offline_messages_legacy(&mut self) {
        log::info!("requesting offline messages (legacy)");
        let mut m = RetrieveInstantMessages::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        self.send_main(&m, true);
    }

    /// friendship_offer_callback for an offer received while offline: by
    /// capability when the region has it, else by message when the offer
    /// came with a transaction id.
    fn answer_offline_friendship(&mut self, from: uuid::Uuid, transaction: uuid::Uuid, folder: uuid::Uuid, accept: bool, agent_name: &str) {
        let cap = self.main_cap(if accept { "AcceptFriendship" } else { "DeclineFriendship" });
        let Some(cap) = cap else {
            if transaction.is_nil() {
                log::warn!("friendship offer: neither capability nor transaction id, cannot answer");
            } else if accept {
                let mut m = AcceptFriendship::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.transaction_block.transaction_id = transaction;
                m.folder_data = vec![accept_friendship::FolderData { folder_id: folder }];
                self.send_main(&m, true);
            } else {
                let mut m = DeclineFriendship::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.transaction_block.transaction_id = transaction;
                self.send_main(&m, true);
            }
            return;
        };
        let http = self.sh.caps_http.clone();
        let url = if accept {
            offline_im::accept_friendship_url(&cap, &from, agent_name)
        } else {
            offline_im::decline_friendship_url(&cap, &from)
        };
        tokio::spawn(async move {
            let request = if accept {
                // an empty LLSD body
                http.post(&url)
                    .header("Content-Type", "application/llsd+xml")
                    .body(aurora_llsd::to_xml(&aurora_llsd::Llsd::Undef))
            } else {
                http.delete(&url)
            };
            let what = if accept { "accept" } else { "decline" };
            match request.header("Accept", "application/llsd+xml").send().await {
                Ok(resp) if resp.status().is_success() => {
                    let ok = resp
                        .bytes()
                        .await
                        .ok()
                        .and_then(|b| aurora_llsd::from_xml(&b).ok())
                        .is_some_and(|v| v["success"].as_bool());
                    if !ok {
                        log::warn!("friendship offer {what}: the server did not process it");
                    }
                }
                Ok(resp) => log::warn!("friendship offer {what}: HTTP {}", resp.status()),
                Err(e) => log::warn!("friendship offer {what}: {}", e.without_url()),
            }
        });
    }

    /// send_join_group_response with use_offline_cap.
    fn answer_offline_group_invite(&mut self, group: uuid::Uuid, accept: bool) {
        let Some(url) = self.main_cap(if accept { "AcceptGroupInvite" } else { "DeclineGroupInvite" }) else {
            // "if sim has no this cap, we can do nothing - regular request will fail"
            log::warn!("group invitation: no capability, cannot answer an offline invitation");
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            match crate::caps::post_llsd(&http, &url, &llsd_map! { "group" => group }).await {
                Ok(v) if v["success"].as_bool() => {
                    // accepted: the group list is asked again
                    if accept {
                        let _ = events.send(NetEvent::OfflineGroupInviteAccepted(group));
                    }
                }
                Ok(_) => log::warn!("group invitation: the server did not process the answer"),
                Err(e) => log::warn!("group invitation answer: {e}"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::REQUESTED_CAPS;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UdpSocket;
    use uuid::Uuid;

    #[tokio::test]
    async fn login_source_saved_http_success_and_failures_retain_the_original_login_after_session_end() {
        for outcome in ["success", "invalid_xml", "http", "transport"] {
            let main = "127.0.0.1:13001".parse().unwrap();
            let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
            let (events, rx) = crossbeam_channel::unbounded();
            let sh = super::super::event_queue_tests::shared(crate::EventSender(events));
            let mut session = super::super::event_queue_tests::two_regions(&sh, sock, main, "127.0.0.1:13002".parse().unwrap());
            let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/saved", server.local_addr().unwrap());
            session.sims.get_mut(&main).unwrap().caps = Arc::new(
                offline_im::REQUIRED_CAPS
                    .iter()
                    .map(|cap| ((*cap).to_owned(), url.clone()))
                    .collect(),
            );
            let original = session.login.clone();
            let replacement = Arc::new((*original).clone());
            assert!(session.on_offline_command(&NetCommand::RequestOfflineMessages));
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), server.accept())
                .await
                .unwrap()
                .unwrap();
            let mut request = [0; 4096];
            let length = stream.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..length]).starts_with("GET /saved "));
            drop(session);
            if outcome == "transport" {
                drop(stream);
            } else {
                let body = if outcome == "invalid_xml" {
                    b"invalid".to_vec()
                } else {
                    aurora_llsd::to_xml(&llsd_map! { "messages" => aurora_llsd::Llsd::Array(vec![llsd_map! {
                        "from_agent_id" => Uuid::from_u128(9), "to_agent_id" => original.agent_id,
                        "from_agent_name" => "Synthetic Resident", "message" => "Saved message",
                        "dialog" => 0, "offline" => 0
                    }]) })
                };
                let status = if outcome == "http" { "503 Unavailable" } else { "200 OK" };
                stream
                    .write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes())
                    .await
                    .unwrap();
                stream.write_all(&body).await.unwrap();
                crate::test_http::finish(stream);
            }
            let event = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Ok((_, event)) = rx.try_recv() {
                        break event;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("original saved-message completion");
            let source = match event {
                NetEvent::SavedInstantMessage { source, message } if outcome == "success" => {
                    assert_eq!(message.message, "Saved message");
                    assert!(!message.offline, "delivery provenance never rewrites the server's offline flag");
                    source
                }
                NetEvent::OfflineMessagesFailed { source } if outcome != "success" => source,
                other => panic!("{outcome}: unexpected {other:?}"),
            };
            assert!(source.matches_login(&original, original.agent_id));
            assert!(!source.matches_login(&replacement, original.agent_id));
        }
    }

    #[test]
    fn the_capabilities_of_offline_messages_are_requested() {
        for cap in [
            "ReadOfflineMsgs",
            "AcceptFriendship",
            "DeclineFriendship",
            "AcceptGroupInvite",
            "DeclineGroupInvite",
        ] {
            assert!(REQUESTED_CAPS.contains(&cap), "{cap}");
        }
    }
}
