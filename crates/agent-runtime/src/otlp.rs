//! The local OTLP receiver (FR-801, NFR-4): OTLP/HTTP with JSON bodies on
//! 127.0.0.1 only (records carry the user's email and account ids). Claude
//! Code's `claude_code.api_request` log events become per-request usage
//! samples (tokens, cost, model) attributed through the resource attribute
//! `orch.agent_id`. Metrics and traces are accepted and dropped: the events
//! carry the same numbers per request, without temporality questions.
//!
//! A minimal HTTP/1.1 server on std (no extra dependency): `POST /v1/logs`,
//! `/v1/metrics`, `/v1/traces`, `Content-Length` or chunked bodies up to
//! 8 MB, keep-alive. Protobuf and compressed bodies get 415 (agents are set to
//! `http/json` without compression).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_BODY: usize = 8 * 1024 * 1024;
const IDLE_KEEPALIVE: Duration = Duration::from_secs(10);

/// One `claude_code.api_request` event.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiRequest {
    pub ts_ms: u64,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_usd: f64,
    pub duration_ms: u64,
}

/// An OTLP `AnyValue` as JSON (strings, ints sent as strings, doubles, bools).
fn any_value(v: &Value) -> Option<Value> {
    for k in ["stringValue", "intValue", "doubleValue", "boolValue"] {
        if let Some(x) = v.get(k) {
            return Some(x.clone());
        }
    }
    None
}

fn attrs(list: &Value) -> BTreeMap<String, Value> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|kv| Some((kv.get("key")?.as_str()?.to_string(), any_value(kv.get("value")?)?)))
        .collect()
}

fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Extracts `api_request` events from an OTLP logs export.
pub fn parse_logs(body: &Value) -> Vec<ApiRequest> {
    let mut out = vec![];
    for rl in body.get("resourceLogs").and_then(Value::as_array).into_iter().flatten() {
        let res = attrs(rl.pointer("/resource/attributes").unwrap_or(&Value::Null));
        let agent = text(res.get("orch.agent_id"));
        for sl in rl.get("scopeLogs").and_then(Value::as_array).into_iter().flatten() {
            for rec in sl.get("logRecords").and_then(Value::as_array).into_iter().flatten() {
                let a = attrs(rec.get("attributes").unwrap_or(&Value::Null));
                let name = text(a.get("event.name")).or_else(|| text(rec.pointer("/body/stringValue"))).unwrap_or_default();
                if name != "api_request" && name != "claude_code.api_request" {
                    continue;
                }
                let ts_ns = num(rec.get("timeUnixNano")).or_else(|| num(rec.get("observedTimeUnixNano"))).unwrap_or(0.0);
                let ts_ms = if ts_ns > 0.0 { (ts_ns / 1e6) as u64 } else { orch_protocol::now_ms() };
                let u = |k: &str| num(a.get(k)).unwrap_or(0.0).max(0.0) as u64;
                out.push(ApiRequest {
                    ts_ms,
                    agent_id: agent.clone().or_else(|| text(a.get("orch.agent_id"))),
                    session_id: text(a.get("session.id")).or_else(|| text(res.get("session.id"))),
                    model: text(a.get("model")),
                    input_tokens: u("input_tokens"),
                    output_tokens: u("output_tokens"),
                    cache_read_tokens: u("cache_read_tokens"),
                    cache_creation_tokens: u("cache_creation_tokens"),
                    cost_usd: num(a.get("cost_usd")).unwrap_or(0.0).max(0.0),
                    duration_ms: u("duration_ms"),
                });
            }
        }
    }
    out
}

pub type Sink = Arc<dyn Fn(Vec<ApiRequest>) + Send + Sync>;

/// A running receiver; stops when dropped.
pub struct OtlpReceiver {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    pub requests: Arc<AtomicU64>,
}

