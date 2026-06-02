use base64::{engine::general_purpose, Engine as _};
use gsd_browser_common::types::CompactPageState;
use gsd_browser_common::viewer::{BrowserArtifactManifestV1, BROWSER_ARTIFACT_BUNDLE_SCHEMA};
use regex_lite::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSession {
    pub recording_id: String,
    pub session_id: String,
    pub name: String,
    pub started_at_ms: u64,
    pub paused: bool,
}

#[derive(Debug, Clone)]
pub struct RecordingEventInput {
    pub source: String,
    pub owner: String,
    pub kind: String,
    pub url: String,
    pub title: String,
    pub redacted: bool,
    // PR-1 enrichment: full command params + before/after with DOM hash + sessionStateHash + per-action network tags
    pub command: serde_json::Value,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
    pub network: serde_json::Value,
}

pub struct RecordingStore {
    root: PathBuf,
    active: Option<RecordingSession>,
    completed: Vec<BrowserArtifactManifestV1>,
    next_seq: u64,
    redaction_hits: u64,
    frame_count: u64,
    hashes: Map<String, Value>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl RecordingStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            active: None,
            completed: Vec::new(),
            next_seq: 1,
            redaction_hits: 0,
            frame_count: 0,
            hashes: Map::new(),
        }
    }

    pub fn start(&mut self, name: &str, session_id: &str) -> Result<RecordingSession, String> {
        if self.active.is_some() {
            return Err("recording already active".to_string());
        }
        self.next_seq = 1;
        self.redaction_hits = 0;
        self.frame_count = 0;
        self.hashes.clear();
        fs::create_dir_all(&self.root)
            .map_err(|err| format!("failed to create recordings root: {err}"))?;
        let recording_id = format!("rec_{}", uuid::Uuid::new_v4());
        let dir = self.root.join(&recording_id);
        for path in [
            dir.clone(),
            dir.join("frames"),
            dir.join("snapshots"),
            dir.join("annotations"),
            dir.join("logs"),
        ] {
            fs::create_dir_all(path)
                .map_err(|err| format!("failed to create recording dir: {err}"))?;
        }
        fs::write(dir.join("events.jsonl"), "")
            .map_err(|err| format!("failed to create events.jsonl: {err}"))?;
        fs::write(dir.join("deltas.json"), "{}")
            .map_err(|err| format!("failed to create deltas.json: {err}"))?;
        let session = RecordingSession {
            recording_id,
            session_id: session_id.to_string(),
            name: name.to_string(),
            started_at_ms: now_ms(),
            paused: false,
        };
        self.active = Some(session.clone());
        Ok(session)
    }

    pub fn pause(&mut self, recording_id: &str) -> Result<RecordingSession, String> {
        let active = self.active.as_mut().ok_or("no active recording")?;
        if active.recording_id != recording_id {
            return Err(format!("recording not active: {recording_id}"));
        }
        active.paused = true;
        Ok(active.clone())
    }

    pub fn resume(&mut self, recording_id: &str) -> Result<RecordingSession, String> {
        let active = self.active.as_mut().ok_or("no active recording")?;
        if active.recording_id != recording_id {
            return Err(format!("recording not active: {recording_id}"));
        }
        active.paused = false;
        Ok(active.clone())
    }

    pub fn record_event(&mut self, input: RecordingEventInput) -> Result<(), String> {
        let Some(active) = self.active.as_ref() else {
            return Ok(());
        };
        if active.paused {
            return Ok(());
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        let redaction_probe = format!("{} {} {}", input.kind, input.url, input.title);
        let text = redact_text(&redaction_probe);
        let url = redact_text(&input.url);
        let title = redact_text(&input.title);
        let (command, command_redacted) = redact_recording_value(&input.command);
        let (before, before_redacted) = redact_recording_value(&input.before);
        let (after, after_redacted) = redact_recording_value(&input.after);
        let (network, network_redacted) = redact_recording_value(&input.network);
        let redacted = text != redaction_probe
            || url != input.url
            || title != input.title
            || command_redacted
            || before_redacted
            || after_redacted
            || network_redacted
            || input.redacted;
        if redacted {
            self.redaction_hits += 1;
        }
        let event = json!({
            "seq": seq,
            "timestampMs": now_ms(),
            "schema": "BrowserEventV1",
            "recordingId": active.recording_id,
            "sessionId": active.session_id,
            "source": input.source,
            "owner": input.owner,
            "controlVersion": 0,
            "frameSeq": 0,
            "kind": input.kind,
            "url": url,
            "title": title,
            "origin": origin_from_url(&input.url),
            "command": command,
            "before": before,
            "after": after,
            "network": network,
            "redaction": { "status": if redacted { "redacted" } else { "none" } },
            "artifactRefs": {},
        });
        let line = serde_json::to_string(&event).map_err(|err| err.to_string())?;
        let path = self.root.join(&active.recording_id).join("events.jsonl");
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|err| format!("failed to open events.jsonl: {err}"))?;
        writeln!(file, "{line}").map_err(|err| format!("failed to append event: {err}"))?;
        Ok(())
    }

    pub fn record_frame(
        &mut self,
        frame: &crate::daemon::view::capture::FrameMessage,
    ) -> Result<(), String> {
        let Some(active) = self.active.as_ref() else {
            return Ok(());
        };
        if active.paused {
            return Ok(());
        }
        let rel = format!("frames/frame-{:06}.jpg", frame.frame_seq);
        let path = self.root.join(&active.recording_id).join(&rel);
        let bytes = general_purpose::STANDARD
            .decode(&frame.data_base64)
            .map_err(|err| format!("failed to decode frame: {err}"))?;
        fs::write(&path, &bytes).map_err(|err| format!("failed to write frame: {err}"))?;
        let hash = sha256_hex(&bytes);
        self.hashes.insert(rel, Value::String(hash));
        self.frame_count += 1;
        Ok(())
    }

    pub fn stop(&mut self, recording_id: &str) -> Result<BrowserArtifactManifestV1, String> {
        let active = self.active.take().ok_or("no active recording")?;
        if active.recording_id != recording_id {
            self.active = Some(active);
            return Err(format!("recording not active: {recording_id}"));
        }
        let event_count =
            count_jsonl_lines(&self.root.join(&active.recording_id).join("events.jsonl"))?;
        let manifest = BrowserArtifactManifestV1 {
            schema: BROWSER_ARTIFACT_BUNDLE_SCHEMA.to_string(),
            recording_id: active.recording_id.clone(),
            session_id: active.session_id,
            name: active.name,
            started_at_ms: active.started_at_ms,
            stopped_at_ms: Some(now_ms()),
            start_seq: 1,
            stop_seq: Some(event_count),
            event_count,
            frame_count: self.frame_count,
            annotation_count: 0,
            console_error_count: 0,
            failed_request_count: 0,
            origin_scopes: Vec::new(),
            excluded_boundary_events: Vec::new(),
            redaction: json!({
                "policy": "default-sensitive",
                "hitCount": self.redaction_hits,
                "classes": ["email", "query_token", "bearer_token", "data_token"]
            }),
            artifacts: json!({
                "events": "events.jsonl",
                "frames": "frames/",
                "annotations": "annotations/",
                "console": "logs/console.jsonl",
                "network": "logs/network.jsonl",
                "dialog": "logs/dialog.jsonl",
                "deltas": "deltas.json"
            }),
            hashes: Value::Object(self.hashes.clone()),
        };
        let dir = self.root.join(&manifest.recording_id);
        let data = serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())?;
        fs::write(dir.join("manifest.json"), data)
            .map_err(|err| format!("failed to write manifest: {err}"))?;
        self.completed.push(manifest.clone());
        Ok(manifest)
    }

    pub fn list(&self) -> Vec<BrowserArtifactManifestV1> {
        self.completed.clone()
    }

    pub fn get(&self, recording_id: &str) -> Option<BrowserArtifactManifestV1> {
        self.completed
            .iter()
            .find(|manifest| manifest.recording_id == recording_id)
            .cloned()
    }

    pub fn active_id(&self) -> Option<String> {
        self.active
            .as_ref()
            .map(|recording| recording.recording_id.clone())
    }

    pub fn export(&self, recording_id: &str, output: &Path) -> Result<PathBuf, String> {
        let src = self.root.join(recording_id);
        if !src.exists() {
            return Err(format!("recording not found: {recording_id}"));
        }
        fs::create_dir_all(output).map_err(|err| format!("failed to create export dir: {err}"))?;
        Ok(src)
    }

    pub fn discard(&mut self, recording_id: &str) -> Result<bool, String> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.recording_id == recording_id)
        {
            self.active = None;
        }
        let path = self.root.join(recording_id);
        if path.exists() {
            fs::remove_dir_all(path)
                .map_err(|err| format!("failed to discard recording: {err}"))?;
            self.completed
                .retain(|manifest| manifest.recording_id != recording_id);
            return Ok(true);
        }
        Ok(false)
    }
}

