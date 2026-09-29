//! System One client (TypeSafe Jev, or Laya's `laya-serve`, which speaks the same
//! API): request/response types, the retry policy and the HTTP
//! transport (spec 5.1, 5.7). Everything is tested offline; the transport tests
//! talk to a scripted HTTP server on 127.0.0.1. For debug mode the client can also
//! record each request and its responses as a [`JevExchange`], with the API key
//! redacted (spec 9.5).
//!
//! Never enable TRACE-level logging for `ureq` or `ureq_proto`: it prints request
//! headers, including the `Authorization` bearer key.

use std::collections::HashMap;
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

use super::annotate::Bucket;
use super::config::EngineConfig;

/// The TypeSafe System One endpoint Jev requests go to (the URL of the TypeSafe
/// quickstart). It is fixed; Laya's endpoint comes from `LAYA_URL`.
pub const JEV_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

/// Maximum attempts for one request, counting the first.
const MAX_ATTEMPTS: u32 = 3;
const BASE_BACKOFF: Duration = Duration::from_millis(250);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(2);
/// Longest error-body excerpt kept in a `JevError`.
const ERROR_SNIPPET_CHARS: usize = 200;
/// Largest response body read, in bytes; a real answer is a few kilobytes.
const MAX_BODY_BYTES: u64 = 1 << 20;
/// The header that carries the API key, as `Bearer <key>` ([`bearer`]).
const AUTHORIZATION: &str = "Authorization";
/// The header naming the body's type.
const CONTENT_TYPE_HEADER: &str = "Content-Type";
/// The request's `Content-Type`, as the TypeSafe quickstart sends it.
const CONTENT_TYPE: &str = "application/json";
/// What a recorded exchange shows in place of the API key.
const REDACTED: &str = "<redacted>";
/// What a recorded exchange shows in place of a response body that is not JSON
/// and still names the API key once its JSON escapes are decoded.
const WITHHELD_BODY: &str = "<redacted: the body contained the API key>";

/// One candidate move offered to Jev.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChoiceOption {
    /// SAN, used as the option key.
    pub key: String,
    /// Plain-language facts about the move.
    pub effect: String,
    /// The engine's bucket for the move.
    pub assessment: Bucket,
}

/// One `choice` question about one position.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceRequest {
    /// The position description (a serialized `JevState`).
    pub state: Value,
    /// The question Jev answers.
    pub question: String,
    /// How Jev should weigh the options.
    pub guidance: String,
    /// The shortlisted moves, best first.
    pub options: Vec<ChoiceOption>,
}

impl ChoiceRequest {
    /// JSON body for `POST /v1/systemone`; the question id is `move`. serde_json sorts
    /// object keys, so the state fields and the options (criteria) reach Jev in
    /// alphabetical order, not in shortlist order.
    pub fn to_body(&self, model: &str) -> Value {
        let criteria: Map<String, Value> = self
            .options
            .iter()
            .map(|o| {
                (
                    o.key.clone(),
                    json!({ "effect": o.effect, "assessment": o.assessment }),
                )
            })
            .collect();
        json!({
            "model": model,
            "state": self.state,
            "questions": {
                "move": {
                    "type": "choice",
                    "instructions": { "question": self.question, "guidance": self.guidance },
                    "criteria": criteria,
                }
            }
        })
    }
}

/// Jev's answer to the `move` question.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceAnswer {
    /// Key of the option Jev chose.
    pub choice: String,
    /// Every option with its probability, most likely first (ties by key).
    pub probabilities: Vec<(String, f32)>,
    /// Jev's confidence in its choice, from 0 to 1.
    pub confidence: f32,
    /// Versioned model ID that answered, e.g. `jev-1.13.0`; `laya-serve` may leave it out.
    pub model: Option<String>,
    /// Input tokens billed for the request, when the server reports usage.
    pub input_tokens: Option<u32>,
}

/// Why a Jev request failed. Messages never contain the API key: `JevClient`
/// replaces any occurrence of it with `<redacted>`.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum JevError {
    /// The API answered with a status other than 200.
    #[error("HTTP {status}: {message}")]
    Http {
        /// The HTTP status code.
        status: u16,
        /// Up to 200 printable characters of the response body.
        message: String,
        /// The `Retry-After` delay, when the server sent one in seconds.
        retry_after: Option<Duration>,
    },
    /// The request did not finish within the configured timeout.
    #[error("request timed out")]
    Timeout,
    /// The connection failed (host lookup, connect or I/O); worth retrying.
    #[error("network error: {0}")]
    Transport(String),
    /// A 200 response whose body is not a usable answer.
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    /// The request failed in a way retrying cannot fix: a malformed URL, a protocol
    /// error or a response body over 1 MiB.
    #[error("request failed: {0}")]
    Request(String),
}

/// One HTTP request to Jev and every attempt made for it, for debug mode (spec 9.5).
/// The API key never appears in it: the `Authorization` header reads
/// `Bearer <redacted>`, and any occurrence of the key in the body, a response or
/// an error is replaced by `<redacted>`.
#[derive(Clone, Debug, PartialEq)]
pub struct JevExchange {
    /// The HTTP method, `POST`.
    pub method: String,
    /// The endpoint the request went to.
    pub url: String,
    /// The headers the client sets, in the order it sets them.
    pub headers: Vec<(String, String)>,
    /// The JSON body sent with every attempt.
    pub body: Value,
    /// Every attempt in order, including the ones that were retried.
    pub attempts: Vec<JevAttempt>,
}

/// One attempt of a [`JevExchange`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JevAttempt {
    /// The HTTP status, or `None` when no response arrived.
    pub status: Option<u16>,
    /// The response body as text (read under the 1 MiB cap) with the key redacted,
    /// or `None` when no response arrived or its body could not be read. A JSON body
    /// in which the key had to be redacted after decoding is re-encoded, so it is not
    /// byte for byte what the server sent; a body that is not JSON and names the key
    /// only through JSON escapes is replaced by
    /// `<redacted: the body contained the API key>`.
    pub response: Option<String>,
    /// Why the attempt failed, as the `JevError` message; `None` for an answer.
    pub error: Option<String>,
    /// Time from sending the request to reading the whole response, without the
    /// backoff before the next attempt.
    pub elapsed: Duration,
}

/// Something that can answer a `choice` question: `JevClient`, or a mock in tests.
pub trait MoveChooser: Send + Sync {
    /// Answers one `choice` question. An error is final: retries happen inside.
    fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceAnswer, JevError>;

    /// Like [`choose`](MoveChooser::choose), and records the HTTP exchange in
    /// `trace`, whether it succeeded or not. The default calls `choose` and records
    /// nothing, leaving `trace` as it was (callers pass `None`).
    fn choose_traced(
        &self,
        request: &ChoiceRequest,
        _trace: &mut Option<JevExchange>,
    ) -> Result<ChoiceAnswer, JevError> {
        self.choose(request)
    }
}

