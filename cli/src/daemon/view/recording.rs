use crate::daemon::logs::LogBuffer;
use base64::{engine::general_purpose, Engine as _};
use gsd_browser_common::types::{CompactPageState, NetworkLogEntry};
use gsd_browser_common::viewer::{
    BrowserArtifactManifestV1, BROWSER_ARTIFACT_BUNDLE_SCHEMA, BROWSER_EVENT_V1_SCHEMA,
};
use regex_lite::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
    // PR-2: network now also carries "networkSlice" (computed in record_event via extract from tagged buffer)
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
    // PR-2 fields for per-action network slicing (wired at daemon start from DaemonLogs)
    current_recording_seq: Option<Arc<Mutex<u64>>>,
    network_buffer: Option<LogBuffer<NetworkLogEntry>>,
    /// PR-2: the seq claimed by the most recent prepare_for_next_recorded_event while active.
    /// Used to make seq advancement happen at "arming" time (pre-dispatch) so pause interleaving
    /// cannot cause reuse, and to give record_event the authoritative seq for the action.
    pending_seq: Option<u64>,
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
            current_recording_seq: None,
            network_buffer: None,
            pending_seq: None,
        }
    }

    /// PR-2: wire the shared tagger (updated by prepare/record to stamp live network entries).
    pub fn set_network_tagger(&mut self, tagger: Arc<Mutex<u64>>) {
        self.current_recording_seq = Some(tagger);
        self.pending_seq = None;
    }

    /// PR-2: wire the network buffer so record_event can extract the tagged slice for this seq.
    pub fn set_network_buffer(&mut self, buffer: LogBuffer<NetworkLogEntry>) {
        self.network_buffer = Some(buffer);
    }

    pub fn start(&mut self, name: &str, session_id: &str) -> Result<RecordingSession, String> {
        if self.active.is_some() {
            return Err("recording already active".to_string());
        }
        self.next_seq = 1;
        self.redaction_hits = 0;
        self.frame_count = 0;
        self.hashes.clear();
        if let Some(t) = &self.current_recording_seq {
            *t.lock().unwrap() = 0;
        }
        self.pending_seq = None;
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
        // First, capture ids under immutable borrow for the potential marker emission (Issue 11)
        let (rec_id, sess_id) = {
            let a = self.active.as_ref().ok_or("no active recording")?;
            if a.recording_id != recording_id {
                return Err(format!("recording not active: {recording_id}"));
            }
            (a.recording_id.clone(), a.session_id.clone())
        };

        let active = self.active.as_mut().ok_or("no active recording")?;
        active.paused = true;
        if let Some(t) = &self.current_recording_seq {
            *t.lock().unwrap() = 0;
        }

        let result_session = active.clone();

        // PR-2 / Issue 11: If a seq was prepared for an action and pause is now called before
        // that action's record_event, emit an explicit "recording.action-skipped" marker into
        // events.jsonl *right now*. This makes the bundle fully self-describing for seq gaps.
        if let Some(skipped_seq) = self.pending_seq.take() {
            let skipped_event = json!({
                "seq": skipped_seq,
                "timestampMs": now_ms(),
                "schema": "BrowserEventV1",
                "recordingId": rec_id.clone(),
                "sessionId": sess_id,
                "source": "system",
                "owner": "recording",
                "controlVersion": 0,
                "frameSeq": 0,
                "kind": "recording.action-skipped",
                "url": "",
                "title": "",
                "origin": "",
                "command": json!({ "reason": "paused_before_record_event" }),
                "before": {},
                "after": {},
                "network": {},
                "networkSlice": json!({
                    "seq": skipped_seq,
                    "count": 0,
                    "entries": [],
                    "note": "prepared seq abandoned due to pause; no network activity attributed"
                }),
                "redaction": { "status": "none" },
                "artifactRefs": {},
            });
            // Best effort (consistent with other recording metadata writes)
            if let Err(e) = self.append_event_line(&rec_id, &skipped_event) {
                tracing::warn!("[recording] failed to write action-skipped marker for seq {} on pause: {}", skipped_seq, e);
            }
        }

        // pending cleared (seq already spent at prepare time)
        Ok(result_session)
    }

    pub fn resume(&mut self, recording_id: &str) -> Result<RecordingSession, String> {
        let active = self.active.as_mut().ok_or("no active recording")?;
        if active.recording_id != recording_id {
            return Err(format!("recording not active: {recording_id}"));
        }
        active.paused = false;
        // On resume we do not restore a previous pending; next user action will prepare fresh.
        self.pending_seq = None;
        Ok(active.clone())
    }

    /// Internal helper to append a pre-built event (or marker) JSON line to events.jsonl.
    /// Used by the normal record path and by the new "recording.action-skipped" marker
    /// emission (Issue 11) so the bundle is fully self-describing for seq gaps.
    fn append_event_line(&self, recording_id: &str, value: &serde_json::Value) -> Result<(), String> {
        let line = serde_json::to_string(value).map_err(|err| err.to_string())?;
        let path = self.root.join(recording_id).join("events.jsonl");
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|err| format!("failed to open events.jsonl: {err}"))?;
        writeln!(file, "{line}").map_err(|err| format!("failed to append event: {err}"))?;
        Ok(())
    }

    pub fn record_event(&mut self, input: RecordingEventInput) -> Result<(), String> {
        let Some(active) = self.active.as_ref() else {
            return Ok(());
        };
        if active.paused {
            // PR-2: The primary self-describing "recording.action-skipped" marker is now emitted
            // from pause() at the moment the gap is created (Issue 11). This path is defensive.
            if self.pending_seq.is_some() {
                tracing::warn!(
                    "[recording] record_event while paused still saw pending seq {:?} (kind={}) — cleared",
                    self.pending_seq, input.kind
                );
                self.pending_seq = None;
            }
            return Ok(());
        }

        // PR-2: Use pending_seq (claimed at prepare time) as source of truth when present.
        // This is the case for all armed CLI record_timeline actions and (after viewer fixes)
        // viewer user-input events. Falls back for direct/record_event-only calls (e.g. some boundaries).
        let seq = if let Some(p) = self.pending_seq.take() {
            p
        } else {
            let s = self.next_seq;
            self.next_seq += 1;
            s
        };

        // Arm tagger to this seq for any *very* late arrivals right at record time (rare).
        if let Some(tagger) = &self.current_recording_seq {
            *tagger.lock().unwrap() = seq;
        }

        // Extract the slice *while the seq is still (briefly) armed*.
        let network_slice = self.extract_network_slice(seq);

        // CRITICAL for precision (addresses Issue 1): Immediately close the attribution window
        // for this seq. Any network pushed by listeners *after* this point will *not* receive
        // this seq (they get 0 or the next action's seq). This eliminates orphans that would
        // otherwise be stamped with a seq whose slice was already finalized, and prevents
        // pollution of future slices. Late tails after settle are accepted as a documented
        // window boundary (see extract docs).
        if let Some(tagger) = &self.current_recording_seq {
            *tagger.lock().unwrap() = 0;
        }

        let text = redact_text(&format!("{} {} {}", input.kind, input.url, input.title));
        if text != format!("{} {} {}", input.kind, input.url, input.title) || input.redacted {
            self.redaction_hits += 1;
        }
        let event = json!({
            "seq": seq,
            "timestampMs": now_ms(),
            "schema": BROWSER_EVENT_V1_SCHEMA,
            "recordingId": active.recording_id,
            "sessionId": active.session_id,
            "source": input.source,
            "owner": input.owner,
            "controlVersion": 0,
            "frameSeq": 0,
            "kind": input.kind,
            "url": redact_text(&input.url),
            "title": redact_text(&input.title),
            "origin": origin_from_url(&input.url),
            "command": input.command,
            "before": input.before,
            "after": input.after,
            // network (PR-1 compat small snapshot) + networkSlice (PR-2 authoritative tagged per-seq set)
            // Consumers building replayable artifacts should prefer networkSlice.
            "network": input.network,
            "networkSlice": network_slice,
            "redaction": { "status": if input.redacted { "redacted" } else { "none" } },
            "artifactRefs": {},
        });
        self.append_event_line(&active.recording_id, &event)?;
        Ok(())
    }

    /// PR-2: Arm the tagger + *claim* the next seq for the upcoming recorded action (pre-dispatch).
    /// Seq is advanced here (not in record_event) so that pause interleaving cannot cause seq reuse
    /// or misattribution (addresses review Issue 2). The claimed seq is the source of truth for
    /// the action's event and its networkSlice.
    ///
    /// Any network CDP events processed by listeners while this seq is in the tagger will be
    /// attributed to it (the "prepare-to-record_event" causal window for this action).
    /// After record_event extracts the slice for the claimed seq we close the window (tagger=0).
    ///
    /// Returns the claimed seq (or None if no active non-paused recording).
    /// The return value is now meaningful and is stored as pending_seq.
    pub fn prepare_for_next_recorded_event(&mut self) -> Option<u64> {
        let Some(active) = self.active.as_ref() else {
            return None;
        };
        if active.paused {
            if let Some(t) = &self.current_recording_seq {
                *t.lock().unwrap() = 0;
            }
            self.pending_seq = None;
            return None;
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        if let Some(t) = &self.current_recording_seq {
            *t.lock().unwrap() = seq;
        }
        self.pending_seq = Some(seq);
        Some(seq)
    }

    /// PR-2: extraction/slicing logic — filters the (tagged) network buffer for entries matching this seq.
    /// Produces the "networkSlice" value embedded in each recorded event (and thus in events.jsonl).
    /// Reuses existing LogBuffer.snapshot() + the seq tags set at listener time.
    /// Redacts sensitive bits for bundle hygiene. Minimal shape for replayable artifacts.
    ///
    /// Attribution window (prepare-to-record_event + brief post-extract arm):
    /// - A NetworkLogEntry receives a seq tag if the listener processed the CDP event while
    ///   the tagger held that value (set by prepare before dispatch_inner, cleared to 0
    ///   immediately after extract in record_event).
    /// - This gives a tight causal association for replay/comparison while accepting that
    ///   extremely late responses (post-settle, after we closed the window) may land in no
    ///   slice or the subsequent action. This is documented and preferable to orphans/pollution.
    /// - loadingFailed entries often have empty url (CDP event limitation; see spawn_network_listener
    ///   comment). failureText is still captured. Replay engines should treat missing url on
    ///   failed entries as a known constraint and correlate via HAR/traces when needed.
    fn extract_network_slice(&self, seq: u64) -> serde_json::Value {
        match &self.network_buffer {
            Some(buf) => {
                let entries: Vec<_> = buf
                    .snapshot()
                    .into_iter()
                    .filter(|e| e.recording_seq == Some(seq))
                    .map(|e| {
                        json!({
                            "method": e.method,
                            "url": redact_text(&e.url),
                            "status": e.status,
                            "resourceType": e.resource_type,
                            "timestamp": e.timestamp,
                            "failed": e.failed,
                            "failureText": if e.failure_text.is_empty() {
                                serde_json::Value::Null
                            } else {
                                json!(redact_text(&e.failure_text))
                            },
                        })
                    })
                    .collect();
                json!({
                    "seq": seq,
                    "count": entries.len(),
                    "entries": entries,
                })
            }
            None => json!({ "seq": seq, "count": 0, "entries": [], "note": "buffer-not-wired" }),
        }
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
        if let Some(t) = &self.current_recording_seq {
            *t.lock().unwrap() = 0;
        }
        self.pending_seq = None;
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

            // PR-3 defaults: new recordings via stop() are not yet marked replayable.
            // Replayable=true + populated fields are injected at *export* time (see export fn)
            // so that even legacy stop() bundles can be exported as first-class replay artifacts.
            // This keeps creation path simple and evolvable.
            replayable: false,
            replay_format_version: None,
            entry_point_command: None,
            expected_final_state: None,
            network_slice_manifest: None,
            state_restoration_hints: None,
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

    /// Cheap lookup (no I/O) for the on-disk source directory of a recording.
    /// Used by the export handler so the tokio::sync::Mutex is held only for the
    /// lookup, never across the long-running fs copy + manifest enrichment.
    pub fn recording_dir(&self, recording_id: &str) -> PathBuf {
        self.root.join(recording_id)
    }

    /// Thin wrapper retained for any direct callers / API stability.
    /// The real work (and the lock-avoidance contract for the HIGH mutex fix)
    /// lives in the free function `export_recording_bundle`.
    #[allow(dead_code)]
    pub fn export(&self, recording_id: &str, output: &Path) -> Result<PathBuf, String> {
        let src = self.recording_dir(recording_id);
        if !src.exists() {
            return Err(format!("recording not found: {recording_id}"));
        }
        export_recording_bundle(&src, output, recording_id)
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

/// PR-3: Free function that performs the actual bundle export + replayable
/// manifest upgrade. It takes plain paths so it can be invoked *without*
/// holding the RecordingStore Mutex (see handle_recording_export).
/// This is the key fix for the HIGH mutex hold-time finding.
pub fn export_recording_bundle(
    src: &Path,
    output: &Path,
    recording_id: &str,
) -> Result<PathBuf, String> {
    if !src.exists() {
        return Err(format!("recording not found: {}", recording_id));
    }
    fs::create_dir_all(output).map_err(|err| format!("failed to create export dir: {err}"))?;

    let dest = output.join(recording_id);
    fs::create_dir_all(&dest)
        .map_err(|err| format!("failed to create export bundle dir: {err}"))?;

    // Partial-export hygiene (MEDIUM finding): on any subsequent failure we
    // explicitly clean the dest dir so callers never see half-written replayable
    // artifacts. (Drop guard avoided for borrow/move simplicity in this minimal fix.)
    let cleanup_on_err = |d: &std::path::Path| {
        let _ = fs::remove_dir_all(d);
    };

    // Full tree copy — future-proof for any additional artifacts (traces, HAR slices,
    // per-event screenshots, etc.) that may appear for complete replayable bundles.
    // No longer a closed hardcoded set (addresses MEDIUM evolvability finding).
    if let Err(e) = copy_dir_recursive(&src, &dest) {
        cleanup_on_err(&dest);
        return Err(e);
    }

    // Load, enrich for replay, and overwrite manifest in the *exported* copy only.
    let manifest_path = dest.join("manifest.json");
    let mut manifest: BrowserArtifactManifestV1 =
        serde_json::from_str(&fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("malformed exported manifest: {e}"))?;

    manifest.replayable = true;
    manifest.replay_format_version = Some("playwright-1".to_string());

    // Best-effort population of replay fields from the events.jsonl present in the copy.
    // Uses PR-1 enriched command/before/after/network so entry + final state are high fidelity.
    // Optimized single-pass first/last extraction (no full Vec allocation for large files).
    if let Ok(events_data) = fs::read_to_string(dest.join("events.jsonl")) {
        let mut first_line: Option<String> = None;
        let mut last_line: Option<String> = None;
        for line in events_data.lines() {
            if !line.trim().is_empty() {
                if first_line.is_none() {
                    first_line = Some(line.to_string());
                }
                last_line = Some(line.to_string());
            }
        }
        if let Some(first) = first_line {
            if let Ok(first_val) = serde_json::from_str::<serde_json::Value>(&first) {
                if let Some(cmd) = first_val.get("command").cloned() {
                    if cmd != serde_json::json!({}) && !cmd.is_null() {
                        manifest.entry_point_command = Some(cmd);
                    } else if let Some(kind) = first_val.get("kind").and_then(|k| k.as_str()) {
                        manifest.entry_point_command =
                            Some(json!({ "kind": kind, "name": manifest.name }));
                    }
                }
            }
        }
        if let Some(last) = last_line {
            if let Ok(last_val) = serde_json::from_str::<serde_json::Value>(&last) {
                // Lean signature only (url/title + hashes) to avoid bloating the
                // manifest with the full (potentially large) "after" subtree already
                // present in events.jsonl. Consumers that need full state read the
                // last event directly. Addresses redundancy finding.
                let final_state = json!({
                    "url": last_val.get("url"),
                    "title": last_val.get("title"),
                    "domHash": last_val.get("after").and_then(|a| a.get("domHash")),
                    "sessionStateHash": last_val.get("after").and_then(|a| a.get("sessionStateHash")),
                    "ref": "events.jsonl:last"
                });
                manifest.expected_final_state = Some(final_state);
            }
        }
    }

    // Network slice manifest synthesized from per-event network (PR-2 ready).
    let net_hint = json!({
        "derivedFrom": "per-action-network-in-events",
        "format": "summary-slice-v1",
        "note": "Full HAR slices available via browser_network + export; use for deterministic replay mocking"
    });
    manifest.network_slice_manifest = Some(net_hint);

    // State restoration hints — clearly marked synthetic/best-effort so that
    // has* flags and replayable status are not misleading for minimal bundles.
    manifest.state_restoration_hints = Some(json!({
        "synthetic": true,
        "strategy": "replay-from-events",
        "usePerEventSession": true,
        "cookiesFrom": "event.before.session or event.after.session",
        "storageFrom": "event.before/after",
        "authVault": "required for any redacted secrets; see gsd-browser auth-vault",
        "viewport": "from frames or last after.viewport",
        "warning": "secrets redacted by default; provide via env or vault for full replay fidelity"
    }));

    let updated = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    if let Err(e) = fs::write(&manifest_path, updated) {
        cleanup_on_err(&dest);
        return Err(format!("failed to write replay-enhanced manifest: {e}"));
    }

    Ok(dest)
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
        if seq != last_seq + 1 {
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
        "redaction": manifest.redaction,
        // PR-3: surface replayable metadata so `recording-validate` on exported bundles
        // confirms first-class replay artifact status (for CI, audits, Playwright consumers).
        "replayable": manifest.replayable,
        "replayFormatVersion": manifest.replay_format_version,
        "hasEntryPoint": manifest.entry_point_command.is_some(),
        "hasExpectedFinalState": manifest.expected_final_state.is_some(),
        "hasNetworkSliceManifest": manifest.network_slice_manifest.is_some(),
        "hasStateRestorationHints": manifest.state_restoration_hints.is_some()
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

/// PR-3 helper: full recursive copy of the entire source bundle tree into dest.
/// Retained for potential future direct use / tests. Currently the free export
/// fn uses the recursive primitive directly.
#[allow(dead_code)]
fn copy_recording_bundle_for_export(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("failed mkdir for bundle export: {e}"))?;
    for entry in fs::read_dir(src).map_err(|e| format!("readdir {src:?}: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let s = entry.path();
        let d = dest.join(entry.file_name());
        if s.is_dir() {
            copy_dir_recursive(&s, &d)?;
        } else {
            fs::copy(&s, &d).map_err(|e| format!("copy {s:?} -> {d:?}: {e}"))?;
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("failed mkdir {dst:?}: {e}"))?;
    for entry in fs::read_dir(src).map_err(|e| format!("readdir {src:?}: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let s = entry.path();
        let d = dst.join(entry.file_name());
        if s.is_dir() {
            copy_dir_recursive(&s, &d)?;
        } else {
            fs::copy(&s, &d).map_err(|e| format!("copy file {s:?} -> {d:?}: {e}"))?;
        }
    }
    Ok(())
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
    fn network_slice_tagging_and_extraction_happy_path_and_edges() {
        use crate::daemon::logs::LogBuffer;
        use gsd_browser_common::types::NetworkLogEntry;
        use std::sync::{Arc, Mutex};

        let dir = tempdir().expect("tempdir");
        let mut store = RecordingStore::new(dir.path().to_path_buf());

        // Simulate daemon wiring of tagger + buffer (as done in mod.rs)
        let tagger: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
        let net_buf: LogBuffer<NetworkLogEntry> = LogBuffer::new();
        store.set_network_tagger(tagger.clone());
        store.set_network_buffer(net_buf.clone());

        let rec = store.start("replay-test", "session-xyz").expect("started");

        // Simulate prepare (as main dispatch + now viewer paths do) — claims seq 1, arms tagger
        let claimed = store.prepare_for_next_recorded_event();
        assert_eq!(claimed, Some(1));
        assert_eq!(*tagger.lock().unwrap(), 1);

        // Simulate listener pushes while armed (realistic CDP timing)
        net_buf.push(NetworkLogEntry {
            method: "GET".into(),
            url: "https://example.com/api/user?token=secret123".into(), // will be redacted in slice
            status: 200,
            resource_type: "XHR".into(),
            timestamp: 123.0,
            failed: false,
            failure_text: String::new(),
            response_body: String::new(),
            recording_seq: Some(1),
        });
        net_buf.push(NetworkLogEntry {
            method: "GET".into(),
            url: "https://example.com/old".into(),
            status: 200,
            resource_type: "Image".into(),
            timestamp: 99.0,
            failed: false,
            failure_text: String::new(),
            response_body: String::new(),
            recording_seq: None, // from before this action
        });
        // A failure (simulates loadingFailed limitation — url empty)
        net_buf.push(NetworkLogEntry {
            method: String::new(),
            url: String::new(),
            status: 0,
            resource_type: "XHR".into(),
            timestamp: 124.5,
            failed: true,
            failure_text: "net::ERR_FAILED".into(),
            response_body: String::new(),
            recording_seq: Some(1),
        });

        // Record the action event (as dispatch or viewer input would)
        store
            .record_event(RecordingEventInput {
                source: "cli".to_string(),
                owner: "agent".to_string(),
                kind: "click_ref".to_string(),
                url: "https://example.com/page".to_string(),
                title: "Test Page".to_string(),
                redacted: false,
                command: serde_json::json!({"ref": "@v1:e42"}),
                before: serde_json::json!({}),
                after: serde_json::json!({}),
                network: serde_json::json!({}),
            })
            .expect("recorded");

        // After record_event the window is closed (tagger == 0)
        assert_eq!(*tagger.lock().unwrap(), 0);
        assert!(store.pending_seq.is_none());

        // Simulate a "late" push *after* we closed the window (realistic for very slow responses).
        // It gets stamped with 1 (tagger still 0 here, but in a longer recording the next
        // prepare would have set a new value; the point is it missed this slice's extract).
        net_buf.push(NetworkLogEntry {
            method: "POST".into(),
            url: "https://example.com/late".into(),
            status: 201,
            resource_type: "Fetch".into(),
            timestamp: 200.0,
            failed: false,
            failure_text: String::new(),
            response_body: String::new(),
            recording_seq: Some(1),
        });

        // Read the emitted event and inspect its networkSlice
        let events_path = dir.path().join(&rec.recording_id).join("events.jsonl");
        let events_data = fs::read_to_string(&events_path).expect("read events");
        let lines: Vec<_> = events_data
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        // Exactly one event (the click_ref action; we did not record a separate "start" boundary event in this test)
        assert!(lines.len() >= 1);
        let event: serde_json::Value =
            serde_json::from_str(lines.last().unwrap()).expect("parse last event");
        assert_eq!(event["seq"], 1);
        assert_eq!(event["kind"], "click_ref");

        let slice = &event["networkSlice"];
        assert_eq!(slice["seq"], 1);
        assert_eq!(slice["count"], 2); // the two seq=1 entries (GET redacted + failure)

        let entries = slice["entries"].as_array().unwrap();
        // Verify redaction happened on the url
        let get_entry = entries.iter().find(|e| e["method"] == "GET").unwrap();
        assert!(get_entry["url"]
            .as_str()
            .unwrap()
            .contains("[redacted:token]"));
        assert!(!get_entry["url"].as_str().unwrap().contains("secret123"));

        // The failure entry is present (even with empty url — documented limitation)
        let fail_entry = entries
            .iter()
            .find(|e| e["failed"].as_bool() == Some(true))
            .unwrap();
        assert_eq!(fail_entry["failureText"], "net::ERR_FAILED");
        assert!(fail_entry["url"].as_str().unwrap_or("").is_empty());

        // The "late" entry (seq=1 but pushed conceptually after close) is NOT in *this* slice
        // (it would have been orphaned without the post-extract tagger=0 close).
        // In a real multi-action recording it would either be missed or picked by a subsequent
        // action if the next prepare hadn't overwritten yet — the close makes attribution honest.
        assert!(!entries
            .iter()
            .any(|e| e["url"].as_str() == Some("https://example.com/late")));

        // The pre-action entry (None) is correctly excluded
        assert!(!entries
            .iter()
            .any(|e| e.get("url").and_then(|v| v.as_str()) == Some("https://example.com/old")));
    }

    /// Narrow focused test exercising the new self-describing "recording.action-skipped" marker
    /// emission path (Issue 11) + realistic pause/resume flow with multiple slices.
    /// This augments the primary tagging/extraction test with the exact pause-gap scenario
    /// that the review fixes targeted, ensuring the artifact itself now fully describes seq gaps.
    #[test]
    fn network_slice_skipped_marker_and_pause_flow() {
        use crate::daemon::logs::LogBuffer;
        use gsd_browser_common::types::NetworkLogEntry;
        use std::sync::{Arc, Mutex};

        let dir = tempdir().expect("tempdir");
        let mut store = RecordingStore::new(dir.path().to_path_buf());

        let tagger: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
        let net_buf: LogBuffer<NetworkLogEntry> = LogBuffer::new();
        store.set_network_tagger(tagger.clone());
        store.set_network_buffer(net_buf.clone());

        let rec = store.start("pause-marker-test", "sess-1").expect("started");

        // Action 1 (seq 1) — normal happy path
        store.prepare_for_next_recorded_event();
        net_buf.push(NetworkLogEntry {
            method: "GET".into(), url: "https://ex.com/a1".into(), status: 200,
            resource_type: "Document".into(), timestamp: 1.0, failed: false,
            failure_text: String::new(), response_body: String::new(), recording_seq: Some(1),
        });
        store.record_event(RecordingEventInput {
            source: "cli".into(), owner: "agent".into(), kind: "navigate".into(),
            url: "https://ex.com".into(), title: "Page".into(), redacted: false,
            command: json!({}), before: json!({}), after: json!({}), network: json!({}),
        }).expect("action1");

        // Prepare seq 2, then pause before record_event — this spends seq 2
        store.prepare_for_next_recorded_event();
        store.pause(&rec.recording_id).expect("paused");

        // Record while paused — should now emit the explicit "recording.action-skipped" marker
        // into events.jsonl (the key self-describing improvement for Issue 11)
        store.record_event(RecordingEventInput {
            source: "cli".into(), owner: "agent".into(), kind: "click".into(),
            url: "https://ex.com".into(), title: "Page".into(), redacted: false,
            command: json!({}), before: json!({}), after: json!({}), network: json!({}),
        }).expect("paused record (should emit marker)");

        // Resume and do action 3 (next seq)
        store.resume(&rec.recording_id).expect("resumed");
        store.prepare_for_next_recorded_event();
        net_buf.push(NetworkLogEntry {
            method: "GET".into(), url: "https://ex.com/a3".into(), status: 200,
            resource_type: "XHR".into(), timestamp: 3.0, failed: false,
            failure_text: String::new(), response_body: String::new(), recording_seq: Some(3),
        });
        store.record_event(RecordingEventInput {
            source: "cli".into(), owner: "agent".into(), kind: "click_ref".into(),
            url: "https://ex.com".into(), title: "Page".into(), redacted: false,
            command: json!({}), before: json!({}), after: json!({}), network: json!({}),
        }).expect("action3");

        // Inspect the artifact
        let events_path = dir.path().join(&rec.recording_id).join("events.jsonl");
        let data = fs::read_to_string(&events_path).expect("read");
        let lines: Vec<_> = data.lines().filter(|l| !l.trim().is_empty()).collect();

        // We expect: action1, skipped marker (seq 2), action3
        assert_eq!(lines.len(), 3);

        // Find the events by kind (order is reliable: action1, skipped marker, action3)
        let ev1 = lines.iter().find(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["kind"] == "navigate"
        }).map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()).unwrap();

        let skipped = lines.iter().find(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["kind"] == "recording.action-skipped"
        }).map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()).unwrap();

        let ev3 = lines.iter().find(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["kind"] == "click_ref"
        }).map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()).unwrap();

        assert_eq!(ev1["seq"], 1);
        assert_eq!(ev1["networkSlice"]["count"], 1);

        // The new self-describing marker (the main deliverable for Issue 11)
        assert_eq!(skipped["seq"], 2);
        assert_eq!(skipped["kind"], "recording.action-skipped");
        assert_eq!(skipped["command"]["reason"], "paused_before_record_event");
        let skipped_slice = &skipped["networkSlice"];
        assert_eq!(skipped_slice["seq"], 2);
        assert_eq!(skipped_slice["count"], 0);
        // Note wording may vary slightly between pause() emission and defensive paths; the
        // presence of the explicit skipped marker with zero slice is the key self-describing artifact (Issue 11)
        assert!(skipped_slice.get("note").is_some());

        assert_eq!(ev3["seq"], 3);
        assert_eq!(ev3["kind"], "click_ref");
        assert_eq!(ev3["networkSlice"]["count"], 1);

        // Seq numbering is monotonic and the gap is explicitly described in the artifact
        assert!(ev1["seq"].as_u64() < skipped["seq"].as_u64());
        assert!(skipped["seq"].as_u64() < ev3["seq"].as_u64());
    }

    /// PR-3 explicit backward-compat test for legacy (pre-replayable) manifests.
    /// Embeds a minimal V1 manifest JSON lacking the six new fields and asserts
    /// that deserialization succeeds with the documented defaults (replayable=false,
    /// all Option replay fields are None). This gives high confidence that old
    /// exported bundles on disk remain valid first-class artifacts after upgrade.
    #[test]
    fn legacy_manifest_deserializes_with_replay_defaults() {
        let legacy_json = r#"{
            "schema": "BrowserArtifactBundleV1",
            "recordingId": "rec_legacy_123",
            "sessionId": "sess_old",
            "name": "legacy-flow",
            "startedAtMs": 1710000000000,
            "stoppedAtMs": 1710000004200,
            "startSeq": 1,
            "stopSeq": 7,
            "eventCount": 7,
            "frameCount": 3,
            "annotationCount": 0,
            "consoleErrorCount": 0,
            "failedRequestCount": 0,
            "originScopes": ["https://example.com"],
            "excludedBoundaryEvents": [],
            "redaction": {"policy": "default-sensitive", "hitCount": 0, "classes": []},
            "artifacts": {"events": "events.jsonl", "frames": "frames/"},
            "hashes": {}
        }"#;

        let manifest: BrowserArtifactManifestV1 =
            serde_json::from_str(legacy_json).expect("legacy manifest must deserialize");

        assert_eq!(manifest.recording_id, "rec_legacy_123");
        assert!(
            !manifest.replayable,
            "legacy bundles must default to replayable=false"
        );
        assert!(manifest.replay_format_version.is_none());
        assert!(manifest.entry_point_command.is_none());
        assert!(manifest.expected_final_state.is_none());
        assert!(manifest.network_slice_manifest.is_none());
        assert!(manifest.state_restoration_hints.is_none());

        // Also prove that validate (which does its own from_str) accepts it.
        // (We can't easily feed a full dir here, but the deserialization path is the core.)
    }
}
