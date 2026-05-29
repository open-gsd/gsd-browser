//! Handlers for multi-page management and frame switching.
//!
//! These handlers operate on the PageRegistry in DaemonState, not directly on
//! a single Page reference. This is the only handler module that accesses
//! the registry — all other handlers receive a resolved `&Page`.

use crate::daemon::state::DaemonState;
use chromiumoxide::cdp::browser_protocol::target::{
    EventTargetCreated, EventTargetDestroyed, EventTargetInfoChanged, GetTargetsParams,
};
use chromiumoxide::Browser;
use chromiumoxide::Page;
use futures::StreamExt;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// List all open pages with their id, title, url, and active status.
pub fn handle_list_pages(state: &DaemonState) -> Result<Value, String> {
    let pages = state.pages.lock().unwrap();
    let active_id = pages.active_page_id;

    let entries: Vec<Value> = pages
        .entries
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "targetId": e.target_id,
                "title": e.title,
                "url": e.url,
                "isActive": e.id == active_id,
            })
        })
        .collect();

    Ok(json!({
        "pages": entries,
        "count": entries.len(),
        "activePageId": active_id,
    }))
}

/// Switch the active page. Clears selected_frame. Re-injects helpers on the new active page.
pub async fn handle_switch_page(
    state: &DaemonState,
    params: &Value,
) -> Result<(Value, Arc<Page>), String> {
    let id = params
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "missing required parameter 'id'".to_string())?;

    // Switch active page in registry (synchronous)
    let new_page = {
        let mut pages = state.pages.lock().unwrap();
        if !pages.set_active(id) {
            return Err(format!("page id {id} not found"));
        }
        pages
            .active_page()
            .ok_or_else(|| "failed to resolve active page".to_string())?
    };

    // Clear selected frame
    {
        let mut frame = state.selected_frame.lock().unwrap();
        *frame = None;
    }

    // Immediately read fresh title/url and update registry metadata *before*
    // the (potentially slower) inject + mutation work. This eliminates the
    // window where list-pages would see the newly-active page with stale
    // cached title/url from a previous visit.
    let url = new_page.url().await.ok().flatten().unwrap_or_default();
    let title = new_page
        .evaluate("document.title")
        .await
        .ok()
        .and_then(|v| v.into_value::<String>().ok())
        .unwrap_or_default();

    {
        let mut pages = state.pages.lock().unwrap();
        pages.update_metadata(id, title.clone(), url.clone());
    }

    // Re-inject helpers and mutation counter on the newly active page.
    // These are important for subsequent ref/snapshot work but do not need
    // to gate the registry view for list-pages / observers.
    crate::daemon::helpers::inject_helpers(&new_page).await;
    crate::daemon::settle::ensure_mutation_counter(&new_page).await;

    info!("[pages] switched to page {id}: {url}");

    Ok((
        json!({
            "switched": true,
            "id": id,
            "title": title,
            "url": url,
        }),
        new_page,
    ))
}

/// Close a page by ID. Cannot close the last remaining page.
/// Falls back active to another page if the closed page was active.
pub async fn handle_close_page(state: &DaemonState, params: &Value) -> Result<Value, String> {
    let id = params
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "missing required parameter 'id'".to_string())?;

    let (removed_page, new_active_id) = {
        let mut pages = state.pages.lock().unwrap();
        let removed = pages.remove(id)?;
        let new_active = pages.active_page_id;
        (removed, new_active)
    };

    // Clear selected frame since page context changed
    {
        let mut frame = state.selected_frame.lock().unwrap();
        *frame = None;
    }

    // Close the CDP page — best-effort, don't fail if it errors
    // Arc::try_unwrap may fail if there are still references held
    match Arc::try_unwrap(removed_page) {
        Ok(page) => {
            if let Err(e) = page.close().await {
                warn!("[pages] close page {id} CDP error (non-fatal): {e}");
            }
        }
        Err(_arc) => {
            // Other references exist — just drop and let them clean up
            warn!("[pages] close page {id}: could not unwrap Arc, dropping reference");
        }
    }

    info!("[pages] closed page {id}, active now: {new_active_id}");

    Ok(json!({
        "closed": true,
        "id": id,
        "activePageId": new_active_id,
    }))
}

