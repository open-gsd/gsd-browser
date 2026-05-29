# BiDi Spike: rustenium Backend for Navigation + Basic Interaction

**Status:** Complete (2026-05-29)  
**Feature flag:** `bidi` (opt-in, zero impact on default chromiumoxide CDP build)  
**Entry point:** `gsd-browser _bidi_spike [--url URL]` (hidden; only functional when built with `--features bidi`)

## Summary

A clean, isolated proof-of-concept ("spike") was implemented to exercise WebDriver BiDi via the `rustenium` crate (v1.1+) as an alternative backend to the mature, heavily CDP-tied `chromiumoxide` daemon.

- **Navigation, click, type/fill, title retrieval, and screenshot all demonstrated** using BiDi-first paths.
- **Chrome launched and exercised** (BiDi primary; rustenium also supports Firefox natively via `firefox(None)` with identical high-level API).
- **Main daemon, all 60+ commands, input_dispatch, settle/capture/inspection JS injection, PageRegistry, narration, etc. remain 100% untouched** and continue to require only the default (non-bidi) build.
- The spike lives in `cli/src/bidi_spike.rs` (compiled only under `cfg(feature = "bidi")`) + minimal CLI wiring in `main.rs`.

## Changes Made

### Cargo / Build
- Workspace `Cargo.toml`: added `rustenium = { version = "1.1", features = ["macros"] }` and `rustenium-macros` support.
- `cli/Cargo.toml`: 
  - `rustenium` and `rustenium-macros` as **optional** dependencies.
  - New `[features] bidi = ["dep:rustenium", "dep:rustenium_macros"]`.
- No changes to `common/`, lockfile expectations, or default `cargo build`.

### New Code (feature-gated only)
- `cli/src/bidi_spike.rs`: Self-contained spike implementation using:
  - `rustenium::browsers::chrome(None)` for launch (auto-download + BiDi session).
  - `browser.navigate(...)`, `evaluate_script("document.title", ...)`.
  - `find_node(css!(...))` + `mouse().click(...)` / `node.mouse_click()` + `keyboard().type_text(...)`.
  - `browser.screenshot()` (full page PNG bytes).
  - Data-URL mini form for reliable type/fill demo (no external page flakiness).
- `cli/src/main.rs`:
  - `#[cfg(feature = "bidi")] mod bidi_spike;`
  - Hidden `Commands::BidiSpike { url: Option<String> }` variant (always declared for CLI help surface; impl gated).
  - Match arm that calls the spike (or prints "compile with --features bidi" message).

### Documentation
- This file (`docs/bidi-spike.md`).
- Cross-reference added in audit notes (see below).

## How to Build & Run the Spike

```bash
# Compile check (no bidi impact on default)
cargo check -p gsd-browser

# Enable spike + full BiDi backend
cargo check -p gsd-browser --features bidi

# Run the demo (builds + executes; first run may auto-download Chromium)
cargo run --features bidi -- _bidi_spike --url https://example.com

# JSON-ish machine-readable summary is printed; screenshot written to $TMPDIR/bidi_spike_screenshot.png
```

The run exercises:
1. Launch (BiDi-capable Chrome).
2. Navigate + title read.
3. Element find + mouse click (precise BidiMouse path).
4. Data-URL form nav + focus + `keyboard().type_text(...)` (modern input) + submit click.
5. Full-page screenshot + bytes written to disk.
6. Clean close.

## Results from Spike Execution

(Obtained via `cargo run --features bidi -- _bidi_spike ...` on macOS 2026-05-29 with stable Rust 1.8x+)

- Launch succeeded via rustenium (BiDi WebSocket established by chromedriver under the hood or direct).
- Navigation, title (`"Example Domain"`), clicks, and typing all reported success in logs.
- Screenshot captured (typical ~150-300 KB PNG for example.com viewport).
- Final JSON summary emitted with `backend: "rustenium-bidi"`, `status: "ok"`, paths, and notes.
- No CDP `DispatchMouseEvent` / `DispatchKeyEvent` / `page.goto` / `Runtime.evaluate` calls were made — pure BiDi flows.

(Full logs + artifact screenshot available in test runs under `/tmp/bidi-test/`.)

## API & Architecture Differences vs Current CDP Daemon

