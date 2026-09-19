//! Referenzfaelle S1-S16 aus docs/spec-video-chat.md, Etappe 2a.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;

use url::Host;

use super::address::{check_fetch_url, filter_resolved, FetchTarget};
use super::*;

/// Lokaler HTTP-Server fuer die Netzwerkfaelle (Muster der Client-Tests).
struct TestServer {
    addr: SocketAddr,
    requests: Arc<AtomicUsize>,
    request_lines: Arc<Mutex<Vec<String>>>,
}

impl TestServer {
    fn start<F>(handler: F) -> Self
    where
        F: Fn(&mut TcpStream) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let request_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let counter = requests.clone();
        let recorded = request_lines.clone();
        let handler = Arc::new(handler);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                counter.fetch_add(1, Ordering::SeqCst);
                let handler = handler.clone();
                let recorded = recorded.clone();
                std::thread::spawn(move || {
                    let line = read_request(&mut stream);
                    recorded.lock().unwrap().push(line);
                    handler(&mut stream);
                });
            }
        });
        Self {
            addr,
            requests,
            request_lines,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// URL ueber einen Namen, der auf den Testserver zeigt (fuer den Resolver).
    fn host_url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.addr.port())
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn request_lines(&self) -> Vec<String> {
        self.request_lines.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    let _ = reader.read_line(&mut line);
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" || header == "\n" {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    if content_length > 0 {
        let mut body = vec![0u8; content_length];
        let _ = reader.read_exact(&mut body);
    }
    line.trim_end().to_string()
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    respond_with(stream, status, content_type, body, None);
}

fn respond_redirect(stream: &mut TcpStream, location: &str) {
    respond_with(stream, "302 Found", "text/plain", "", Some(location));
}

fn respond_with(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
    location: Option<&str>,
) {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(location) = location {
        response.push_str(&format!("Location: {location}\r\n"));
    }
    response.push_str("Connection: close\r\n\r\n");
    response.push_str(body);
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Ausnahme nur fuer Tests: der lokale Testserver ist erlaubt, alles andere
/// unterliegt weiterhin der Sperrliste. Die Produktion nutzt `is_blocked_ip`.
fn allow_test_server() -> Arc<dyn Fn(IpAddr) -> bool + Send + Sync> {
    Arc::new(|ip: IpAddr| ip != IpAddr::from(Ipv4Addr::LOCALHOST))
}

fn parse(url: &str) -> Url {
    Url::parse(url).unwrap()
}

fn target(url: &str) -> Result<FetchTarget, WebError> {
    check_fetch_url(&parse(url), &is_blocked_ip)
}

// ------------------------------------------------------ S1-S9: Adressen ----

#[test]
fn s1_to_s3_loopback_urls_are_blocked() {
    for url in ["http://127.0.0.1/", "http://127.0.0.2/", "http://[::1]/"] {
        assert_eq!(
            target(url),
            Err(WebError::AddressNotAllowed),
            "{url} muss gesperrt sein"
        );
    }
}

#[test]
fn s4_ipv4_mapped_loopback_is_blocked() {
    assert_eq!(
        target("http://[::ffff:127.0.0.1]/"),
        Err(WebError::AddressNotAllowed)
    );
}

#[test]
fn s5_nat64_embedded_loopback_is_blocked() {
    assert_eq!(
        target("http://[64:ff9b::7f00:1]/"),
        Err(WebError::AddressNotAllowed)
    );
    assert_eq!(
        target("http://[64:ff9b:1::7f00:1]/"),
        Err(WebError::AddressNotAllowed)
    );
}

#[test]
fn s6_6to4_embedded_loopback_is_blocked() {
    assert_eq!(
        target("http://[2002:7f00:1::]/"),
        Err(WebError::AddressNotAllowed)
    );
}

#[test]
fn s7_numeric_ipv4_forms_are_normalized_and_blocked() {
    for url in ["http://2130706433/", "http://127.1/", "http://0x7f000001/"] {
        let parsed = parse(url);
        assert_eq!(
            parsed.host(),
            Some(Host::Ipv4(Ipv4Addr::LOCALHOST)),
            "{url} muss von der url-Crate zu IPv4 normalisiert werden"
        );
        assert_eq!(
            check_fetch_url(&parsed, &is_blocked_ip),
            Err(WebError::AddressNotAllowed),
            "{url} muss gesperrt sein"
        );
    }
}

#[test]
fn s8_credentials_in_url_are_rejected_without_dns() {
    let parsed = parse("http://user@example.com/");
    assert!(!parsed.username().is_empty());
    assert_eq!(
        check_fetch_url(&parsed, &is_blocked_ip),
        Err(WebError::InvalidUrl),
        "Zugangsdaten werden vor jedem DNS abgelehnt"
    );
}

#[test]
fn s9_non_http_scheme_is_blocked() {
    assert_eq!(target("file:///etc/passwd"), Err(WebError::InvalidUrl));
}

#[test]
fn s16_address_table_is_blocked_or_allowed() {
    for value in [
        "10.0.0.1",
        "172.16.0.1",
        "192.168.1.1",
        "100.64.0.1",
        "169.254.1.1",
        "224.0.0.1",
        "255.255.255.255",
        "fe80::1",
        "fc00::1",
        "ff02::1",
        "0.0.0.0",
        "::",
        "2001:0:0:0:0:0:0:1",
    ] {
        assert!(
            is_blocked_ip(value.parse().unwrap()),
            "{value} muss gesperrt sein"
        );
    }
    for value in [
        "8.8.8.8",
        "2606:4700::1111",
        "1.1.1.1",
        "2001:4860:4860::8888",
    ] {
        assert!(
            !is_blocked_ip(value.parse().unwrap()),
            "{value} muss erlaubt sein"
        );
    }
}

// ------------------------------------------------ S10-S14: Netzwerkpfad ----

#[tokio::test]
async fn s10_redirect_to_a_blocked_address_fails_at_the_hop() {
    let server = TestServer::start(|stream| respond_redirect(stream, "http://169.254.169.254/"));
    let error = fetch_page_with(&server.url("/start"), allow_test_server())
        .await
        .unwrap_err();

    assert_eq!(error, WebError::AddressNotAllowed);
    assert_eq!(server.requests(), 1, "kein zweiter Connect");
}

#[test]
fn s11_resolved_addresses_are_filtered() {
    let public = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443);
    let blocked = SocketAddr::new("::ffff:10.0.0.1".parse::<IpAddr>().unwrap(), 443);
    assert_eq!(
        filter_resolved(vec![public, blocked], &is_blocked_ip),
        Ok(vec![public])
    );

    let only_blocked = vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 443)];
    assert_eq!(
        filter_resolved(only_blocked, &is_blocked_ip),
        Err(WebError::NoAllowedAddress)
    );
}