/// List all frames in the active page by walking window.frames recursively via JS.
pub async fn handle_list_frames(page: &Page) -> Result<Value, String> {
    let js = r#"(function() {
        var results = [];
        function walk(win, parentName, depth) {
            if (depth > 10) return;
            try {
                var name = '';
                try { name = win.name || ''; } catch(e) {}
                var url = '';
                try { url = win.location.href || ''; } catch(e) { url = '(cross-origin)'; }
                var isMain = (win === window.top);
                results.push({
                    index: results.length,
                    name: name,
                    url: url,
                    isMain: isMain,
                    parentName: parentName
                });
                for (var i = 0; i < win.frames.length; i++) {
                    try {
                        walk(win.frames[i], name || ('frame-' + results.length), depth + 1);
                    } catch(e) {}
                }
            } catch(e) {}
        }
        walk(window.top, '', 0);
        return JSON.stringify(results);
    })()"#;

    let raw = page
        .evaluate(js)
        .await
        .map_err(|e| format!("list_frames JS eval failed: {}", super::clean_cdp_error(&e)))?;

    let json_str = raw
        .into_value::<String>()
        .map_err(|e| format!("list_frames parse error: {e}"))?;

    let frames: Value =
        serde_json::from_str(&json_str).map_err(|e| format!("list_frames JSON parse: {e}"))?;

    let count = frames.as_array().map(|a| a.len()).unwrap_or(0);

    Ok(json!({
        "frames": frames,
        "count": count,
    }))
}

/// Select a frame for subsequent JS evaluations.
/// Pass name="main" or null to reset to the main frame.
pub fn handle_select_frame(state: &DaemonState, params: &Value) -> Result<Value, String> {
    let name = params.get("name").and_then(|v| v.as_str());
    let index = params.get("index").and_then(|v| v.as_u64());
    let url_pattern = params.get("urlPattern").and_then(|v| v.as_str());

    // Determine the frame identifier to store
    let frame_id = if let Some(n) = name {
        if n == "main" || n == "null" || n.is_empty() {
            None // Reset to main frame
        } else {
            Some(format!("name:{n}"))
        }
    } else if let Some(idx) = index {
        Some(format!("index:{idx}"))
    } else if let Some(pat) = url_pattern {
        Some(format!("url:{pat}"))
    } else {
        // No params = reset to main
        None
    };

    let selected = frame_id.is_some();
    let label = frame_id.clone().unwrap_or_else(|| "main".to_string());

    {
        let mut frame = state.selected_frame.lock().unwrap();
        *frame = frame_id;
    }

    debug!("[pages] selected frame: {label}");

    Ok(json!({
        "selected": selected,
        "frame": label,
    }))
}