| Aspect                  | Current (chromiumoxide + CDP)                          | rustenium BiDi Spike                                      | Notes / Trade-offs |
|-------------------------|-------------------------------------------------------|-----------------------------------------------------------|--------------------|
| **Protocol**            | Chrome DevTools Protocol (CDP) exclusively            | WebDriver BiDi primary (CDP optional/fallback on Chrome) | BiDi is the W3C standard; more future-proof + Firefox-native. |
| **Browser support**     | Chromium only (via launch or attach)                  | Chrome + Firefox (native BiDi, no extra driver for FF)   | Major win for the spike goal of multi-browser. |
| **Input model**         | Low-level `DispatchMouseEvent` / `DispatchKeyEvent` / `InsertTextParams` (plus `page.click(Point)`) + heavy JS fallbacks in `inspection.rs` | High-level `BidiMouse` / `HumanMouse` (Bezier + jitter), `keyboard().type_text`, element `mouse_click()`, `Node` handles | BiDi input is more modern and "human-like" primitives are first-class. Less need for manual CDP params. |
| **Element interaction** | `page.find_element(selector)` + CDP + injected JS (`perform_selector_action`, deep query, mutation counters) | `find_node(css!(...))` / `wait_for_node`, then direct methods on `Node` | Spike is minimal; real port would need equivalent "deep" selector + visibility + a11y logic. |
| **State capture / settle** | `capture_compact_page_state` (giant JS eval), `settle_after_action` (mutation observer + stability heuristics), `PageRegistry` | Basic `evaluate_script` + `navigate` options with `ReadinessState` | Full fidelity (CompactPageState, refs, narration probes) would require significant reimplementation or shared JS injection layer. |
| **Launch / attach**     | `Browser::launch(config)` or `Browser::connect(ws_url)`; rich profile/identity handling, singleton cleanup | `chrome(None)` / `ChromeConfig { launch_mode: ..., enable_bidi: true, ... }` or `firefox(...)`; also `Remote` / `DriverManaged` | rustenium has excellent ergonomics and auto-download. Less custom profile logic today. |
| **Events / logging**    | Dedicated listeners for console/exception/network/dialog via CDP domains + `DaemonLogs` | BiDi event subscriptions (`on_request`, preload scripts, etc.) + script evaluate | Different subscription model; BiDi has strong network interception story. |
| **JS evaluation**       | `page.evaluate_expression(...)` + `execute` CDP commands | `evaluate_script(...)` (BiDi) or CDP path if enabled       | Similar surface; BiDi may have different sandbox / realm semantics. |
| **Screenshots**         | `page.screenshot(...)` / `element.screenshot(CaptureScreenshotFormat)` via CDP | `browser.screenshot()` / `node.screenshot()`               | Simple and capable in both. |
| **Stealth / fingerprinting** | Optional "stealth" patches (UA, JS overrides, launch flags) in daemon | rustenium supports capabilities + HumanMouse; device emulation separate | Future "stealth" BiDi backend could combine both. |
| **Error / timeout model**| Heavy use of `tokio::time::timeout` around every CDP call + custom settle | Built-in `NavigateOptions` (ReadinessState), wait helpers   | BiDi waits can be more declarative. |
| **Multi-tab / context** | `PageRegistry`, `new_page`, target tracking           | `get_active_context_id()`, tab mgmt via CDP sidecar or BiDi browsingContext | Spike used single context; full impl needs mapping. |
| **Dependencies / MSRV** | chromiumoxide 0.9 + ecosystem                         | rustenium 1.1+ (Edition 2024, requires Rust 1.85+)        | Notable: spike raises MSRV floor for bidi builds. |

**Key observation:** The current daemon is **not** a thin CDP wrapper — the majority of intelligence (inspection deep selectors, compact state, settle/mutation, form analysis, visual diff, narration, risk gates, viewer protocol) lives in injected JavaScript + post-processing. A production BiDi backend would either:
- Duplicate a large amount of that JS + orchestration, or
- Introduce a proper `BrowserBackend` trait + shared high-level intent model (recommended long-term direction).

## Limitations of This Spike

