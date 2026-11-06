//! Credential transport checks against synthetic, loopback-only services.

use super::*;
use aurora_llsd::llsd_map;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
struct Request {
    method: String,
    path: String,
    body: String,
}

struct Reply {
    status: &'static str,
    location: Option<String>,
    body: String,
}

impl Reply {
    fn xml(value: Llsd) -> Self {
        Self {
            status: "200 OK",
            location: None,
            body: xmlrpc::build_call("test", &value)
                .replace("methodCall", "methodResponse")
                .replace("<methodName>test</methodName>", ""),
        }
    }

    fn refused() -> Self {
        Self::xml(llsd_map! { "login" => "false", "reason" => "test", "message" => "synthetic refusal" })
    }

    fn redirect(status: &'static str, location: String) -> Self {
        Self {
            status,
            location: Some(location),
            body: String::new(),
        }
    }
}

/// The listener and TLS server both stay local. Connection counts also
/// detect a forbidden request whose TLS handshake fails before HTTP.
struct Server {
    base: String,
    root: Option<reqwest::Certificate>,
    requests: Arc<Mutex<Vec<Request>>>,
    connections: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn new(tls: bool, reply: impl Fn(&Request, &str) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("{}://{}", if tls { "https" } else { "http" }, listener.local_addr().unwrap());
        let (config, root) = if tls {
            use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};
            let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
            params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            let issuer = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
            let signing_key = KeyPair::generate().unwrap();
            let cert = CertificateParams::new(vec!["127.0.0.1".into()])
                .unwrap()
                .signed_by(&signing_key, &issuer)
                .unwrap();
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![cert.der().clone()],
                    rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
                )
                .unwrap();
            (Some(Arc::new(config)), Some(reqwest::Certificate::from_der(issuer.der()).unwrap()))
        } else {
            (None, None)
        };
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_base, thread_requests, thread_connections, thread_stop) =
            (base.clone(), requests.clone(), connections.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let socket = match listener.accept() {
                    Ok((socket, _)) => socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("loopback listener: {error}"),
                };
                thread_connections.fetch_add(1, Ordering::Relaxed);
                socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                socket.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
                if let Some(config) = &config {
                    let connection = rustls::ServerConnection::new(config.clone()).unwrap();
                    serve(rustls::StreamOwned::new(connection, socket), &thread_requests, &thread_base, &reply);
                } else {
                    serve(socket, &thread_requests, &thread_base, &reply);
                }
            }
        });
        Self {
            base,
            root,
            requests,
            connections,
            stop,
            thread: Some(thread),
        }
    }

    fn client(&self) -> reqwest::Client {
        let mut builder = http_client_builder().no_proxy().timeout(Duration::from_secs(5));
        if let Some(root) = &self.root {
            builder = builder.tls_certs_merge([root.clone()]);
        }
        builder.build().unwrap()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn serve(mut socket: impl Read + Write, requests: &Mutex<Vec<Request>>, base: &str, reply: &impl Fn(&Request, &str) -> Reply) {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let header_end = loop {
        match socket.read(&mut buffer) {
            Ok(0) | Err(_) => return, // The untrusted-certificate test aborts the TLS handshake.
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
        }
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
        assert!(bytes.len() < 64 * 1024, "bounded synthetic request");
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, value)| value.trim().parse::<usize>().unwrap());
    assert!(length < 64 * 1024, "bounded synthetic body");
    while bytes.len() < header_end + length {
        match socket.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
        }
    }
    let mut line = headers.lines().next().unwrap().split_whitespace();
    let request = Request {
        method: line.next().unwrap().into(),
        path: line.next().unwrap().into(),
        body: String::from_utf8(bytes[header_end..header_end + length].to_vec()).unwrap(),
    };
    let response = reply(&request, base);
    requests.lock().unwrap().push(request);
    let location = response.location.map_or(String::new(), |url| format!("Location: {url}\r\n"));
    let wire = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
        response.status,
        response.body.len(),
        location,
        response.body
    );
    // The client may close after rejecting a redirect.
    let _ = socket.write_all(wire.as_bytes());
    let _ = socket.flush();
}

fn request(uri: String) -> LoginRequest {
    LoginRequest {
        login_uri: uri,
        username: "synthetic".into(),
        password: "synthetic-password".into(),
        start: StartLocation::Last,
        agree_to_tos: false,
        read_critical: false,
        mfa_token: "synthetic-token".into(),
        mfa_hash: "synthetic-authorization".into(),
        mac: String::new(),
        id0: String::new(),
        last_exec: LastExec::default(),
    }
}

fn run(http: &reqwest::Client, uri: String) -> LoginError {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(login(http, &request(uri)))
        .unwrap_err()
}

