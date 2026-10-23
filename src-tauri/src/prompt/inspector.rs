//! Opt-in, memory-only snapshots at the provider boundary. Release calls are no-ops.
#[cfg(debug_assertions)]
mod development {
    use serde::Serialize;
    use serde_json::Value;
    use std::sync::{Mutex, OnceLock};

    const MAX_REQUESTS: usize = 10;
    const MAX_REQUEST_BYTES: usize = 256 * 1024;

    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Snapshot {
        id: u64,
        stage: String,
        captured_at: String,
        body: Value,
        note: Option<String>,
    }

    #[derive(Default, Serialize)]
    pub struct Inspector {
        enabled: bool,
        requests: Vec<Snapshot>,
        #[serde(skip)]
        next_id: u64,
        #[serde(skip)]
        revision: u64,
    }

    impl Inspector {
        fn set_enabled(&mut self, enabled: bool) {
            self.enabled = enabled;
            self.revision += 1;
            if !enabled {
                self.requests.clear();
            }
        }

        fn push(&mut self, stage: &str, mut body: Value) {
            redact_images(&mut body);
            let note = if body.to_string().len() > MAX_REQUEST_BYTES {
                body = Value::Null;
                Some("Request exceeds the 256 KiB inspector limit; body omitted.".into())
            } else {
                None
            };
            self.next_id += 1;
            self.requests.push(Snapshot {
                id: self.next_id,
                stage: stage.into(),
                captured_at: chrono::Utc::now().to_rfc3339(),
                body,
                note,
            });
            if self.requests.len() > MAX_REQUESTS {
                self.requests.remove(0);
            }
        }
    }

    fn redact_images(value: &mut Value) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    if key == "images" {
                        if let Value::Array(images) = value {
                            for image in images {
                                *image = Value::String("<image data omitted>".into());
                            }
                        }
                    } else {
                        redact_images(value);
                    }
                }
            }
            Value::Array(values) => values.iter_mut().for_each(redact_images),
            _ => {}
        }
    }

    fn state() -> &'static Mutex<Inspector> {
        static STATE: OnceLock<Mutex<Inspector>> = OnceLock::new();
        STATE.get_or_init(|| Mutex::new(Inspector::default()))
    }

    pub fn configure(enabled: bool) -> Result<(), String> {
        state()
            .lock()
            .map_err(|_| "Request inspector unavailable".to_owned())?
            .set_enabled(enabled);
        Ok(())
    }

    pub fn read(clear: bool) -> Result<Value, String> {
        let mut state = state()
            .lock()
            .map_err(|_| "Request inspector unavailable".to_owned())?;
        if clear {
            state.requests.clear();
            state.revision += 1;
        }
        serde_json::to_value(&*state).map_err(|error| error.to_string())
    }

    pub fn record(stage: &str, body: &impl Serialize) {
        let revision = match state().lock() {
            Ok(state) if state.enabled => state.revision,
            _ => return,
        };
        if let Ok(body) = serde_json::to_value(body) {
            if let Ok(mut state) = state().lock() {
                // A clear/disable during serialization must not restore old content.
                if state.enabled && state.revision == revision {
                    state.push(stage, body);
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        #[test]
        fn disabled_capture_never_serializes_request_content() {
            struct MustNotSerialize;
            impl Serialize for MustNotSerialize {
                fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                    panic!("disabled inspector serialized a request");
                }
            }
            configure(false).unwrap();
            record("disabled", &MustNotSerialize);
            let state = read(false).unwrap();
            assert_eq!(state["enabled"], false);
            assert_eq!(state["requests"], json!([]));
        }

        #[test]
        fn bounds_history_redacts_images_and_clears_on_disable() {
            let mut state = Inspector::default();
            assert!(!state.enabled);
            state.set_enabled(true);
            for _ in 0..12 {
                state.push("answer", json!({"messages":[{"content":"literal {{task}}", "images":["secret"]}],"images":["other"]}));
            }
            assert_eq!(state.requests.len(), 10);
            let body = &state.requests[0].body;
            assert_eq!(body["messages"][0]["content"], "literal {{task}}");
            assert_eq!(body["messages"][0]["images"][0], "<image data omitted>");
            assert_eq!(body["images"][0], "<image data omitted>");
            state.set_enabled(false);
            assert!(state.requests.is_empty());
        }

        #[test]
        fn oversized_bodies_are_explicitly_omitted() {
            let mut state = Inspector::default();
            state.push("answer", json!({"prompt":"x".repeat(MAX_REQUEST_BYTES)}));
            assert!(state.requests[0].body.is_null());
            assert!(state.requests[0].note.is_some());
        }
    }
}

#[cfg(debug_assertions)]
pub use development::{configure, read, record};

#[cfg(not(debug_assertions))]
#[inline]
pub fn record(_stage: &str, _body: &impl serde::Serialize) {}