/// Spawn a long-lived background task that subscribes to CDP Target.* events
/// on the Browser and keeps the PageRegistry in sync with dynamically created
/// tabs (window.open, target="_blank", other automation clients, etc.).
///
/// This is the fix for "JS-opened tabs not appearing in list-pages".
/// It runs for the lifetime of the daemon (independent of the live viewer).
pub async fn spawn_core_target_tracker(browser: Arc<Mutex<Browser>>, state: Arc<DaemonState>) {
    // Subscribe to creation events (required for the reported bug)
    let mut created_stream = match browser
        .lock()
        .await
        .event_listener::<EventTargetCreated>()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!("[pages] target tracker: failed to subscribe to Target.targetCreated: {e}");
            return;
        }
    };

    // Destroyed is best-effort (keeps registry clean when tabs closed externally)
    let destroyed_stream = browser
        .lock()
        .await
        .event_listener::<EventTargetDestroyed>()
        .await
        .ok();

    // InfoChanged lets us keep title/url reasonably fresh without polling
    let info_stream = browser
        .lock()
        .await
        .event_listener::<EventTargetInfoChanged>()
        .await
        .ok();

    // One-shot discovery of targets that already exist at daemon startup
    // (especially important for --cdp-url / attached mode where the user may have
    // many tabs already open). We register what we can attach to.
    if let Ok(resp) = browser
        .lock()
        .await
        .execute(GetTargetsParams::default())
        .await
    {
        for ti in resp.result.target_infos {
            if ti.r#type != "page" {
                continue;
            }
            let url = ti.url.as_str();
            if url.starts_with("chrome://") || url.starts_with("devtools://") {
                continue;
            }
            let target_id = ti.target_id.as_ref().to_string();

            if state
                .pages
                .lock()
                .unwrap()
                .find_by_target_id(&target_id)
                .is_some()
            {
                continue;
            }

            // Best-effort attach for pre-existing targets
            if let Ok(page) = browser.lock().await.get_page(ti.target_id.clone()).await {
                crate::daemon::set_default_viewport(&page).await;
                crate::daemon::helpers::inject_helpers(&page).await;
                crate::daemon::settle::ensure_mutation_counter(&page).await;

                let current_url = page
                    .url()
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| url.to_string());
                let title = page
                    .evaluate("document.title")
                    .await
                    .ok()
                    .and_then(|v| v.into_value::<String>().ok())
                    .unwrap_or_default();

                let page_arc = Arc::new(page);
                let assigned = {
                    let mut reg = state.pages.lock().unwrap();
                    if reg.find_by_target_id(&target_id).is_some() {
                        None
                    } else {
                        let id = reg.register(page_arc.clone(), title.clone(), current_url.clone());
                        // During initial discovery we register everything we can see so list-pages
                        // is complete, but we deliberately do NOT steal the active page from the
                        // one the daemon just created/attached as its primary control surface.
                        // Agents can use switch_page or the new tab will become active on future
                        // targetCreated events if desired.
                        Some(id)
                    }
                };
                if let Some(id) = assigned {
                    info!("[pages] tracker discovered pre-existing page {id}: {current_url}");
                }
            }
        }
    }

    info!("[pages] core target tracker active (listening for targetCreated / targetDestroyed / targetInfoChanged)");

    // Created listener task (main one for the bug)
    let b1 = Arc::clone(&browser);
    let s1 = Arc::clone(&state);
    tokio::spawn(async move {
        while let Some(evt) = created_stream.next().await {
            let ti = &evt.target_info;
            if ti.r#type != "page" {
                continue;
            }
            let url = ti.url.as_str();
            if url.starts_with("chrome://") || url.starts_with("devtools://") {
                continue;
            }
            let target_id = ti.target_id.as_ref().to_string();

            // Fast dedup check
            if s1
                .pages
                .lock()
                .unwrap()
                .find_by_target_id(&target_id)
                .is_some()
            {
                continue;
            }

            // Attach with bounded retry — the target is often not immediately attachable
            // the instant targetCreated fires.
            let mut page = None;
            for _ in 0..15 {
                match b1.lock().await.get_page(ti.target_id.clone()).await {
                    Ok(p) => {
                        page = Some(p);
                        break;
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(80)).await,
                }
            }
            let Some(page) = page else {
                warn!("[pages] tracker: get_page timed out for new target {target_id} ({url})");
                continue;
            };

            // Prepare the page the same way we prepare the initial page
            crate::daemon::set_default_viewport(&page).await;
            crate::daemon::helpers::inject_helpers(&page).await;
            crate::daemon::settle::ensure_mutation_counter(&page).await;

            let current_url = page
                .url()
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| url.to_string());
            let title = page
                .evaluate("document.title")
                .await
                .ok()
                .and_then(|v| v.into_value::<String>().ok())
                .unwrap_or_default();

            let page_arc = Arc::new(page);
            let assigned_id = {
                let mut reg = s1.pages.lock().unwrap();
                if let Some(existing) = reg.find_by_target_id(&target_id) {
                    reg.update_metadata(existing, title.clone(), current_url.clone());
                    existing
                } else {
                    let id = reg.register(page_arc.clone(), title.clone(), current_url.clone());
                    // Newly discovered tabs (popups, window.open, external opens) become the active context.
                    // This matches the behavior of the viewer target-follow path and is the least surprising
                    // default for agents.
                    reg.set_active(id);
                    id
                }
            };

            // Reset frame scope when the active page context changes
            *s1.selected_frame.lock().unwrap() = None;

            // Keep session manifest reasonably up to date (non-fatal)
            let _ = crate::daemon::handlers::session::sync_session_manifest(
                page_arc.as_ref(),
                &s1,
                None,
                None,
            )
            .await;

            info!(
                "[pages-tracker] auto-registered dynamic page {assigned_id}: {current_url} (target_id={target_id})"
            );
        }
    });

    // Destroyed listener (best effort hygiene)
    if let Some(mut ds) = destroyed_stream {
        let s2 = Arc::clone(&state);
        tokio::spawn(async move {
            while let Some(evt) = ds.next().await {
                let target_id = evt.target_id.as_ref().to_string();
                let removed = s2.pages.lock().unwrap().remove_by_target_id(&target_id);
                if let Some(id) = removed {
                    info!("[pages] target tracker removed closed page {id} (target {target_id})");
                    // Clear frame selection if we lost context
                    *s2.selected_frame.lock().unwrap() = None;
                }
            }
        });
    }

    // InfoChanged — keep title/url for known pages reasonably current (best-effort).
    if let Some(mut is) = info_stream {
        let s3 = Arc::clone(&state);
        tokio::spawn(async move {
            while let Some(evt) = is.next().await {
                let ti = &evt.target_info;
                if ti.r#type != "page" {
                    continue;
                }
                let target_id = ti.target_id.as_ref().to_string();
                let mut reg = s3.pages.lock().unwrap();
                if let Some(id) = reg.find_by_target_id(&target_id) {
                    reg.update_metadata(id, ti.title.clone(), ti.url.clone());
                }
            }
        });
    }
}
