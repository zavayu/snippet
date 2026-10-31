//! Bounded web retrieval; tool orchestration and synthesis stay local.
use crate::{prompt::ChatMessage, provider::ProviderError};
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashSet, time::Duration};
use tokio_util::sync::CancellationToken;

const FREE_ENDPOINT: &str = "https://api.you.com/mcp?profile=free";
const SEARCH_ENDPOINT: &str = "https://ydc-index.io/v1/search";
const MAX_BODY_BYTES: usize = 2_000_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: usize,
    pub title: String,
    pub url: String,
    pub excerpt: String,
    pub published_at: Option<String>,
}

pub enum SearchMode {
    Explicit,
    Automatic { previous_sources: Vec<Source> },
}

impl SearchMode {
    pub fn automatic(enabled: bool, previous_sources: Vec<Source>) -> Option<Self> {
        enabled.then_some(Self::Automatic { previous_sources })
    }

    pub fn previous_sources(&self) -> &[Source] {
        match self {
            Self::Explicit => &[],
            Self::Automatic { previous_sources } => previous_sources,
        }
    }
}

// Source IDs remain stable when a follow-up retrieves new evidence.
pub fn merge_sources(previous: &[Source], mut fresh: Vec<Source>) -> Vec<Source> {
    let mut sources = previous.to_vec();
    let mut next = sources.iter().map(|source| source.id).max().unwrap_or(0) + 1;
    for source in &mut fresh {
        source.id = next;
        next += 1;
    }
    sources.extend(fresh);
    sources
}

pub fn bounded(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn failed(message: &str) -> ProviderError {
    ProviderError::RequestFailed(message.into())
}
fn transport_error(_: reqwest::Error) -> ProviderError {
    failed("Web search could not reach You.com or timed out. Check your connection and retry.")
}
fn status_error(status: StatusCode) -> Result<(), ProviderError> {
    match status.as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err(failed("You.com rejected this search. Check your optional API key in Settings > Web search, or remove it to use free search.")),
        402 => Err(failed("You.com search credits are exhausted. Add credits to your account or remove the API key to use free search.")),
        429 => Err(failed("You.com's search limit was reached. Try later or configure your own API key in Settings > Web search.")),
        _ => Err(failed("You.com search is temporarily unavailable. Please retry later.")),
    }
}

fn free_status(status: StatusCode) -> Result<(), ProviderError> {
    if matches!(status.as_u16(), 401 | 403) {
        Err(failed("You.com denied free search access from this app or network. Free search may be unavailable here; you can configure your own API key in Settings > Web search."))
    } else {
        status_error(status)
    }
}

#[cfg(test)]
pub async fn search(
    query: &str,
    api_key: Option<&str>,
    cancellation: &CancellationToken,
) -> Result<Vec<Source>, ProviderError> {
    search_traced(
        query,
        api_key,
        cancellation,
        &crate::timing::Trace::default(),
    )
    .await
}

