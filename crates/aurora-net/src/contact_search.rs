//! Captured host resident lookup (Folderstorm abbe3cb0 fssessionworker.cpp,
//! LGPL-2.1-or-later). Ordinary native query-only searches stay independent.
//! The process lease outlives a discarded World/Session and is released only
//! when the HTTP future completes or is dropped. Never retain a viewer pointer.
use crate::{EventSender, LoginSource, NetEvent};
use aurora_llsd::Llsd;
use reqwest::Url;
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::sync::Semaphore;
use uuid::Uuid;

pub const MAX_RESULTS: usize = 100;
const MAX_BYTES: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);
static SEARCH: Semaphore = Semaphore::const_new(1);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resident {
    pub id: Uuid,
    pub display: String,
    pub username: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Results {
    pub residents: Vec<Resident>,
    pub limited: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    SessionEnded,
    Busy,
    Unavailable,
    Invalid,
    Transport,
    Http(u16),
    TooLarge,
    Malformed,
    Timeout,
}

fn search_url(cap: Option<&str>, query: &str) -> Result<Url, Error> {
    if !(2..=127).contains(&query.len()) || query.chars().any(char::is_control) {
        return Err(Error::Invalid);
    }
    let mut url = Url::parse(cap.ok_or(Error::Unavailable)?).map_err(|_| Error::Unavailable)?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err(Error::Unavailable);
    }
    // Preserve the capability's existing opaque path/query authentication.
    // Reject ambiguous pre-filled operation parameters rather than duplicating
    // them or rewriting unrelated auth pairs through a lossy query roundtrip.
    if url.query_pairs().any(|(key, _)| key == "names" || key == "page_size") {
        return Err(Error::Invalid);
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    url.query_pairs_mut()
        .append_pair("page_size", "100")
        .append_pair("names", &query.replace('.', " "));
    Ok(url)
}
fn bounded(text: &str, limit: usize) -> String {
    let mut output = String::new();
    for ch in text.chars().filter(|ch| !ch.is_control()) {
        if output.len() + ch.len_utf8() > limit {
            break;
        }
        output.push(ch);
    }
    output
}
fn parse(bytes: &[u8], owner: Uuid) -> Result<Results, Error> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let value = aurora_llsd::from_xml(bytes).map_err(|_| Error::Malformed)?;
    let Llsd::Map(map) = value else {
        return Err(Error::Malformed);
    };
    let Some(Llsd::Array(agents)) = map.get("agents") else {
        return Err(Error::Malformed);
    };
    let mut seen = BTreeSet::new();
    let mut residents = Vec::new();
    // Bound inspected rows, not just accepted rows. A larger response is
    // explicitly limited and can never enlarge captured selection authority.
    for row in agents.iter().take(MAX_RESULTS) {
        let id = row["id"].as_uuid();
        if id.is_nil() || id == owner || !seen.insert(id) {
            continue;
        }
        let Llsd::String(display) = &row["display_name"] else {
            continue;
        };
        let Llsd::String(username) = &row["username"] else {
            continue;
        };
        let username = bounded(username, 127);
        let mut display = bounded(display, 255);
        if display.is_empty() {
            display = username.clone();
        }
        if display.is_empty() {
            continue;
        }
        residents.push(Resident { id, display, username });
    }
    Ok(Results {
        residents,
        limited: agents.len() > MAX_RESULTS,
    })
}
async fn fetch(http: &reqwest::Client, url: Url, owner: Uuid) -> Result<Results, Error> {
    let mut response = http
        .get(url)
        .header("Accept", "application/llsd+xml")
        .send()
        .await
        .map_err(|_| Error::Transport)?;
    if !response.status().is_success() {
        return Err(Error::Http(response.status().as_u16()));
    }
    if response.content_length().is_some_and(|length| length > MAX_BYTES as u64) {
        return Err(Error::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
        if chunk.len() > MAX_BYTES - bytes.len() {
            return Err(Error::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    parse(&bytes, owner)
}
pub(crate) fn launch(
    http: reqwest::Client,
    cap: Option<&str>,
    query: &str,
    owner: Uuid,
    source: LoginSource,
    request: Uuid,
    events: EventSender,
) {
    let emit = |result| {
        events.send(NetEvent::ContactSearch {
            source: source.clone(),
            request,
            result,
        });
    };
    if request.is_nil() {
        emit(Err(Error::Invalid));
        return;
    }
    let url = match search_url(cap, query) {
        Ok(url) => url,
        Err(error) => {
            emit(Err(error));
            return;
        }
    };
    let permit = match SEARCH.try_acquire() {
        Ok(permit) => permit,
        Err(_) => {
            emit(Err(Error::Busy));
            return;
        }
    };
    tokio::spawn(async move {
        let result = tokio::time::timeout(TIMEOUT, fetch(&http, url, owner))
            .await
            .unwrap_or(Err(Error::Timeout));
        // Completion is visible only after another real request can acquire.
        drop(permit);
        events.send(NetEvent::ContactSearch { source, request, result });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contact_search_url_preserves_capability_auth_and_encodes_normalized_query_once() {
        let url = search_url(Some("https://example.test/cap%2Fopaque?auth=a%2Fb+%20"), "Some.Name & 界").unwrap();
        assert_eq!(url.path(), "/cap%2Fopaque/");
        assert!(url.query().unwrap().starts_with("auth=a%2Fb+%20&"));
        let pairs: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
        assert_eq!(pairs["names"], "Some Name & 界");
        assert_eq!(pairs["page_size"], "100");
        for query in ["a", "\nname", &"界".repeat(43)] {
            assert_eq!(search_url(Some("https://example.test/cap"), query), Err(Error::Invalid));
        }
        assert_eq!(search_url(None, "name"), Err(Error::Unavailable));
        assert_eq!(search_url(Some("https://example.test/cap?names=old"), "name"), Err(Error::Invalid));
    }
    #[test]
    fn contact_search_parser_bounds_labels_and_membership_and_rejects_missing_schema() {
        let owner = Uuid::from_u128(1);
        let peer = Uuid::from_u128(2);
        let row = aurora_llsd::llsd_map! {"id"=>peer,"display_name"=>"界".repeat(200),"username"=>"u".repeat(200)};
        let data = aurora_llsd::llsd_map! {"agents"=>vec![row.clone(),row,aurora_llsd::llsd_map!{"id"=>owner,"display_name"=>"Self","username"=>"self"}]};
        let result = parse(aurora_llsd::to_xml(&data).as_bytes(), owner).unwrap();
        assert_eq!(result.residents.len(), 1);
        assert_eq!(result.residents[0].id, peer);
        assert_eq!(result.residents[0].display.len(), 255);
        assert_eq!(result.residents[0].username.len(), 127);
        assert_eq!(parse(b"<llsd><map/></llsd>", owner), Err(Error::Malformed));
        assert_eq!(parse(&vec![b' '; MAX_BYTES + 1], owner), Err(Error::TooLarge));
    }
}
