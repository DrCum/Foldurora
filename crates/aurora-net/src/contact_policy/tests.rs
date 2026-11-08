use super::*;
use aurora_llsd::llsd_map;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn body(agents: Llsd) -> Vec<u8> {
    aurora_llsd::to_xml(&llsd_map! { "Agents" => agents })
}

#[test]
fn contact_policy_parser_rejects_a_malformed_sender_as_a_whole_and_keeps_explicit_zero() {
    let sender = Uuid::from_u128(0xabcd);
    let peer = Uuid::from_u128(0xdef0);
    let valid = body(llsd_map! { sender.to_string() => NO_SUPPORT, peer.to_string() => SUPPORT });
    let parsed = parse(&valid).unwrap();
    assert_eq!(parsed.agents, [(sender, NO_SUPPORT), (peer, SUPPORT)].into());
    assert!(!parsed.legacy);
    for invalid in [
        llsd_map! { sender.to_string() => "16", peer.to_string() => SUPPORT },
        llsd_map! { sender.to_string() => -1 },
        llsd_map! { "not-a-uuid" => SUPPORT },
        llsd_map! { Uuid::nil().to_string() => SUPPORT },
        llsd_map! { sender.to_string() => NO_SUPPORT, sender.to_string().to_uppercase() => 0 },
    ] {
        assert_eq!(parse(&body(invalid)), Err(Error::Invalid));
    }
    assert_eq!(parse(&body(llsd_map! { sender.to_string() => 0 })).unwrap().agents[&sender], 0);
}

#[test]
fn contact_policy_legacy_support_is_key_presence_and_cannot_supply_no_support() {
    let peer = Uuid::from_u128(0xdef0);
    let parsed = parse(&aurora_llsd::to_xml(&llsd_map! {
        "SupportAgents" => llsd_map! { peer.to_string() => llsd_map! { "support" => false } }
    }))
    .unwrap();
    assert!(parsed.legacy);
    assert_eq!(parsed.agents[&peer], SUPPORT);
    assert!(
        parse(&aurora_llsd::to_xml(&llsd_map! { "MOTD" => "No flags" }))
            .unwrap()
            .agents
            .is_empty()
    );
    assert_eq!(parse(&body(Llsd::Array(vec![]))), Err(Error::Invalid));
    assert_eq!(parse(&vec![b' '; MAX_BYTES + 1]), Err(Error::TooLarge));
}

#[test]
fn contact_policy_feeds_are_fixed_sl_only_and_never_use_login_userinfo() {
    assert!(supported_grid(crate::login::SL_MAIN_GRID));
    assert!(supported_grid(crate::login::SL_BETA_GRID));
    for uri in [
        "https://grid.example/login",
        "https://login.agni.lindenlab.com.attacker.invalid/login",
        "http://login.agni.lindenlab.com/cgi-bin/login.cgi",
    ] {
        assert!(!supported_grid(uri));
    }
    for feed in Feed::ALL {
        let url = reqwest::Url::parse(&feed.url()).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("phoenixviewer.com"));
        assert!(url.username().is_empty() && url.password().is_none() && url.query().is_none());
    }
}

#[tokio::test]
async fn contact_policy_public_client_refuses_http_without_contacting_the_host() {
    let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/policy", server.local_addr().unwrap());
    assert!(policy_client().unwrap().get(&url).send().await.is_err());
    assert!(tokio::time::timeout(Duration::from_millis(25), server.accept()).await.is_err());
}

#[tokio::test]
async fn contact_policy_fetch_keeps_parse_http_redirect_and_size_failures_distinct() {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    for (status, bytes, length, expected) in [
        ("200 OK", b"invalid".as_slice(), 7, Error::Invalid),
        ("503 Unavailable", b"".as_slice(), 0, Error::Http(503)),
        ("302 Found", b"".as_slice(), 0, Error::Http(302)),
        ("200 OK", b"".as_slice(), MAX_BYTES + 1, Error::TooLarge),
    ] {
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/policy", server.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = server.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nLocation: http://127.0.0.1:1/not-allowed\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            stream.write_all(bytes).await.unwrap();
            crate::test_http::finish(stream);
        });
        assert_eq!(fetch(&client, &url).await, Err(expected));
        task.await.unwrap();
    }
}