impl OtlpReceiver {
    /// Binds 127.0.0.1 on an ephemeral port (`port` 0) or the given one.
    pub fn start(port: u16, sink: Sink) -> Result<OtlpReceiver> {
        let l = TcpListener::bind(("127.0.0.1", port))?;
        l.set_nonblocking(true)?;
        let addr = l.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicU64::new(0));
        let (st, rq) = (stop.clone(), requests.clone());
        let handle = std::thread::Builder::new().name("orch-otlp".into()).spawn(move || {
            while !st.load(Ordering::SeqCst) {
                match l.accept() {
                    Ok((s, peer)) => {
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let (sink, rq, st) = (sink.clone(), rq.clone(), st.clone());
                        std::thread::spawn(move || serve_conn(s, &sink, &rq, &st));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        })?;
        Ok(OtlpReceiver { addr, stop, handle: Some(handle), requests })
    }

    /// `OTEL_EXPORTER_OTLP_ENDPOINT` for agents.
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }
}

impl Drop for OtlpReceiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

struct Request {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn read_request(r: &mut BufReader<TcpStream>) -> Result<Option<Request>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let mut headers = BTreeMap::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 {
            bail!("eof in headers");
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
        if headers.len() > 100 {
            bail!("too many headers");
        }
    }
    let mut body = vec![];
    if headers.get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut sz = String::new();
            r.read_line(&mut sz)?;
            let n = usize::from_str_radix(sz.trim().split(';').next().unwrap_or("0"), 16)?;
            if n == 0 {
                let mut end = String::new();
                r.read_line(&mut end)?;
                break;
            }
            if body.len() + n > MAX_BODY {
                bail!("body too large");
            }
            let start = body.len();
            body.resize(start + n, 0);
            r.read_exact(&mut body[start..])?;
            let mut crlf = [0u8; 2];
            r.read_exact(&mut crlf)?;
        }
    } else {
        let n: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        if n > MAX_BODY {
            bail!("body too large");
        }
        body.resize(n, 0);
        r.read_exact(&mut body)?;
    }
    Ok(Some(Request { method, path, headers, body }))
}