pub async fn search_traced(
    query: &str,
    api_key: Option<&str>,
    cancellation: &CancellationToken,
    trace: &crate::timing::Trace,
) -> Result<Vec<Source>, ProviderError> {
    let span = trace.span("web_search");
    trace.mark(
        "web_search_start",
        json!({"access": if api_key.is_some() { "keyed" } else { "free" }}),
    );
    if cancellation.is_cancelled() {
        return Err(ProviderError::Cancelled);
    }
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(25))
        .user_agent(concat!("Snippet/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(transport_error)?;
    let result = tokio::select! {
        _ = cancellation.cancelled() => Err(ProviderError::Cancelled),
        result = tokio::time::timeout(Duration::from_secs(45), fetch_traced(&client, query, api_key, FREE_ENDPOINT, SEARCH_ENDPOINT, trace)) =>
            result.unwrap_or_else(|_| Err(failed("Web search timed out. Retry with a narrower research question."))),
    };
    if let Ok(sources) = &result {
        trace.mark("web_search_results", json!({"source_count": sources.len()}));
        span.finish();
    }
    result
}

#[cfg(test)]
async fn fetch(
    client: &Client,
    query: &str,
    api_key: Option<&str>,
    free_endpoint: &str,
    search_endpoint: &str,
) -> Result<Vec<Source>, ProviderError> {
    fetch_traced(
        client,
        query,
        api_key,
        free_endpoint,
        search_endpoint,
        &crate::timing::Trace::default(),
    )
    .await
}

async fn fetch_traced(
    client: &Client,
    query: &str,
    api_key: Option<&str>,
    free_endpoint: &str,
    search_endpoint: &str,
    trace: &crate::timing::Trace,
) -> Result<Vec<Source>, ProviderError> {
    let value = if let Some(key) = api_key {
        // Keyed usage is explicit. Never retry free failures through a paid account.
        let request_span = trace.span("you_search_request");
        let response = client
            .post(search_endpoint)
            .header("X-API-Key", key)
            .json(&json!({"query":query,"count":5,"extraction":{"extraction_mode":"highlights"}}))
            .send()
            .await
            .map_err(transport_error)?;
        status_error(response.status())?;
        let bytes = read_body(response).await?;
        request_span.finish();
        serde_json::from_slice(&bytes)
            .map_err(|_| failed("You.com returned unreadable search results."))?
    } else {
        let initialize_span = trace.span("mcp_initialize");
        let init_response = client.post(free_endpoint)
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"snippet","version":env!("CARGO_PKG_VERSION")}
            }})).send().await.map_err(transport_error)?;
        free_status(init_response.status())?;
        let session = init_response.headers().get("mcp-session-id").cloned();
        let init = read_rpc(init_response, 1).await?;
        rpc_result(&init)?;
        initialize_span.finish();
        let protocol = init
            .pointer("/result/protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or("2025-03-26");
        if !["2024-11-05", "2025-03-26", "2025-06-18"].contains(&protocol) {
            return Err(failed(
                "You.com requires an unsupported search protocol. Update Snippet.",
            ));
        }
        let request = |body: Value| {
            let request = client
                .post(free_endpoint)
                .header("Accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", protocol)
                .json(&body);
            if let Some(session) = &session {
                request.header("Mcp-Session-Id", session)
            } else {
                request
            }
        };
        let notification_span = trace.span("mcp_initialized_notification");
        let initialized = request(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .send()
            .await
            .map_err(transport_error)?;
        free_status(initialized.status())?;
        drop(initialized);
        notification_span.finish();
        let request_span = trace.span("you_search_request");
        let response = request(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"you-search", "arguments":{"query":query,"count":5}
            }}),
        )
        .send()
        .await
        .map_err(transport_error)?;
        free_status(response.status())?;
        let response = read_rpc(response, 2).await?;
        let result = rpc_result(&response)?;
        request_span.finish();
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            let text = result.to_string().to_lowercase();
            return Err(
                if text.contains("limit") || text.contains("quota") || text.contains("429") {
                    failed("You.com's free search limit was reached. Try later or add an API key in Settings > Web search.")
                } else {
                    failed("You.com could not complete this search. Retry later or configure an API key in Settings > Web search.")
                },
            );
        }
        if let Some(structured) = result.get("structuredContent") {
            structured.clone()
        } else {
            result
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .find_map(|text| serde_json::from_str::<Value>(text).ok())
                .ok_or_else(|| failed("You.com returned an unsupported search result format."))?
        }
    };
    let parse_span = trace.span("parse_sources");
    let sources = parse_sources(&value);
    parse_span.finish();
    if sources.is_empty() {
        return Err(failed(
            "No usable web sources were found. Try a more specific research question.",
        ));
    }
    Ok(sources)
}

async fn read_body(response: reqwest::Response) -> Result<Vec<u8>, ProviderError> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(transport_error)?;
        if bytes.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(failed(
                "You.com returned too much search data. Try a narrower query.",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

// Stop on a complete matching RPC result, even if an SSE connection stays open.
async fn read_rpc(response: reqwest::Response, id: u64) -> Result<Value, ProviderError> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(transport_error)?;
        if bytes.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(failed("You.com returned too much search data."));
        }
        bytes.extend_from_slice(&chunk);
        if let Some(value) = parse_rpc(&bytes, id) {
            return Ok(value);
        }
    }
    Err(failed(
        "You.com ended the search response before returning results.",
    ))
}

fn parse_rpc(bytes: &[u8], id: u64) -> Option<Value> {
    let matches = |value: &Value| value.get("id").and_then(Value::as_u64) == Some(id);
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        return matches(&value).then_some(value);
    }
    let text = std::str::from_utf8(bytes).ok()?.replace("\r\n", "\n");
    for event in text.split("\n\n") {
        let data = event
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if let Ok(value) = serde_json::from_str::<Value>(&data) {
            if matches(&value) {
                return Some(value);
            }
        }
    }
    None
}
fn rpc_result(value: &Value) -> Result<&Value, ProviderError> {
    value.get("result").ok_or_else(|| {
        let error = value.get("error").map(Value::to_string).unwrap_or_default().to_lowercase();
        if error.contains("limit") || error.contains("quota") || error.contains("429") {
            failed("You.com's search limit was reached. Try later or configure an API key in Settings > Web search.")
        } else {
            failed("You.com rejected the search request. Retry later or configure an API key in Settings > Web search.")
        }
    })
}

