//! Independent profile, storage and forbidden destinations exercise the
//! credential boundary with synthetic cookies and no external network.

use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn headers(&self, name: &str) -> Vec<&str> {
        self.headers.iter().filter(|(n, _)| n == name).map(|(_, v)| v.as_str()).collect()
    }
}

struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

fn reply(status: u16, headers: Vec<(&'static str, String)>, body: impl Into<String>) -> Reply {
    Reply {
        status,
        headers,
        body: body.into(),
    }
}

struct Fixture {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    fn start(handler: impl Fn(&Request, &str) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let (requests, stop) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(false)));
        let (base, kept, stopped) = (url.clone(), requests.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(e) => panic!("fixture accept: {e}"),
                };
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let request = read_request(&stream);
                let answer = handler(&request, &base);
                kept.lock().unwrap().push(request);
                let headers: String = answer.headers.iter().map(|(name, value)| format!("{name}: {value}\r\n")).collect();
                write!(
                    stream,
                    "HTTP/1.1 {} Test\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    answer.status,
                    answer.body.len(),
                    answer.body
                )
                .unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn web(&self) -> Web {
        Web::demo(
            &demo::Server {
                url: self.url.clone(),
                posts: Default::default(),
            },
            "security test".into(),
        )
        .unwrap()
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn read_request(stream: &TcpStream) -> Request {
    let mut input = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    input.read_line(&mut first).unwrap();
    let mut parts = first.split_whitespace();
    let (method, target) = (parts.next().unwrap().into(), parts.next().unwrap().into());
    let mut headers = Vec::new();
    let mut length = 0;
    loop {
        let mut line = String::new();
        input.read_line(&mut line).unwrap();
        if line.trim().is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').unwrap();
        let (name, value) = (name.to_ascii_lowercase(), value.trim().to_owned());
        if name == "content-length" {
            length = value.parse().unwrap();
        }
        headers.push((name, value));
    }
    let mut body = vec![0; length];
    input.read_exact(&mut body).unwrap();
    Request {
        method,
        target,
        headers,
        body,
    }
}

fn config(upload: &str, completion: &str) -> String {
    serde_json::json!({"url": upload, "data": {
        "key": "demo", "AWSAccessKeyId": "demo", "acl": "public-read", "Content-Type": "image/png",
        "policy": "synthetic", "signature": "synthetic", "success_action_redirect": completion,
    }})
    .to_string()
}

fn auth_reply() -> Reply {
    reply(
        200,
        vec![("Set-Cookie", "profile-session=synthetic; Path=/; HttpOnly; Secure".into())],
        "profile",
    )
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

#[test]
fn profile_security_separates_storage_from_authenticated_profile_requests() {
    let other_profile = Fixture::start(|_, _| reply(200, vec![], "must not be reached"));
    let completion = Arc::new(Mutex::new(String::new()));
    let done = completion.clone();
    let storage = Fixture::start(move |_, _| reply(303, vec![("Location", done.lock().unwrap().clone())], ""));
    let upload = format!("{}upload", storage.url);
    let profile = Fixture::start(move |request, base| match request.target.as_str() {
        "/" => auth_reply(),
        "/done" => reply(200, vec![], "done"),
        target if target.starts_with("/snapshots/s3_upload_config?") => reply(200, vec![], config(&upload, "/done")),
        _ => reply(404, vec![], base),
    });
    *completion.lock().unwrap() = format!("{}done", profile.url);
    let web = profile.web();
    runtime().block_on(async {
        let auth = auth_cookie(&web).await.unwrap();
        post(&web, &auth, b"PNG".to_vec(), "caption & location", true).await.unwrap();
        assert_eq!(
            post(&other_profile.web(), &auth, b"PNG".to_vec(), "", false).await,
            Err(Error::NoCookie)
        );
    });
    assert!(
        other_profile.requests().is_empty(),
        "cached authorization must not cross profile origins"
    );
    let uploads = storage.requests();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].method, "POST");
    assert!(uploads[0].headers("cookie").is_empty());
    assert!(uploads[0].headers("authorization").is_empty());
    assert!(uploads[0].body.windows(3).any(|s| s == b"PNG"));
    let requests = profile.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].target.contains("caption=caption%20%26%20location&add_loc=1"));
    assert_eq!(requests[0].headers("cookie"), [demo::OPENID]);
    for request in &requests[1..] {
        assert_eq!(
            request.headers("cookie"),
            [format!("{}; profile-session=synthetic", demo::OPENID).as_str()]
        );
    }
}

#[test]
fn profile_security_rejects_configured_and_actual_foreign_completion_before_contact() {
    let forbidden = Fixture::start(|_, _| reply(200, vec![], "must not be reached"));
    for malicious_config in [true, false] {
        let foreign = forbidden.url.clone();
        let storage = Fixture::start(move |_, _| reply(303, vec![("Location", foreign.clone())], ""));
        let (upload, foreign) = (storage.url.clone(), forbidden.url.clone());
        let profile = Fixture::start(move |request, _| {
            if request.target == "/" {
                auth_reply()
            } else {
                reply(200, vec![], config(&upload, if malicious_config { &foreign } else { "/done" }))
            }
        });
        let web = profile.web();
        runtime().block_on(async {
            let auth = auth_cookie(&web).await.unwrap();
            assert_eq!(post(&web, &auth, b"PNG".to_vec(), "", false).await, Err(Error::UnsafeDestination));
        });
        assert!(forbidden.requests().is_empty());
        assert_eq!(storage.requests().len(), usize::from(!malicious_config));
    }
}

