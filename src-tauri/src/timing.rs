//! Opt-in, request-scoped timings. No prompt, response, URL, or key is recorded.
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
pub struct Trace(Option<Arc<Inner>>);

struct Inner {
    start: Instant,
    path: PathBuf,
    record: Mutex<Record>,
}

#[derive(Serialize)]
struct Record {
    schema_version: u8,
    request_id: String,
    started_unix_ms: u128,
    model: String,
    status: String,
    total_ms: f64,
    events: Vec<Value>,
}

impl Trace {
    pub fn enabled(&self) -> bool {
        self.0.is_some()
    }

    pub fn from_env(request_id: u64, model: &str) -> Self {
        std::env::var_os("SNIPPET_TRACE")
            .filter(|path| !path.is_empty())
            .map(|path| Self::new(path.into(), request_id.to_string(), model))
            .unwrap_or_default()
    }

    pub fn new(path: PathBuf, request_id: String, model: &str) -> Self {
        Self(Some(Arc::new(Inner {
            start: Instant::now(),
            path,
            record: Mutex::new(Record {
                schema_version: 1,
                request_id,
                started_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
                model: model.into(),
                status: "running".into(),
                total_ms: 0.0,
                events: vec![],
            }),
        })))
    }

    pub fn mark(&self, name: &str, detail: Value) {
        if let Some(inner) = &self.0 {
            if let Ok(mut record) = inner.record.lock() {
                record.events.push(json!({"name": name, "at_ms": inner.start.elapsed().as_secs_f64() * 1000.0, "detail": detail}));
            }
        }
    }

    pub fn span(&self, name: &str) -> Span {
        Span {
            trace: self.clone(),
            name: name.into(),
            start: Instant::now(),
            complete: false,
        }
    }

    pub fn ollama(&self, stage: &str, value: &Value) {
        if !self.enabled() {
            return;
        }
        // Ollama's durations are nanoseconds. Keep missing metrics absent, not zero.
        let mut metrics = serde_json::Map::new();
        for field in [
            "total_duration",
            "load_duration",
            "prompt_eval_duration",
            "eval_duration",
        ] {
            if let Some(ns) = value.get(field).and_then(Value::as_u64) {
                metrics.insert(
                    field.replace("_duration", "_ms"),
                    json!(ns as f64 / 1_000_000.0),
                );
            }
        }
        for field in ["prompt_eval_count", "eval_count"] {
            if let Some(count) = value.get(field).and_then(Value::as_u64) {
                metrics.insert(field.into(), json!(count));
            }
        }
        if let (Some(count), Some(ns)) = (
            value["eval_count"].as_u64(),
            value["eval_duration"].as_u64(),
        ) {
            if ns > 0 {
                metrics.insert(
                    "tokens_per_second".into(),
                    json!(count as f64 * 1e9 / ns as f64),
                );
            }
        }
        self.mark(&format!("{stage}.ollama"), Value::Object(metrics));
    }

    /// Write once per request, after all spans have ended. A failed write cannot fail generation.
    pub fn finish(&self, status: &str) {
        if let Some(inner) = &self.0 {
            if let Ok(mut record) = inner.record.lock() {
                if record.status != "running" {
                    return;
                }
                record.status = status.into();
                record.total_ms = inner.start.elapsed().as_secs_f64() * 1000.0;
                let write = || -> Result<(), Box<dyn std::error::Error>> {
                    let mut bytes = serde_json::to_vec(&*record)?;
                    bytes.push(b'\n');
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&inner.path)?
                        .write_all(&bytes)?;
                    Ok(())
                };
                if let Err(error) = write() {
                    eprintln!("Snippet timing trace could not be saved: {error}");
                }
            }
        }
    }
}

pub struct Span {
    trace: Trace,
    name: String,
    start: Instant,
    complete: bool,
}
impl Span {
    pub fn finish(mut self) {
        self.complete = true;
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        self.trace.mark(&self.name, json!({"duration_ms": self.start.elapsed().as_secs_f64() * 1000.0, "completed": self.complete}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_metrics_and_incomplete_spans_without_response_content() {
        let path =
            std::env::temp_dir().join(format!("snippet-trace-test-{}.jsonl", std::process::id()));
        let trace = Trace::new(path.clone(), "test".into(), "model");
        drop(trace.span("interrupted"));
        trace.ollama("answer", &json!({"load_duration": 2_000_000, "eval_count": 10, "eval_duration": 500_000_000, "message": {"content": "private"}}));
        trace.finish("cancelled");
        trace.finish("complete");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(!text.contains("private"));
        let record: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(record["status"], "cancelled");
        assert_eq!(record["events"][0]["detail"]["completed"], false);
        assert_eq!(record["events"][1]["detail"]["load_ms"], 2.0);
        assert_eq!(record["events"][1]["detail"]["tokens_per_second"], 20.0);
        assert!(record["events"][1]["detail"]
            .get("prompt_eval_count")
            .is_none());
        std::fs::remove_file(path).unwrap();
    }
}
