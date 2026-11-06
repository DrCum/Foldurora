//! Photos to the profile feed (LLWebProfile::uploadImageCoro / buildPostData
//! and LLViewerMedia::getOpenIDCookieCoro, LLPanelSnapshotProfile,
//! indra/newview, originally LGPL 2.1). The web profile is reached with the
//! OpenID cookie of the login (`media::openid`): a first GET of the profile
//! gives its own session cookie (the "auth cookie"), then
//! `snapshots/s3_upload_config` gives the form of the storage the PNG is
//! posted to, whose 303 is followed to finish the post.
//!
//! Cookies stay on the profile's exact HTTPS origin. The storage gets only
//! its signed multipart form, and its completion URL is checked before any
//! authenticated request. Cookies and network URLs are never logged.

use super::Aspect;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// LLWebProfile::MAX_WEB_DATASIZE: a larger PNG is likely refused.
pub const MAX_WEB_DATASIZE: usize = 8 * 1024 * 1024;
/// The caption's text editor (panel_snapshot_profile.xml, max_length).
pub const MAX_CAPTION: usize = 700;
/// buildPostData's boundary.
const BOUNDARY: &str = "----------------------------0123abcdefab";
const TIMEOUT: Duration = Duration::from_secs(60);

/// The sizes of panel_snapshot_profile.xml's combo (Current Window,
/// 640x480, 800x600, 1024x768), as long sides with the chosen proportions
/// (LastSnapshotToProfileResolution: 640 by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileSize {
    Window,
    S640,
    S800,
    S1024,
}

impl ProfileSize {
    pub const ALL: [ProfileSize; 4] = [ProfileSize::S640, ProfileSize::S800, ProfileSize::S1024, ProfileSize::Window];

    pub fn side(self) -> Option<u32> {
        match self {
            ProfileSize::S640 => Some(640),
            ProfileSize::S800 => Some(800),
            ProfileSize::S1024 => Some(1024),
            ProfileSize::Window => None,
        }
    }
}

/// The size of a photo for the profile: the preset as its long side with
/// the proportions, or the window's size.
pub fn capture_size(aspect: Aspect, size: ProfileSize, window: (u32, u32)) -> (u32, u32) {
    match size.side() {
        Some(long) => super::photo_size(aspect, super::Resolution::Custom, sides(aspect, long, window), window),
        None => super::photo_size(aspect, super::Resolution::Window, [0, 0], window),
    }
}

fn sides(aspect: Aspect, long: u32, window: (u32, u32)) -> [u32; 2] {
    let ratio = aspect.ratio((window.0.max(1), window.1.max(1)));
    if ratio >= 1.0 {
        [long, (long as f32 / ratio).round() as u32]
    } else {
        [(long as f32 * ratio).round() as u32, long]
    }
}

/// The grid's web profile address (LLGridManager::getWebProfileURL):
/// Second Life's, the beta grid's, or the `web_profile_url` of the login.
pub fn grid_web_profile(grid: &crate::settings::GridChoice, login_url: &str) -> Option<String> {
    match grid {
        crate::settings::GridChoice::SecondLife => Some("https://my.secondlife.com/".to_owned()),
        crate::settings::GridChoice::SecondLifeBeta => Some("https://my.secondlife-beta.com/".to_owned()),
        crate::settings::GridChoice::Custom => Some(login_url.to_owned()).filter(|u| !u.trim().is_empty()),
    }
}

/// Normalize the profile base without changing case-sensitive paths.
pub fn profile_url(base: &str) -> Option<String> {
    let mut url = destination(base.trim(), false).ok()?;
    if url.query().is_some() {
        return None;
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Some(url.into())
}

/// A validated profile and the session secret scoped to it. HTTP is only
/// available through the demo constructor, which supplies its own cookie.
#[derive(Clone)]
pub struct Web {
    profile: reqwest::Url,
    openid: String,
    user_agent: String,
    demo: bool,
}

impl Web {
    pub fn new(base: &str, openid: String, user_agent: String) -> Result<Self, Error> {
        let base = profile_url(base).ok_or(Error::UnsafeDestination)?;
        Ok(Self {
            profile: destination(&base, false)?,
            openid: cookie_pair(&openid).ok_or(Error::NoCookie)?,
            user_agent,
            demo: false,
        })
    }

    pub fn demo(server: &demo::Server, user_agent: String) -> Result<Self, Error> {
        Ok(Self {
            profile: destination(&server.url, true)?,
            openid: demo::OPENID.into(),
            user_agent,
            demo: true,
        })
    }

    fn profile_destination(&self, base: &reqwest::Url, reference: &str) -> Result<reqwest::Url, Error> {
        let url = base.join(reference).map_err(|_| Error::UnsafeDestination)?;
        destination(url.as_str(), self.demo)?;
        if url.origin() != self.profile.origin() {
            return Err(Error::UnsafeDestination);
        }
        Ok(url)
    }

    fn client(&self) -> Result<reqwest::Client, Error> {
        let builder = reqwest::Client::builder()
            .https_only(!self.demo)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT);
        // The demo must not send even its synthetic cookies through a proxy.
        let builder = if self.demo { builder.no_proxy() } else { builder };
        builder.build().map_err(|_| Error::Network)
    }
}