#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    model: Option<String>,
    answers: HashMap<String, ApiAnswer>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
struct ApiAnswer {
    choice: String,
    probabilities: HashMap<String, f32>,
    confidence: f32,
}

#[derive(Deserialize)]
struct ApiUsage {
    input_tokens: u32,
}

/// Parses a successful `/v1/systemone` response body.
pub fn parse_answer(body: &str) -> Result<ChoiceAnswer, JevError> {
    let response: ApiResponse =
        serde_json::from_str(body).map_err(|e| JevError::InvalidResponse(e.to_string()))?;
    let answer = response
        .answers
        .get("move")
        .ok_or_else(|| JevError::InvalidResponse("no answer for question `move`".to_string()))?;
    let mut probabilities: Vec<(String, f32)> = answer
        .probabilities
        .iter()
        .map(|(k, p)| (k.clone(), *p))
        .collect();
    probabilities.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(ChoiceAnswer {
        choice: answer.choice.clone(),
        probabilities,
        confidence: answer.confidence,
        model: response.model,
        input_tokens: response.usage.map(|u| u.input_tokens),
    })
}

/// At most `max_chars` characters of `text`, with every control character (CR, LF,
/// ESC, ...) replaced by a space so server text cannot break a one-line display.
pub fn printable(text: &str, max_chars: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max_chars)
        .collect()
}

/// Error for a non-200 response, keeping at most 200 printable characters of the body.
pub fn http_error(status: u16, body: &str, retry_after: Option<Duration>) -> JevError {
    JevError::Http {
        status,
        message: printable(body.trim(), ERROR_SNIPPET_CHARS),
        retry_after,
    }
}

/// Delay before the next attempt after `error` on attempt number `attempt`
/// (1-based), or `None` when the failure is final.
pub fn retry_delay(error: &JevError, attempt: u32) -> Option<Duration> {
    if attempt >= MAX_ATTEMPTS {
        return None;
    }
    let backoff = BASE_BACKOFF * 2u32.pow(attempt - 1);
    match error {
        JevError::Http {
            status,
            retry_after,
            ..
        } if *status == 429 || *status >= 500 => {
            Some(retry_after.map_or(backoff, |d| d.min(MAX_RETRY_AFTER)))
        }
        JevError::Timeout | JevError::Transport(_) => Some(backoff),
        JevError::Http { .. } | JevError::InvalidResponse(_) | JevError::Request(_) => None,
    }
}

/// `text` with every occurrence of `secret` replaced by `<redacted>`. An empty
/// secret redacts nothing.
pub fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, REDACTED)
    }
}

/// A response body as it is recorded: the key redacted, both in the raw text and,
/// when the body is JSON, in the text a parser decodes from it: a server can echo the
/// key with `\u` escapes that the raw text does not match. When decoding shows the
/// key, the body is re-encoded from the redacted JSON, so no later parse can bring it
/// back. The raw text also loses the key with `/` written as `\/`, JSON's other way
/// to write it. A body that is not JSON as a whole but still names the key once its
/// JSON escapes are decoded (a reader could decode part of it) is withheld whole.
/// Only the record is redacted: the answer is parsed from the body as received.
fn redact_body(text: &str, secret: &str) -> String {
    let text = redact(text, secret);
    let text = if secret.contains('/') {
        redact(&text, &secret.replace('/', "\\/"))
    } else {
        text
    };
    let Ok(json) = serde_json::from_str::<Value>(&text) else {
        if !secret.is_empty() && decode_json_escapes(&text).contains(secret) {
            return WITHHELD_BODY.to_string();
        }
        return text;
    };
    let redacted = redact_value(&json, secret);
    if redacted == json {
        text
    } else {
        redacted.to_string()
    }
}

/// `text` with the escapes of a JSON string (`\uXXXX`, surrogate pairs included,
/// `\/`, `\"`, `\\`, `\b`, `\f`, `\n`, `\r`, `\t`) decoded, read left to right as
/// a JSON parser would. A backslash that starts no valid escape stays as it is.
fn decode_json_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match json_escape(rest) {
            Some((c, used)) => {
                out.push(c);
                rest = &rest[used..];
            }
            None => {
                out.push('\\');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The character the JSON escape at the start of `text` stands for and the bytes it
/// takes, or `None` when `text` does not start with a valid escape.
fn json_escape(text: &str) -> Option<(char, usize)> {
    let simple = match text.as_bytes().get(1)? {
        b'u' => {
            let code = hex4(&text[2..])?;
            if !(0xD800..=0xDBFF).contains(&code) {
                return char::from_u32(code).map(|c| (c, 6));
            }
            let low = text[6..].strip_prefix("\\u").and_then(hex4)?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return None;
            }
            let code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
            return char::from_u32(code).map(|c| (c, 12));
        }
        b'/' => '/',
        b'"' => '"',
        b'\\' => '\\',
        b'b' => '\u{8}',
        b'f' => '\u{c}',
        b'n' => '\n',
        b'r' => '\r',
        b't' => '\t',
        _ => return None,
    };
    Some((simple, 2))
}

/// The value of the four hex digits `text` starts with.
fn hex4(text: &str) -> Option<u32> {
    let digits = text.get(..4)?;
    if digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        u32::from_str_radix(digits, 16).ok()
    } else {
        None
    }
}