#[test]
fn initial_addresses_require_web_transport_and_linden_requires_https() {
    for accepted in [
        SL_MAIN_GRID,
        SL_BETA_GRID,
        "http://127.0.0.1:9000/",
        "https://grid.example/login?mode=test",
    ] {
        assert!(address(accepted).is_ok(), "{accepted}");
    }
    for refused in [
        "http://login.agni.lindenlab.com/cgi-bin/login.cgi",
        "http://LOGIN.agni.lindenlab.com./login",
        "https://user:synthetic-password@grid.example/login",
        "https://grid.example/login#fragment",
        "file:///tmp/login",
        "data:text/plain,test",
        "not an address",
    ] {
        let error = address(refused).unwrap_err().to_string();
        assert!(!error.contains(refused) && !error.contains("synthetic-password"));
    }
}

#[test]
fn redirects_bind_to_the_parsed_original_origin() {
    let original = address(SL_MAIN_GRID).unwrap();
    for accepted in ["https://LOGIN.agni.lindenlab.com:443/another/path?query=synthetic", SL_MAIN_GRID] {
        assert!(allowed_destination(&original, &reqwest::Url::parse(accepted).unwrap()));
    }
    for refused in [
        SL_BETA_GRID,
        "http://login.agni.lindenlab.com/cgi-bin/login.cgi",
        "https://login.agni.lindenlab.com:444/login",
        "https://login.agni.lindenlab.com.attacker.example/login",
        "https://attacker.example/lindenlab.com",
        "https://synthetic@login.agni.lindenlab.com/login",
        "https://login.agni.lindenlab.com/login#fragment",
    ] {
        assert!(!allowed_destination(&original, &reqwest::Url::parse(refused).unwrap()), "{refused}");
    }
    let http_grid = address("http://grid.example:8002/login").unwrap();
    assert!(allowed_destination(&http_grid, &address("http://grid.example:8002/next").unwrap()));
    assert!(!allowed_destination(
        &http_grid,
        &address("https://grid.example:8002/next").unwrap()
    ));
}

#[test]
fn local_http_grid_keeps_http_redirect_method_semantics() {
    for status in [
        "301 Moved Permanently",
        "302 Found",
        "303 See Other",
        "307 Temporary Redirect",
        "308 Permanent Redirect",
    ] {
        let server = Server::new(false, move |request, _| match request.path.as_str() {
            "/login" => Reply::redirect(status, "/done".into()),
            _ => Reply::refused(),
        });
        assert!(matches!(run(&server.client(), server.url("/login")), LoginError::Refused { .. }));
        let received = server.requests();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].method, "POST");
        assert!(received[0].body.contains("synthetic-authorization"));
        if status.starts_with("307") || status.starts_with("308") {
            assert_eq!(received[1].method, "POST");
            assert_eq!(received[1].body, received[0].body);
        } else {
            assert_eq!(received[1].method, "GET");
            assert!(received[1].body.is_empty());
        }
    }
}

#[test]
fn http_redirects_never_contact_a_forbidden_destination() {
    let forbidden = Server::new(false, |_, _| Reply::refused());
    let destination = forbidden.url("/synthetic-private-path?secret=synthetic");
    let server = Server::new(false, move |_, _| Reply::redirect("307 Temporary Redirect", destination.clone()));
    let error = run(&server.client(), server.url("/synthetic-source-path")).to_string();
    assert!(!error.contains("synthetic-private-path") && !error.contains("synthetic-source-path"));
    assert_eq!(server.requests().len(), 1);
    assert_eq!(forbidden.connections.load(Ordering::Relaxed), 0);
}