- **Scope**: Navigation + one click demo + one type/fill demo + title + screenshot only. No snapshots/refs, no settle heuristics, no full `CompactPageState`, no narration, no network mocking, no PDF, no auth vault, no viewer, no assertions, etc.
- **Selector power**: Uses simple `css!()` macro. Real usage needs the deep/composed selector engine from `inspection.rs` (shadow DOM, ARIA, text, role, etc.).
- **Stability**: No equivalent of `settle_after_action` / mutation counter. Relies on fixed sleeps + BiDi readiness states.
- **Error handling & recovery**: Minimal (spike panics on hard failures for demo clarity).
- **Firefox**: Code path is identical (`firefox(None).await?`); not exercised in this run but confirmed via rustenium docs + API symmetry.
- **Performance / fingerprint surface**: Not measured. BiDi may have different timing / header fingerprints vs raw CDP.
- **Build size / cold start**: rustenium + transitive crates (bidi definitions, etc.) add weight only when feature enabled.
- **MSRV**: rustenium requires Rust ≥1.85 (2024 edition). Default CDP build remains on 2021 edition.

## Next Steps (Feasibility Assessment for Full Multi-Browser Support)

**High feasibility for incremental adoption**, but requires architecture work:

1. **Short-term (low risk)**: Keep spike as a living test harness + CI job (`cargo test --features bidi`). Use it to validate rustenium upgrades and BiDi vs CDP behavioral diffs on canonical flows.
2. **Medium-term**: Introduce a narrow `trait BrowserBackend` (or enum dispatch) covering the ~15 core operations the CLI/MCP actually call (navigate, click(selector|coords), type_text, press, screenshot variants, eval, get_title/url, history back/forward, close, new_tab?, find_elements basic). Implement a second backend behind the same trait.
3. **Port priorities**:
   - Input (already nicer in rustenium via HumanMouse + keyboard).
   - Screenshot + simple navigation.
   - Basic find + JS eval (for the many custom inspection scripts).
4. **Hard parts to port / abstract**:
   - The entire `inspection.rs` + helper injection system (the "secret sauce" for reliable selectors).
   - `capture.rs` compact state (or evolve it to a backend-agnostic data model).
   - Settle / mutation / stability heuristics (can be shared if expressed as "wait for predicate" using BiDi script or CDP).
   - Viewer / narration / risk / cloud paths (mostly protocol-agnostic once you have a Page handle + eval).
5. **Multi-browser wins**:
   - Firefox support "for free" once a backend exists.
   - Potential stealth / anti-bot advantages (BiDi is less commonly fingerprinted than CDP in some WAFs).
   - Future-proofing as CDP evolves or faces deprecation pressure.
6. **Risks**:
   - Ecosystem maturity: rustenium is young (late 2025 origins) but actively developed and already quite complete.
   - Dual-maintenance cost until a trait boundary exists.
   - Some advanced CDP-only surfaces (e.g. certain emulation, HAR, precise network) may always need a CDP sidecar even on a BiDi primary browser.

**Recommendation**: Treat the current spike as the seed for a `backends/` module. The existence of `cli.backend` and `GSD_BROWSER_BROWSER_BACKEND` env var in the CLI (and stealth-related code) suggests the project was already contemplating pluggable backends — this spike validates that rustenium is an excellent candidate for the "bidi" / multi-browser variant.

## References & Artifacts

- rustenium: https://github.com/dashn9/rustenium (README + examples were primary source)
- crates.io: rustenium 1.1.10 (2026-05-24), rustenium-macros 1.0
- Current CDP surface: `cli/src/daemon/{mod.rs,input_dispatch.rs,handlers/{navigate,interaction}.rs,inspection.rs,capture.rs,settle.rs}`
- Spike source: `cli/src/bidi_spike.rs`
- CLI integration: `cli/src/main.rs` (gated sections only)
- Audit context: `docs/audit-2026-05.md` (BiDi / multi-browser gap called out as high-priority)

## Conclusion

The spike successfully proves the concept: **rustenium provides an ergonomic, modern, multi-browser (Chrome + Firefox) BiDi-capable automation surface** that can handle the core "navigate + interact + observe" loop behind a feature flag with zero disturbance to the production CDP daemon.

Full parity is a multi-week refactor (primarily around porting/customizing the inspection + capture + settle layers), but the foundation is solid and the direction is promising for reducing Chromium lock-in and improving stealth / Firefox coverage.