/// `value` with [`redact`] applied to every string and object key in it.
fn redact_value(value: &Value, secret: &str) -> Value {
    match value {
        Value::String(text) => Value::String(redact(text, secret)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| redact_value(v, secret)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (redact(k, secret), redact_value(v, secret)))
                .collect(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
    }
}

/// The `Authorization` value for `key`. [`JevClient`] sends it with the real key and
/// records it with `<redacted>`, so the record shows what was sent.
fn bearer(key: &str) -> String {
    format!("Bearer {key}")
}

/// `error` with [`redact`] applied to its text. Transport and request errors carry
/// text from the HTTP library, which can quote the URL or server bytes: after the
/// key is redacted it goes through [`printable`] like the HTTP error snippet.
fn redact_error(error: JevError, secret: &str) -> JevError {
    match error {
        JevError::Http {
            status,
            message,
            retry_after,
        } => JevError::Http {
            status,
            message: redact(&message, secret),
            retry_after,
        },
        JevError::Transport(text) => {
            JevError::Transport(printable(&redact(&text, secret), ERROR_SNIPPET_CHARS))
        }
        JevError::InvalidResponse(text) => JevError::InvalidResponse(redact(&text, secret)),
        JevError::Request(text) => {
            JevError::Request(printable(&redact(&text, secret), ERROR_SNIPPET_CHARS))
        }
        JevError::Timeout => JevError::Timeout,
    }
}

/// What one attempt produced: the answer or error, plus the status and the
/// redacted response body when they arrived. [`JevClient::post`] turns it into the
/// public [`JevAttempt`] record.
struct RawAttempt {
    result: Result<ChoiceAnswer, JevError>,
    status: Option<u16>,
    response: Option<String>,
}

/// Classifies a `ureq` failure: timeouts and connection problems are worth
/// retrying; anything else (a bad URL, a protocol error, an oversized body) is final.
fn request_error(error: ureq::Error) -> JevError {
    match error {
        ureq::Error::Timeout(_) => JevError::Timeout,
        e @ (ureq::Error::ConnectionFailed | ureq::Error::HostNotFound | ureq::Error::Io(_)) => {
            JevError::Transport(e.to_string())
        }
        other => JevError::Request(other.to_string()),
    }
}

/// HTTP client for a System One endpoint (`config.endpoint`: [`JEV_ENDPOINT`] or
/// `laya-serve`), sending `Authorization: Bearer <key>` when the config has a key and
/// a JSON body as in the TypeSafe quickstart.
pub struct JevClient {
    agent: ureq::Agent,
    url: String,
    model: String,
    api_key: Option<String>,
}

impl fmt::Debug for JevClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JevClient")
            .field("url", &self.url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl JevClient {
    /// A client for `config.endpoint`; `None` when the config is not
    /// [`enabled`](EngineConfig::enabled).
    pub fn new(config: &EngineConfig) -> Option<JevClient> {
        if !config.enabled() {
            return None;
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(config.timeout))
            .http_status_as_error(false)
            .build()
            .into();
        Some(JevClient {
            agent,
            url: config.endpoint.clone(),
            model: config.model.clone(),
            api_key: config.api_key.clone(),
        })
    }

    /// The key to redact from everything recorded; empty (redacting nothing) without one.
    fn secret(&self) -> &str {
        self.api_key.as_deref().unwrap_or("")
    }

    /// The exchange record for `body` before any attempt: the redacted headers the
    /// client sends and the body with the key redacted.
    fn exchange(&self, body: &Value) -> JevExchange {
        let mut headers = Vec::new();
        if self.api_key.is_some() {
            headers.push((AUTHORIZATION.to_string(), bearer(REDACTED)));
        }
        headers.push((CONTENT_TYPE_HEADER.to_string(), CONTENT_TYPE.to_string()));
        JevExchange {
            method: "POST".to_string(),
            url: self.url.clone(),
            headers,
            body: redact_value(body, self.secret()),
            attempts: Vec::new(),
        }
    }

    /// Posts `body` up to [`MAX_ATTEMPTS`] times, retrying as [`retry_delay`] says,
    /// and pushes each attempt to `attempts` when it is given.
    fn post(
        &self,
        body: &Value,
        mut attempts: Option<&mut Vec<JevAttempt>>,
    ) -> Result<ChoiceAnswer, JevError> {
        let mut attempt = 1;
        loop {
            let started = Instant::now();
            let RawAttempt {
                result,
                status,
                response,
            } = self.post_once(body);
            if let Some(attempts) = attempts.as_deref_mut() {
                attempts.push(JevAttempt {
                    status,
                    response,
                    error: result.as_ref().err().map(JevError::to_string),
                    elapsed: started.elapsed(),
                });
            }
            match result {
                Ok(answer) => return Ok(answer),
                Err(error) => match retry_delay(&error, attempt) {
                    Some(delay) => {
                        thread::sleep(delay);
                        attempt += 1;
                    }
                    None => return Err(error),
                },
            }
        }
    }

    /// One HTTP attempt. An answer is parsed from the body as received; the recorded
    /// body and an error snippet come from the redacted body ([`redact_body`]), so no
    /// error can carry part of the key, and every error is redacted too.
    fn post_once(&self, body: &Value) -> RawAttempt {
        let mut request = self.agent.post(&self.url);
        if let Some(key) = &self.api_key {
            request = request.header(AUTHORIZATION, &bearer(key));
        }
        // `send_json` alone would send `application/json; charset=utf-8`; the
        // TypeSafe quickstart sends plain `application/json`.
        let sent = request.content_type(CONTENT_TYPE).send_json(body);
        let mut response = match sent {
            Ok(response) => response,
            Err(error) => {
                return RawAttempt {
                    result: Err(redact_error(request_error(error), self.secret())),
                    status: None,
                    response: None,
                };
            }
        };
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_string();
        let (result, response) = match text {
            Ok(text) => {
                let recorded = redact_body(&text, self.secret());
                let result = if status == 200 {
                    parse_answer(&text)
                } else {
                    Err(http_error(status, &recorded, retry_after))
                };
                (result, Some(recorded))
            }
            // Keep the status and Retry-After even when the body cannot be read.
            Err(_) if status != 200 => (
                Err(JevError::Http {
                    status,
                    message: "<unreadable body>".into(),
                    retry_after,
                }),
                None,
            ),
            Err(error) => (Err(request_error(error)), None),
        };
        RawAttempt {
            result: result.map_err(|error| redact_error(error, self.secret())),
            status: Some(status),
            response,
        }
    }
}

impl MoveChooser for JevClient {
    fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceAnswer, JevError> {
        self.post(&request.to_body(&self.model), None)
    }

    /// Records the method, URL, redacted headers, the body and every attempt,
    /// including retried ones; `trace` is `Some` afterwards, answer or error.
    fn choose_traced(
        &self,
        request: &ChoiceRequest,
        trace: &mut Option<JevExchange>,
    ) -> Result<ChoiceAnswer, JevError> {
        let body = request.to_body(&self.model);
        let exchange = trace.insert(self.exchange(&body));
        self.post(&body, Some(&mut exchange.attempts))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    use crate::engine::Provider;

    fn request() -> ChoiceRequest {
        ChoiceRequest {
            state: json!({ "side_to_move": "White" }),
            question: "Which move?".to_string(),
            guidance: "Prefer good moves.".to_string(),
            options: vec![
                ChoiceOption {
                    key: "Nxe5".to_string(),
                    effect: "captures the knight on e5, undefended".to_string(),
                    assessment: Bucket::Good,
                },
                ChoiceOption {
                    key: "O-O".to_string(),
                    effect: "castles kingside".to_string(),
                    assessment: Bucket::Neutral,
                },
            ],
        }
    }

    #[test]
    fn request_body_matches_the_api_shape() {
        assert_eq!(
            request().to_body("jev-latest"),
            json!({
                "model": "jev-latest",
                "state": { "side_to_move": "White" },
                "questions": {
                    "move": {
                        "type": "choice",
                        "instructions": { "question": "Which move?", "guidance": "Prefer good moves." },
                        "criteria": {
                            "Nxe5": { "effect": "captures the knight on e5, undefended", "assessment": "good" },
                            "O-O": { "effect": "castles kingside", "assessment": "neutral" }
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn parses_a_choice_answer() {
        let body = r#"{
            "model": "jev-1.13.0",
            "answers": { "move": { "type": "choice", "choice": "Nxe5",
                "probabilities": { "O-O": 0.2, "Nxe5": 0.7, "d4": 0.1 }, "confidence": 0.55 } },
            "usage": { "input_tokens": 812, "output_tokens": 20 }
        }"#;
        let answer = parse_answer(body).unwrap();
        assert_eq!(answer.choice, "Nxe5");
        assert_eq!(
            answer.probabilities,
            vec![
                ("Nxe5".to_string(), 0.7),
                ("O-O".to_string(), 0.2),
                ("d4".to_string(), 0.1)
            ]
        );
        assert_eq!(answer.confidence, 0.55);
        assert_eq!(answer.model.as_deref(), Some("jev-1.13.0"));
        assert_eq!(answer.input_tokens, Some(812));
    }

    #[test]
    fn rejects_malformed_answers() {
        assert!(matches!(
            parse_answer("<html>502 Bad Gateway</html>"),
            Err(JevError::InvalidResponse(_))
        ));
        let no_move = r#"{"model":"m","answers":{},"usage":{"input_tokens":1}}"#;
        assert!(matches!(
            parse_answer(no_move),
            Err(JevError::InvalidResponse(_))
        ));
    }

    #[test]
    fn http_errors_keep_a_short_snippet() {
        let long_body = "x".repeat(1000);
        let JevError::Http {
            status,
            message,
            retry_after,
        } = http_error(502, &long_body, None)
        else {
            panic!("expected an HTTP error");
        };
        assert_eq!(status, 502);
        assert_eq!(message.len(), 200);
        assert_eq!(retry_after, None);
        assert_eq!(
            http_error(401, "  bad key\n", None).to_string(),
            "HTTP 401: bad key"
        );
    }

    #[test]
    fn http_errors_replace_control_characters() {
        let body = "<html>\r\nBad Gateway\x1b[31m red\x07</html>";
        let JevError::Http { message, .. } = http_error(502, body, None) else {
            panic!("expected an HTTP error");
        };
        assert!(!message.chars().any(char::is_control), "{message:?}");
        assert_eq!(message, "<html>  Bad Gateway [31m red </html>");
        // Replacement happens before the 200-character cut.
        let long = "\r\n".repeat(150) + "tail";
        let JevError::Http { message, .. } = http_error(502, &format!("x{long}"), None) else {
            panic!("expected an HTTP error");
        };
        assert_eq!(message.chars().count(), 200);
        assert!(!message.chars().any(char::is_control));
    }

    #[test]
    fn retry_decisions() {
        let http = |status| http_error(status, "", None);
        let ms = Duration::from_millis;
        assert_eq!(retry_delay(&http(429), 1), Some(ms(250)));
        assert_eq!(retry_delay(&http(529), 2), Some(ms(500)));
        assert_eq!(retry_delay(&http(503), 1), Some(ms(250)));
        assert_eq!(retry_delay(&http(429), 3), None, "three attempts at most");
        assert_eq!(retry_delay(&http(401), 1), None);
        assert_eq!(retry_delay(&http(422), 1), None);
        assert_eq!(retry_delay(&JevError::Timeout, 1), Some(ms(250)));
        assert_eq!(
            retry_delay(&JevError::Transport("reset".into()), 2),
            Some(ms(500))
        );
        assert_eq!(
            retry_delay(&JevError::InvalidResponse("bad".into()), 1),
            None
        );
        let slow = http_error(429, "", Some(Duration::from_secs(1)));
        assert_eq!(retry_delay(&slow, 1), Some(Duration::from_secs(1)));
        let very_slow = http_error(429, "", Some(Duration::from_secs(30)));
        assert_eq!(
            retry_delay(&very_slow, 1),
            Some(Duration::from_secs(2)),
            "retry-after is capped"
        );
        assert_eq!(
            retry_delay(&JevError::Request("bad uri".into()), 1),
            None,
            "a malformed request fails the same way every time"
        );
    }

    #[test]
    fn classifies_transport_errors() {
        assert_eq!(
            request_error(ureq::Error::Timeout(ureq::Timeout::Global)),
            JevError::Timeout
        );
        for retryable in [
            ureq::Error::ConnectionFailed,
            ureq::Error::HostNotFound,
            ureq::Error::Io(std::io::Error::other("reset")),
        ] {
            let error = request_error(retryable);
            assert!(matches!(error, JevError::Transport(_)), "{error:?}");
            assert!(retry_delay(&error, 1).is_some());
        }
        for final_error in [
            ureq::Error::BadUri("not a uri".into()),
            ureq::Error::BodyExceedsLimit(1 << 20),
            ureq::Error::RequireHttpsOnly("http://x".into()),
        ] {
            let error = request_error(final_error);
            assert!(matches!(error, JevError::Request(_)), "{error:?}");
            assert_eq!(retry_delay(&error, 1), None);
        }
    }

    /// Every raw request (head and body) the scripted server has read, in order.
    type Requests = Arc<Mutex<Vec<String>>>;

    /// Serves `responses` in order, one per connection, on 127.0.0.1, then stops
    /// listening. Returns a client pointed at its `/v1/systemone` and the requests read.
    fn serve(responses: Vec<String>) -> (JevClient, Requests) {
        serve_with_key(responses, "test-key")
    }

    /// [`serve`] with a client that sends `api_key`.
    fn serve_with_key(responses: Vec<String>, api_key: &str) -> (JevClient, Requests) {
        serve_config(responses, keyed_config(api_key))
    }

    /// [`serve`] with a client built from `config`, its endpoint pointed at the server.
    fn serve_config(responses: Vec<String>, config: EngineConfig) -> (JevClient, Requests) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Requests::default();
        let received = Arc::clone(&requests);
        thread::spawn(move || {
            for response in responses {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream);
                let raw = read_request(&mut reader);
                received.lock().unwrap().push(raw);
                let mut stream = reader.into_inner();
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        let config = EngineConfig {
            endpoint: format!("http://127.0.0.1:{port}/v1/systemone"),
            ..config
        };
        (
            JevClient::new(&config).expect("an enabled config"),
            requests,
        )
    }

    /// A Jev-shaped test config sending `api_key` with a 2 s timeout.
    fn keyed_config(api_key: &str) -> EngineConfig {
        EngineConfig {
            api_key: Some(api_key.to_string()),
            timeout: Duration::from_secs(2),
            ..EngineConfig::default()
        }
    }

    /// A Laya test config without a key (the endpoint is set by [`serve_config`]).
    fn laya_config() -> EngineConfig {
        EngineConfig {
            provider: Provider::Laya,
            endpoint: "http://127.0.0.1:9/v1/systemone".to_string(),
            api_key: None,
            timeout: Duration::from_secs(2),
            ..EngineConfig::default()
        }
    }

    /// The header values named `name` (lower case) in a raw request.
    fn header_values(raw: &str, name: &str) -> Vec<String> {
        let head = raw.split_once("\r\n\r\n").map_or(raw, |(head, _)| head);
        head.split("\r\n")
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_string())
            .collect()
    }

    fn count(requests: &Requests) -> usize {
        requests.lock().unwrap().len()
    }

    /// Reads one request: the head, then a body of `Content-Length` bytes. Returns
    /// both as text.
    fn read_request(reader: &mut BufReader<TcpStream>) -> String {
        let mut raw = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            raw.push_str(&line);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0; length];
        let _ = reader.read_exact(&mut body);
        raw.push_str(&String::from_utf8_lossy(&body));
        raw
    }

    fn response(status: &str, content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn transport_parses_a_successful_answer() {
        let body = r#"{"model":"jev-1.13.0","answers":{"move":{"type":"choice","choice":"O-O",
            "probabilities":{"Nxe5":0.25,"O-O":0.75},"confidence":0.6}},
            "usage":{"input_tokens":321,"output_tokens":9}}"#;
        let (client, requests) = serve(vec![response("200 OK", "application/json", body)]);
        let answer = client.choose(&request()).unwrap();
        assert_eq!(answer.choice, "O-O");
        assert_eq!(
            answer.probabilities,
            vec![("O-O".to_string(), 0.75), ("Nxe5".to_string(), 0.25)]
        );
        assert_eq!(answer.model.as_deref(), Some("jev-1.13.0"));
        assert_eq!(answer.input_tokens, Some(321));
        assert_eq!(count(&requests), 1);

        // The wire format of the TypeSafe quickstart: `curl -X POST
        // https://api.typesafe.ai/v1/systemone -H "Authorization: Bearer $KEY"
        // -H "Content-Type: application/json"` with a JSON body.
        let raw = requests.lock().unwrap()[0].clone();
        let (head, body) = raw.split_once("\r\n\r\n").expect("a head and a body");
        let mut lines = head.split("\r\n");
        assert_eq!(lines.next(), Some("POST /v1/systemone HTTP/1.1"));
        let headers: Vec<(String, String)> = lines
            .map(|line| {
                let (name, value) = line.split_once(':').expect("a header line");
                (name.to_ascii_lowercase(), value.trim().to_string())
            })
            .collect();
        let header = |name: &str| {
            headers
                .iter()
                .filter(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(header("authorization"), vec!["Bearer test-key"], "{head}");
        assert_eq!(header("content-type"), vec!["application/json"], "{head}");
        let body: Value = serde_json::from_str(body).expect("a JSON body");
        let keys: BTreeSet<&str> = body
            .as_object()
            .expect("a JSON object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, BTreeSet::from(["model", "questions", "state"]));
        assert_eq!(body, request().to_body("jev-latest"));
    }

    #[test]
    fn transport_retries_a_bad_gateway_three_times() {
        let page = format!(
            "<html>\r\n<head><title>502 Bad Gateway</title></head>\r\n<body>\x1b[31m\r\n{}\r\n</body></html>",
            "<p>upstream unavailable</p>".repeat(20)
        );
        let bad_gateway = response("502 Bad Gateway", "text/html", &page);
        let (client, requests) = serve(vec![bad_gateway; 3]);
        let error = client.choose(&request()).unwrap_err();
        let JevError::Http {
            status, message, ..
        } = &error
        else {
            panic!("expected an HTTP error, got {error:?}");
        };
        assert_eq!(*status, 502);
        assert!(message.chars().count() <= 200, "{message}");
        assert!(!message.chars().any(char::is_control), "{message:?}");
        assert_eq!(count(&requests), 3);
    }

    #[test]
    fn transport_does_not_retry_a_rejected_key() {
        let rejected = response(
            "401 Unauthorized",
            "application/json",
            r#"{"error":"invalid api key"}"#,
        );
        let (client, requests) = serve(vec![rejected]);
        let error = client.choose(&request()).unwrap_err();
        assert!(
            matches!(error, JevError::Http { status: 401, .. }),
            "{error:?}"
        );
        assert_eq!(count(&requests), 1);
    }

    #[test]
    fn transport_keeps_the_status_when_the_error_body_is_unreadable() {
        // The body is cut short: 5 bytes arrive of the 100 announced.
        let truncated =
            "HTTP/1.1 404 Not Found\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort"
                .to_string();
        let (client, requests) = serve(vec![truncated]);
        let error = client.choose(&request()).unwrap_err();
        assert_eq!(
            error,
            JevError::Http {
                status: 404,
                message: "<unreadable body>".to_string(),
                retry_after: None,
            }
        );
        assert_eq!(count(&requests), 1);
    }

    #[test]
    fn transport_rejects_a_body_over_one_mebibyte() {
        let huge = response("200 OK", "application/json", &" ".repeat((1 << 20) + 1));
        let (client, requests) = serve(vec![huge]);
        let error = client.choose(&request()).unwrap_err();
        assert!(matches!(error, JevError::Request(_)), "{error:?}");
        assert_eq!(count(&requests), 1);
    }

    /// A successful answer for [`request`]: Jev picks O-O.
    const ANSWER: &str = r#"{"model":"jev-1.13.0","answers":{"move":{"type":"choice","choice":"O-O","probabilities":{"Nxe5":0.25,"O-O":0.75},"confidence":0.6}},"usage":{"input_tokens":321,"output_tokens":9}}"#;

    /// An API key the redaction tests look for in everything the client records.
    const SENTINEL_KEY: &str = "sentinel-key-7Qx9";

    #[test]
    fn traced_exchange_records_the_request_and_every_attempt() {
        let busy = response("503 Service Unavailable", "text/plain", "busy, try again");
        let ok = response("200 OK", "application/json", ANSWER);
        let (client, requests) = serve(vec![busy, ok]);
        let mut trace = None;
        let started = std::time::Instant::now();
        let answer = client.choose_traced(&request(), &mut trace).unwrap();
        let total = started.elapsed();
        assert_eq!(answer.choice, "O-O");
        assert_eq!(count(&requests), 2);

        let exchange = trace.expect("the client records the exchange");
        assert_eq!(exchange.method, "POST");
        assert!(
            exchange.url.starts_with("http://127.0.0.1:"),
            "{}",
            exchange.url
        );
        assert!(exchange.url.ends_with("/v1/systemone"), "{}", exchange.url);
        assert_eq!(
            exchange.headers,
            vec![
                ("Authorization".to_string(), "Bearer <redacted>".to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
            ]
        );
        assert_eq!(exchange.body, request().to_body("jev-latest"));
        let [busy, ok] = exchange.attempts.as_slice() else {
            panic!("expected two attempts, got {:?}", exchange.attempts);
        };
        assert_eq!(busy.status, Some(503));
        assert_eq!(busy.response.as_deref(), Some("busy, try again"));
        assert_eq!(busy.error.as_deref(), Some("HTTP 503: busy, try again"));
        assert_eq!(ok.status, Some(200));
        assert_eq!(ok.response.as_deref(), Some(ANSWER));
        assert_eq!(ok.error, None);
        // Each attempt times its own request, not the 250 ms backoff between them.
        assert!(
            busy.elapsed + ok.elapsed + BASE_BACKOFF <= total,
            "{:?} + {:?} + backoff > {total:?}",
            busy.elapsed,
            ok.elapsed
        );

        // Only the record is redacted: the wire carries the real key. Every recorded
        // header is one the server read, with the same value once the key is put back.
        for raw in requests.lock().unwrap().iter() {
            assert!(raw.contains("Bearer test-key"), "{raw}");
            let head: Vec<(&str, &str)> = raw
                .lines()
                .take_while(|line| !line.is_empty())
                .filter_map(|line| line.split_once(':'))
                .map(|(name, value)| (name.trim(), value.trim()))
                .collect();
            for (name, value) in &exchange.headers {
                let sent = value.replace(REDACTED, "test-key");
                assert!(
                    head.iter()
                        .any(|(n, v)| n.eq_ignore_ascii_case(name) && *v == sent),
                    "{name}: {sent} not in {raw}"
                );
            }
        }
    }

    #[test]
    fn traced_exchange_redacts_the_key_in_the_request_body() {
        let ok = response("200 OK", "application/json", ANSWER);
        let (client, requests) = serve_with_key(vec![ok], SENTINEL_KEY);
        let mut asked = request();
        asked.state = json!({ "note": format!("key {SENTINEL_KEY}") });
        asked.guidance = format!("Never repeat {SENTINEL_KEY}.");
        let mut trace = None;
        client.choose_traced(&asked, &mut trace).unwrap();

        let body = trace.unwrap().body;
        assert_eq!(body["state"]["note"], "key <redacted>");
        assert_eq!(
            body["questions"]["move"]["instructions"]["guidance"],
            "Never repeat <redacted>."
        );
        assert!(!body.to_string().contains(SENTINEL_KEY), "{body}");
        // The wire carries the request as it was asked.
        let raw = &requests.lock().unwrap()[0];
        assert!(raw.contains(&format!("key {SENTINEL_KEY}")), "{raw}");
        assert!(
            raw.contains(&format!("Never repeat {SENTINEL_KEY}.")),
            "{raw}"
        );
    }

    #[test]
    fn traced_exchange_redacts_a_key_with_an_escaped_solidus() {
        // JSON may write `/` as `\/`: a parser reads it back as the key, though the raw
        // text does not contain it. Text that is not JSON is redacted in that form too.
        let key = "sentinel/key-7Qx9";
        let escaped = key.replace('/', "\\/");
        let (client, _requests) = serve_with_key(
            vec![
                response(
                    "401 Unauthorized",
                    "application/json",
                    &format!(r#"{{"error":"invalid api key {escaped}"}}"#),
                ),
                response(
                    "401 Unauthorized",
                    "text/plain",
                    &format!("no such key: {escaped} ("),
                ),
            ],
            key,
        );

        let mut trace = None;
        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
        assert_eq!(
            trace.unwrap().attempts[0].response.as_deref(),
            Some(r#"{"error":"invalid api key <redacted>"}"#)
        );
        assert_eq!(
            error.to_string(),
            r#"HTTP 401: {"error":"invalid api key <redacted>"}"#
        );

        let mut trace = None;
        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
        assert_eq!(
            trace.unwrap().attempts[0].response.as_deref(),
            Some("no such key: <redacted> (")
        );
        assert_eq!(error.to_string(), "HTTP 401: no such key: <redacted> (");
    }

    #[test]
    fn traced_exchange_records_a_failed_connection_without_a_status() {
        // Port 1 (tcpmux) is privileged and never listened on here, so the connection
        // is refused; a freed ephemeral port could be reused by a parallel test.
        let client = JevClient::new(&EngineConfig {
            endpoint: "http://127.0.0.1:1/v1/systemone".to_string(),
            ..keyed_config("test-key")
        })
        .unwrap();
        let mut trace = None;
        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
        assert!(matches!(error, JevError::Transport(_)), "{error:?}");
        let exchange = trace.expect("the client records a failed exchange too");
        assert_eq!(exchange.attempts.len(), 3, "{:?}", exchange.attempts);
        for attempt in &exchange.attempts {
            assert_eq!(attempt.status, None);
            assert_eq!(attempt.response, None);
            let text = attempt.error.as_deref().expect("an error for each attempt");
            assert!(text.starts_with("network error: "), "{text}");
        }
    }

    #[test]
    fn traced_exchange_redacts_a_key_the_server_echoes() {
        let echo = format!(r#"{{"error":"invalid api key {SENTINEL_KEY}"}}"#);
        // The key straddles the 200-character cut of the error snippet.
        let long = format!("{}{SENTINEL_KEY}", "x".repeat(195));
        let (client, requests) = serve_with_key(
            vec![
                response("401 Unauthorized", "application/json", &echo),
                // serde_json quotes the offending string in its error.
                response("200 OK", "application/json", &format!("\"{SENTINEL_KEY}\"")),
                response("422 Unprocessable Entity", "text/plain", &long),
            ],
            SENTINEL_KEY,
        );

        let mut trace = None;
        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
        let exchange = trace.unwrap();
        assert_eq!(
            exchange.attempts[0].response.as_deref(),
            Some(r#"{"error":"invalid api key <redacted>"}"#)
        );
        assert_eq!(
            exchange.attempts[0].error.as_deref(),
            Some(r#"HTTP 401: {"error":"invalid api key <redacted>"}"#)
        );
        assert_eq!(exchange.headers[0].1, "Bearer <redacted>");
        let mut texts = vec![error.to_string(), format!("{exchange:?}")];

        let mut trace = None;
        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
        assert!(matches!(error, JevError::InvalidResponse(_)), "{error:?}");
        let exchange = trace.unwrap();
        assert_eq!(
            exchange.attempts[0].response.as_deref(),
            Some("\"<redacted>\"")
        );
        texts.extend([error.to_string(), format!("{exchange:?}")]);

        let mut trace = None;
        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
        let JevError::Http { message, .. } = &error else {
            panic!("expected an HTTP error, got {error:?}");
        };
        assert_eq!(*message, format!("{}<reda", "x".repeat(195)));
        texts.extend([error.to_string(), format!("{:?}", trace.unwrap())]);

        assert_eq!(count(&requests), 3);
        for text in &texts {
            assert!(!text.contains(&SENTINEL_KEY[..8]), "{text}");
        }
        // The server did receive the key.
        assert!(requests.lock().unwrap()[0].contains(SENTINEL_KEY));
    }

    /// `text` with every character written as a JSON `\u` escape: text a JSON parser
    /// reads back as `text`, though it does not contain it.
    fn json_escaped(text: &str) -> String {
        text.chars()
            .map(|c| format!("\\u{:04x}", u32::from(c)))
            .collect()
    }

    /// The exchange a traced client sending `api_key` records against a local server
    /// that echoes the key in a 503 body, then JSON-escaped in another, then answers.
    /// The TUI's key tests render and log it, so they check an exchange as the engine
    /// really records it.
    pub(crate) fn recorded_exchange(api_key: &str) -> JevExchange {
        let echo = format!(r#"{{"error":"overloaded, key {api_key} is queued"}}"#);
        let escaped = format!(
            r#"{{"error":"overloaded, key {} is queued"}}"#,
            json_escaped(api_key)
        );
        let (client, requests) = serve_with_key(
            vec![
                response("503 Service Unavailable", "application/json", &echo),
                response("503 Service Unavailable", "application/json", &escaped),
                response("200 OK", "application/json", ANSWER),
            ],
            api_key,
        );
        let mut trace = None;
        client
            .choose_traced(&request(), &mut trace)
            .expect("the server answers the retry");
        assert!(requests.lock().unwrap()[0].contains(api_key));
        trace.expect("the client records the exchange")
    }

    #[test]
    fn traced_exchange_redacts_a_key_the_server_escapes() {
        // `\u` escapes do not match the key in the raw text, but every JSON parser, the
        // exchange view's and the debug log's included, decodes them back into it.
        let escaped = json_escaped(SENTINEL_KEY);
        let (client, _requests) = serve_with_key(
            vec![
                response(
                    "401 Unauthorized",
                    "application/json",
                    &format!(r#"{{"error":"invalid api key {escaped}"}}"#),
                ),
                response(
                    "200 OK",
                    "application/json",
                    &ANSWER.replace("jev-1.13.0", &escaped),
                ),
            ],
            SENTINEL_KEY,
        );

        let mut trace = None;
        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
        assert_eq!(
            trace.unwrap().attempts[0].response.as_deref(),
            Some(r#"{"error":"invalid api key <redacted>"}"#)
        );
        assert_eq!(
            error.to_string(),
            r#"HTTP 401: {"error":"invalid api key <redacted>"}"#
        );

        let mut trace = None;
        let answer = client.choose_traced(&request(), &mut trace).unwrap();
        // The answer is parsed as received; only the record is redacted.
        assert_eq!(answer.model.as_deref(), Some(SENTINEL_KEY));
        let text = trace.unwrap().attempts[0].response.clone().unwrap();
        let decoded: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(decoded["model"], "<redacted>");
        assert_eq!(decoded["answers"]["move"]["choice"], "O-O");
    }

    #[test]
    fn a_body_that_is_not_json_but_decodes_to_the_key_is_withheld() {
        // `\u` escapes (and `\/`) in text that is not JSON as a whole: a reader that
        // decodes part of it would get the key back, so the whole body is withheld.
        let key = "sentinel/key-7Qx9";
        let unicode = json_escaped(key);
        let mixed = format!("sentinel\\/key{}7Qx9", json_escaped("-"));
        let (client, _requests) = serve_with_key(
            vec![
                response(
                    "401 Unauthorized",
                    "text/plain",
                    &format!("no such key: \"{unicode}\" ("),
                ),
                response(
                    "502 Bad Gateway",
                    "text/html",
                    &format!("<p>key {mixed}</p>"),
                ),
                response("502 Bad Gateway", "text/html", "<p>busy</p>"),
                response("502 Bad Gateway", "text/html", "<p>busy</p>"),
            ],
            key,
        );
        let withheld = "<redacted: the body contained the API key>";

        let mut trace = None;
        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
        assert_eq!(
            trace.unwrap().attempts[0].response.as_deref(),
            Some(withheld)
        );
        assert_eq!(error.to_string(), format!("HTTP 401: {withheld}"));

        let mut trace = None;
        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
        let attempts = trace.unwrap().attempts;
        assert_eq!(attempts[0].response.as_deref(), Some(withheld));
        assert_eq!(
            attempts[0].error.as_deref(),
            Some(format!("HTTP 502: {withheld}").as_str())
        );
        assert_eq!(attempts[1].response.as_deref(), Some("<p>busy</p>"));
        assert_eq!(error.to_string(), "HTTP 502: <p>busy</p>");
    }

    #[test]
    fn the_answer_is_parsed_as_received_and_only_the_record_is_redacted() {
        // A key that also occurs in a valid answer must not change what is parsed.
        let key = "O-O";
        let ok = response("200 OK", "application/json", ANSWER);
        let (client, _requests) = serve_with_key(vec![ok.clone(), ok], key);
        let untraced = client.choose(&request()).unwrap();
        assert_eq!(untraced.choice, "O-O");
        assert_eq!(
            untraced.probabilities,
            vec![("O-O".to_string(), 0.75), ("Nxe5".to_string(), 0.25)]
        );

        let mut trace = None;
        let traced = client.choose_traced(&request(), &mut trace).unwrap();
        assert_eq!(traced, untraced, "tracing does not change the answer");
        let recorded = trace.unwrap().attempts[0].response.clone().unwrap();
        assert_eq!(recorded, ANSWER.replace("O-O", "<redacted>"));
    }

    #[test]
    fn error_text_is_redacted_then_made_printable() {
        let key = "sentinel-key-7Qx9";
        let transport = JevError::Transport(format!("reset\r\n\x1b[2J by {key}"));
        assert_eq!(
            redact_error(transport, key),
            JevError::Transport("reset   [2J by <redacted>".to_string())
        );
        // The key straddles the 200-character cut: it is redacted before the cut.
        let long = format!("{}{key}", "x".repeat(195));
        assert_eq!(
            redact_error(JevError::Request(long), key),
            JevError::Request(format!("{}<reda", "x".repeat(195)))
        );
        let request = JevError::Request("bad uri: http://x\u{7}/\n".to_string());
        assert_eq!(
            redact_error(request, key).to_string(),
            "request failed: bad uri: http://x / "
        );
    }

    #[test]
    fn decodes_json_escapes_as_a_parser_would() {
        assert_eq!(
            decode_json_escapes(r#"a\u0062c \/ \" \\ \t \ud83d\ude00"#),
            "abc / \" \\ \t \u{1f600}"
        );
        // An escaped backslash does not start another escape.
        assert_eq!(decode_json_escapes(r"\\u0041"), r"\u0041");
        // Invalid or cut-short escapes stay as they are.
        for text in [r"\x", r"\u00", r"\uzzzz", r"\ud83d", "end\\"] {
            assert_eq!(decode_json_escapes(text), text);
        }
        // A lone high surrogate stays; the escape after it is still decoded.
        assert_eq!(decode_json_escapes(r"\ud83d\u0041"), r"\ud83dA");
    }

    #[test]
    fn untraced_errors_redact_an_echoed_key_too() {
        let echo = format!("no such key: {SENTINEL_KEY}");
        let (client, _requests) = serve_with_key(
            vec![response("401 Unauthorized", "text/plain", &echo)],
            SENTINEL_KEY,
        );
        let error = client.choose(&request()).unwrap_err();
        assert_eq!(error.to_string(), "HTTP 401: no such key: <redacted>");
    }

    #[test]
    fn redaction_covers_every_string_in_a_json_body() {
        let body = json!({
            "model": format!("m-{SENTINEL_KEY}"),
            "nested": [{ SENTINEL_KEY: [SENTINEL_KEY, 1, true, null] }],
        });
        assert_eq!(
            redact_value(&body, SENTINEL_KEY),
            json!({
                "model": "m-<redacted>",
                "nested": [{ "<redacted>": ["<redacted>", 1, true, null] }],
            })
        );
        assert_eq!(redact("a key b key", "key"), "a <redacted> b <redacted>");
        assert_eq!(redact("text", ""), "text", "an empty key redacts nothing");
    }

    #[test]
    fn default_choose_traced_forwards_to_choose_and_records_nothing() {
        struct Plain;
        impl MoveChooser for Plain {
            fn choose(&self, _: &ChoiceRequest) -> Result<ChoiceAnswer, JevError> {
                Err(JevError::Timeout)
            }
        }
        let mut trace = None;
        assert_eq!(
            Plain.choose_traced(&request(), &mut trace),
            Err(JevError::Timeout)
        );
        assert_eq!(trace, None);
    }

    #[test]
    fn client_needs_a_key_and_hides_it() {
        assert!(JevClient::new(&EngineConfig::default()).is_none());
        let config = EngineConfig {
            api_key: Some("secret-key-123".to_string()),
            ..EngineConfig::default()
        };
        let client = JevClient::new(&config).unwrap();
        let text = format!("{client:?}");
        assert!(!text.contains("secret-key-123"), "{text}");
        assert!(text.contains(JEV_ENDPOINT), "{text}");
    }

    #[test]
    fn the_endpoint_is_the_quickstart_url() {
        // The host and scheme the offline transport tests cannot see; they check the path.
        assert!(
            JEV_ENDPOINT.starts_with("https://api.typesafe.ai/"),
            "{JEV_ENDPOINT}"
        );
        assert!(JEV_ENDPOINT.ends_with("/v1/systemone"), "{JEV_ENDPOINT}");
    }

    /// Real API round trip. Run with: cargo test --lib engine::jev -- --ignored
    #[test]
    #[ignore]
    fn live_choice_round_trip() {
        let config = EngineConfig::from_env();
        let client = JevClient::new(&config).expect("set JEV_API_KEY to run the live test");
        let answer = client.choose(&request()).expect("Jev answered");
        assert!(
            ["Nxe5", "O-O"].contains(&answer.choice.as_str()),
            "{answer:?}"
        );
        assert!(
            answer
                .model
                .as_deref()
                .is_some_and(|m| m.starts_with("jev-"))
        );
        assert!(answer.input_tokens.is_some_and(|n| n > 0));
    }

    #[test]
    fn laya_without_a_key_sends_and_records_no_authorization() {
        let (client, requests) = serve_config(
            vec![response("200 OK", "application/json", ANSWER)],
            laya_config(),
        );
        let mut trace = None;
        let answer = client.choose_traced(&request(), &mut trace).unwrap();
        assert_eq!(answer.choice, "O-O");
        let raw = requests.lock().unwrap()[0].clone();
        assert!(header_values(&raw, "authorization").is_empty(), "{raw}");
        assert_eq!(
            header_values(&raw, "content-type"),
            vec!["application/json"]
        );
        let exchange = trace.expect("recorded");
        assert_eq!(
            exchange.headers,
            vec![("Content-Type".to_string(), "application/json".to_string())]
        );
        assert!(
            exchange.url.starts_with("http://127.0.0.1:"),
            "{}",
            exchange.url
        );
    }

    #[test]
    fn laya_with_a_key_sends_a_bearer_and_redacts_it() {
        let config = EngineConfig {
            api_key: Some(SENTINEL_KEY.to_string()),
            ..laya_config()
        };
        let (client, requests) =
            serve_config(vec![response("200 OK", "application/json", ANSWER)], config);
        let mut trace = None;
        client.choose_traced(&request(), &mut trace).unwrap();
        let raw = requests.lock().unwrap()[0].clone();
        assert_eq!(
            header_values(&raw, "authorization"),
            vec![format!("Bearer {SENTINEL_KEY}")]
        );
        let recorded = format!("{:?}", trace.expect("recorded"));
        assert!(!recorded.contains(SENTINEL_KEY), "{recorded}");
        assert!(recorded.contains("Bearer <redacted>"), "{recorded}");
    }

    #[test]
    fn a_refused_connection_is_tried_three_times() {
        // Port 1 is privileged and never listened on; a freed ephemeral port could be
        // reused by a parallel test.
        let config = EngineConfig {
            endpoint: "http://127.0.0.1:1/v1/systemone".to_string(),
            ..laya_config()
        };
        let client = JevClient::new(&config).unwrap();
        let mut trace = None;
        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
        assert!(matches!(error, JevError::Transport(_)), "{error:?}");
        let attempts = trace.expect("recorded").attempts;
        assert_eq!(attempts.len(), 3);
        assert!(attempts.iter().all(|a| a.status.is_none()), "{attempts:?}");
    }

    #[test]
    fn an_unprocessable_question_is_final() {
        let rejected = response(
            "422 Unprocessable Entity",
            "application/json",
            r#"{"detail":"criteria: too many options"}"#,
        );
        let (client, requests) = serve_config(vec![rejected], laya_config());
        let error = client.choose(&request()).unwrap_err();
        let JevError::Http {
            status, message, ..
        } = &error
        else {
            panic!("expected an HTTP error, got {error:?}");
        };
        assert_eq!(*status, 422);
        assert!(message.contains("too many options"), "{message}");
        assert_eq!(count(&requests), 1);
    }

    #[test]
    fn parses_an_answer_without_usage_or_model() {
        let body = r#"{"answers":{"move":{"type":"choice","choice":"Nxe5",
            "probabilities":{"Nxe5":0.8,"O-O":0.2},"confidence":0.7}}}"#;
        let answer = parse_answer(body).unwrap();
        assert_eq!(answer.choice, "Nxe5");
        assert_eq!(answer.model, None);
        assert_eq!(answer.input_tokens, None);
    }

    #[test]
    fn a_disabled_config_builds_no_client() {
        assert!(JevClient::new(&EngineConfig::default()).is_none());
        let off = EngineConfig {
            endpoint: String::new(),
            ..laya_config()
        };
        assert!(JevClient::new(&off).is_none());
        assert!(
            JevClient::new(&laya_config()).is_some(),
            "Laya needs no key"
        );
    }

    /// Real `laya-serve` round trip. Run with:
    /// LAYA_URL=http://127.0.0.1:8000/v1/systemone cargo test --lib engine::jev -- --ignored live_laya
    #[test]
    #[ignore]
    fn live_laya_choice_round_trip() {
        let config = EngineConfig::laya_from_env();
        let client = JevClient::new(&config).expect("set LAYA_URL to run the live Laya test");
        let answer = client.choose(&request()).expect("Laya answered");
        assert!(
            ["Nxe5", "O-O"].contains(&answer.choice.as_str()),
            "{answer:?}"
        );
    }
}