#[test]
fn http_redirects_with_userinfo_or_fragments_never_send_another_request() {
    for userinfo in [false, true] {
        let server = Server::new(false, move |_, base| {
            let destination = if userinfo {
                base.replacen("://", "://synthetic:synthetic-password@", 1) + "/next"
            } else {
                format!("{base}/next#synthetic-fragment")
            };
            Reply::redirect("307 Temporary Redirect", destination)
        });
        let error = run(&server.client(), server.url("/login"));
        assert!(matches!(error, LoginError::UnsafeAddress(_)));
        assert!(!error.to_string().contains("synthetic-password"));
        assert_eq!(server.connections.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn indeterminate_redirects_are_validated_before_another_post() {
    let forbidden = Server::new(false, |_, _| Reply::refused());
    for destination in [forbidden.url("/synthetic-private-path"), "not an address".into()] {
        let server = Server::new(false, move |_, _| {
            Reply::xml(llsd_map! { "login" => "indeterminate", "next_url" => destination.clone() })
        });
        let error = run(&server.client(), server.url("/login"));
        assert!(matches!(error, LoginError::UnsafeAddress(_)));
        assert!(!error.to_string().contains("synthetic-private-path"));
        assert_eq!(server.requests().len(), 1);
    }
    assert_eq!(forbidden.connections.load(Ordering::Relaxed), 0);
}

#[test]
fn same_origin_indeterminate_redirects_keep_the_login_method_and_credentials() {
    let server = Server::new(false, |request, base| match request.path.as_str() {
        "/login" => Reply::xml(llsd_map! {
            "login" => "indeterminate", "next_url" => format!("{base}/next"), "next_method" => "login_next"
        }),
        _ => Reply::refused(),
    });
    assert!(matches!(run(&server.client(), server.url("/login")), LoginError::Refused { .. }));
    let received = server.requests();
    assert_eq!(received.len(), 2);
    assert_eq!(received[1].method, "POST");
    assert!(received[1].body.contains("<methodName>login_next</methodName>"));
    assert!(received[1].body.contains("synthetic-token") && received[1].body.contains("synthetic-authorization"));
}

#[test]
fn mixed_http_and_xmlrpc_redirects_cannot_escape_the_original_origin() {
    let forbidden = Server::new(false, |_, _| Reply::refused());
    let destination = forbidden.url("/synthetic-private-path");
    let server = Server::new(false, move |request, base| match request.path.as_str() {
        "/login" => Reply::redirect("308 Permanent Redirect", "/moved".into()),
        "/moved" => Reply::xml(llsd_map! { "login" => "indeterminate", "next_url" => format!("{base}/second") }),
        _ => Reply::redirect("307 Temporary Redirect", destination.clone()),
    });
    assert!(matches!(run(&server.client(), server.url("/login")), LoginError::UnsafeAddress(_)));
    assert_eq!(server.requests().len(), 3);
    assert_eq!(forbidden.connections.load(Ordering::Relaxed), 0);
}

#[test]
fn exactly_ten_http_redirects_are_allowed_but_an_eleventh_is_refused() {
    for hops in [HTTP_REDIRECT_LIMIT, HTTP_REDIRECT_LIMIT + 1] {
        let server = Server::new(false, move |request, _| {
            let step = request.path.trim_start_matches('/').parse::<usize>().unwrap();
            if step < hops {
                Reply::redirect("307 Temporary Redirect", format!("/{}", step + 1))
            } else {
                Reply::refused()
            }
        });
        let error = run(&server.client(), server.url("/0"));
        if hops == HTTP_REDIRECT_LIMIT {
            assert!(matches!(error, LoginError::Refused { .. }));
        } else {
            assert!(matches!(error, LoginError::Http(_)));
        }
        assert_eq!(server.requests().len(), HTTP_REDIRECT_LIMIT + 1);
    }
}

#[test]
fn http_and_xmlrpc_redirect_loops_have_separate_bounds() {
    let http_loop = Server::new(false, |_, _| Reply::redirect("307 Temporary Redirect", "/loop".into()));
    let error = run(&http_loop.client(), http_loop.url("/loop"));
    assert!(matches!(error, LoginError::Http(_)));
    assert_eq!(http_loop.requests().len(), HTTP_REDIRECT_LIMIT + 1);
    let xml_loop = Server::new(false, |_, base| {
        Reply::xml(llsd_map! { "login" => "indeterminate", "next_url" => format!("{base}/loop") })
    });
    assert!(matches!(run(&xml_loop.client(), xml_loop.url("/loop")), LoginError::Malformed(_)));
    assert_eq!(xml_loop.requests().len(), 5);
}

#[test]
fn trusted_https_supports_same_origin_redirects_and_rejects_downgrades() {
    for xml in [false, true] {
        let server = Server::new(true, move |request, base| match request.path.as_str() {
            "/login" if xml => Reply::xml(llsd_map! { "login" => "indeterminate", "next_url" => format!("{base}/done") }),
            "/login" => Reply::redirect("307 Temporary Redirect", "/done".into()),
            _ => Reply::refused(),
        });
        assert!(matches!(run(&server.client(), server.url("/login")), LoginError::Refused { .. }));
        assert_eq!(server.requests().len(), 2);
        assert_eq!(server.requests()[1].method, "POST");

        let downgrade = Server::new(true, move |_, base| {
            let destination = base.replacen("https://", "http://", 1) + "/synthetic-private-path";
            if xml {
                Reply::xml(llsd_map! { "login" => "indeterminate", "next_url" => destination })
            } else {
                Reply::redirect("307 Temporary Redirect", destination)
            }
        });
        let error = run(&downgrade.client(), downgrade.url("/login")).to_string();
        assert!(!error.contains("synthetic-private-path"));
        assert_eq!(downgrade.requests().len(), 1);
        assert_eq!(
            downgrade.connections.load(Ordering::Relaxed),
            1,
            "no connection to the downgraded address"
        );
    }
}

#[test]
fn login_https_does_not_accept_an_untrusted_certificate() {
    let server = Server::new(true, |_, _| Reply::refused());
    let http = http_client_builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
    assert!(matches!(run(&http, server.url("/login")), LoginError::Http(_)));
    assert!(server.requests().is_empty());
    assert_eq!(server.connections.load(Ordering::Relaxed), 1);
}
