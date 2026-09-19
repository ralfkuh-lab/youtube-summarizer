//! Referenzfaelle S1-S16 aus docs/spec-video-chat.md, Etappe 2a.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;

use url::Host;

use url::Url;

use super::address::{check_fetch_url, filter_resolved, is_blocked_ip, FetchTarget};
use super::fetch::{fetch_page, fetch_page_with};
use super::html::html_to_text;
use super::search::{search_endpoint, web_search};
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

/// Antwort ganz ohne Content-Type (K9).
fn respond_without_content_type(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Fuehrt einen `#[ignore]`-Test im Kindprozess aus. Proxy-Variablen werden nur
/// dort gesetzt, damit parallele Tests im Elternprozess unberuehrt bleiben.
///
/// Geprueft wird die Ausgabe: libtest endet auch dann mit 0, wenn `--exact` auf
/// keinen Test passt. Deshalb muss genau ein Test gelaufen und bestanden sein.
fn run_ignored_child(test_name: &str, env: &[(&str, String)]) -> Result<(), String> {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--ignored", "--exact", test_name, "--nocapture"])
        .env_remove("NO_PROXY")
        .env_remove("no_proxy");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command
        .output()
        .map_err(|error| format!("Kindprozess konnte nicht gestartet werden: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        return Err(format!(
            "Kindprozess fehlgeschlagen ({:?}):\n{stdout}",
            output.status
        ));
    }
    if !stdout.contains("test result: ok. 1 passed") {
        return Err(format!(
            "genau ein bestandener Test erwartet, Ausgabe war:\n{stdout}"
        ));
    }
    Ok(())
}

/// Der Helfer muss einen Tippfehler im Testnamen bemerken (libtest endet sonst
/// mit 0).
#[test]
fn q2_run_ignored_child_rejects_an_unknown_test_name() {
    let result = run_ignored_child("websearch::tests::diesen_test_gibt_es_nicht", &[]);
    assert!(
        result.is_err(),
        "ein nicht existierender Test darf nicht als Erfolg gelten"
    );
}

fn proxy_env(url: &str) -> Vec<(&'static str, String)> {
    vec![
        ("HTTP_PROXY", url.to_string()),
        ("http_proxy", url.to_string()),
        ("ALL_PROXY", url.to_string()),
        ("all_proxy", url.to_string()),
    ]
}