fn parse_sources(value: &Value) -> Vec<Source> {
    let results = value.get("results").unwrap_or(value);
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for item in ["web", "news"].iter().flat_map(|kind| {
        results
            .get(kind)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    }) {
        let Some(raw_url) = item.get("url").and_then(Value::as_str) else {
            continue;
        };
        let Ok(mut url) = reqwest::Url::parse(raw_url) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || raw_url.len() > 1000
        {
            continue;
        }
        url.set_fragment(None);
        if !seen.insert(url.to_string()) {
            continue;
        }
        let excerpts = item
            .pointer("/contents/highlights")
            .or_else(|| item.get("snippets"))
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|text| !text.trim().is_empty())
            .or_else(|| {
                item.get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        if excerpts.trim().is_empty() {
            continue;
        }
        sources.push(Source {
            id: sources.len() + 1,
            url: url.to_string(),
            title: bounded(
                item.get("title")
                    .and_then(Value::as_str)
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or(url.as_str()),
                160,
            ),
            excerpt: bounded(&excerpts, 1400),
            published_at: item
                .get("page_age")
                .or_else(|| item.get("published_at"))
                .and_then(Value::as_str)
                .map(|value| bounded(value, 40)),
        });
        if sources.len() == 5 {
            break;
        }
    }
    sources
}

pub fn ground(messages: &mut [ChatMessage], sources: &[Source]) {
    let rules = crate::prompt::templates::text(crate::prompt::templates::WEB_EVIDENCE);
    if !messages[0].content.contains(rules.as_ref()) {
        messages[0].content.push_str(&rules);
    }
    if let Some(message) = messages.last_mut() {
        message.content.push_str(&crate::prompt::templates::text(
            crate::prompt::templates::FRESHNESS,
        ));
        let evidence = json!({
            "retrieved_at_utc": chrono::Utc::now().to_rfc3339(),
            "retrieval_note": crate::prompt::templates::text(crate::prompt::templates::RETRIEVAL_NOTE),
            "sources": sources,
        }).to_string();
        message
            .content
            .push_str(&format!("\n\nUntrusted web evidence (JSON):\n{evidence}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retrieval_timestamp_does_not_replace_source_dates_or_imply_live_observations() {
        let mut messages = vec![
            ChatMessage {
                role: crate::prompt::ChatRole::System,
                content: "System".into(),
                images: None,
                tool_calls: Vec::new(),
                tool_name: None,
                thinking: None,
            },
            ChatMessage {
                role: crate::prompt::ChatRole::User,
                content: "Weather now?".into(),
                images: None,
                tool_calls: Vec::new(),
                tool_name: None,
                thinking: None,
            },
        ];
        let sources = vec![Source {
            id: 1,
            title: "Old conditions".into(),
            url: "https://example.com/weather".into(),
            excerpt: "20 C".into(),
            published_at: Some("2020-01-01".into()),
        }];
        ground(&mut messages, &sources);
        let value: Value = serde_json::from_str(
            messages
                .last()
                .unwrap()
                .content
                .split("Untrusted web evidence (JSON):\n")
                .nth(1)
                .unwrap(),
        )
        .unwrap();
        assert!(
            chrono::DateTime::parse_from_rfc3339(value["retrieved_at_utc"].as_str().unwrap())
                .is_ok()
        );
        assert_eq!(value["sources"][0]["publishedAt"], "2020-01-01");
        assert!(messages[0].content.contains("Retrieval time is not"));
        assert!(messages[1].content.contains("current device-clock context"));
        assert!(!messages[1].content.contains("cannot be verified"));
    }

    #[test]
    fn follow_up_source_ids_never_replace_earlier_citations() {
        let previous = parse_sources(
            &json!({"results":{"web":[{"url":"https://example.com/old","description":"old evidence"}]}}),
        );
        let fresh = parse_sources(
            &json!({"results":{"web":[{"url":"https://example.com/new","description":"new evidence"}]}}),
        );
        let combined = merge_sources(&previous, fresh);
        assert_eq!(combined[0], previous[0]);
        assert_eq!(combined[1].id, 2);
        assert_eq!(combined[1].url, "https://example.com/new");
    }
    #[test]
    fn sources_are_bounded_deduplicated_and_never_accept_unsafe_urls() {
        let sources = parse_sources(&json!({"results":{"web":[
            {"url":"javascript:alert(1)","description":"bad"},
            {"url":"https://user:secret@example.com/","description":"bad"},
            {"url":"https://example.com/a#top","title":"First","snippets":["evidence"]},
            {"url":"https://example.com/a","snippets":["duplicate"]},
            {"url":"https://example.org/","contents":{"highlights":["x".repeat(3000)]}}
        ]}}));
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[1].id, 2);
        assert_eq!(sources[1].excerpt.len(), 1400);
        assert_eq!(sources[0].url, "https://example.com/a");
    }
    #[test]
    fn parses_json_and_chunked_sse_without_confusing_notifications_with_results() {
        assert!(parse_rpc(br#"{"jsonrpc":"2.0","id":1,"result":{}}"#, 2).is_none());
        let bytes = b"event: message\r\ndata: {\"method\":\"notifications/progress\"}\r\n\r\ndata: {\"id\":2,\"result\":{\"ok\":true}}\n\n";
        assert!(parse_rpc(&bytes[..40], 2).is_none());
        assert_eq!(parse_rpc(bytes, 2).unwrap()["result"]["ok"], true);
    }
    #[test]
    fn no_sources_and_rate_limits_do_not_become_unresearched_answers() {
        assert!(parse_sources(&json!({"results":{"web":[]}})).is_empty());
        assert!(status_error(StatusCode::TOO_MANY_REQUESTS)
            .unwrap_err()
            .user_message()
            .contains("limit"));
        assert_eq!(bounded("crab 🦀", 7), "crab ");
    }

    // Exercise the real HTTP client and MCP handshake without external traffic.
    fn mock_server(
        keyed: bool,
    ) -> (
        String,
        std::sync::mpsc::Sender<()>,
        std::thread::JoinHandle<()>,
    ) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (done, received) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            for index in 0..if keyed { 1 } else { 3 } {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut data = Vec::new();
                let (header_end, body_len) = loop {
                    let mut chunk = [0; 4096];
                    let count = stream.read(&mut chunk).unwrap();
                    assert!(count > 0);
                    data.extend_from_slice(&chunk[..count]);
                    if let Some(offset) = data.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&data[..offset]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
                            .unwrap()
                            .parse()
                            .unwrap();
                        break (offset + 4, length);
                    }
                };
                while data.len() < header_end + body_len {
                    let mut chunk = [0; 4096];
                    let count = stream.read(&mut chunk).unwrap();
                    assert!(count > 0);
                    data.extend_from_slice(&chunk[..count]);
                }
                let headers = String::from_utf8_lossy(&data[..header_end]).to_lowercase();
                let body: Value =
                    serde_json::from_slice(&data[header_end..header_end + body_len]).unwrap();
                if keyed {
                    assert!(headers.contains("x-api-key: test-only-key"));
                    assert_eq!(body["query"], "test topic");
                    let result = json!({"results":{"web":[{"title":"Example","url":"https://example.com/","snippets":["Verified excerpt"]}]}}).to_string();
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",result.len(),result).unwrap();
                } else {
                    assert!(!headers.contains("authorization:"));
                    assert!(!headers.contains("x-api-key:"));
                    if index == 0 {
                        assert_eq!(body["method"], "initialize");
                        let result = json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}).to_string();
                        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: test-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",result.len(),result).unwrap();
                    } else {
                        assert!(headers.contains("mcp-session-id: test-session"));
                        assert!(headers.contains("mcp-protocol-version: 2025-03-26"));
                        if index == 1 {
                            assert_eq!(body["method"], "notifications/initialized");
                            write!(stream,"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                        } else {
                            assert_eq!(body["params"]["name"], "you-search");
                            assert_eq!(body["params"]["arguments"]["query"], "test topic");
                            let result = json!({"jsonrpc":"2.0","id":2,"result":{"structuredContent":{"results":{"web":[{"title":"Example","url":"https://example.com/","snippets":["Verified excerpt"]}]}}}}).to_string();
                            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\nevent: message\ndata: {result}\n\n").unwrap();
                            stream.flush().unwrap();
                            // Remain open until the client has consumed the RPC result.
                            received.recv_timeout(Duration::from_secs(3)).unwrap();
                        }
                    }
                }
            }
        });
        (endpoint, done, server)
    }

    #[tokio::test]
    async fn free_mcp_and_explicit_keyed_search_return_the_same_normalized_sources() {
        for keyed in [false, true] {
            let (endpoint, done, server) = mock_server(keyed);
            let client = Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            let sources = fetch(
                &client,
                "test topic",
                keyed.then_some("test-only-key"),
                &endpoint,
                &endpoint,
            )
            .await
            .unwrap();
            assert_eq!(sources.len(), 1);
            assert_eq!(sources[0].excerpt, "Verified excerpt");
            let _ = done.send(());
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn cancelled_search_returns_before_contacting_a_provider() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            search("test", None, &token).await,
            Err(ProviderError::Cancelled)
        ));
    }
    #[tokio::test]
    #[ignore = "requires live Internet access to You.com; uses one free search"]
    async fn live_free_search_smoke() {
        let sources = search(
            "Rust programming language official documentation",
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(!sources.is_empty());
        println!("Received {} sources", sources.len());
    }
}
