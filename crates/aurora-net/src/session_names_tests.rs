use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn next(rx: &crossbeam_channel::Receiver<(Instant, NetEvent)>) -> NetEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok((_, event)) = rx.try_recv() {
                break event;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("name reply")
}

#[tokio::test]
async fn name_source_http_original_login_survives_success_parse_status_and_transport_failures() {
    for outcome in ["success", "parse", "status", "transport"] {
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let main = "127.0.0.1:13001".parse().unwrap();
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let (events, rx) = crossbeam_channel::unbounded();
        let sh = event_queue_tests::shared(crate::EventSender(events));
        let mut session = event_queue_tests::two_regions(&sh, sock, main, "127.0.0.1:13002".parse().unwrap());
        session.sims.get_mut(&main).unwrap().caps = Arc::new(HashMap::from([(
            "GetDisplayNames".into(),
            format!("http://{}/names", server.local_addr().unwrap()),
        )]));
        let original = session.login.clone();
        let same_wire_ids_new_login = Arc::new((*original).clone());
        let peer = Uuid::from_u128(9);
        session.request_display_names(vec![peer]);
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), server.accept())
            .await
            .unwrap()
            .unwrap();
        let mut request = [0; 4096];
        let length = stream.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..length]).starts_with(&format!("GET /names/?ids={peer} ")));
        // The session can end and another login reuse the same server IDs
        // while the HTTP job remains outstanding.
        drop(session);
        if outcome == "transport" {
            drop(stream);
        } else {
            let body = if outcome == "parse" {
                b"invalid".to_vec()
            } else {
                aurora_llsd::to_xml(&aurora_llsd::llsd_map! {
                    "agents" => Llsd::Array(vec![aurora_llsd::llsd_map! {
                        "id" => peer, "username" => "peer.resident", "display_name" => "Peer"
                    }]),
                    "bad_ids" => Llsd::Array(vec![])
                })
            };
            let status = if outcome == "status" { "503 Unavailable" } else { "200 OK" };
            stream
                .write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes())
                .await
                .unwrap();
            stream.write_all(&body).await.unwrap();
            crate::test_http::finish(stream);
        }
        let event = next(&rx).await;
        let source = match event {
            NetEvent::DisplayNames { source, names, .. } if outcome == "success" => {
                assert_eq!(names.len(), 1);
                assert_eq!(names[0].id, peer);
                source
            }
            NetEvent::DisplayNamesFailed { source, ids } if outcome != "success" => {
                assert_eq!(ids, vec![peer]);
                source
            }
            other => panic!("{outcome}: unexpected {other:?}"),
        };
        assert!(source.matches_login(&original, original.agent_id));
        assert!(!source.matches_login(&same_wire_ids_new_login, original.agent_id));
    }
}

#[tokio::test]
async fn name_source_missing_capability_and_event_queue_capture_current_original_login() {
    let main = "127.0.0.1:13001".parse().unwrap();
    let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let (events, rx) = crossbeam_channel::unbounded();
    let sh = event_queue_tests::shared(crate::EventSender(events));
    let mut session = event_queue_tests::two_regions(&sh, sock, main, "127.0.0.1:13002".parse().unwrap());
    session.request_display_names(vec![Uuid::from_u128(9)]);
    assert!(matches!(next(&rx).await, NetEvent::DisplayNamesFailed { source, .. }
        if source.matches_login(&session.login, session.agent_id())));
    for (message, body) in [
        (
            "DisplayNameUpdate",
            aurora_llsd::llsd_map! {
                "agent_id" => Uuid::from_u128(9), "agent" => aurora_llsd::llsd_map! { "display_name" => "Updated" }
            },
        ),
        ("SetDisplayNameReply", aurora_llsd::llsd_map! { "status" => 200, "reason" => "OK" }),
    ] {
        session.on_eq_event(EqEvent {
            sim: main,
            message: message.into(),
            body,
        });
        let source = match next(&rx).await {
            NetEvent::DisplayNameUpdate { source, .. } | NetEvent::SetDisplayNameReply { source, .. } => source,
            other => panic!("unexpected {other:?}"),
        };
        assert!(source.matches_login(&session.login, session.agent_id()));
    }
}