fn respond(w: &mut TcpStream, code: u16, reason: &str, body: &str) -> std::io::Result<()> {
    write!(w, "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len())?;
    w.flush()
}

fn serve_conn(s: TcpStream, sink: &Sink, requests: &AtomicU64, stop: &AtomicBool) {
    let _ = s.set_read_timeout(Some(IDLE_KEEPALIVE));
    let Ok(mut w) = s.try_clone() else { return };
    let mut r = BufReader::new(s);
    while !stop.load(Ordering::SeqCst) {
        let req = match read_request(&mut r) {
            Ok(Some(q)) => q,
            Ok(None) => break,
            Err(_) => {
                let _ = respond(&mut w, 400, "Bad Request", "{}");
                break;
            }
        };
        requests.fetch_add(1, Ordering::Relaxed);
        let close = req.headers.get("connection").is_some_and(|v| v.eq_ignore_ascii_case("close"));
        let ct = req.headers.get("content-type").cloned().unwrap_or_default();
        let compressed = req.headers.get("content-encoding").is_some_and(|v| !v.eq_ignore_ascii_case("identity"));
        let res = if req.method != "POST" {
            respond(&mut w, 405, "Method Not Allowed", "{}")
        } else if compressed || !(ct.is_empty() || ct.starts_with("application/json")) {
            respond(&mut w, 415, "Unsupported Media Type", r#"{"error":"use http/json without compression"}"#)
        } else {
            match req.path.as_str() {
                "/v1/logs" => match serde_json::from_slice::<Value>(&req.body) {
                    Ok(v) => {
                        let reqs = parse_logs(&v);
                        if !reqs.is_empty() {
                            sink(reqs);
                        }
                        respond(&mut w, 200, "OK", "{}")
                    }
                    Err(_) => respond(&mut w, 400, "Bad Request", "{}"),
                },
                "/v1/metrics" | "/v1/traces" => respond(&mut w, 200, "OK", "{}"),
                _ => respond(&mut w, 404, "Not Found", "{}"),
            }
        };
        if res.is_err() || close {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn logs_body() -> Value {
        json!({"resourceLogs": [{
            "resource": {"attributes": [
                {"key": "service.name", "value": {"stringValue": "claude-code"}},
                {"key": "orch.agent_id", "value": {"stringValue": "a1"}}]},
            "scopeLogs": [{"logRecords": [
                {"timeUnixNano": "1793000000000000000", "body": {"stringValue": "claude_code.api_request"},
                 "attributes": [
                    {"key": "event.name", "value": {"stringValue": "api_request"}},
                    {"key": "model", "value": {"stringValue": "claude-haiku-5-5"}},
                    {"key": "input_tokens", "value": {"stringValue": "1200"}},
                    {"key": "output_tokens", "value": {"intValue": "80"}},
                    {"key": "cache_read_tokens", "value": {"stringValue": "9000"}},
                    {"key": "cache_creation_tokens", "value": {"stringValue": "300"}},
                    {"key": "cost_usd", "value": {"doubleValue": 0.0042}},
                    {"key": "duration_ms", "value": {"stringValue": "1500"}},
                    {"key": "session.id", "value": {"stringValue": "sid"}}]},
                {"timeUnixNano": "1793000000000000001", "attributes": [
                    {"key": "event.name", "value": {"stringValue": "tool_result"}}]}
            ]}]
        }]})
    }

    #[test]
    fn parses_api_requests_only() {
        let r = parse_logs(&logs_body());
        assert_eq!(r.len(), 1);
        let a = &r[0];
        assert_eq!(a.agent_id.as_deref(), Some("a1"));
        assert_eq!((a.input_tokens, a.output_tokens, a.cache_read_tokens, a.cache_creation_tokens), (1200, 80, 9000, 300));
        assert_eq!(a.cost_usd, 0.0042);
        assert_eq!(a.ts_ms, 1_793_000_000_000);
        assert_eq!(a.session_id.as_deref(), Some("sid"));
        assert!(parse_logs(&json!({})).is_empty());
    }

    fn post(addr: SocketAddr, path: &str, body: &str, extra: &str) -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        write!(s, "POST {path} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn http_server_keepalive_chunked_and_rejections() {
        let got: Arc<Mutex<Vec<ApiRequest>>> = Arc::default();
        let g2 = got.clone();
        let rx = OtlpReceiver::start(0, Arc::new(move |v| g2.lock().unwrap().extend(v))).unwrap();
        assert!(rx.endpoint().starts_with("http://127.0.0.1:"));
        let body = logs_body().to_string();
        assert!(post(rx.addr, "/v1/logs", &body, "").starts_with("HTTP/1.1 200"));
        assert!(post(rx.addr, "/v1/metrics", "{}", "").starts_with("HTTP/1.1 200"));
        assert!(post(rx.addr, "/v1/logs", "{}", "Content-Encoding: gzip\r\n").starts_with("HTTP/1.1 415"));
        assert!(post(rx.addr, "/nope", "{}", "").starts_with("HTTP/1.1 404"));
        // Keep-alive with a chunked body, then a second request on the same connection.
        let mut s = TcpStream::connect(rx.addr).unwrap();
        let (a, b) = body.split_at(body.len() / 2);
        write!(s, "POST /v1/logs HTTP/1.1\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{a}\r\n{:x}\r\n{b}\r\n0\r\n\r\n", a.len(), b.len()).unwrap();
        write!(s, "POST /v1/traces HTTP/1.1\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert_eq!(out.matches("HTTP/1.1 200").count(), 2, "{out}");
        assert_eq!(got.lock().unwrap().len(), 2);
        assert!(rx.requests.load(Ordering::Relaxed) >= 6);
        drop(rx);
    }
}