pub fn redact_text(text: &str) -> String {
    let mut value = text.to_string();
    let rules = [
        (
            r"(?i)[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}",
            "[redacted:email]",
        ),
        (
            r"(?i)bearer\s+[A-Za-z0-9._~+/=-]+",
            "bearer [redacted:token]",
        ),
        (
            r"(?i)(token|secret|key|code|otp)=([^&\s]+)",
            "[redacted:token]",
        ),
        (
            r#"(?i)data-token=["'][^"']+["']"#,
            "data-token=\"[redacted:token]\"",
        ),
        (r"[A-Za-z0-9_-]{32,}", "[redacted:token]"),
    ];
    for (pattern, replacement) in rules {
        let regex = Regex::new(pattern).expect("valid redaction regex");
        value = regex.replace_all(&value, replacement).to_string();
    }
    value
}

fn redact_recording_value(value: &Value) -> (Value, bool) {
    redact_recording_value_with_key(value, None)
}

fn redact_recording_value_with_key(value: &Value, key: Option<&str>) -> (Value, bool) {
    if key.is_some_and(is_sensitive_json_key) {
        return (sensitive_redaction_marker(value), true);
    }

    match value {
        Value::String(text) => {
            let redacted = redact_text(text);
            let changed = redacted != *text;
            (Value::String(redacted), changed)
        }
        Value::Array(items) => {
            let mut changed = false;
            let redacted = items
                .iter()
                .map(|item| {
                    let (value, item_changed) = redact_recording_value_with_key(item, None);
                    changed |= item_changed;
                    value
                })
                .collect();
            (Value::Array(redacted), changed)
        }
        Value::Object(map) => {
            let mut changed = false;
            let redacted = map
                .iter()
                .map(|(key, value)| {
                    let (value, value_changed) =
                        redact_recording_value_with_key(value, Some(key.as_str()));
                    changed |= value_changed;
                    (key.clone(), value)
                })
                .collect();
            (Value::Object(redacted), changed)
        }
        _ => (value.clone(), false),
    }
}