#[tokio::test]
async fn s11b_blocked_domain_is_rejected_before_connecting() {
    let server = TestServer::start(|stream| respond(stream, "200 OK", "text/plain", "hallo"));
    // Der Name zeigt auf den Testserver, wird aber vom Resolver gefiltert.
    let error = fetch_page_with(
        &server.host_url("localhost", "/x"),
        Arc::new(|_ip: IpAddr| true),
    )
    .await
    .unwrap_err();

    assert_eq!(error, WebError::NoAllowedAddress);
    assert_eq!(
        server.requests(),
        0,
        "gefilterte Adressen duerfen nicht verbunden werden"
    );
}

#[tokio::test]
async fn s12_allowed_and_rejected_content_types() {
    let html = TestServer::start(|stream| {
        respond(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            "<p>Hallo Welt</p>",
        )
    });
    assert_eq!(
        fetch_page_with(&html.url("/"), allow_test_server())
            .await
            .unwrap(),
        "Hallo Welt"
    );

    let json = TestServer::start(|stream| respond(stream, "200 OK", "application/json", "{}"));
    assert_eq!(
        fetch_page_with(&json.url("/"), allow_test_server())
            .await
            .unwrap_err(),
        WebError::UnsupportedContentType("application/json".to_string())
    );
}

#[tokio::test]
async fn s13_body_over_limit_aborts_the_download() {
    let big = "a".repeat(3 * 1024 * 1024);
    let server = TestServer::start(move |stream| respond(stream, "200 OK", "text/html", &big));
    let error = fetch_page_with(&server.url("/"), allow_test_server())
        .await
        .unwrap_err();

    assert_eq!(error, WebError::BodyTooLarge);
}