/// Schreibt den Body in Bloecken und zaehlt die erfolgreich gesendeten Bytes
/// (bricht ab, sobald der Client nicht mehr liest).
fn respond_streaming(
    stream: &mut TcpStream,
    content_type: &str,
    total: usize,
    chunk: usize,
    sent: &AtomicUsize,
) {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(header.as_bytes()).is_err() {
        return;
    }
    let block = vec![b'a'; chunk];
    let mut written = 0usize;
    while written < total {
        let length = chunk.min(total - written);
        if stream.write_all(&block[..length]).is_err() {
            break;
        }
        written += length;
        sent.store(written, Ordering::SeqCst);
    }
    let _ = stream.flush();
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
fn s5_nat64_addresses_are_handled() {
    for value in [
        // Well-known-Praefix mit eingebettetem Loopback
        "64:ff9b::7f00:1",
        // Local-Use-Praefix: komplett gesperrt (IPv4 steckt in Bits 48-63/72-87)
        "64:ff9b:1::7f00:1",
        "64:ff9b:1:7f00:0:1:808:808",
        "64:ff9b:1::808:808",
    ] {
        assert!(
            is_blocked_ip(value.parse().unwrap()),
            "{value} muss gesperrt sein"
        );
    }
    for value in [
        // eingebettetes 8.8.8.8 bleibt erlaubt
        "64:ff9b::808:808",
        // kein NAT64-Praefix -> nichts eingebettet
        "64:ff9b:0:1::7f00:1",
    ] {
        assert!(
            !is_blocked_ip(value.parse().unwrap()),
            "{value} muss erlaubt sein"
        );
    }
    assert_eq!(
        target("http://[64:ff9b::7f00:1]/"),
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
    let error = fetch_page_with(&server.url("/start"), allow_test_server(), FETCH_BUDGET)
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
        FETCH_BUDGET,
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
    for (content_type, body, expected) in [
        (
            "text/html; charset=utf-8",
            "<p>Hallo Welt</p>",
            "Hallo Welt",
        ),
        (
            "TEXT/HTML; charset=UTF-8",
            "<p>Hallo Welt</p>",
            "Hallo Welt",
        ),
        ("application/xhtml+xml", "<p>Hallo Welt</p>", "Hallo Welt"),
        // text/plain wird unveraendert uebernommen (kein Tag-Strippen).
        ("Text/Plain", "Hallo Welt", "Hallo Welt"),
    ] {
        let server = TestServer::start(move |stream| respond(stream, "200 OK", content_type, body));
        assert_eq!(
            fetch_page_with(&server.url("/"), allow_test_server(), FETCH_BUDGET)
                .await
                .unwrap(),
            expected,
            "Content-Type {content_type}"
        );
    }

    for content_type in ["application/json", "APPLICATION/JSON", "image/png"] {
        let server = TestServer::start(move |stream| respond(stream, "200 OK", content_type, "{}"));
        let error = fetch_page_with(&server.url("/"), allow_test_server(), FETCH_BUDGET)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            WebError::UnsupportedContentType(content_type.to_ascii_lowercase()),
            "Content-Type {content_type}"
        );
    }
}

#[tokio::test]
async fn s13_body_over_limit_aborts_the_download() {
    let big = "a".repeat(3 * 1024 * 1024);
    let server = TestServer::start(move |stream| respond(stream, "200 OK", "text/html", &big));
    let error = fetch_page_with(&server.url("/"), allow_test_server(), FETCH_BUDGET)
        .await
        .unwrap_err();

    assert_eq!(error, WebError::BodyTooLarge);
}

#[tokio::test]
async fn s13b_body_limit_aborts_while_the_server_still_writes() {
    const TOTAL: usize = 16 * 1024 * 1024;
    const CHUNK: usize = 64 * 1024;
    let sent = Arc::new(AtomicUsize::new(0));
    let server = TestServer::start({
        let sent = sent.clone();
        move |stream| respond_streaming(stream, "text/html", TOTAL, CHUNK, &sent)
    });

    let error = fetch_page_with(&server.url("/"), allow_test_server(), FETCH_BUDGET)
        .await
        .unwrap_err();
    let sent_bytes = sent.load(Ordering::SeqCst);

    assert_eq!(error, WebError::BodyTooLarge);
    assert!(
        sent_bytes < TOTAL,
        "der Client darf den Body nicht zu Ende lesen (gesendet: {sent_bytes} von {TOTAL})"
    );
}

#[tokio::test]
async fn s14_sixth_redirect_is_rejected() {
    let server = TestServer::start(|stream| respond_redirect(stream, "/loop"));
    let error = fetch_page_with(&server.url("/loop"), allow_test_server(), FETCH_BUDGET)
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

// ------------------------------------------------ K2/K3: Praefixgrenzen ----

#[test]
fn s16b_prefix_boundaries_and_embedded_forms() {
    let cases: [(&str, bool); 43] = [
        // IPv4-Praefixgrenzen: letzte erlaubte davor / erste gesperrte /
        // letzte gesperrte / erste erlaubte danach
        ("9.255.255.255", false),
        ("10.0.0.0", true),
        ("10.255.255.255", true),
        ("11.0.0.0", false),
        ("100.63.255.255", false),
        ("100.64.0.0", true),
        ("100.127.255.255", true),
        ("100.128.0.0", false),
        ("126.255.255.255", false),
        ("127.0.0.0", true),
        ("127.255.255.255", true),
        ("128.0.0.0", false),
        ("169.253.255.255", false),
        ("169.254.0.0", true),
        ("169.254.255.255", true),
        ("169.255.0.0", false),
        ("172.15.255.255", false),
        ("172.16.0.0", true),
        ("172.31.255.255", true),
        ("172.32.0.0", false),
        ("192.167.255.255", false),
        ("192.168.0.0", true),
        ("192.168.255.255", true),
        ("192.169.0.0", false),
        ("223.255.255.255", false),
        ("224.0.0.0", true),
        ("0.255.255.255", true),
        ("1.0.0.0", false),
        // IPv6-Grenzen
        ("fe7f:ffff::1", false),
        ("fe80::", true),
        ("febf:ffff::1", true),
        ("fec0::", true),
        ("feff:ffff::1", true),
        ("fe00::1", false),
        ("fbff::1", false),
        ("fc00::", true),
        ("fdff:ffff::1", true),
        ("ff00::", true),
        // eingebettete Formen (K2)
        ("::ffff:0:127.0.0.1", true),
        ("2001:0::80ff:fffe", true),
        ("2001:0::f7f7:f7f7", false),
        ("fe80:0:0:0:0:0:0:1", true),
        ("8.8.4.4", false),
    ];
    for (value, blocked) in cases {
        assert_eq!(
            is_blocked_ip(value.parse().unwrap()),
            blocked,
            "{value} (erwartet gesperrt: {blocked})"
        );
    }
}

// ------------------------------------------- K1/K4: Proxy-Umgebung, Suche ---

#[test]
fn s17_fetch_page_ignores_proxy_environment() {
    let proxy = TestServer::start(|_stream| {});
    run_ignored_child(
        "websearch::tests::s17b_inner_fetch_page_ignores_proxy",
        &proxy_env(&proxy.url("")),
    )
    .expect("Kindprozess muss genau einen bestandenen Test melden");
    assert_eq!(
        proxy.requests(),
        0,
        "der Proxy aus der Umgebung darf nicht kontaktiert werden"
    );
}

#[tokio::test]
#[ignore]
async fn s17b_inner_fetch_page_ignores_proxy() {
    // Laeuft nur im Kindprozess mit gesetzten Proxy-Variablen.
    let result = fetch_page("http://example.invalid./x").await;
    assert!(
        result.is_err(),
        "example.invalid darf nicht erreichbar sein: {result:?}"
    );
}

#[tokio::test]
async fn k4_search_redirect_is_reported() {
    let server =
        TestServer::start(|stream| respond_redirect(stream, "http://127.0.0.1:9/search?q=x"));
    let error = web_search(&server.url(""), "rust").await.unwrap_err();

    assert_eq!(
        error,
        WebError::SearchRedirect("http://127.0.0.1:9/search?q=x".to_string())
    );
    assert_eq!(
        error.to_string(),
        "SearXNG leitet weiter nach http://127.0.0.1:9/search?q=x – bitte die endgültige URL eintragen"
    );
    assert_eq!(server.requests(), 1, "kein zweiter Request");
}

#[test]
fn k4_local_search_ignores_proxy_environment() {
    let proxy = TestServer::start(|_stream| {});
    let searxng = TestServer::start(|stream| {
        respond(stream, "200 OK", "application/json", r#"{"results":[]}"#)
    });
    let mut env = proxy_env(&proxy.url(""));
    // Hostname statt IP-Literal: die Proxy-Regel darf nicht am Namen scheitern.
    env.push(("YTS_SEARXNG_URL", searxng.host_url("localhost", "")));
    run_ignored_child(
        "websearch::tests::k4b_inner_local_search_ignores_proxy",
        &env,
    )
    .expect("Kindprozess muss genau einen bestandenen Test melden");
    assert_eq!(
        proxy.requests(),
        0,
        "eine lokale Instanz darf nicht ueber den Proxy laufen"
    );
    assert_eq!(
        searxng.requests(),
        1,
        "die lokale Instanz muss direkt gefragt werden"
    );
}

#[tokio::test]
#[ignore]
async fn k4b_inner_local_search_ignores_proxy() {
    let url = std::env::var("YTS_SEARXNG_URL").expect("YTS_SEARXNG_URL fehlt");
    let results = web_search(&url, "rust").await.expect("lokale Suche");
    assert!(results.is_empty());
}

// ------------------------------------------------- K8/K9/K10: Abrufpfad -----

#[tokio::test]
async fn k8_slow_fetch_exceeds_the_total_budget() {
    let server = TestServer::start(|stream| {
        std::thread::sleep(Duration::from_millis(400));
        respond(stream, "200 OK", "text/html", "<p>spaet</p>");
    });
    let error = fetch_page_with(
        &server.url("/"),
        allow_test_server(),
        Duration::from_millis(60),
    )
    .await
    .unwrap_err();

    assert_eq!(error, WebError::Timeout);
}

#[tokio::test]
async fn k9_missing_content_type_is_reported() {
    let server = TestServer::start(|stream| respond_without_content_type(stream, "hallo"));
    let error = fetch_page_with(&server.url("/"), allow_test_server(), FETCH_BUDGET)
        .await
        .unwrap_err();

    assert_eq!(error, WebError::MissingContentType);
    assert_eq!(error.model_message(), "Fehler: Antwort ohne Content-Type");
}

#[tokio::test]
async fn k9_redirect_without_location_reports_the_status() {
    let server =
        TestServer::start(|stream| respond_with(stream, "302 Found", "text/plain", "", None));
    let error = fetch_page_with(&server.url("/"), allow_test_server(), FETCH_BUDGET)
        .await
        .unwrap_err();

    assert_eq!(error, WebError::HttpStatus(302));
}

#[tokio::test]
async fn k10_empty_extraction_is_an_error() {
    let server = TestServer::start(|stream| {
        respond(stream, "200 OK", "text/html", "<script>alert(1)</script>")
    });
    let error = fetch_page_with(&server.url("/"), allow_test_server(), FETCH_BUDGET)
        .await
        .unwrap_err();

    assert_eq!(error, WebError::EmptyText);
    assert_eq!(error.model_message(), "Fehler: kein Text extrahiert");
}

#[test]
fn q1_unterminated_raw_text_is_discarded() {
    // Unabgeschlossen: oeffnendes Tag ueberspringen und den folgenden Rohtext
    // bis zum naechsten '<' verwerfen (kein Quelltext im Modellkontext).
    assert_eq!(html_to_text("<script>alert(1)"), "");
    assert_eq!(html_to_text("<noscript>ohne js"), "");
    assert_eq!(html_to_text("<script>x<p>Text</p>"), "Text");
    // In HTML ist keines dieser Elemente selbstschliessend: `<style/>` oeffnet
    // einen Rohtext-Block, dessen Rest bis zum naechsten '<' verworfen wird.
    assert_eq!(html_to_text("<style/>rest"), "");
    assert_eq!(html_to_text("<style />rest"), "");
    assert_eq!(html_to_text("<style/>rest<p>Text</p>"), "Text");
    assert_eq!(html_to_text("<script/>alert(1)<p>Text</p>"), "Text");
    // Mit Abschluss-Tag wird der ganze Block verworfen.
    assert_eq!(html_to_text("<script>x</script>rest"), "rest");
    assert_eq!(
        html_to_text("<script>a</script>keep<script>b</script>"),
        "keep"
    );
}

#[test]
fn q1_quoted_angle_brackets_do_not_end_tags() {
    assert_eq!(html_to_text(r#"<a title="<script>">x</a>"#), "x");
    assert_eq!(html_to_text("<p title='a > b'>y</p>"), "y");
    assert_eq!(html_to_text(r#"<span data-x="1">z"#), "z");
    // Ohne schliessendes '>' bleibt der Rest Text.
    assert_eq!(html_to_text("<span>offen"), "offen");
}

#[test]
fn k10_bare_angle_brackets_stay_text() {
    assert_eq!(html_to_text("a < b und c > d"), "a < b und c > d");
    assert_eq!(html_to_text("1 <2 und 3> 4"), "1 <2 und 3> 4");
}

#[test]
fn k10_comments_are_removed_as_a_unit() {
    assert_eq!(html_to_text("<!-- a > b -->x"), "x");
    assert_eq!(html_to_text("vor<!-- a > b -->nach"), "vornach");
    assert_eq!(html_to_text("<!-- offen"), "");
}

#[test]
fn k10_block_elements_produce_line_breaks() {
    assert_eq!(html_to_text("<p>eins</p><p>zwei</p>"), "eins\n\nzwei");
    assert_eq!(html_to_text("a<br>b"), "a\nb");
    assert_eq!(html_to_text("<div>a</div><div>b</div>"), "a\n\nb");
    assert_eq!(html_to_text("<h2>T</h2><ul><li>x</li></ul>"), "T\n\nx");
    assert_eq!(
        html_to_text("<tr><td>a</td></tr><tr><td>b</td></tr>"),
        "a\n\nb"
    );
    // Anfangs- und End-Tag eines Blockelements erzeugen je einen Umbruch,
    // zwischen zwei Bloecken steht deshalb genau eine Leerzeile.
    assert_eq!(
        html_to_text("<section>a</section><article>b</article>"),
        "a\n\nb"
    );
    assert_eq!(
        html_to_text("<blockquote>a</blockquote><pre>b</pre>"),
        "a\n\nb"
    );
    assert_eq!(html_to_text("<p>a</p>\n\n\n<p>b</p>"), "a\n\nb");
    assert_eq!(html_to_text("  <p>  a  </p>  "), "a");
}

/// Fuehrt `html_to_text` in einem eigenen Thread aus und meldet, ob es innerhalb
/// der Schranke fertig wurde (der Thread laeuft bei einem Fehlschlag aus).
fn html_to_text_within(input: String, limit: Duration) -> bool {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = html_to_text(&inputs_guard(input));
        let _ = sender.send(());
    });
    receiver.recv_timeout(limit).is_ok()
}

fn inputs_guard(input: String) -> &'static str {
    Box::leak(input.into_boxed_str())
}

#[test]
fn q1_large_inputs_stay_linear() {
    let cases: [(&str, String); 7] = [
        ("1 MB nur '<'", "<".repeat(1_000_000)),
        ("500k '<a' ohne '>'", "<a".repeat(500_000)),
        (
            "200k '<script>x' ohne Abschluss",
            "<script>x".repeat(200_000),
        ),
        ("200k '<!--' ohne Abschluss", "<!--".repeat(200_000)),
        ("100k '<a title=\"'", "<a title=\"".repeat(100_000)),
        (
            "500k '<!--' mit Abschluss",
            format!("{}-->", "<!--".repeat(500_000)),
        ),
        (
            "200k '<script>x' mit Abschluss",
            format!("{}</script>", "<script>x".repeat(200_000)),
        ),
    ];
    let mut slow = Vec::new();
    for (label, input) in cases {
        if !html_to_text_within(input, Duration::from_secs(2)) {
            slow.push(label);
        }
    }
    assert!(slow.is_empty(), "zu langsam (Schranke 2 s): {slow:?}");
}

// ------------------------------------------------- K2b: Testsuche/403 -------

#[tokio::test]
async fn web_search_test_counts_hits() {
    let server = TestServer::start(|stream| {
        respond(
            stream,
            "200 OK",
            "application/json",
            r#"{"results":[{"title":"A"},{"title":"B"}]}"#,
        )
    });
    assert_eq!(web_search_test(server.url("")).await.unwrap(), 2);
}

#[tokio::test]
async fn web_search_test_explains_a_403() {
    let server = TestServer::start(|stream| respond(stream, "403 Forbidden", "text/html", "nope"));
    let error = web_search_test(server.url("")).await.unwrap_err();
    assert_eq!(
        error,
        "SearXNG lehnt JSON ab – in settings.yml unter search.formats „json“ erlauben"
    );
}

#[tokio::test]
async fn web_search_test_rejects_an_empty_url() {
    assert_eq!(
        web_search_test("   ".to_string()).await.unwrap_err(),
        "Ungültige SearXNG-URL"
    );
}