#[test]
fn profile_security_bootstrap_redirects_stay_on_origin_and_are_bounded() {
    let forbidden = Fixture::start(|_, _| reply(200, vec![], "must not be reached"));
    let foreign = forbidden.url.clone();
    let profile = Fixture::start(move |_, _| reply(302, vec![("Location", foreign.clone())], ""));
    assert!(matches!(
        runtime().block_on(auth_cookie(&profile.web())),
        Err(Error::UnsafeDestination)
    ));
    assert!(forbidden.requests().is_empty());

    let profile = Fixture::start(|request, _| {
        if request.target == "/" {
            reply(302, vec![("Location", "signin".into())], "")
        } else {
            auth_reply()
        }
    });
    assert!(runtime().block_on(auth_cookie(&profile.web())).is_ok());
    let requests = profile.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].target, "/signin");
    assert!(requests.iter().all(|request| request.headers("cookie") == [demo::OPENID]));

    let looping = Fixture::start(|_, _| reply(302, vec![("Location", "/again".into())], ""));
    assert!(matches!(
        runtime().block_on(auth_cookie(&looping.web())),
        Err(Error::UnsafeDestination)
    ));
    assert_eq!(looping.requests().len(), 11);
}

#[test]
fn profile_security_never_replays_storage_redirects_or_follows_completion_redirects() {
    let forbidden = Fixture::start(|_, _| reply(200, vec![], "must not be reached"));
    for storage_redirect in [true, false] {
        let (foreign, completion) = (forbidden.url.clone(), Arc::new(Mutex::new(String::new())));
        let done = completion.clone();
        let storage = Fixture::start(move |_, _| {
            let (status, location) = if storage_redirect {
                (307, foreign.clone())
            } else {
                (303, done.lock().unwrap().clone())
            };
            reply(status, vec![("Location", location)], "")
        });
        let (upload, foreign) = (storage.url.clone(), forbidden.url.clone());
        let profile = Fixture::start(move |request, _| match request.target.as_str() {
            "/" => auth_reply(),
            "/done" => reply(302, vec![("Location", foreign.clone())], ""),
            _ => reply(200, vec![], config(&upload, "/done")),
        });
        *completion.lock().unwrap() = format!("{}done", profile.url);
        let web = profile.web();
        runtime().block_on(async {
            let auth = auth_cookie(&web).await.unwrap();
            let expected = if storage_redirect { Error::Upload(307) } else { Error::Finish(302) };
            assert_eq!(post(&web, &auth, b"PNG".to_vec(), "", false).await, Err(expected));
        });
        assert!(forbidden.requests().is_empty());
        assert_eq!(storage.requests().len(), 1);
    }
}

#[test]
fn profile_security_allows_relative_completion_on_the_profile_origin() {
    let profile = Fixture::start(|request, base| match request.target.as_str() {
        "/" => auth_reply(),
        "/upload" => reply(303, vec![("Location", "/done?key=synthetic".into())], ""),
        "/done?key=synthetic" => reply(200, vec![], "done"),
        _ => reply(200, vec![], config(&format!("{base}upload"), "/done")),
    });
    let web = profile.web();
    runtime().block_on(async {
        let auth = auth_cookie(&web).await.unwrap();
        post(&web, &auth, b"PNG".to_vec(), "", false).await.unwrap();
    });
    assert_eq!(profile.requests().len(), 4);
}

#[test]
fn profile_security_production_urls_and_cache_binding_cannot_use_demo_exceptions() {
    for invalid in [
        "http://profile.example/",
        "http://127.0.0.1:80/",
        "https://name:password@profile.example/",
        "https://profile.example/#secret",
        "file:///profile",
        "not a URL",
    ] {
        assert!(matches!(
            Web::new(invalid, "session=synthetic".into(), "test".into()),
            Err(Error::UnsafeDestination)
        ));
        assert!(destination(invalid, false).is_err());
    }
    assert_eq!(
        profile_url("https://PROFILE.example/CaseSensitive"),
        Some("https://profile.example/CaseSensitive/".into())
    );
    let web = Web::new(
        "https://profile.example/CaseSensitive/",
        "session=synthetic; Path=/; Secure".into(),
        "test".into(),
    )
    .unwrap();
    assert_eq!(web.openid, "session=synthetic");
    let auth = AuthCookie {
        profile: web.profile.clone(),
        openid: web.openid.clone(),
        cookie: "auth=synthetic".into(),
    };
    assert!(auth.matches(&web));
    for (base, openid) in [
        ("https://other.example/CaseSensitive/", "session=synthetic"),
        ("https://profile.example:444/CaseSensitive/", "session=synthetic"),
        ("https://profile.example/casesensitive/", "session=synthetic"),
        ("https://profile.example/CaseSensitive/", "session=other"),
    ] {
        assert!(!auth.matches(&Web::new(base, openid.into(), "test".into()).unwrap()));
    }
    for reference in [
        "http://profile.example/done",
        "https://other.example/done",
        "//other.example/done",
        "https://name@profile.example/done",
        "/done#secret",
    ] {
        assert_eq!(web.profile_destination(&web.profile, reference), Err(Error::UnsafeDestination));
    }
    assert!(
        Web::demo(
            &demo::Server {
                url: "http://remote.example/".into(),
                posts: Default::default()
            },
            "test".into()
        )
        .is_err()
    );
    assert!(destination("http://[::1]:1234/", true).is_ok());
    assert_eq!(
        cookie_pair("session=\"synthetic\"; Path=/; Secure"),
        Some("session=\"synthetic\"".into())
    );
    for invalid in [
        "session=one\r\nInjected: true",
        "bad name=value",
        "session=",
        "no-equals",
        "session=\"\"",
        "session=\"one",
        "session=one\"",
        "session=\"one\"more",
        "session=\"one\"\"",
        "session=\"one\r\nInjected: true\"",
    ] {
        assert!(cookie_pair(invalid).is_none());
    }
}