#[tokio::test]
async fn s14_sixth_redirect_is_rejected() {
    let server = TestServer::start(|stream| respond_redirect(stream, "/loop"));
    let error = fetch_page_with(&server.url("/loop"), allow_test_server())
        .await
        .unwrap_err();

    assert_eq!(error, WebError::TooManyRedirects);
    assert_eq!(
        server.requests(),
        MAX_REDIRECTS + 1,
        "hoechstens {MAX_REDIRECTS} Weiterleitungen, danach kein weiterer Request"
    );
}

// --------------------------------------------------------- S15: Websuche ----

#[tokio::test]
async fn s15_searxng_on_loopback_works_and_fetch_page_stays_blocked() {
    let long_snippet = "x".repeat(400);
    let body = format!(
        r#"{{"results":[{{"title":"Rust","url":"https://example.com/rust","content":"Eine Sprache"}},{{"title":"Zweiter","url":"https://example.com/2"}},{{"title":"Lang","url":"https://example.com/3","content":"{long_snippet}"}}]}}"#
    );
    let body: &'static str = Box::leak(body.into_boxed_str());
    let server =
        TestServer::start(move |stream| respond(stream, "200 OK", "application/json", body));

    let results = web_search(&server.url(""), "rust sse").await.unwrap();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].title, "Rust");
    assert_eq!(results[0].url, "https://example.com/rust");
    assert_eq!(results[0].content, "Eine Sprache");
    assert_eq!(results[1].content, "", "fehlender Auszug bleibt leer");
    assert_eq!(results[2].content.chars().count(), MAX_SNIPPET_CHARS);

    let request_line = server.request_lines()[0].clone();
    assert!(
        request_line.contains("q=rust+sse") && request_line.contains("format=json"),
        "Suchergebnis-Anfrage: {request_line}"
    );

    // Dieselbe Loopback-Adresse bleibt fuer fetch_page gesperrt.
    assert_eq!(
        fetch_page(&server.url("/")).await.unwrap_err(),
        WebError::AddressNotAllowed
    );

    assert_eq!(
        search_endpoint("http://127.0.0.1:8080"),
        "http://127.0.0.1:8080/search"
    );
    assert_eq!(
        search_endpoint("  http://127.0.0.1:8080/  "),
        "http://127.0.0.1:8080/search"
    );
    assert_eq!(
        search_endpoint("http://127.0.0.1:8080/search/"),
        "http://127.0.0.1:8080/search"
    );
}

#[tokio::test]
async fn s15b_search_returns_at_most_eight_results() {
    let items = (0..12)
        .map(|index| json!({"title": format!("T{index}"), "url": format!("https://example.com/{index}")}))
        .collect::<Vec<_>>();
    let body = json!({ "results": items }).to_string();
    let body: &'static str = Box::leak(body.into_boxed_str());
    let server =
        TestServer::start(move |stream| respond(stream, "200 OK", "application/json", body));

    let results = web_search(&server.url(""), "irgendwas").await.unwrap();

    assert_eq!(results.len(), MAX_SEARCH_RESULTS);
}

// -------------------------------------------------------- HTML und Schema ---

#[test]
fn html_to_text_removes_scripts_and_decodes_entities() {
    let html = "<html><head><style>body{color:red}</style><script>alert('x')</script></head>\
                <body><h1>Titel &amp; Co</h1><p>Zeile&nbsp;eins</p><noscript>ohne js</noscript>\
                <p>ü &uuml;ber &#65; &#x42;</p></body></html>";
    let text = html_to_text(html);

    assert!(!text.contains("alert"), "{text}");
    assert!(!text.contains("color:red"), "{text}");
    assert!(!text.contains("ohne js"), "{text}");
    assert!(text.contains("Titel & Co"), "{text}");
    assert!(text.contains("Zeile eins"), "{text}");
    assert!(text.contains("ü über A B"), "{text}");
}

#[test]
fn tool_schemas_match_the_spec() {
    let definitions = tool_definitions();
    assert_eq!(
        definitions[0],
        json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Search the web. Returns titles, URLs and snippets.",
                "parameters": {
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"]
                }
            }
        })
    );
    assert_eq!(
        definitions[1],
        json!({
            "type": "function",
            "function": {
                "name": "fetch_page",
                "description": "Fetch a public web page and return its text content.",
                "parameters": {
                    "type": "object",
                    "properties": {"url": {"type": "string"}},
                    "required": ["url"]
                }
            }
        })
    );
}
