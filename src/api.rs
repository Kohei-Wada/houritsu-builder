use std::io::Read;
use std::time::Duration;

use serde_json::Value;

pub const BASE: &str = "https://laws.e-gov.go.jp/api/2";
const BACKOFF: f64 = 2.0;

#[derive(Debug)]
pub struct ApiError(pub String);

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ApiError {}

pub enum HttpError {
    Status(u16),
    Other(String),
}

pub trait Api: Sync {
    fn get_json(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, ApiError>;
}

type Opener = Box<dyn Fn(&str) -> Result<Vec<u8>, HttpError> + Send + Sync>;
type Sleep = Box<dyn Fn(f64) + Send + Sync>;

pub struct Client {
    pub base: String,
    pub delay: f64,
    pub retries: u32,
    opener: Opener,
    sleep: Sleep,
}

fn urlopen(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, HttpError> {
    match agent.get(url).call() {
        Ok(resp) => {
            let mut body = Vec::new();
            resp.into_reader()
                .read_to_end(&mut body)
                .map_err(|e| HttpError::Other(e.to_string()))?;
            Ok(body)
        }
        Err(ureq::Error::Status(code, _)) => Err(HttpError::Status(code)),
        Err(e) => Err(HttpError::Other(e.to_string())),
    }
}

fn quote_plus(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl Client {
    pub fn new(delay: f64) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(120))
            .build();
        Self::with(
            BASE,
            delay,
            Box::new(move |url| urlopen(&agent, url)),
            Box::new(|s| std::thread::sleep(Duration::from_secs_f64(s))),
        )
    }

    pub fn with(base: &str, delay: f64, opener: Opener, sleep: Sleep) -> Self {
        Client {
            base: base.to_string(),
            delay,
            retries: 5,
            opener,
            sleep,
        }
    }
}

impl Api for Client {
    fn get_json(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, ApiError> {
        let mut url = format!("{}/{path}", self.base);
        if !params.is_empty() {
            let query: Vec<String> = params
                .iter()
                .map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v)))
                .collect();
            url = format!("{url}?{}", query.join("&"));
        }
        let mut last = String::new();
        for attempt in 0..self.retries {
            match (self.opener)(&url) {
                Ok(body) => match serde_json::from_slice(&body) {
                    Ok(value) => {
                        (self.sleep)(self.delay);
                        return Ok(value);
                    }
                    Err(e) => last = format!("JSONDecodeError({e})"),
                },
                Err(HttpError::Status(404)) => return Err(ApiError(format!("404 {url}"))),
                Err(HttpError::Status(code)) => last = format!("HTTPError({code})"),
                Err(HttpError::Other(e)) => last = e,
            }
            if attempt < self.retries - 1 {
                (self.sleep)(BACKOFF * 2f64.powi(attempt as i32));
            }
        }
        Err(ApiError(format!(
            "gave up after {} attempts: {url}: {last}",
            self.retries
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    type Log<T> = Arc<Mutex<Vec<T>>>;

    fn client(
        retries: u32,
        responses: Vec<Result<Vec<u8>, HttpError>>,
    ) -> (Client, Log<String>, Log<f64>) {
        let calls = Arc::new(Mutex::new(vec![]));
        let sleeps = Arc::new(Mutex::new(vec![]));
        let queue = Mutex::new(responses.into_iter());
        let (c, s) = (calls.clone(), sleeps.clone());
        let mut client = Client::with(
            "https://x",
            1.0,
            Box::new(move |url| {
                c.lock().unwrap().push(url.to_string());
                queue
                    .lock()
                    .unwrap()
                    .next()
                    .unwrap_or(Err(HttpError::Other("down".into())))
            }),
            Box::new(move |secs| s.lock().unwrap().push(secs)),
        );
        client.retries = retries;
        (client, calls, sleeps)
    }

    #[test]
    fn spaces_requests_and_builds_query() {
        let (c, calls, sleeps) = client(5, vec![Ok(br#"{"ok": 1}"#.to_vec())]);
        assert_eq!(
            c.get_json("laws", &[("limit", "1000"), ("offset", "0")])
                .unwrap(),
            json!({"ok": 1})
        );
        assert_eq!(
            *calls.lock().unwrap(),
            ["https://x/laws?limit=1000&offset=0"]
        );
        assert_eq!(*sleeps.lock().unwrap(), [1.0]);
    }

    #[test]
    fn retries_with_backoff_then_succeeds() {
        let (c, _, sleeps) = client(
            5,
            vec![
                Err(HttpError::Other("reset".into())),
                Err(HttpError::Other("reset".into())),
                Ok(b"[]".to_vec()),
            ],
        );
        assert_eq!(c.get_json("laws", &[]).unwrap(), json!([]));
        assert_eq!(*sleeps.lock().unwrap(), [2.0, 4.0, 1.0]);
    }

    #[test]
    fn gives_up_after_retries_keeps_last_error_and_skips_final_backoff() {
        let (c, _, sleeps) = client(2, vec![]);
        let e = c.get_json("laws", &[]).unwrap_err().0;
        assert!(
            e.starts_with("gave up after 2 attempts") && e.ends_with(": down"),
            "{e}"
        );
        assert_eq!(*sleeps.lock().unwrap(), [2.0]);
    }

    #[test]
    fn not_found_is_not_retried() {
        let (c, calls, _) = client(5, vec![Err(HttpError::Status(404))]);
        assert!(c
            .get_json("law_data/x", &[])
            .unwrap_err()
            .0
            .starts_with("404"));
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn transient_failures_are_retried() {
        for first in [
            Err(HttpError::Other("incomplete".into())),
            Ok(b"\xff\xff{".to_vec()),
            Err(HttpError::Status(500)),
        ] {
            let (c, _, sleeps) = client(5, vec![first, Ok(br#"{"ok": 1}"#.to_vec())]);
            assert_eq!(c.get_json("laws", &[]).unwrap(), json!({"ok": 1}));
            assert_eq!(*sleeps.lock().unwrap(), [2.0, 1.0]);
        }
    }
}