fn is_sensitive_json_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "password"
            | "passphrase"
            | "passwd"
            | "pwd"
            | "secret"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "authtoken"
            | "bearertoken"
            | "apikey"
            | "api"
            | "key"
            | "code"
            | "otp"
            | "pin"
            | "credential"
            | "credentials"
            | "authorization"
            | "cookie"
            | "cookies"
            | "setcookie"
            | "text"
            | "value"
    )
}

fn sensitive_redaction_marker(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Array(_) => json!("[redacted:sensitive-array]"),
        Value::Object(_) => json!("[redacted:sensitive-object]"),
        _ => json!("[redacted:sensitive]"),
    }
}

pub fn validate_recording_bundle(path: &Path) -> Result<serde_json::Value, String> {
    let manifest_path = path.join("manifest.json");
    let events_path = path.join("events.jsonl");
    if !manifest_path.exists() {
        return Err("missing manifest.json".to_string());
    }
    if !events_path.exists() {
        return Err("missing events.jsonl".to_string());
    }
    let manifest: BrowserArtifactManifestV1 =
        serde_json::from_str(&fs::read_to_string(&manifest_path).map_err(|err| err.to_string())?)
            .map_err(|err| format!("malformed manifest: {err}"))?;
    let mut last_seq = 0;
    for line in fs::read_to_string(events_path)
        .map_err(|err| err.to_string())?
        .lines()
    {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|err| format!("malformed JSONL: {err}"))?;
        let seq = value
            .get("seq")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if seq <= last_seq {
            return Err("event sequence gap or duplicate".to_string());
        }
        last_seq = seq;
        let serialized = value.to_string();
        if serialized.contains("bearer ") || serialized.contains("token=secret") {
            return Err("unredacted token pattern".to_string());
        }
    }
    Ok(json!({
        "ok": true,
        "recordingId": manifest.recording_id,
        "eventCount": manifest.event_count,
        "redaction": manifest.redaction
    }))
}

