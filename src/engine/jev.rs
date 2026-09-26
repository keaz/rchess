//! TypeSafe Jev client: request/response types, the retry policy and the HTTP
//! transport (spec 5.1, 5.7). Everything is tested offline; the transport tests
//! talk to a scripted HTTP server on 127.0.0.1.
//!
//! Never enable TRACE-level logging for `ureq` or `ureq_proto`: it prints request
//! headers, including the `Authorization` bearer key.

use std::collections::HashMap;
use std::fmt;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

use super::annotate::Bucket;
use super::config::EngineConfig;

/// Maximum attempts for one request, counting the first.
const MAX_ATTEMPTS: u32 = 3;
const BASE_BACKOFF: Duration = Duration::from_millis(250);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(2);
/// Longest error-body excerpt kept in a `JevError`.
const ERROR_SNIPPET_CHARS: usize = 200;
/// Largest response body read, in bytes; a real answer is a few kilobytes.
const MAX_BODY_BYTES: u64 = 1 << 20;

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
    /// JSON body for `POST /v1/systemone`; the question id is `move`.
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
    /// Versioned model ID that answered, e.g. `jev-1.13.0`.
    pub model: String,
    /// Input tokens billed for the request.
    pub input_tokens: u32,
}

/// Why a Jev request failed. Messages never contain the API key.
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

/// Something that can answer a `choice` question: `JevClient`, or a mock in tests.
pub trait MoveChooser: Send + Sync {
    /// Answers one `choice` question. An error is final: retries happen inside.
    fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceAnswer, JevError>;
}

#[derive(Deserialize)]
struct ApiResponse {
    model: String,
    answers: HashMap<String, ApiAnswer>,
    usage: ApiUsage,
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
        input_tokens: response.usage.input_tokens,
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

/// HTTP client for `POST {base_url}/v1/systemone`.
pub struct JevClient {
    agent: ureq::Agent,
    url: String,
    model: String,
    api_key: String,
}

impl fmt::Debug for JevClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JevClient")
            .field("url", &self.url)
            .field("model", &self.model)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl JevClient {
    /// `None` when the config has no API key.
    pub fn new(config: &EngineConfig) -> Option<JevClient> {
        let api_key = config.api_key.clone()?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(config.timeout))
            .http_status_as_error(false)
            .build()
            .into();
        Some(JevClient {
            agent,
            url: format!("{}/v1/systemone", config.base_url),
            model: config.model.clone(),
            api_key,
        })
    }

    fn post_once(&self, body: &Value) -> Result<ChoiceAnswer, JevError> {
        let mut response = self
            .agent
            .post(&self.url)
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(body)
            .map_err(request_error)?;
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
        if status != 200 {
            // Keep the status and Retry-After even when the body cannot be read.
            return Err(match text {
                Ok(text) => http_error(status, &text, retry_after),
                Err(_) => JevError::Http {
                    status,
                    message: "<unreadable body>".into(),
                    retry_after,
                },
            });
        }
        parse_answer(&text.map_err(request_error)?)
    }
}

impl MoveChooser for JevClient {
    fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceAnswer, JevError> {
        let body = request.to_body(&self.model);
        let mut attempt = 1;
        loop {
            match self.post_once(&body) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        assert_eq!(answer.model, "jev-1.13.0");
        assert_eq!(answer.input_tokens, 812);
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

    /// Serves `responses` in order, one per connection, on 127.0.0.1, then stops
    /// listening. Returns a client pointed at it and the number of requests read.
    fn serve(responses: Vec<String>) -> (JevClient, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        thread::spawn(move || {
            for response in responses {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream);
                read_request(&mut reader);
                counter.fetch_add(1, Ordering::SeqCst);
                let mut stream = reader.into_inner();
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                // Dropping the stream closes the connection.
            }
        });
        let config = EngineConfig {
            api_key: Some("test-key".to_string()),
            base_url: format!("http://127.0.0.1:{port}"),
            timeout: Duration::from_secs(2),
            ..EngineConfig::default()
        };
        (JevClient::new(&config).unwrap(), requests)
    }

    /// Reads one request: the head, then a body of `Content-Length` bytes.
    fn read_request(reader: &mut BufReader<TcpStream>) {
        let mut length = 0;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0; length];
        let _ = reader.read_exact(&mut body);
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
        assert_eq!(answer.model, "jev-1.13.0");
        assert_eq!(answer.input_tokens, 321);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
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
        assert_eq!(requests.load(Ordering::SeqCst), 3);
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
        assert_eq!(requests.load(Ordering::SeqCst), 1);
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
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn transport_rejects_a_body_over_one_mebibyte() {
        let huge = response("200 OK", "application/json", &" ".repeat((1 << 20) + 1));
        let (client, requests) = serve(vec![huge]);
        let error = client.choose(&request()).unwrap_err();
        assert!(matches!(error, JevError::Request(_)), "{error:?}");
        assert_eq!(requests.load(Ordering::SeqCst), 1);
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
        assert!(text.contains("https://api.typesafe.ai/v1/systemone"));
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
        assert!(answer.model.starts_with("jev-"));
        assert!(answer.input_tokens > 0);
    }
}