/// A profile session cookie is reusable only with the same profile base
/// and OpenID session. Keep the binding with the secret, including in caches.
#[derive(Clone)]
pub struct AuthCookie {
    profile: reqwest::Url,
    openid: String,
    cookie: String,
}

impl AuthCookie {
    pub fn matches(&self, web: &Web) -> bool {
        self.profile == web.profile && self.openid == web.openid
    }
}

fn destination(value: &str, demo: bool) -> Result<reqwest::Url, Error> {
    let url = reqwest::Url::parse(value).map_err(|_| Error::UnsafeDestination)?;
    let scheme_ok = if demo {
        url.scheme() == "http"
            && url.host_str().is_some_and(|h| {
                h.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            })
    } else {
        url.scheme() == "https"
    };
    if !scheme_ok || !url.has_host() || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(Error::UnsafeDestination);
    }
    Ok(url)
}

/// Set-Cookie attributes describe the server's cookie; they are not part
/// of the Cookie request header. Refuse malformed pairs before networking.
fn cookie_pair(raw: &str) -> Option<String> {
    let pair = raw.split(';').next()?.trim();
    let (name, value) = pair.split_once('=')?;
    let name_ok = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
    // RFC 6265 allows a quoted value; quotes must surround the complete
    // value and may not appear inside it. Keep the original wire pair.
    let value = match value.strip_prefix('"') {
        Some(quoted) => quoted.strip_suffix('"')?,
        None => value,
    };
    let value_ok = !value.is_empty()
        && value
            .bytes()
            .all(|b| matches!(b, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e));
    (name_ok && value_ok).then(|| pair.to_owned())
}

/// Where a post stopped (logged; the window says it failed, like
/// reportImageUploadStatus).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The web profile gave no session cookie.
    NoCookie,
    /// The upload configuration was refused (HTTP status).
    Config(u16),
    /// The upload configuration could not be read.
    ConfigData,
    /// The image was refused (HTTP status).
    Upload(u16),
    /// No address to finish the post.
    NoRedirect,
    /// The post's last step failed (HTTP status).
    Finish(u16),
    UnsafeDestination,
    Network,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoCookie => write!(f, "the web profile gave no cookie"),
            Error::Config(s) => write!(f, "upload configuration refused (HTTP {s})"),
            Error::ConfigData => write!(f, "upload configuration unreadable"),
            Error::Upload(s) => write!(f, "image refused (HTTP {s})"),
            Error::NoRedirect => write!(f, "no address after the upload"),
            Error::Finish(s) => write!(f, "post not finished (HTTP {s})"),
            Error::UnsafeDestination => write!(f, "insecure or unexpected profile destination"),
            Error::Network => write!(f, "network error"),
        }
    }
}

/// LLURI::escape: everything but the unreserved characters percent-encoded.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Authenticated requests are confined to the validated profile origin.
/// Storage requests deliberately use a separate, cookie-free path below.
fn profile_request(
    http: &reqwest::Client,
    url: &reqwest::Url,
    web: &Web,
    auth: Option<&AuthCookie>,
) -> Result<reqwest::RequestBuilder, Error> {
    web.profile_destination(&web.profile, url.as_str())?;
    let cookie = if let Some(auth) = auth {
        if !auth.matches(web) {
            return Err(Error::NoCookie);
        }
        format!("{}; {}", web.openid, auth.cookie)
    } else {
        web.openid.clone()
    };
    Ok(http
        .get(url.clone())
        .header("Accept", "*/*")
        .header("Content-Type", "application/xml")
        .header("Cookie", cookie)
        .header("User-Agent", web.user_agent.as_str())
        .timeout(TIMEOUT))
}