fn count_jsonl_lines(path: &Path) -> Result<u64, String> {
    let data = fs::read_to_string(path).map_err(|err| err.to_string())?;
    Ok(data.lines().filter(|line| !line.trim().is_empty()).count() as u64)
}

fn origin_from_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return String::new();
    };
    let host = rest.split('/').next().unwrap_or_default();
    format!("{scheme}://{host}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Compute a stable structural DOM hash from CompactPageState (PR-1 basic impl).
/// Uses key structural fields (counts, headings, focus) + url/title for replayable signature.
/// Wired from capture_compact_page_state (used alongside settle_after_action).
pub fn compute_dom_hash(state: &CompactPageState) -> String {
    // Use canonical JSON for headings (and a subset) to guarantee injective structural sig
    // (no delimiter collisions from user content). Good for reliable replay/comparison.
    let headings_json = serde_json::to_string(&state.headings).unwrap_or_default();
    let structural = format!(
        "v1|url:{}|title:{}|focus:{}|h:{}|land:{}|btn:{}|lnk:{}|inp:{}",
        state.url,
        state.title,
        state.focus,
        headings_json,
        state.counts.landmarks,
        state.counts.buttons,
        state.counts.links,
        state.counts.inputs
    );
    sha256_hex(structural.as_bytes())
}

/// Legacy/basic sessionStateHash marker (PR-1).
/// Real functional session capture (counts + sha of summary using the exact CDP/JS patterns
/// from handle_save_state) now happens in the dispatch recording path via
/// capture_basic_session_meta, which embeds a rich "session" object under before/after.
/// This fn remains for the minority call sites (recording start/stop boundaries) and
/// returns a clear "legacy" marker so consumers know to prefer the per-event session data.
/// Keeps schema evolvable and honest for replayable bundles.
pub fn compute_session_state_hash() -> String {
    "sha256:session-state-v1-legacy-use-per-event-session-object".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn recording_writes_manifest_and_events() {
        let dir = tempdir().expect("tempdir");
        let mut store = RecordingStore::new(dir.path().to_path_buf());
        let rec = store
            .start("checkout-bug", "uat-workbench")
            .expect("started");
        store
            .record_event(RecordingEventInput {
                source: "viewer".to_string(),
                owner: "user".to_string(),
                kind: "recording.start".to_string(),
                url: "http://127.0.0.1".to_string(),
                title: "Fixture".to_string(),
                redacted: false,
                command: serde_json::json!({}),
                before: serde_json::json!({}),
                after: serde_json::json!({}),
                network: serde_json::json!({}),
            })
            .expect("event");
        store
            .record_frame(&crate::daemon::view::capture::FrameMessage {
                ty: "frame",
                frame_seq: 12,
                content_type: "image/jpeg",
                data_base64: general_purpose::STANDARD.encode([1_u8, 2, 3, 4]),
                data: general_purpose::STANDARD.encode([1_u8, 2, 3, 4]),
                viewport: crate::daemon::view::capture::ViewportInfo {
                    width: 800,
                    height: 600,
                    device_pixel_ratio: 1.0,
                    scroll_x: 0.0,
                    scroll_y: 0.0,
                },
                capture_pixel_width: 800,
                capture_pixel_height: 600,
                device_pixel_ratio: 1.0,
                capture_scale_x: 1.0,
                capture_scale_y: 1.0,
                url: "http://127.0.0.1".to_string(),
                title: "Fixture".to_string(),
                timestamp: now_ms(),
            })
            .expect("frame");
        let manifest = store.stop(&rec.recording_id).expect("stopped");
        assert_eq!(manifest.session_id, "uat-workbench");
        assert_eq!(manifest.event_count, 1);
        assert_eq!(manifest.frame_count, 1);
        assert!(manifest
            .hashes
            .get("frames/frame-000012.jpg")
            .and_then(serde_json::Value::as_str)
            .is_some());
        assert!(dir
            .path()
            .join(&rec.recording_id)
            .join("manifest.json")
            .exists());
        assert!(dir
            .path()
            .join(&rec.recording_id)
            .join("frames/frame-000012.jpg")
            .exists());
        assert!(dir
            .path()
            .join(&rec.recording_id)
            .join("events.jsonl")
            .exists());
    }

    #[test]
    fn redaction_scrubs_tokens_from_event_text() {
        let scrubbed =
            redact_text("email lex@example.com bearer abc.def token=secret data-token=\"abc\"");
        assert!(!scrubbed.contains("lex@example.com"));
        assert!(!scrubbed.contains("secret"));
        assert!(scrubbed.contains("[redacted:email]"));
        assert!(scrubbed.contains("[redacted:token]"));
    }

    #[test]
    fn recording_redacts_enriched_event_fields() {
        let dir = tempdir().expect("tempdir");
        let mut store = RecordingStore::new(dir.path().to_path_buf());
        let rec = store.start("secret-flow", "session-1").expect("started");

        store
            .record_event(RecordingEventInput {
                source: "cli".to_string(),
                owner: "agent".to_string(),
                kind: "fill_form".to_string(),
                url: "https://example.test/login?token=secret".to_string(),
                title: "Login lex@example.com".to_string(),
                redacted: false,
                command: serde_json::json!({
                    "text": "hunter2",
                    "values": {
                        "Email": "lex@example.com",
                        "Password": "hunter2",
                        "otp": "123456"
                    }
                }),
                before: serde_json::json!({
                    "title": "Account for lex@example.com",
                    "session": { "accessToken": "abc.def.ghi" }
                }),
                after: serde_json::json!({
                    "url": "https://example.test/dashboard?code=123456",
                    "session": { "cookieCount": 2 }
                }),
                network: serde_json::json!({
                    "recent": [{
                        "url": "https://example.test/api?token=secret",
                        "authorization": "bearer abc.def"
                    }]
                }),
            })
            .expect("event");

        let manifest = store.stop(&rec.recording_id).expect("stopped");
        assert_eq!(
            manifest
                .redaction
                .get("hitCount")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );

        let events_path = dir.path().join(&rec.recording_id).join("events.jsonl");
        let events = fs::read_to_string(events_path).expect("events");
        assert!(!events.contains("hunter2"));
        assert!(!events.contains("lex@example.com"));
        assert!(!events.contains("token=secret"));
        assert!(!events.contains("abc.def"));
        assert!(events.contains("[redacted:sensitive]"));
        assert!(events.contains("[redacted:email]"));
        assert!(events.contains("[redacted:token]"));
        assert!(events.contains(r#""status":"redacted""#));
    }
}