/// getOpenIDCookieCoro, with bounded same-origin redirects checked before
/// sending the OpenID cookie on each hop.
pub async fn auth_cookie(web: &Web) -> Result<AuthCookie, Error> {
    let http = web.client()?;
    let mut url = web.profile.clone();
    let mut redirects = 0;
    let response = loop {
        let response = profile_request(&http, &url, web, None)?.send().await.map_err(|_| Error::Network)?;
        if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            break response;
        }
        if redirects >= 10 {
            return Err(Error::UnsafeDestination);
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(Error::UnsafeDestination)?;
        url = web.profile_destination(&url, location)?;
        redirects += 1;
    };
    if !response.status().is_success() {
        log::warn!("photo: web profile: HTTP {}", response.status().as_u16());
        return Err(Error::NoCookie);
    }
    let cookie = response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(cookie_pair)
        .ok_or(Error::NoCookie)?;
    Ok(AuthCookie {
        profile: web.profile.clone(),
        openid: web.openid.clone(),
        cookie,
    })
}

/// uploadImageCoro: the upload configuration, the form posted to its URL,
/// then the 303's address.
pub async fn post(web: &Web, auth: &AuthCookie, png: Vec<u8>, caption: &str, add_location: bool) -> Result<(), Error> {
    let http = web.client()?;
    let mut config_url = web.profile_destination(&web.profile, "snapshots/s3_upload_config")?;
    config_url.set_query(Some(&format!(
        "caption={}&add_loc={}",
        escape(caption),
        if add_location { "1" } else { "0" }
    )));
    let response = profile_request(&http, &config_url, web, Some(auth))?
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() {
        return Err(Error::Config(response.status().as_u16()));
    }
    let bytes = response.bytes().await.map_err(|_| Error::Network)?;
    let config: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| Error::ConfigData)?;
    let upload_url = config["url"].as_str().filter(|u| !u.is_empty()).ok_or(Error::ConfigData)?;
    let upload_url = destination(upload_url, web.demo)?;
    // S3's success_action_redirect is a signed form field. Check it before
    // uploading; check the actual Location independently after the upload.
    let completion = config["data"]["success_action_redirect"]
        .as_str()
        .filter(|u| !u.is_empty())
        .ok_or(Error::ConfigData)?;
    web.profile_destination(&config_url, completion)?;
    let body = form(&config["data"], &png);
    // A storage provider gets the signed form and PNG, never profile
    // cookies, Authorization or automatic redirects (including 307/308).
    let response = http
        .post(upload_url.clone())
        .header("Accept", "*/*")
        .header("User-Agent", web.user_agent.as_str())
        .header("Content-Type", format!("multipart/form-data; boundary={BOUNDARY}"))
        .body(body)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    let status = response.status();
    if !status.is_success() && status != reqwest::StatusCode::SEE_OTHER {
        return Err(Error::Upload(status.as_u16()));
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .filter(|l| !l.is_empty())
        .ok_or(Error::NoRedirect)?;
    let location = web.profile_destination(&upload_url, location)?;
    let response = profile_request(&http, &location, web, Some(auth))?
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(Error::Finish(response.status().as_u16()));
    }
    Ok(())
}

/// buildPostData: the storage form's fields, in the order the server needs,
/// then the PNG.
pub fn form(data: &serde_json::Value, png: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(png.len() + 2048);
    for name in [
        "key",
        "AWSAccessKeyId",
        "acl",
        "Content-Type",
        "policy",
        "signature",
        "success_action_redirect",
    ] {
        let value = match &data[name] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        };
        body.extend_from_slice(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"snapshot.png\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// The PNG of a photo (RGBA rows top to bottom), like LLImagePNG.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    if rgba.len() != width as usize * height as usize * 4 {
        return Err("photo pixels".into());
    }
    let rgb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut out = Vec::new();
    let progress = std::sync::atomic::AtomicU32::new(0);
    super::save::encode(&mut out, width, height, &rgb, super::FileFormat::Png, 0, &progress)?;
    Ok(out)
}

pub mod demo;

#[cfg(test)]
mod security_tests;
