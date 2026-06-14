#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn unique_temp_home(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    PathBuf::from(format!(
        "/tmp/gb-{test_name}-{}-{nanos}",
        std::process::id()
    ))
}

fn browser_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct BrowserTestEnv {
    home: PathBuf,
    session: String,
}

fn process_rows() -> Vec<(u32, String)> {
    let output = Command::new("ps").args(["-axo", "pid=,command="]).output();
    let Ok(output) = output else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let (pid, command) = trimmed.split_once(char::is_whitespace)?;
            Some((pid.parse().ok()?, command.trim_start().to_string()))
        })
        .collect()
}

fn kill_processes(pids: &[u32]) {
    if pids.is_empty() {
        return;
    }
    let pid_args: Vec<String> = pids.iter().map(|pid| pid.to_string()).collect();
    let _ = Command::new("kill").arg("-TERM").args(&pid_args).output();
    thread::sleep(Duration::from_millis(250));
    let _ = Command::new("kill").arg("-KILL").args(&pid_args).output();
}

fn cleanup_matching_browser_processes(
    session: Option<&str>,
    session_prefix: Option<&str>,
    home_prefix: Option<&str>,
) {
    let current_pid = std::process::id();
    let pids: Vec<u32> = process_rows()
        .into_iter()
        .filter_map(|(pid, command)| {
            if pid == current_pid {
                return None;
            }
            let session_match = session
                .map(|session| {
                    command.contains(&format!("--session {session}"))
                        || command.contains(&format!("--session={session}"))
                })
                .unwrap_or(false);
            let session_prefix_match = session_prefix
                .map(|session_prefix| {
                    command.contains(&format!("--session {session_prefix}"))
                        || command.contains(&format!("--session={session_prefix}"))
                })
                .unwrap_or(false);
            let home_match = home_prefix
                .map(|home_prefix| command.contains(home_prefix))
                .unwrap_or(false);
            let managed_browser = command.contains("gsd-browser _serve")
                || (command.contains("Google Chrome") && command.contains("/tmp/gb-"));
            if managed_browser && (session_match || session_prefix_match || home_match) {
                Some(pid)
            } else {
                None
            }
        })
        .collect();
    kill_processes(&pids);
}

impl BrowserTestEnv {
    fn new(name: &str) -> Self {
        cleanup_matching_browser_processes(
            None,
            Some(&format!("{name}-")),
            Some(&format!("/tmp/gb-{name}-")),
        );
        Self {
            home: unique_temp_home(name),
            session: format!("{name}-{}", std::process::id()),
        }
    }

    fn output(&self, args: &[String]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_gsd-browser"))
            .env("HOME", &self.home)
            .args(["--session", &self.session])
            .args(args)
            .output()
            .expect("run gsd-browser")
    }

    fn json(&self, args: &[String]) -> Value {
        let mut full_args = vec!["--json".to_string()];
        full_args.extend(args.iter().cloned());
        let output = self.output(&full_args);
        assert!(
            output.status.success(),
            "command failed: args={full_args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("parse JSON output")
    }

    fn stop(&self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_gsd-browser"))
            .env("HOME", &self.home)
            .args(["--session", &self.session, "daemon", "stop"])
            .output();
        cleanup_matching_browser_processes(
            Some(&self.session),
            None,
            Some(self.home.to_string_lossy().as_ref()),
        );
    }
}

impl Drop for BrowserTestEnv {
    fn drop(&mut self) {
        self.stop();
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn act_instruction(env: &BrowserTestEnv, instruction: &str) -> Value {
    env.json(&[
        "act-instruction".to_string(),
        "--min-confidence".to_string(),
        "0.1".to_string(),
        instruction.to_string(),
    ])
}

fn act_instruction_dry_run(env: &BrowserTestEnv, instruction: &str) -> Value {
    env.json(&[
        "act-instruction".to_string(),
        "--dry-run".to_string(),
        "--min-confidence".to_string(),
        "0.1".to_string(),
        instruction.to_string(),
    ])
}

#[test]
fn act_instruction_sets_native_value_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-value-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label>Date <input id="date" type="date"></label>
  <label>Time <input id="time" type="time"></label>
  <label>Color <input id="color" type="color"></label>
  <label>Number <input id="num" type="number" min="-10" max="10"></label>
  <output id="out"></output>
`;
for (const el of document.querySelectorAll('input')) {
  el.addEventListener('change', () => {
    document.querySelector('#out').textContent =
      Array.from(document.querySelectorAll('input'))
        .map(input => input.id + '=' + input.value)
        .join(';');
  });
}
"##
        .to_string(),
    ]);

    let date = act_instruction(&env, "set date to 2026-06-04");
    let time = act_instruction(&env, "set time to 3:15 pm");
    let color = act_instruction(&env, "set color to blue");
    let number = act_instruction(&env, "use the number field to select -6");

    let values = env.json(&[
        "eval".to_string(),
        "Array.from(document.querySelectorAll('input')).map(input => input.id + '=' + input.value).join(';')".to_string(),
    ]);
    assert_eq!(
        values["result"],
        "date=2026-06-04;time=15:15;color=#0000ff;num=-6"
    );

    assert_eq!(date["verification"]["status"], "observed");
    assert_eq!(time["verification"]["status"], "observed");
    assert_eq!(color["verification"]["status"], "observed");
    assert_eq!(number["verification"]["status"], "observed");
    assert_eq!(date["result"]["typed"]["kind"], "date");
    assert_eq!(time["result"]["typed"]["actual"], "15:15");
    assert_eq!(color["result"]["typed"]["actual"], "#0000ff");
    assert_eq!(number["result"]["typed"]["actual"], "-6");

    env.stop();
}

#[test]
fn act_instruction_sets_custom_typed_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-typed-value-hosts");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
function semanticValueHostClass() {
  return class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    connectedCallback() {
      this.tabIndex = 0;
      this.style.display = 'block';
      this.style.width = '180px';
      this.style.minHeight = '20px';
      this.style.border = '1px solid #555';
      this.textContent = this._value;
    }
    get value() {
      return this._value;
    }
    set value(next) {
      this._value = String(next);
      this.textContent = this._value;
      this.setAttribute('data-current-value', this._value);
    }
  };
}
customElements.define('date-box', semanticValueHostClass());
customElements.define('time-box', semanticValueHostClass());
customElements.define('color-box', semanticValueHostClass());
document.body.innerHTML = `
  <label id="start-date-label" for="start-date">Start date</label>
  <date-box id="start-date" aria-labelledby="start-date-label" data-field="start-date"></date-box>
  <label id="start-time-label" for="start-time">Start time</label>
  <time-box id="start-time" aria-labelledby="start-time-label" data-field="start-time"></time-box>
  <label id="accent-color-label" for="accent-color">Accent color</label>
  <color-box id="accent-color" aria-labelledby="accent-color-label" data-field="accent-color"></color-box>
`;
for (const el of document.querySelectorAll('date-box, time-box, color-box')) {
  el.addEventListener('input', () => {
    el.dataset.inputSeen = 'true';
  });
}
"##
        .to_string(),
    ]);

    let date = act_instruction(&env, "Fill Start date field with June 4, 2026.");
    let time = act_instruction(&env, "Set Start time to 3:15 pm.");
    let color = act_instruction(&env, "Set Accent color to blue.");
    let values = env.json(&[
        "eval".to_string(),
        "JSON.stringify({date: document.querySelector('#start-date').value, time: document.querySelector('#start-time').value, color: document.querySelector('#accent-color').value, dateInput: document.querySelector('#start-date').dataset.inputSeen === 'true', timeInput: document.querySelector('#start-time').dataset.inputSeen === 'true', colorInput: document.querySelector('#accent-color').dataset.inputSeen === 'true'})".to_string(),
    ]);

    let state: Value = serde_json::from_str(values["result"].as_str().unwrap_or("{}"))
        .expect("parse custom typed host state");
    assert_eq!(state["date"], "2026-06-04");
    assert_eq!(state["time"], "15:15");
    assert_eq!(state["color"], "#0000ff");
    assert_eq!(state["dateInput"], true);
    assert_eq!(state["timeInput"], true);
    assert_eq!(state["colorInput"], true);
    assert_eq!(date["result"]["typed"]["kind"], "date");
    assert_eq!(time["result"]["typed"]["kind"], "time");
    assert_eq!(color["result"]["typed"]["kind"], "color");
    assert_eq!(date["verification"]["status"], "observed");
    assert_eq!(time["verification"]["status"], "observed");
    assert_eq!(color["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_visible_calendar_dates_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-calendar-date-picker");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="request">
    <section id="calendar" aria-label="March 2016 calendar">
      <h2>March 2016</h2>
      <button type="button" class="day" data-date="2016-03-15">15</button>
      <button type="button" class="day" data-date="2016-03-16">16</button>
      <button type="button" class="day" data-date="2016-03-17">17</button>
      <button type="button" class="day" data-date="2016-03-18">18</button>
    </section>
    <input id="selected-date" name="selected_date" aria-label="Selected date">
    <button id="submit" type="submit">Submit</button>
  </form>
  <output id="out"></output>
`;
document.querySelector('#calendar').addEventListener('click', event => {
  const button = event.target.closest('[data-date]');
  if (!button) return;
  document.querySelector('#selected-date').value = button.dataset.date;
});
request.addEventListener('submit', event => {
  event.preventDefault();
  out.textContent = document.querySelector('#selected-date').value;
});
"##
        .to_string(),
    ]);

    let selected = act_instruction(&env, "Select 03/17/2016 as the date and hit submit.");
    let state = env.json(&[
        "eval".to_string(),
        "({value: document.querySelector('#selected-date').value, submitted: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(state["result"].as_str().unwrap_or("{}")).expect("parse date state");

    assert_eq!(state["value"], "2016-03-17");
    assert_eq!(state["submitted"], "2016-03-17");
    assert_eq!(selected["plan"]["action"], "sequence");
    assert_eq!(
        selected["plan"]["capability"]["name"],
        "date-picker-selection"
    );
    assert_eq!(selected["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_jquery_style_datepicker_day_cells_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-jquery-style-date-picker");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #ui-datepicker-div {
      position: relative;
      width: 320px;
      height: 260px;
      padding: 8px;
      border: 1px solid #999;
      background: white;
    }
    .ui-datepicker-title { font-weight: 600; margin-bottom: 8px; }
    .ui-datepicker-calendar {
      display: grid;
      grid-template-columns: repeat(7, 1fr);
      gap: 4px;
      width: 220px;
    }
    .ui-datepicker-calendar a {
      display: block;
      min-height: 22px;
      text-align: center;
      border: 1px solid #ddd;
    }
    #center-decoy {
      position: absolute;
      left: 145px;
      top: 118px;
      width: 30px;
      height: 30px;
      z-index: 2;
      background: #f8f8f8;
    }
  </style>
  <form id="request">
    <input id="datepicker" readonly aria-label="Date">
    <div id="ui-datepicker-div" class="ui-datepicker" role="dialog" aria-label="December 2016 calendar">
      <div class="ui-datepicker-title">December 2016</div>
      <div class="ui-datepicker-calendar">
        ${Array.from({ length: 31 }, (_, index) => {
          const day = index + 1;
          const id = day === 7 ? ' id="center-decoy"' : '';
          return `<a${id} href="#" data-date="2016-12-${String(day).padStart(2, '0')}">${day}</a>`;
        }).join('')}
      </div>
    </div>
    <button id="submit" type="submit">Submit</button>
  </form>
  <output id="out"></output>
`;
document.querySelector('#ui-datepicker-div').addEventListener('click', event => {
  const link = event.target.closest('[data-date]');
  if (!link) return;
  event.preventDefault();
  datepicker.value = link.dataset.date;
  document.querySelector('#ui-datepicker-div').style.display = 'none';
});
request.addEventListener('submit', event => {
  event.preventDefault();
  out.textContent = datepicker.value;
});
"##
        .to_string(),
    ]);

    let selected = act_instruction(&env, "Select 12/28/2016 as the date and hit submit.");
    let state = env.json(&[
        "eval".to_string(),
        "({value: datepicker.value, submitted: out.textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(state["result"].as_str().unwrap_or("{}")).expect("parse date state");

    assert_eq!(state["value"], "2016-12-28");
    assert_eq!(state["submitted"], "2016-12-28");
    assert_eq!(selected["plan"]["action"], "sequence");
    assert_eq!(
        selected["plan"]["capability"]["name"],
        "date-picker-selection"
    );
    assert_eq!(selected["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_opens_readonly_date_picker_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-readonly-date-picker");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="request">
    <label>Date <input id="appointment-date" name="appointment_date" readonly></label>
    <div id="calendar-root"></div>
    <button id="submit" type="submit">Submit</button>
  </form>
  <output id="out"></output>
`;
function renderCalendar() {
  calendarRoot.innerHTML = `
    <section class="calendar" aria-label="March 2016 calendar">
      <h2>March 2016</h2>
      <button type="button" class="day" data-date="2016-03-16">16</button>
      <button type="button" class="day" data-date="2016-03-17">17</button>
      <button type="button" class="day" data-date="2016-03-18">18</button>
    </section>
  `;
}
const input = document.querySelector('#appointment-date');
const calendarRoot = document.querySelector('#calendar-root');
input.addEventListener('focus', () => setTimeout(renderCalendar, 20));
input.addEventListener('click', () => setTimeout(renderCalendar, 20));
calendarRoot.addEventListener('click', event => {
  const button = event.target.closest('[data-date]');
  if (!button) return;
  input.value = button.dataset.date === '2016-03-17' ? '03/17/2016' : button.dataset.date;
  input.dataset.selected = button.dataset.date;
});
request.addEventListener('submit', event => {
  event.preventDefault();
  out.textContent = input.value + '|' + (input.dataset.selected || '');
});
"##
        .to_string(),
    ]);

    let selected = act_instruction(&env, "Select 03/17/2016 as the date and hit submit.");
    let state = env.json(&[
        "eval".to_string(),
        "({value: document.querySelector('#appointment-date').value, selected: document.querySelector('#appointment-date').dataset.selected || '', submitted: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(state["result"].as_str().unwrap_or("{}")).expect("parse date state");

    assert_eq!(state["value"], "03/17/2016");
    assert_eq!(state["selected"], "2016-03-17");
    assert_eq!(state["submitted"], "03/17/2016|2016-03-17");
    assert_eq!(selected["plan"]["action"], "sequence");
    assert_eq!(selected["plan"]["steps"][0]["action"], "date_picker");
    assert_eq!(
        selected["plan"]["capability"]["name"],
        "date-picker-selection"
    );

    env.stop();
}

#[test]
fn act_instruction_uses_table_row_and_column_relationships_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-table-cell-lookup");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="lookup">
    <table id="accounts">
      <thead>
        <tr><th>Account</th><th>Status</th><th>Owner</th></tr>
      </thead>
      <tbody>
        <tr><th>Atlas</th><td>Approved</td><td>Riley</td></tr>
        <tr><th>Beacon</th><td>Review</td><td>Morgan</td></tr>
      </tbody>
    </table>
    <label>Answer <input id="answer" name="answer"></label>
    <button id="submit" type="submit">Submit</button>
  </form>
  <output id="out"></output>
`;
lookup.addEventListener('submit', event => {
  event.preventDefault();
  out.textContent = answer.value;
});
"##
        .to_string(),
    ]);

    let lookup = act_instruction(
        &env,
        "Find the value in row Atlas and column Status, enter it as the answer, then press Submit.",
    );
    let state = env.json(&[
        "eval".to_string(),
        "({answer: answer.value, submitted: out.textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(state["result"].as_str().unwrap_or("{}")).expect("parse table state");

    assert_eq!(state["answer"], "Approved");
    assert_eq!(state["submitted"], "Approved");
    assert_eq!(lookup["plan"]["action"], "sequence");
    assert_eq!(lookup["plan"]["capability"]["name"], "table-cell-lookup");
    assert_eq!(lookup["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_reads_key_value_tables_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-key-value-table-lookup");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="profile">
    <table id="details">
      <tbody>
        <tr><td>Account ID</td><td>A-204</td></tr>
        <tr><td>Region</td><td>Northwest</td></tr>
        <tr><td>Status</td><td>Active</td></tr>
      </tbody>
    </table>
    <input id="response" aria-label="Response text field">
    <button id="submit" type="submit">Submit</button>
  </form>
  <output id="out"></output>
`;
profile.addEventListener('submit', event => {
  event.preventDefault();
  out.textContent = response.value;
});
"##
        .to_string(),
    ]);

    let lookup = act_instruction(
        &env,
        "Enter the value of Region into the text field and press Submit.",
    );
    let state = env.json(&[
        "eval".to_string(),
        "({answer: response.value, submitted: out.textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse key value table state");

    assert_eq!(state["answer"], "Northwest");
    assert_eq!(state["submitted"], "Northwest");
    assert_eq!(lookup["plan"]["action"], "sequence");
    assert_eq!(lookup["plan"]["capability"]["name"], "table-cell-lookup");
    assert_eq!(lookup["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_dynamic_ranked_form_workflows_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-dynamic-ranked-form-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="request">
    <label>Origin <input id="origin" name="origin" autocomplete="off"></label>
    <label>Destination <input id="destination" name="destination" autocomplete="off"></label>
    <label>Service date <input id="service-date" name="service_date" type="date"></label>
    <button id="search" type="submit">Search</button>
  </form>
  <section id="results" aria-label="Results"></section>
  <output id="out"></output>
`;
request.addEventListener('submit', event => {
  event.preventDefault();
  results.innerHTML = `
    <article class="result"><span>Morning option</span><span>$420</span><span>2h 10m</span><button data-choice="morning">Select</button></article>
    <article class="result"><span>Evening option</span><span>$180</span><span>3h 25m</span><button data-choice="evening">Select</button></article>
  `;
});
results.addEventListener('click', event => {
  const button = event.target.closest('button[data-choice]');
  if (!button) return;
  out.textContent = JSON.stringify({
    choice: button.dataset.choice,
    origin: document.querySelector('#origin').value,
    destination: document.querySelector('#destination').value,
    date: document.querySelector('#service-date').value
  });
});
"##
        .to_string(),
    ]);

    let workflow = act_instruction(
        &env,
        "I need the cheapest service option between North Depot and South Hub for July 9, 2026.",
    );
    let state = env.json(&["eval".to_string(), "out.textContent".to_string()]);
    let state: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse request state");

    assert_eq!(state["choice"], "evening");
    assert_eq!(state["origin"], "North Depot");
    assert_eq!(state["destination"], "South Hub");
    assert_eq!(state["date"], "2026-07-09");
    assert_eq!(workflow["plan"]["action"], "form_workflow");
    assert_eq!(
        workflow["plan"]["capability"]["name"],
        "form-result-workflow"
    );
    assert_eq!(workflow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_matches_form_fields_by_referenced_accessible_names_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-referenced-accessible-form-fields");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <span id="origin-name" hidden>Origin</span>
  <span id="destination-name" hidden>Destination</span>
  <form id="request">
    <div id="f1" role="searchbox" aria-labelledby="origin-name" tabindex="0" style="min-height: 20px; width: 200px; border: 1px solid #888;"></div>
    <div id="f2" role="textbox" aria-labelledby="destination-name" tabindex="0" style="min-height: 20px; width: 200px; border: 1px solid #888;"></div>
    <button id="search" type="button">Search</button>
  </form>
  <output id="out"></output>
`;
document.querySelector('#search').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    origin: document.querySelector('#f1').textContent,
    destination: document.querySelector('#f2').textContent
  });
});
"##
        .to_string(),
    ]);

    let workflow = act_instruction(&env, "Find a service request from Alpha Site to Beta Site.");
    let state = env.json(&["eval".to_string(), "out.textContent".to_string()]);
    let state: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse request state");

    assert_eq!(state["origin"], "Alpha Site");
    assert_eq!(state["destination"], "Beta Site");
    assert_eq!(workflow["plan"]["action"], "form_workflow");
    assert_eq!(workflow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_form_workflows_with_custom_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-form-workflow-custom-value-hosts");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="request">
    <form-field-box id="origin" aria-label="Origin" data-field="origin" tabindex="0" style="display:block; min-height: 24px; width: 220px; border: 1px solid #888;"></form-field-box>
    <form-field-box id="destination" aria-label="Destination" data-field="destination" tabindex="0" style="display:block; min-height: 24px; width: 220px; border: 1px solid #888;"></form-field-box>
    <button id="search" type="button">Search</button>
  </form>
  <output id="out"></output>
`;
if (!customElements.get('form-field-box')) {
  customElements.define('form-field-box', class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    get value() {
      return this._value || '';
    }
    set value(next) {
      this._value = String(next);
      this.textContent = this._value;
      this.setAttribute('data-current-value', this._value);
    }
  });
}
document.querySelector('#search').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    origin: document.querySelector('#origin').value,
    destination: document.querySelector('#destination').value
  });
});
"##
        .to_string(),
    ]);

    let workflow = act_instruction(&env, "Find a service request from Alpha Site to Beta Site.");
    let state = env.json(&["eval".to_string(), "out.textContent".to_string()]);
    let state: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse custom form workflow state");

    assert_eq!(state["origin"], "Alpha Site");
    assert_eq!(state["destination"], "Beta Site");
    assert_eq!(workflow["plan"]["action"], "form_workflow");
    assert_eq!(workflow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_commits_custom_value_host_form_suggestions_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-form-workflow-custom-host-suggestions");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="search-form">
    <label for="query">Origin</label>
    <query-combobox
      id="query"
      role="combobox"
      aria-label="Origin"
      aria-autocomplete="list"
      aria-controls="query-options"
      data-field="origin"
      tabindex="0"
      style="display:block; min-height: 24px; width: 220px; border: 1px solid #888;"></query-combobox>
    <ul id="query-options" role="listbox" hidden></ul>
    <query-combobox
      id="destination"
      aria-label="Destination"
      data-field="destination"
      tabindex="0"
      style="display:block; min-height: 24px; width: 220px; border: 1px solid #888;"></query-combobox>
    <button id="search" type="button">Search</button>
  </form>
  <output id="out"></output>
`;
if (!customElements.get('query-combobox')) {
  customElements.define('query-combobox', class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    get value() {
      return this._value || '';
    }
    set value(next) {
      this._value = String(next);
      this.textContent = this._value;
      this.setAttribute('data-current-value', this._value);
    }
  });
}
const query = document.querySelector('#query');
const options = document.querySelector('#query-options');
const records = ['Comoros City', 'Colombo Harbor', 'Copenhagen Hub'];
query.addEventListener('input', () => {
  options.innerHTML = '';
  const prefix = query.value.toLowerCase();
  records
    .filter(record => record.toLowerCase().startsWith(prefix))
    .forEach(record => {
      const option = document.createElement('li');
      option.setAttribute('role', 'option');
      option.textContent = record;
      option.addEventListener('click', () => {
        query.value = record;
        query.dispatchEvent(new Event('change', { bubbles: true }));
        options.hidden = true;
      });
      options.appendChild(option);
    });
  options.hidden = options.children.length === 0;
});
document.querySelector('#search').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    origin: query.value,
    destination: document.querySelector('#destination').value
  });
});
"##
        .to_string(),
    ]);

    let workflow = act_instruction(
        &env,
        "Find a service request from Comoros City to Beta Site.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["origin"], "Comoros City");
    assert_eq!(state["destination"], "Beta Site");
    assert_eq!(workflow["plan"]["action"], "form_workflow");
    assert_eq!(
        workflow["result"]["formWorkflow"]["filled"][0]["selector"],
        "#query"
    );
    assert_eq!(workflow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_skips_readonly_duplicate_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-readonly-duplicate-fields");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="profile">
    <label>Contact <input id="locked-contact" readonly value="archived"></label>
    <div id="locked-custom" role="textbox" aria-label="Contact" aria-readonly="true" tabindex="0" style="height: 24px; width: 200px;">legacy</div>
    <input id="contact" aria-label="Contact">
    <button id="save" type="button">Save</button>
  </form>
  <output id="out"></output>
`;
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    readonlyValue: document.querySelector('#locked-contact').value,
    customText: document.querySelector('#locked-custom').textContent,
    contact: document.querySelector('#contact').value
  });
});
"##
        .to_string(),
    ]);

    let workflow = act_instruction(&env, "Set Contact to Ada Lovelace and click Save.");
    let state = env.json(&["eval".to_string(), "out.textContent".to_string()]);
    let state: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse readonly duplicate state");

    assert_eq!(state["readonlyValue"], "archived");
    assert_eq!(state["customText"], "legacy");
    assert_eq!(state["contact"], "Ada Lovelace");
    assert_eq!(workflow["analysis"]["kind"], "fill");
    assert_eq!(workflow["analysis"]["targetHint"], "Contact");
    assert_eq!(workflow["analysis"]["value"], "Ada Lovelace");
    assert_eq!(workflow["plan"]["action"], "sequence");
    assert_eq!(workflow["plan"]["steps"][0]["action"], "type");
    assert_eq!(
        workflow["plan"]["steps"][0]["params"]["selector"],
        "#contact"
    );
    assert_eq!(workflow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_skips_disabled_ancestor_duplicate_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-disabled-ancestor-duplicate-fields");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="profile">
    <fieldset disabled>
      <label>Contact <input id="fieldset-contact" value="fieldset"></label>
    </fieldset>
    <section aria-disabled="true">
      <div id="custom-contact" role="textbox" aria-label="Contact" tabindex="0" style="height: 24px; width: 200px;">custom</div>
    </section>
    <input id="contact" aria-label="Contact">
    <button id="save" type="button">Save</button>
  </form>
  <output id="out"></output>
`;
document.querySelector('#custom-contact').addEventListener('input', () => {
  document.querySelector('#custom-contact').dataset.edited = 'true';
});
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    fieldsetValue: document.querySelector('#fieldset-contact').value,
    customText: document.querySelector('#custom-contact').textContent,
    customEdited: document.querySelector('#custom-contact').dataset.edited === 'true',
    contact: document.querySelector('#contact').value
  });
});
"##
        .to_string(),
    ]);

    let workflow = act_instruction(&env, "Set Contact to Grace Hopper and click Save.");
    let state = env.json(&["eval".to_string(), "out.textContent".to_string()]);
    let state: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse disabled ancestor duplicate state");

    assert_eq!(state["fieldsetValue"], "fieldset");
    assert_eq!(state["customText"], "custom");
    assert_eq!(state["customEdited"], false);
    assert_eq!(state["contact"], "Grace Hopper");
    assert_eq!(workflow["analysis"]["kind"], "fill");
    assert_eq!(workflow["analysis"]["targetHint"], "Contact");
    assert_eq!(workflow["analysis"]["value"], "Grace Hopper");
    assert_eq!(workflow["plan"]["action"], "sequence");
    assert_eq!(workflow["plan"]["steps"][0]["action"], "type");
    assert_eq!(
        workflow["plan"]["steps"][0]["params"]["selector"],
        "#contact"
    );
    assert_eq!(workflow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_treats_first_legend_controls_as_available_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-fieldset-first-legend-control");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="profile">
    <fieldset disabled>
      <legend><label>Contact <input id="legend-contact"></label></legend>
      <label>Contact <input id="disabled-contact" value="disabled"></label>
    </fieldset>
    <button id="save" type="button">Save</button>
  </form>
  <output id="out"></output>
`;
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    legendContact: document.querySelector('#legend-contact').value,
    disabledContact: document.querySelector('#disabled-contact').value
  });
});
"##
        .to_string(),
    ]);

    let workflow = act_instruction(&env, "Set Contact to Katherine Johnson and click Save.");
    let state = env.json(&["eval".to_string(), "out.textContent".to_string()]);
    let state: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse first legend fieldset state");

    assert_eq!(state["legendContact"], "Katherine Johnson");
    assert_eq!(state["disabledContact"], "disabled");
    assert_eq!(workflow["analysis"]["kind"], "fill");
    assert_eq!(workflow["plan"]["action"], "sequence");
    assert_eq!(
        workflow["plan"]["steps"][0]["params"]["selector"],
        "#legend-contact"
    );
    assert_eq!(workflow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_commits_typeahead_result_workflows_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-typeahead-result-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="request">
    <input id="origin" placeholder="From:">
    <input id="destination" placeholder="To:">
    <div class="date-row">
      <div class="date-heading">Departure Date</div>
      <input id="depart-date" readonly>
    </div>
    <button id="search">Search</button>
    <section id="results"></section>
  </section>
  <output id="out"></output>
`;
const suggestions = {
  origin: ['North Depot (ND-1)'],
  destination: ['South Hub (SH-2)']
};
const originInput = document.querySelector('#origin');
const destinationInput = document.querySelector('#destination');
const dateInput = document.querySelector('#depart-date');
function showSuggestion(input, values) {
  document.querySelectorAll('.ui-autocomplete').forEach(el => el.remove());
  const menu = document.createElement('ul');
  menu.className = 'ui-autocomplete';
  for (const value of values) {
    const item = document.createElement('li');
    item.className = 'ui-menu-item';
    item.textContent = value;
    item.addEventListener('click', () => {
      input.value = value;
      menu.remove();
    });
    menu.appendChild(item);
  }
  document.body.appendChild(menu);
}
originInput.addEventListener('input', () => showSuggestion(originInput, suggestions.origin));
destinationInput.addEventListener('input', () => showSuggestion(destinationInput, suggestions.destination));
search.addEventListener('click', () => {
  results.innerHTML = '';
  if (originInput.value !== 'North Depot (ND-1)' || destinationInput.value !== 'South Hub (SH-2)' || dateInput.value !== '07/09/2026') {
    out.textContent = 'invalid:' + JSON.stringify({ origin: originInput.value, destination: destinationInput.value, date: dateInput.value });
    return;
  }
  for (const option of [
    { id: 'morning', duration: '3h 20m', price: '$450' },
    { id: 'midday', duration: '1h 15m', price: '$530' },
    { id: 'evening', duration: '2h 10m', price: '$300' }
  ]) {
    const row = document.createElement('article');
    row.className = 'result';
    row.innerHTML = `<div>Duration: ${option.duration}</div><div>Price: ${option.price}</div><button>Select ${option.id}</button>`;
    row.querySelector('button').addEventListener('click', () => out.textContent = option.id);
    results.appendChild(row);
  }
});
"##
        .to_string(),
    ]);

    let workflow = act_instruction(
        &env,
        "Reserve the shortest service option from: North Depot to: South Hub on 07/09/2026.",
    );
    let state = env.json(&["eval".to_string(), "out.textContent".to_string()]);

    assert_eq!(state["result"], "midday");
    assert_eq!(workflow["plan"]["action"], "form_workflow");
    assert_eq!(
        workflow["plan"]["capability"]["name"],
        "form-result-workflow"
    );
    assert_eq!(workflow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scheduled_entity_form_workflows_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scheduled-entity-form-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="event-form">
    <label>Event name <input id="event-name" name="event_name"></label>
    <label>Duration <input id="duration" name="duration"></label>
    <label>Start time <input id="start-time" name="start_time"></label>
    <label>End time <input id="end-time" name="end_time"></label>
    <button id="save" type="submit">Save</button>
  </form>
  <output id="out"></output>
`;
document.querySelector('#event-form').addEventListener('submit', event => {
  event.preventDefault();
  out.textContent = JSON.stringify({
    name: document.querySelector('#event-name').value,
    duration: duration.value,
    start: document.querySelector('#start-time').value,
    end: document.querySelector('#end-time').value
  });
});
"##
        .to_string(),
    ]);

    let workflow = act_instruction(
        &env,
        "Create a 90 mins event named \"Gym\", between 12PM and 4PM.",
    );
    let state = env.json(&["eval".to_string(), "out.textContent".to_string()]);
    let state: Value =
        serde_json::from_str(state["result"].as_str().unwrap_or("{}")).expect("parse event state");

    assert_eq!(state["name"], "Gym");
    assert_eq!(state["duration"], "90 mins");
    assert_eq!(state["start"], "12PM");
    assert_eq!(state["end"], "4PM");
    assert_eq!(workflow["plan"]["action"], "form_workflow");
    assert_eq!(
        workflow["plan"]["capability"]["name"],
        "form-result-workflow"
    );
    assert_eq!(workflow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_creates_scheduled_blocks_on_timeline_grids_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-event-timeline-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="calendar" aria-label="Daily calendar">
    <div class="time-slot" data-index="24" data-time="12PM">12PM</div>
    <div class="time-slot" data-index="25" data-time="12:30PM">12:30PM</div>
    <div class="time-slot" data-index="26" data-time="1PM">1PM</div>
    <div class="time-slot" data-index="27" data-time="1:30PM">1:30PM</div>
    <div class="time-slot" data-index="28" data-time="2PM">2PM</div>
    <div class="time-slot" data-index="29" data-time="2:30PM">2:30PM</div>
    <div class="time-slot" data-index="30" data-time="3PM">3PM</div>
    <div class="time-slot" data-index="31" data-time="3:30PM">3:30PM</div>
  </section>
  <output id="out"></output>
`;
const calendar = document.querySelector('#calendar');
calendar.style.cssText = 'width: 180px;';
for (const slot of document.querySelectorAll('.time-slot')) {
  slot.style.cssText = 'height: 24px; border: 1px solid #999;';
}
let startIndex = null;
let endIndex = null;
for (const slot of document.querySelectorAll('.time-slot')) {
  slot.addEventListener('mousedown', event => {
    startIndex = Number(event.currentTarget.dataset.index);
  });
  slot.addEventListener('mousemove', event => {
    if (startIndex == null) return;
    endIndex = Number(event.currentTarget.dataset.index);
    let draft = document.querySelector('#newEvent');
    if (!draft) {
      draft = document.createElement('div');
      draft.id = 'newEvent';
      draft.textContent = 'New event';
      draft.style.cssText = 'height: 40px; width: 120px; border: 1px solid black;';
      calendar.appendChild(draft);
      draft.addEventListener('mouseup', () => {
        const form = document.createElement('div');
        form.id = 'create-event';
        form.innerHTML = `
          <label>Event name <input id="event-name" placeholder="Event name"></label>
          <button id="create">Create</button>
        `;
        calendar.appendChild(form);
        document.querySelector('#create').addEventListener('click', () => {
          out.textContent = JSON.stringify({
            name: document.querySelector('#event-name').value,
            start: startIndex,
            end: endIndex + 1,
            duration: endIndex + 1 - startIndex
          });
        });
      });
    }
  });
}
"##
        .to_string(),
    ]);

    let workflow = act_instruction(
        &env,
        "Create a 90 mins event named \"Gym\", between 12PM and 4PM.",
    );
    let state = env.json(&["eval".to_string(), "out.textContent".to_string()]);
    let state: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse timeline event state");

    assert_eq!(state["name"], "Gym");
    assert_eq!(state["start"], 24);
    assert_eq!(state["end"], 27);
    assert_eq!(state["duration"], 3);
    assert_eq!(workflow["plan"]["action"], "form_workflow");
    assert_eq!(
        workflow["plan"]["capability"]["name"],
        "form-result-workflow"
    );
    assert_eq!(workflow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_labeled_multi_field_forms_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-multi-field-form");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="profile">
    <label for="first">First name</label>
    <input id="first" name="first_name" autocomplete="given-name">
    <label for="last">Last name</label>
    <input id="last" name="last_name" autocomplete="family-name">
    <label for="email">Email address</label>
    <input id="email" name="email" type="email">
    <label for="state">State</label>
    <select id="state" name="state">
      <option>Oregon</option>
      <option>California</option>
      <option>Ohio</option>
    </select>
    <button id="submit" type="submit">Submit</button>
  </form>
  <output id="out"></output>
`;
profile.addEventListener('submit', event => {
  event.preventDefault();
  out.textContent = JSON.stringify({
    first: first.value,
    last: last.value,
    email: email.value,
    state: state.value
  });
});
"##
        .to_string(),
    ]);

    let form = act_instruction(
        &env,
        "Fill First name with Ada, Last name with Lovelace, Email address with ada@example.com, State with California, then press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let values: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse form state");
    assert_eq!(values["first"], "Ada");
    assert_eq!(values["last"], "Lovelace");
    assert_eq!(values["email"], "ada@example.com");
    assert_eq!(values["state"], "California");
    assert_eq!(form["plan"]["action"], "sequence");
    assert_eq!(form["plan"]["capability"]["name"], "multi-field-form-fill");
    assert_eq!(form["plan"]["capability"]["category"], "form_control");
    assert_eq!(
        form["plan"]["capability"]["expectedEffect"],
        "form_fields_filled"
    );
    assert_eq!(form["plan"]["steps"][0]["action"], "type");
    assert_eq!(form["plan"]["steps"][3]["action"], "select_option");
    assert!(form["verification"]["signals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|signal| signal["kind"] == "planned_capability"
            && signal["name"] == "multi-field-form-fill"
            && signal["expectedEffect"] == "form_fields_filled"));
    assert_eq!(
        form["verification"]["effect"]["expectedEffect"],
        "form_fields_filled"
    );
    assert_eq!(form["verification"]["effect"]["observed"], true);
    assert_eq!(form["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_multiple_quoted_values_into_visible_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-multi-quoted-fields");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="login">
    <label for="locked-user">Username</label>
    <input id="locked-user" name="username_archived" type="text" readonly value="archived-user">
    <label for="user">Username</label>
    <input id="user" name="username" type="text">
    <fieldset disabled>
      <label for="disabled-secret">Password</label>
      <input id="disabled-secret" name="password_disabled" type="password" value="disabled-secret">
    </fieldset>
    <label for="secret">Password</label>
    <input id="secret" name="password" type="password">
    <button id="submit" type="submit">Login</button>
  </form>
  <output id="out"></output>
`;
login.addEventListener('submit', event => {
  event.preventDefault();
  out.textContent = JSON.stringify({
    lockedUser: document.querySelector('#locked-user').value,
    user: user.value,
    disabledSecret: document.querySelector('#disabled-secret').value,
    password: secret.value
  });
});
"##
        .to_string(),
    ]);

    let login = act_instruction(
        &env,
        "Enter the username \"cierra\" and the password \"11L\" into the text fields and press login.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let values: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse login state");

    assert_eq!(values["lockedUser"], "archived-user");
    assert_eq!(values["user"], "cierra");
    assert_eq!(values["disabledSecret"], "disabled-secret");
    assert_eq!(values["password"], "11L");
    assert_eq!(login["plan"]["action"], "sequence");
    assert_eq!(login["plan"]["steps"][0]["candidate"]["selector"], "#user");
    assert_eq!(
        login["plan"]["steps"][1]["candidate"]["selector"],
        "#secret"
    );
    assert_eq!(
        login["plan"]["steps"][2]["candidate"]["selector"],
        "#submit"
    );
    assert_eq!(login["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_search_filter_forms_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-search-filter-form");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="filters">
    <label for="genre">Genre</label>
    <input id="genre" name="genre">
    <label for="director">Director</label>
    <input id="director" name="director">
    <label for="year">Year</label>
    <input id="year" name="year">
    <div id="search" class="ui-submit" role="button" tabindex="0" style="cursor:pointer">Search</div>
  </form>
  <output id="out"></output>
`;
search.addEventListener('click', () => {
  out.textContent = JSON.stringify({
    genre: genre.value,
    director: director.value,
    year: year.value
  });
});
"##
        .to_string(),
    ]);

    let search = act_instruction(
        &env,
        "Search for crime movies directed by Holcomb from year 2010.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let values: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse search filter state");

    assert_eq!(values["genre"], "crime");
    assert_eq!(values["director"], "Holcomb");
    assert_eq!(values["year"], "2010");
    assert_eq!(search["plan"]["action"], "form_workflow");
    assert_eq!(
        search["plan"]["evidence"]["mode"],
        "faceted-query-form-workflow"
    );
    assert_eq!(search["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_shadow_dom_range_sliders_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-range-slider");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <volume-control></volume-control>
  <output id="out"></output>
`;
customElements.define('volume-control', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <label for="volume">Volume slider</label>
      <input id="volume" type="range" min="0" max="100" value="0">
    `;
    root.querySelector('#volume').addEventListener('change', event => {
      document.querySelector('#out').textContent = event.target.value;
    });
  }
});
"##
        .to_string(),
    ]);

    let slider = act_instruction(&env, "Use 72 with the slider.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "72");
    assert_eq!(slider["plan"]["action"], "set_slider");
    assert_eq!(slider["result"]["slider"]["mode"], "native-range");
    assert_eq!(slider["result"]["slider"]["value"], 72);
    assert_eq!(slider["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_shadow_dom_slider_then_clicks_nearby_submit_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-slider-submit");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="decoy-submit">Submit</button>
  <volume-control></volume-control>
  <output id="out"></output>
`;
document.querySelector('#decoy-submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'wrong-submit';
});
customElements.define('volume-control', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <label for="volume">Volume slider</label>
      <input id="volume" type="range" min="0" max="100" value="0">
      <button id="apply-volume">Submit</button>
    `;
    root.querySelector('#apply-volume').addEventListener('click', () => {
      document.querySelector('#out').textContent = root.querySelector('#volume').value;
    });
  }
});
"##
        .to_string(),
    ]);

    let slider = act_instruction(&env, "Use 72 with the slider and hit Submit.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "72");
    assert_eq!(slider["plan"]["action"], "sequence");
    assert_eq!(slider["plan"]["steps"][0]["action"], "set_slider");
    assert_eq!(slider["plan"]["steps"][1]["action"], "click");
    assert_eq!(
        slider["plan"]["steps"][1]["candidate"]["selector"],
        "#apply-volume"
    );
    assert_eq!(slider["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_custom_aria_slider_values_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-aria-slider");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="rating" role="slider" aria-label="Rating" aria-valuemin="1" aria-valuemax="10" aria-valuenow="1" tabindex="0" style="width: 180px; height: 20px;">1</div>
  <button id="apply">Apply</button>
  <output id="out"></output>
`;
const rating = document.querySelector('#rating');
rating.addEventListener('input', () => {
  rating.dataset.inputSeen = 'true';
});
document.querySelector('#apply').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    value: rating.getAttribute('aria-valuenow'),
    valueText: rating.getAttribute('aria-valuetext'),
    text: rating.textContent,
    inputSeen: rating.dataset.inputSeen === 'true'
  });
});
"##
        .to_string(),
    ]);

    let slider = act_instruction(&env, "Set the Rating to 6 and click Apply.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse slider state");
    assert_eq!(state["value"], "6");
    assert_eq!(state["valueText"], "6");
    assert_eq!(state["text"], "6");
    assert_eq!(state["inputSeen"], true);
    assert!(matches!(
        slider["plan"]["action"].as_str(),
        Some("sequence") | Some("form_workflow") | Some("set_slider") | Some("type")
    ));
    assert_eq!(slider["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_custom_value_host_sliders_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-value-slider");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
customElements.define('volume-slider', class extends HTMLElement {
  constructor() {
    super();
    this._value = 0;
  }
  connectedCallback() {
    this.tabIndex = 0;
    this.style.display = 'block';
    this.style.width = '220px';
    this.style.height = '24px';
    this.style.border = '1px solid #555';
    this.textContent = String(this._value);
  }
  get value() {
    return String(this._value);
  }
  set value(next) {
    this._value = Number(next);
    this.textContent = String(this._value);
    this.setAttribute('data-current-value', String(this._value));
  }
});
document.body.innerHTML = `
  <label id="volume-label" for="volume">Volume slider</label>
  <volume-slider id="volume" aria-labelledby="volume-label" data-field="volume-slider" min="0" max="100"></volume-slider>
  <button id="apply">Apply</button>
  <output id="out"></output>
`;
const volume = document.querySelector('#volume');
volume.addEventListener('input', () => {
  volume.dataset.inputSeen = 'true';
});
document.querySelector('#apply').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    value: volume.value,
    text: volume.textContent,
    currentValue: volume.getAttribute('data-current-value'),
    inputSeen: volume.dataset.inputSeen === 'true'
  });
});
"##
        .to_string(),
    ]);

    let slider = act_instruction(&env, "Use 72 with the Volume slider and click Apply.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse custom value slider state");
    assert_eq!(state["value"], "72");
    assert_eq!(state["text"], "72");
    assert_eq!(state["currentValue"], "72");
    assert_eq!(state["inputSeen"], true);
    assert_eq!(slider["plan"]["action"], "sequence");
    assert_eq!(slider["plan"]["steps"][0]["action"], "set_slider");
    assert_eq!(
        slider["result"]["steps"][0]["slider"]["mode"],
        "custom-value-slider"
    );
    assert_eq!(slider["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_table_headers_for_repeated_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-table-header-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <table id="inventory">
    <thead>
      <tr>
        <th>Product</th>
        <th>Quantity</th>
        <th>Status</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <th scope="row">Product A</th>
        <td><input id="a-quantity" name="qty_a" value="0"></td>
        <td>
          <select id="a-status" name="status_a">
            <option>Pending</option>
            <option>Approved</option>
          </select>
        </td>
      </tr>
      <tr>
        <th scope="row">Product B</th>
        <td><input id="b-quantity" name="qty_b" value="0"></td>
        <td>
          <select id="b-status" name="status_b">
            <option>Pending</option>
            <option>Approved</option>
          </select>
        </td>
      </tr>
    </tbody>
  </table>
`;
"##
        .to_string(),
    ]);

    let quantity = act_instruction(&env, "Fill Product B Quantity with 7.");
    let status = act_instruction(&env, "Choose Approved from Product B Status dropdown.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({aQty: document.querySelector('#a-quantity').value, bQty: document.querySelector('#b-quantity').value, aStatus: document.querySelector('#a-status').value, bStatus: document.querySelector('#b-status').value})".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse table header control state");
    assert_eq!(state["aQty"], "0");
    assert_eq!(state["bQty"], "7");
    assert_eq!(state["aStatus"], "Pending");
    assert_eq!(state["bStatus"], "Approved");
    assert_eq!(quantity["plan"]["params"]["selector"], "#b-quantity");
    assert_eq!(status["plan"]["params"]["selector"], "#b-status");
    assert!(quantity["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["selector"] == "#b-quantity"
            && element["label"]
                .as_str()
                .unwrap_or("")
                .contains("Product B")
            && element["label"].as_str().unwrap_or("").contains("Quantity")));
    assert_eq!(quantity["verification"]["status"], "observed");
    assert_eq!(status["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_child_fields_inside_matching_containers_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-field-fill");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="orders">
    <div class="row" role="listitem" data-name="Alice">
      <span class="name">Alice</span>
      <label>Quantity <input id="alice-locked-quantity" type="number" readonly value="99"></label>
      <fieldset disabled>
        <label>Quantity <input id="alice-disabled-quantity" type="number" value="88"></label>
      </fieldset>
      <label>Quantity <input id="alice-quantity" type="number" value="0"></label>
      <label>Status
        <select id="alice-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
    </div>
    <div class="row" role="listitem" data-name="Bob">
      <span class="name">Bob</span>
      <label>Quantity <input id="bob-quantity" type="number" value="0"></label>
      <label>Status
        <select id="bob-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
    </div>
  </section>
`;
"##
        .to_string(),
    ]);

    let quantity = act_instruction(
        &env,
        "Set the Quantity field in the row containing Alice to 3.",
    );
    let status = act_instruction(
        &env,
        "Choose Approved from the Status dropdown in the row containing Alice.",
    );
    let state = env.json(&[
        "eval".to_string(),
        "JSON.stringify({aliceLocked: document.querySelector('#alice-locked-quantity').value, aliceDisabled: document.querySelector('#alice-disabled-quantity').value, alice: document.querySelector('#alice-quantity').value, bob: document.querySelector('#bob-quantity').value, aliceStatus: document.querySelector('#alice-status').value, bobStatus: document.querySelector('#bob-status').value})".to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped field fill state");

    assert_eq!(values["aliceLocked"], "99");
    assert_eq!(values["aliceDisabled"], "88");
    assert_eq!(values["alice"], "3");
    assert_eq!(values["bob"], "0");
    assert_eq!(values["aliceStatus"], "Approved");
    assert_eq!(values["bobStatus"], "Pending");
    assert_eq!(quantity["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(quantity["plan"]["candidate"]["selector"], "#alice-quantity");
    assert_eq!(quantity["plan"]["evidence"]["itemQuery"], "Alice");
    assert_eq!(quantity["plan"]["evidence"]["fieldHint"], "Quantity");
    assert_eq!(quantity["verification"]["status"], "observed");
    assert_eq!(status["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(status["plan"]["evidence"]["itemQuery"], "Alice");
    assert_eq!(status["plan"]["evidence"]["fieldHint"], "Status");
    assert_eq!(status["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_repeated_fields_inside_named_sections_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-named-section-field-fill");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <label>Email <input id="billing-email" value=""></label>
      <label>Status
        <select id="billing-status">
          <option>Pending</option>
          <option>Active</option>
        </select>
      </label>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <label>Email <input id="support-email" value=""></label>
      <label>Status
        <select id="support-status">
          <option>Pending</option>
          <option>Active</option>
        </select>
      </label>
    </section>
  </main>
`;
"##
        .to_string(),
    ]);

    let email = act_instruction(
        &env,
        "In the Billing section, set Email to billing@example.com.",
    );
    let status = act_instruction(&env, "Choose Active from Status in the Support panel.");
    let state = env.json(&[
        "eval".to_string(),
        "JSON.stringify({billingEmail: document.querySelector('#billing-email').value, supportEmail: document.querySelector('#support-email').value, billingStatus: document.querySelector('#billing-status').value, supportStatus: document.querySelector('#support-status').value})".to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse named section field fill state");

    assert_eq!(values["billingEmail"], "billing@example.com");
    assert_eq!(values["supportEmail"], "");
    assert_eq!(values["billingStatus"], "Pending");
    assert_eq!(values["supportStatus"], "Active");
    assert_eq!(email["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(email["plan"]["candidate"]["selector"], "#billing-email");
    assert_eq!(email["plan"]["evidence"]["itemQuery"], "Billing");
    assert_eq!(email["plan"]["evidence"]["fieldHint"], "Email");
    assert_eq!(status["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(status["plan"]["candidate"]["selector"], "#support-status");
    assert_eq!(status["plan"]["evidence"]["itemQuery"], "Support");
    assert_eq!(status["plan"]["evidence"]["fieldHint"], "Status");
    assert_eq!(email["verification"]["status"], "observed");
    assert_eq!(status["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_mixed_field_types_inside_named_sections_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-named-section-field-matrix");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <label>Title <input id="billing-title" value=""></label>
      <label>Summary <textarea id="billing-summary"></textarea></label>
      <label>Priority
        <select id="billing-priority">
          <option>Low</option>
          <option>High</option>
        </select>
      </label>
      <label><input type="checkbox" id="billing-notify"> Notify</label>
      <fieldset>
        <legend>Contact Method</legend>
        <label><input type="radio" name="billing-contact" id="billing-phone" value="Phone"> Phone</label>
        <label><input type="radio" name="billing-contact" id="billing-email-method" value="Email"> Email</label>
      </fieldset>
      <button id="billing-alerts" role="switch" aria-checked="false">Alerts</button>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <label>Title <input id="support-title" value=""></label>
      <div id="support-summary" role="textbox" contenteditable="true" aria-label="Summary"></div>
      <label>Priority
        <select id="support-priority">
          <option>Low</option>
          <option>High</option>
        </select>
      </label>
      <label><input type="checkbox" id="support-notify"> Notify</label>
      <fieldset>
        <legend>Contact Method</legend>
        <label><input type="radio" name="support-contact" id="support-phone" value="Phone"> Phone</label>
        <label><input type="radio" name="support-contact" id="support-email-method" value="Email"> Email</label>
      </fieldset>
      <button id="support-alerts" role="switch" aria-checked="false">Alerts</button>
    </section>
  </main>
`;
document.querySelectorAll('[role=switch]').forEach(button => {
  button.addEventListener('click', () => {
    button.setAttribute('aria-checked', button.getAttribute('aria-checked') === 'true' ? 'false' : 'true');
  });
});
"##
        .to_string(),
    ]);

    let title = act_instruction(&env, "Set Title in the Billing section to Invoice owner.");
    let summary = act_instruction(&env, "In the Support panel, set Summary to Escalated.");
    let priority = act_instruction(&env, "Choose High from Priority in the Support panel.");
    let notify = act_instruction(&env, "Turn on Notify in the Support panel.");
    let contact = act_instruction(&env, "Select Email in the Billing section.");
    let alerts = act_instruction(&env, "Enable Alerts in the Billing section.");

    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingTitle: document.querySelector('#billing-title').value,
  supportTitle: document.querySelector('#support-title').value,
  billingSummary: document.querySelector('#billing-summary').value,
  supportSummary: document.querySelector('#support-summary').textContent,
  billingPriority: document.querySelector('#billing-priority').value,
  supportPriority: document.querySelector('#support-priority').value,
  billingNotify: document.querySelector('#billing-notify').checked,
  supportNotify: document.querySelector('#support-notify').checked,
  billingContact: document.querySelector('input[name=billing-contact]:checked')?.value || '',
  supportContact: document.querySelector('input[name=support-contact]:checked')?.value || '',
  billingAlerts: document.querySelector('#billing-alerts').getAttribute('aria-checked'),
  supportAlerts: document.querySelector('#support-alerts').getAttribute('aria-checked')
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse named section field matrix state");

    assert_eq!(values["billingTitle"], "Invoice owner");
    assert_eq!(values["supportTitle"], "");
    assert_eq!(values["billingSummary"], "");
    assert_eq!(values["supportSummary"], "Escalated");
    assert_eq!(values["billingPriority"], "Low");
    assert_eq!(values["supportPriority"], "High");
    assert_eq!(values["billingNotify"], false);
    assert_eq!(values["supportNotify"], true);
    assert_eq!(values["billingContact"], "Email");
    assert_eq!(values["supportContact"], "");
    assert_eq!(values["billingAlerts"], "true");
    assert_eq!(values["supportAlerts"], "false");
    assert_eq!(title["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(title["plan"]["candidate"]["selector"], "#billing-title");
    assert_eq!(summary["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(summary["plan"]["candidate"]["selector"], "#support-summary");
    assert_eq!(priority["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(
        priority["plan"]["candidate"]["selector"],
        "#support-priority"
    );
    assert_eq!(
        notify["plan"]["capability"]["name"],
        "scoped-checked-control"
    );
    assert_eq!(notify["plan"]["candidate"]["selector"], "#support-notify");
    assert_eq!(
        contact["plan"]["capability"]["name"],
        "scoped-checked-control"
    );
    assert_eq!(
        contact["plan"]["candidate"]["selector"],
        "#billing-email-method"
    );
    assert_eq!(
        alerts["plan"]["capability"]["name"],
        "scoped-checked-control"
    );
    assert_eq!(alerts["plan"]["candidate"]["selector"], "#billing-alerts");
    assert_eq!(title["verification"]["status"], "observed");
    assert_eq!(summary["verification"]["status"], "observed");
    assert_eq!(priority["verification"]["status"], "observed");
    assert_eq!(notify["verification"]["status"], "observed");
    assert_eq!(contact["verification"]["status"], "observed");
    assert_eq!(alerts["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_multi_action_named_sections_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-named-section-multi-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <label>Title <input id="billing-title" value=""></label>
      <label>Priority
        <select id="billing-priority">
          <option>Low</option>
          <option>High</option>
        </select>
      </label>
      <label><input type="checkbox" id="billing-notify"> Notify</label>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <label>Title <input id="support-title" value=""></label>
      <label>Priority
        <select id="support-priority">
          <option>Low</option>
          <option>High</option>
        </select>
      </label>
      <label><input type="checkbox" id="support-notify"> Notify</label>
    </section>
  </main>
`;
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        "In the Support panel, set Title to Escalated, choose High from Priority, and turn on Notify.",
    );
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingTitle: document.querySelector('#billing-title').value,
  supportTitle: document.querySelector('#support-title').value,
  billingPriority: document.querySelector('#billing-priority').value,
  supportPriority: document.querySelector('#support-priority').value,
  billingNotify: document.querySelector('#billing-notify').checked,
  supportNotify: document.querySelector('#support-notify').checked
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse named section multi-action state");

    assert_eq!(values["billingTitle"], "");
    assert_eq!(values["supportTitle"], "Escalated");
    assert_eq!(values["billingPriority"], "Low");
    assert_eq!(values["supportPriority"], "High");
    assert_eq!(values["billingNotify"], false);
    assert_eq!(values["supportNotify"], true);
    assert_eq!(action["plan"]["action"], "sequence");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#support-title"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#support-priority"
    );
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#support-notify"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_preserves_comma_values_inside_named_sections_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-named-section-comma-values");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <label>Address <input id="billing-address" value=""></label>
      <label>Due Date <input id="billing-due-date" type="date"></label>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <label>Address <input id="support-address" value=""></label>
      <label>Due Date <input id="support-due-date" type="date"></label>
    </section>
  </main>
`;
"##
        .to_string(),
    ]);

    let address = act_instruction(
        &env,
        "Set Address in the Billing section to 123 Main, Suite 4.",
    );
    let due_date = act_instruction(
        &env,
        "Set Due Date in the Support panel to January 5, 2027.",
    );
    let state = env.json(&[
        "eval".to_string(),
        "JSON.stringify({billingAddress: document.querySelector('#billing-address').value, supportAddress: document.querySelector('#support-address').value, billingDueDate: document.querySelector('#billing-due-date').value, supportDueDate: document.querySelector('#support-due-date').value})".to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse named section comma value state");

    assert_eq!(values["billingAddress"], "123 Main, Suite 4");
    assert_eq!(values["supportAddress"], "");
    assert_eq!(values["billingDueDate"], "");
    assert_eq!(values["supportDueDate"], "2027-01-05");
    assert_eq!(address["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(address["plan"]["candidate"]["selector"], "#billing-address");
    assert_eq!(address["plan"]["evidence"]["value"], "123 Main, Suite 4");
    assert_eq!(due_date["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(
        due_date["plan"]["candidate"]["selector"],
        "#support-due-date"
    );
    assert_eq!(due_date["plan"]["evidence"]["value"], "January 5, 2027");
    assert_eq!(address["verification"]["status"], "observed");
    assert_eq!(due_date["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_numeric_range_controls_inside_named_sections_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-named-section-numeric-range");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <label>Quantity <input id="billing-quantity" type="number" min="0" max="20" value="1"></label>
      <label>Volume <input id="billing-volume" type="range" min="0" max="100" value="10"></label>
      <div id="billing-retries" role="spinbutton" aria-label="Retries" aria-valuemin="0" aria-valuemax="10" aria-valuenow="1" tabindex="0">1</div>
      <div id="billing-threshold" role="slider" aria-label="Threshold" aria-valuemin="0" aria-valuemax="100" aria-valuenow="20" tabindex="0">20</div>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <label>Quantity <input id="support-quantity" type="number" min="0" max="20" value="2"></label>
      <label>Volume <input id="support-volume" type="range" min="0" max="100" value="30"></label>
      <div id="support-retries" role="spinbutton" aria-label="Retries" aria-valuemin="0" aria-valuemax="10" aria-valuenow="2" tabindex="0">2</div>
      <div id="support-threshold" role="slider" aria-label="Threshold" aria-valuemin="0" aria-valuemax="100" aria-valuenow="40" tabindex="0">40</div>
    </section>
  </main>
`;
"##
        .to_string(),
    ]);

    let quantity = act_instruction(&env, "Set Quantity in the Billing section to 7.");
    let volume = act_instruction(&env, "Set Volume in the Support panel to 75.");
    let retries = act_instruction(&env, "Set Retries in the Support panel to 4.");
    let threshold = act_instruction(&env, "Set Threshold in the Billing section to 90.");
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingQuantity: document.querySelector('#billing-quantity').value,
  supportQuantity: document.querySelector('#support-quantity').value,
  billingVolume: document.querySelector('#billing-volume').value,
  supportVolume: document.querySelector('#support-volume').value,
  billingRetries: document.querySelector('#billing-retries').getAttribute('aria-valuenow'),
  supportRetries: document.querySelector('#support-retries').getAttribute('aria-valuenow'),
  billingThreshold: document.querySelector('#billing-threshold').getAttribute('aria-valuenow'),
  supportThreshold: document.querySelector('#support-threshold').getAttribute('aria-valuenow')
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse named section numeric range state");

    assert_eq!(values["billingQuantity"], "7");
    assert_eq!(values["supportQuantity"], "2");
    assert_eq!(values["billingVolume"], "10");
    assert_eq!(values["supportVolume"], "75");
    assert_eq!(values["billingRetries"], "1");
    assert_eq!(values["supportRetries"], "4");
    assert_eq!(values["billingThreshold"], "90");
    assert_eq!(values["supportThreshold"], "40");
    assert_eq!(quantity["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(
        quantity["plan"]["candidate"]["selector"],
        "#billing-quantity"
    );
    assert_eq!(volume["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(volume["plan"]["action"], "set_slider");
    assert_eq!(volume["plan"]["candidate"]["selector"], "#support-volume");
    assert_eq!(retries["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(retries["plan"]["candidate"]["selector"], "#support-retries");
    assert_eq!(threshold["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(threshold["plan"]["action"], "set_slider");
    assert_eq!(
        threshold["plan"]["candidate"]["selector"],
        "#billing-threshold"
    );
    assert_eq!(quantity["result"]["typed"]["kind"], "number");
    assert_eq!(volume["result"]["slider"]["mode"], "native-range");
    assert_eq!(volume["result"]["slider"]["value"], 75);
    assert_eq!(retries["result"]["typed"]["kind"], "spinbutton");
    assert_eq!(threshold["result"]["slider"]["mode"], "aria");
    assert_eq!(threshold["result"]["slider"]["value"], 90);
    assert_eq!(quantity["verification"]["status"], "observed");
    assert_eq!(volume["verification"]["status"], "observed");
    assert_eq!(retries["verification"]["status"], "observed");
    assert_eq!(threshold["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_custom_selectors_inside_named_sections_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-named-section-custom-selectors");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <button id="billing-plan" role="combobox" aria-label="Plan" aria-haspopup="listbox" aria-controls="billing-plan-options" aria-expanded="false">Plan: Basic</button>
      <div id="billing-plan-options" role="listbox" hidden>
        <div role="option" data-value="basic">Basic</div>
        <div role="option" data-value="enterprise">Enterprise</div>
      </div>
      <div id="billing-routing" role="listbox" aria-label="Routing">
        <div role="option" data-value="automatic" aria-selected="true">Automatic</div>
        <div role="option" data-value="manual" aria-selected="false">Manual</div>
      </div>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <button id="support-plan" role="combobox" aria-label="Plan" aria-haspopup="listbox" aria-controls="support-plan-options" aria-expanded="false">Plan: Basic</button>
      <div id="support-plan-options" role="listbox" hidden>
        <div role="option" data-value="basic">Basic</div>
        <div role="option" data-value="enterprise">Enterprise</div>
      </div>
      <div id="support-routing" role="listbox" aria-label="Routing">
        <div role="option" data-value="automatic" aria-selected="true">Automatic</div>
        <div role="option" data-value="manual" aria-selected="false">Manual</div>
      </div>
    </section>
  </main>
`;
document.querySelectorAll('[role=combobox]').forEach(combo => {
  combo.addEventListener('click', () => {
    const options = document.getElementById(combo.getAttribute('aria-controls'));
    options.hidden = false;
    combo.setAttribute('aria-expanded', 'true');
  });
});
document.querySelectorAll('[role=option]').forEach(option => {
  option.addEventListener('click', () => {
    const owner = option.closest('[role=listbox]');
    owner.querySelectorAll('[role=option]').forEach(peer => peer.setAttribute('aria-selected', 'false'));
    option.setAttribute('aria-selected', 'true');
    const combo = document.querySelector(`[aria-controls="${owner.id}"]`);
    if (combo) {
      combo.dataset.value = option.dataset.value;
      combo.textContent = 'Plan: ' + option.textContent;
      combo.setAttribute('aria-expanded', 'false');
      owner.hidden = true;
    }
  });
});
"##
        .to_string(),
    ]);

    let plan = act_instruction(&env, "Choose Enterprise from Plan in the Support panel.");
    let routing = act_instruction(&env, "Choose Manual from Routing in the Billing section.");
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingPlan: document.querySelector('#billing-plan').dataset.value || 'basic',
  supportPlan: document.querySelector('#support-plan').dataset.value || 'basic',
  billingRouting: document.querySelector('#billing-routing [aria-selected=true]').dataset.value,
  supportRouting: document.querySelector('#support-routing [aria-selected=true]').dataset.value
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse named section custom selector state");

    assert_eq!(values["billingPlan"], "basic");
    assert_eq!(values["supportPlan"], "enterprise");
    assert_eq!(values["billingRouting"], "manual");
    assert_eq!(values["supportRouting"], "automatic");
    assert_eq!(plan["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(plan["plan"]["candidate"]["selector"], "#support-plan");
    assert_eq!(plan["result"]["selected"]["mode"], "custom-option");
    assert_eq!(routing["plan"]["capability"]["name"], "scoped-field-fill");
    assert_eq!(routing["plan"]["candidate"]["selector"], "#billing-routing");
    assert_eq!(routing["result"]["selected"]["mode"], "custom-option");
    assert_eq!(plan["verification"]["status"], "observed");
    assert_eq!(routing["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_shorthand_multi_action_custom_selectors_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-custom-selector-multi-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <button id="billing-plan" role="combobox" aria-label="Plan" aria-haspopup="listbox" aria-controls="billing-plan-options" aria-expanded="false">Plan: Basic</button>
      <div id="billing-plan-options" role="listbox" hidden>
        <div role="option" data-value="basic">Basic</div>
        <div role="option" data-value="enterprise">Enterprise</div>
      </div>
      <div id="billing-routing" role="listbox" aria-label="Routing">
        <div role="option" data-value="automatic" aria-selected="true">Automatic</div>
        <div role="option" data-value="manual" aria-selected="false">Manual</div>
      </div>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <button id="support-plan" role="combobox" aria-label="Plan" aria-haspopup="listbox" aria-controls="support-plan-options" aria-expanded="false">Plan: Basic</button>
      <div id="support-plan-options" role="listbox" hidden>
        <div role="option" data-value="basic">Basic</div>
        <div role="option" data-value="enterprise">Enterprise</div>
      </div>
      <div id="support-routing" role="listbox" aria-label="Routing">
        <div role="option" data-value="automatic" aria-selected="true">Automatic</div>
        <div role="option" data-value="manual" aria-selected="false">Manual</div>
      </div>
    </section>
  </main>
`;
document.querySelectorAll('[role=combobox]').forEach(combo => {
  combo.addEventListener('click', () => {
    const options = document.getElementById(combo.getAttribute('aria-controls'));
    options.hidden = false;
    combo.setAttribute('aria-expanded', 'true');
  });
});
document.querySelectorAll('[role=option]').forEach(option => {
  option.addEventListener('click', () => {
    const owner = option.closest('[role=listbox]');
    owner.querySelectorAll('[role=option]').forEach(peer => peer.setAttribute('aria-selected', 'false'));
    option.setAttribute('aria-selected', 'true');
    const combo = document.querySelector(`[aria-controls="${owner.id}"]`);
    if (combo) {
      combo.dataset.value = option.dataset.value;
      combo.textContent = 'Plan: ' + option.textContent;
      combo.setAttribute('aria-expanded', 'false');
      owner.hidden = true;
    }
  });
});
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        "In the Support panel, Plan Enterprise, Routing Manual.",
    );
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingPlan: document.querySelector('#billing-plan').dataset.value || 'basic',
  supportPlan: document.querySelector('#support-plan').dataset.value || 'basic',
  billingRouting: document.querySelector('#billing-routing [aria-selected=true]').dataset.value,
  supportRouting: document.querySelector('#support-routing [aria-selected=true]').dataset.value
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped custom selector multi-action state");

    assert_eq!(values["billingPlan"], "basic");
    assert_eq!(values["supportPlan"], "enterprise");
    assert_eq!(values["billingRouting"], "automatic");
    assert_eq!(values["supportRouting"], "manual");
    assert_eq!(action["plan"]["action"], "sequence");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#support-plan"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#support-routing"
    );
    assert_eq!(
        action["result"]["steps"][0]["selected"]["mode"],
        "custom-option"
    );
    assert_eq!(
        action["result"]["steps"][1]["selected"]["mode"],
        "custom-option"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_key_value_scoped_multi_action_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-key-value-multi-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <button id="billing-plan" role="combobox" aria-label="Plan" aria-haspopup="listbox" aria-controls="billing-plan-options" aria-expanded="false">Plan: Basic</button>
      <div id="billing-plan-options" role="listbox" hidden>
        <div role="option" data-value="basic">Basic</div>
        <div role="option" data-value="enterprise">Enterprise</div>
      </div>
      <div id="billing-routing" role="listbox" aria-label="Routing">
        <div role="option" data-value="automatic" aria-selected="true">Automatic</div>
        <div role="option" data-value="manual" aria-selected="false">Manual</div>
      </div>
      <label><input type="checkbox" id="billing-notify" checked> Notify</label>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <button id="support-plan" role="combobox" aria-label="Plan" aria-haspopup="listbox" aria-controls="support-plan-options" aria-expanded="false">Plan: Basic</button>
      <div id="support-plan-options" role="listbox" hidden>
        <div role="option" data-value="basic">Basic</div>
        <div role="option" data-value="enterprise">Enterprise</div>
      </div>
      <div id="support-routing" role="listbox" aria-label="Routing">
        <div role="option" data-value="automatic" aria-selected="true">Automatic</div>
        <div role="option" data-value="manual" aria-selected="false">Manual</div>
      </div>
      <label><input type="checkbox" id="support-notify" checked> Notify</label>
    </section>
  </main>
`;
document.querySelectorAll('[role=combobox]').forEach(combo => {
  combo.addEventListener('click', () => {
    const options = document.getElementById(combo.getAttribute('aria-controls'));
    options.hidden = false;
    combo.setAttribute('aria-expanded', 'true');
  });
});
document.querySelectorAll('[role=option]').forEach(option => {
  option.addEventListener('click', () => {
    const owner = option.closest('[role=listbox]');
    owner.querySelectorAll('[role=option]').forEach(peer => peer.setAttribute('aria-selected', 'false'));
    option.setAttribute('aria-selected', 'true');
    const combo = document.querySelector(`[aria-controls="${owner.id}"]`);
    if (combo) {
      combo.dataset.value = option.dataset.value;
      combo.textContent = 'Plan: ' + option.textContent;
      combo.setAttribute('aria-expanded', 'false');
      owner.hidden = true;
    }
  });
});
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        "In the Support panel, Plan: Enterprise; Routing=Manual; Notify: off.",
    );
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingPlan: document.querySelector('#billing-plan').dataset.value || 'basic',
  supportPlan: document.querySelector('#support-plan').dataset.value || 'basic',
  billingRouting: document.querySelector('#billing-routing [aria-selected=true]').dataset.value,
  supportRouting: document.querySelector('#support-routing [aria-selected=true]').dataset.value,
  billingNotify: document.querySelector('#billing-notify').checked,
  supportNotify: document.querySelector('#support-notify').checked
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped key-value multi-action state");

    assert_eq!(values["billingPlan"], "basic");
    assert_eq!(values["supportPlan"], "enterprise");
    assert_eq!(values["billingRouting"], "automatic");
    assert_eq!(values["supportRouting"], "manual");
    assert_eq!(values["billingNotify"], true);
    assert_eq!(values["supportNotify"], false);
    assert_eq!(action["plan"]["action"], "sequence");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#support-plan"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#support-routing"
    );
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#support-notify"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_multi_action_matching_containers_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-row-multi-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="records">
    <div class="row" role="listitem" data-name="Alice">
      <span>Alice</span>
      <label>Quantity <input id="alice-quantity" type="number" value="1"></label>
      <label>Status
        <select id="alice-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <label><input type="checkbox" id="alice-notify"> Notify</label>
      <button id="alice-save">Save</button>
    </div>
    <div class="row" role="listitem" data-name="Bob">
      <span>Bob</span>
      <label>Quantity <input id="bob-quantity" type="number" value="2"></label>
      <label>Status
        <select id="bob-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <label><input type="checkbox" id="bob-notify"> Notify</label>
      <button id="bob-save">Save</button>
    </div>
  </section>
  <output id="out"></output>
`;
for (const row of document.querySelectorAll('.row')) {
  const name = row.querySelector('span').textContent;
  row.querySelector('button').addEventListener('click', () => {
    document.querySelector('#out').textContent = `${name} saved`;
  });
}
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        "In the row containing Alice, set Quantity to 3, choose Approved from Status, turn on Notify, and click Save.",
    );
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  aliceQuantity: document.querySelector('#alice-quantity').value,
  bobQuantity: document.querySelector('#bob-quantity').value,
  aliceStatus: document.querySelector('#alice-status').value,
  bobStatus: document.querySelector('#bob-status').value,
  aliceNotify: document.querySelector('#alice-notify').checked,
  bobNotify: document.querySelector('#bob-notify').checked,
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped row multi-action state");

    assert_eq!(values["aliceQuantity"], "3");
    assert_eq!(values["bobQuantity"], "2");
    assert_eq!(values["aliceStatus"], "Approved");
    assert_eq!(values["bobStatus"], "Pending");
    assert_eq!(values["aliceNotify"], true);
    assert_eq!(values["bobNotify"], false);
    assert_eq!(values["out"], "Alice saved");
    assert_eq!(action["plan"]["action"], "sequence");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#alice-quantity"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#alice-status"
    );
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#alice-notify"
    );
    assert_eq!(
        action["plan"]["steps"][3]["params"]["selector"],
        "#alice-save"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_shorthand_multi_action_matching_containers_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-row-shorthand-multi-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="records">
    <div class="row" role="listitem" data-name="Alice">
      <span>Alice</span>
      <label>Quantity <input id="alice-quantity" type="number" value="1"></label>
      <label>Status
        <select id="alice-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <label><input type="checkbox" id="alice-notify"> Notify</label>
      <button id="alice-save">Save</button>
    </div>
    <div class="row" role="listitem" data-name="Bob">
      <span>Bob</span>
      <label>Quantity <input id="bob-quantity" type="number" value="2"></label>
      <label>Status
        <select id="bob-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <label><input type="checkbox" id="bob-notify"> Notify</label>
      <button id="bob-save">Save</button>
    </div>
  </section>
  <output id="out"></output>
`;
for (const row of document.querySelectorAll('.row')) {
  const name = row.querySelector('span').textContent;
  row.querySelector('button').addEventListener('click', () => {
    document.querySelector('#out').textContent = `${name} saved`;
  });
}
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        "In the row containing Alice, Quantity 4, Status Approved, Notify on, and click Save.",
    );
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  aliceQuantity: document.querySelector('#alice-quantity').value,
  bobQuantity: document.querySelector('#bob-quantity').value,
  aliceStatus: document.querySelector('#alice-status').value,
  bobStatus: document.querySelector('#bob-status').value,
  aliceNotify: document.querySelector('#alice-notify').checked,
  bobNotify: document.querySelector('#bob-notify').checked,
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped row shorthand multi-action state");

    assert_eq!(values["aliceQuantity"], "4");
    assert_eq!(values["bobQuantity"], "2");
    assert_eq!(values["aliceStatus"], "Approved");
    assert_eq!(values["bobStatus"], "Pending");
    assert_eq!(values["aliceNotify"], true);
    assert_eq!(values["bobNotify"], false);
    assert_eq!(values["out"], "Alice saved");
    assert_eq!(action["plan"]["action"], "sequence");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#alice-quantity"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#alice-status"
    );
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#alice-notify"
    );
    assert_eq!(
        action["plan"]["steps"][3]["params"]["selector"],
        "#alice-save"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_bare_completion_clause_in_scoped_multi_action_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-row-bare-completion");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="records">
    <div class="row" role="listitem" data-name="Alice">
      <span>Alice</span>
      <label>Quantity <input id="alice-quantity" type="number" value="1"></label>
      <label>Status
        <select id="alice-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <button id="alice-save">Save</button>
    </div>
    <div class="row" role="listitem" data-name="Bob">
      <span>Bob</span>
      <label>Quantity <input id="bob-quantity" type="number" value="2"></label>
      <label>Status
        <select id="bob-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <button id="bob-save">Save</button>
    </div>
  </section>
  <button id="global-save">Save</button>
  <output id="out"></output>
`;
for (const row of document.querySelectorAll('.row')) {
  const name = row.querySelector('span').textContent;
  row.querySelector('button').addEventListener('click', () => {
    document.querySelector('#out').textContent = `${name} saved`;
  });
}
document.querySelector('#global-save').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'global saved';
});
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        "In the row containing Alice, Quantity 4, Status Approved, Save.",
    );
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  aliceQuantity: document.querySelector('#alice-quantity').value,
  bobQuantity: document.querySelector('#bob-quantity').value,
  aliceStatus: document.querySelector('#alice-status').value,
  bobStatus: document.querySelector('#bob-status').value,
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped row bare completion state");

    assert_eq!(values["aliceQuantity"], "4");
    assert_eq!(values["bobQuantity"], "2");
    assert_eq!(values["aliceStatus"], "Approved");
    assert_eq!(values["bobStatus"], "Pending");
    assert_eq!(values["out"], "Alice saved");
    assert_eq!(action["plan"]["action"], "sequence");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#alice-quantity"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#alice-status"
    );
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#alice-save"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_record_first_scoped_multi_action_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-record-first-scoped-multi-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="records">
    <div class="row" role="listitem" data-name="Alice">
      <span>Alice</span>
      <label>Quantity <input id="alice-quantity" type="number" value="1"></label>
      <label>Status
        <select id="alice-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <button id="alice-save">Save</button>
    </div>
    <div class="row" role="listitem" data-name="Bob">
      <span>Bob</span>
      <label>Quantity <input id="bob-quantity" type="number" value="2"></label>
      <label>Status
        <select id="bob-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <button id="bob-save">Save</button>
    </div>
  </section>
  <button id="global-save">Save</button>
  <output id="out"></output>
`;
for (const row of document.querySelectorAll('.row')) {
  const name = row.querySelector('span').textContent;
  row.querySelector('button').addEventListener('click', () => {
    document.querySelector('#out').textContent = `${name} saved`;
  });
}
document.querySelector('#global-save').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'global saved';
});
"##
        .to_string(),
    ]);

    let alice = act_instruction(&env, "For Alice, Quantity 4, Status Approved, Save.");
    let bob = act_instruction(&env, "Bob row: Quantity: 5; Status=Approved; Save.");
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  aliceQuantity: document.querySelector('#alice-quantity').value,
  bobQuantity: document.querySelector('#bob-quantity').value,
  aliceStatus: document.querySelector('#alice-status').value,
  bobStatus: document.querySelector('#bob-status').value,
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse record-first scoped multi-action state");

    assert_eq!(values["aliceQuantity"], "4");
    assert_eq!(values["bobQuantity"], "5");
    assert_eq!(values["aliceStatus"], "Approved");
    assert_eq!(values["bobStatus"], "Approved");
    assert_eq!(values["out"], "Bob saved");
    assert_eq!(alice["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(bob["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        alice["plan"]["steps"][2]["params"]["selector"],
        "#alice-save"
    );
    assert_eq!(bob["plan"]["steps"][2]["params"]["selector"], "#bob-save");
    assert_eq!(alice["verification"]["status"], "observed");
    assert_eq!(bob["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_leading_container_label_multi_action_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-leading-container-label-multi-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <label>Title <input id="billing-title" value="Billing task"></label>
      <label>Status
        <select id="billing-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <button id="billing-save">Save</button>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <label>Title <input id="support-title" value="Support task"></label>
      <label>Status
        <select id="support-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <button id="support-save">Save</button>
    </section>
  </main>
  <button id="global-save">Save</button>
  <output id="out"></output>
`;
for (const section of document.querySelectorAll('section')) {
  const name = section.querySelector('h2').textContent;
  section.querySelector('button').addEventListener('click', () => {
    document.querySelector('#out').textContent = `${name} saved`;
  });
}
document.querySelector('#global-save').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'global saved';
});
"##
        .to_string(),
    ]);

    let action = act_instruction(&env, "Support: Title Escalated; Status Approved; Save.");
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingTitle: document.querySelector('#billing-title').value,
  supportTitle: document.querySelector('#support-title').value,
  billingStatus: document.querySelector('#billing-status').value,
  supportStatus: document.querySelector('#support-status').value,
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse leading container label multi-action state");

    assert_eq!(values["billingTitle"], "Billing task");
    assert_eq!(values["supportTitle"], "Escalated");
    assert_eq!(values["billingStatus"], "Pending");
    assert_eq!(values["supportStatus"], "Approved");
    assert_eq!(values["out"], "Support saved");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#support-title"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#support-status"
    );
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#support-save"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_bulleted_container_multi_action_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-bulleted-container-multi-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <label>Title <input id="billing-title" value="Billing task"></label>
      <label>Status
        <select id="billing-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <button id="billing-save">Save</button>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <label>Title <input id="support-title" value="Support task"></label>
      <label>Status
        <select id="support-status">
          <option>Pending</option>
          <option>Approved</option>
        </select>
      </label>
      <button id="support-save">Save</button>
    </section>
  </main>
  <button id="global-save">Save</button>
  <output id="out"></output>
`;
for (const section of document.querySelectorAll('section')) {
  const name = section.querySelector('h2').textContent;
  section.querySelector('button').addEventListener('click', () => {
    document.querySelector('#out').textContent = `${name} saved`;
  });
}
document.querySelector('#global-save').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'global saved';
});
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        "Support:\n- Title: Escalated\n- Status: Approved\n- Save",
    );
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingTitle: document.querySelector('#billing-title').value,
  supportTitle: document.querySelector('#support-title').value,
  billingStatus: document.querySelector('#billing-status').value,
  supportStatus: document.querySelector('#support-status').value,
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse bulleted container multi-action state");

    assert_eq!(values["billingTitle"], "Billing task");
    assert_eq!(values["supportTitle"], "Escalated");
    assert_eq!(values["billingStatus"], "Pending");
    assert_eq!(values["supportStatus"], "Approved");
    assert_eq!(values["out"], "Support saved");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#support-title"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#support-status"
    );
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#support-save"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_shorthand_grouped_choices_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-shorthand-grouped-choices");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <fieldset>
        <legend>Priority</legend>
        <label><input type="radio" name="billing-priority" id="billing-low" value="Low" checked> Low</label>
        <label><input type="radio" name="billing-priority" id="billing-high" value="High"> High</label>
      </fieldset>
      <div role="radiogroup" aria-label="Routing">
        <button role="radio" id="billing-auto" aria-checked="true">Automatic</button>
        <button role="radio" id="billing-manual" aria-checked="false">Manual</button>
      </div>
      <button id="billing-save">Save</button>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <fieldset>
        <legend>Priority</legend>
        <label><input type="radio" name="support-priority" id="support-low" value="Low" checked> Low</label>
        <label><input type="radio" name="support-priority" id="support-high" value="High"> High</label>
      </fieldset>
      <div role="radiogroup" aria-label="Routing">
        <button role="radio" id="support-auto" aria-checked="true">Automatic</button>
        <button role="radio" id="support-manual" aria-checked="false">Manual</button>
      </div>
      <button id="support-save">Save</button>
    </section>
  </main>
  <button id="global-save">Save</button>
  <output id="out"></output>
`;
document.querySelectorAll('[role=radio]').forEach(radio => {
  radio.addEventListener('click', () => {
    const group = radio.closest('[role=radiogroup]');
    group.querySelectorAll('[role=radio]').forEach(peer => peer.setAttribute('aria-checked', String(peer === radio)));
  });
});
for (const section of document.querySelectorAll('section')) {
  const name = section.querySelector('h2').textContent;
  section.querySelector('button[id$="-save"]').addEventListener('click', () => {
    document.querySelector('#out').textContent = `${name} saved`;
  });
}
document.querySelector('#global-save').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'global saved';
});
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        "Support:\n- Priority: High\n- Routing: Manual\n- Save",
    );
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingPriority: document.querySelector('input[name="billing-priority"]:checked').value,
  supportPriority: document.querySelector('input[name="support-priority"]:checked').value,
  billingRouting: document.querySelector('#billing-manual').getAttribute('aria-checked'),
  supportRouting: document.querySelector('#support-manual').getAttribute('aria-checked'),
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped shorthand grouped choice state");

    assert_eq!(values["billingPriority"], "Low");
    assert_eq!(values["supportPriority"], "High");
    assert_eq!(values["billingRouting"], "false");
    assert_eq!(values["supportRouting"], "true");
    assert_eq!(values["out"], "Support saved");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#support-high"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#support-manual"
    );
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#support-save"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_shorthand_numeric_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-shorthand-numeric-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <label>Limit <input id="billing-limit" type="range" min="0" max="100" value="15"></label>
      <div id="billing-retries" role="spinbutton" aria-label="Retries" aria-valuemin="0" aria-valuemax="10" aria-valuenow="1" tabindex="0">1</div>
      <div id="billing-threshold" role="slider" aria-label="Threshold" aria-valuemin="0" aria-valuemax="100" aria-valuenow="20" tabindex="0">20</div>
      <button id="billing-save">Save</button>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <label>Limit <input id="support-limit" type="range" min="0" max="100" value="30"></label>
      <div id="support-retries" role="spinbutton" aria-label="Retries" aria-valuemin="0" aria-valuemax="10" aria-valuenow="2" tabindex="0">2</div>
      <div id="support-threshold" role="slider" aria-label="Threshold" aria-valuemin="0" aria-valuemax="100" aria-valuenow="40" tabindex="0">40</div>
      <button id="support-save">Save</button>
    </section>
  </main>
  <button id="global-save">Save</button>
  <output id="out"></output>
`;
for (const control of document.querySelectorAll('[role=spinbutton], [role=slider]')) {
  control.addEventListener('input', () => {
    control.textContent = control.getAttribute('aria-valuenow');
  });
  control.addEventListener('change', () => {
    control.textContent = control.getAttribute('aria-valuenow');
  });
}
for (const section of document.querySelectorAll('section')) {
  const name = section.querySelector('h2').textContent;
  section.querySelector('button[id$="-save"]').addEventListener('click', () => {
    document.querySelector('#out').textContent = `${name} saved`;
  });
}
document.querySelector('#global-save').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'global saved';
});
"##
        .to_string(),
    ]);

    let action = act_instruction(&env, "Support: Limit: 75; Retries: 3; Threshold: 88; Save.");
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingLimit: document.querySelector('#billing-limit').value,
  supportLimit: document.querySelector('#support-limit').value,
  billingRetries: document.querySelector('#billing-retries').getAttribute('aria-valuenow'),
  supportRetries: document.querySelector('#support-retries').getAttribute('aria-valuenow'),
  billingThreshold: document.querySelector('#billing-threshold').getAttribute('aria-valuenow'),
  supportThreshold: document.querySelector('#support-threshold').getAttribute('aria-valuenow'),
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped shorthand numeric control state");

    assert_eq!(values["billingLimit"], "15");
    assert_eq!(values["supportLimit"], "75");
    assert_eq!(values["billingRetries"], "1");
    assert_eq!(values["supportRetries"], "3");
    assert_eq!(values["billingThreshold"], "20");
    assert_eq!(values["supportThreshold"], "88");
    assert_eq!(values["out"], "Support saved");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(action["plan"]["action"], "sequence");
    assert_eq!(action["plan"]["steps"][0]["action"], "set_slider");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#support-limit"
    );
    assert_eq!(action["plan"]["steps"][1]["action"], "type");
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#support-retries"
    );
    assert_eq!(action["plan"]["steps"][2]["action"], "set_slider");
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#support-threshold"
    );
    assert_eq!(
        action["plan"]["steps"][3]["params"]["selector"],
        "#support-save"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_preserves_typed_and_comma_values_in_scoped_multi_action_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-typed-comma-values");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <label>Due Date <input id="billing-due-date" type="date"></label>
      <label>Start Time <input id="billing-start-time" type="time"></label>
      <label>Color <input id="billing-color" type="color" value="#ff0000"></label>
      <label>Notes <textarea id="billing-notes"></textarea></label>
      <button id="billing-save">Save</button>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <label>Due Date <input id="support-due-date" type="date"></label>
      <label>Start Time <input id="support-start-time" type="time"></label>
      <label>Color <input id="support-color" type="color" value="#ff0000"></label>
      <label>Notes <textarea id="support-notes"></textarea></label>
      <button id="support-save">Save</button>
    </section>
  </main>
  <button id="global-save">Save</button>
  <output id="out"></output>
`;
for (const section of document.querySelectorAll('section')) {
  const name = section.querySelector('h2').textContent;
  section.querySelector('button[id$="-save"]').addEventListener('click', () => {
    document.querySelector('#out').textContent = `${name} saved`;
  });
}
document.querySelector('#global-save').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'global saved';
});
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        "Support: Due Date: January 5, 2027; Start Time: 3:15 pm; Color: blue; Notes: Call, email; Save.",
    );
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingDueDate: document.querySelector('#billing-due-date').value,
  supportDueDate: document.querySelector('#support-due-date').value,
  billingStartTime: document.querySelector('#billing-start-time').value,
  supportStartTime: document.querySelector('#support-start-time').value,
  billingColor: document.querySelector('#billing-color').value,
  supportColor: document.querySelector('#support-color').value,
  billingNotes: document.querySelector('#billing-notes').value,
  supportNotes: document.querySelector('#support-notes').value,
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped typed comma value state");

    assert_eq!(values["billingDueDate"], "");
    assert_eq!(values["supportDueDate"], "2027-01-05");
    assert_eq!(values["billingStartTime"], "");
    assert_eq!(values["supportStartTime"], "15:15");
    assert_eq!(values["billingColor"], "#ff0000");
    assert_eq!(values["supportColor"], "#0000ff");
    assert_eq!(values["billingNotes"], "");
    assert_eq!(values["supportNotes"], "Call, email");
    assert_eq!(values["out"], "Support saved");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#support-due-date"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#support-start-time"
    );
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#support-color"
    );
    assert_eq!(
        action["plan"]["steps"][3]["params"]["selector"],
        "#support-notes"
    );
    assert_eq!(
        action["plan"]["steps"][4]["params"]["selector"],
        "#support-save"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_repeated_field_edits_in_scoped_multi_action_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-row-multi-action-edits");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="records">
    <div class="row" role="listitem" data-name="Alice">
      <span>Alice</span>
      <label>Notes <textarea id="alice-notes">old alice note</textarea></label>
      <button id="alice-save">Save</button>
    </div>
    <div class="row" role="listitem" data-name="Bob">
      <span>Bob</span>
      <label>Notes <textarea id="bob-notes">old bob note</textarea></label>
      <button id="bob-save">Save</button>
    </div>
  </section>
  <output id="out"></output>
`;
for (const row of document.querySelectorAll('.row')) {
  const name = row.querySelector('span').textContent;
  row.querySelector('button').addEventListener('click', () => {
    document.querySelector('#out').textContent = `${name} saved`;
  });
}
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        "In the row containing Alice, clear Notes, append done to Notes, and click Save.",
    );
    let state = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  aliceNotes: document.querySelector('#alice-notes').value,
  bobNotes: document.querySelector('#bob-notes').value,
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped row multi-action edit state");

    assert_eq!(values["aliceNotes"], "done");
    assert_eq!(values["bobNotes"], "old bob note");
    assert_eq!(values["out"], "Alice saved");
    assert_eq!(action["plan"]["action"], "sequence");
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#alice-notes"
    );
    assert_eq!(action["plan"]["steps"][0]["params"]["clear_first"], true);
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#alice-notes"
    );
    assert_eq!(action["plan"]["steps"][1]["params"]["clear_first"], false);
    assert_eq!(
        action["plan"]["steps"][2]["params"]["selector"],
        "#alice-save"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_open_shadow_root_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <profile-card></profile-card>
  <output id="out"></output>
`;
customElements.define('profile-card', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <label for="display">Display name</label>
      <input id="display" name="display_name">
      <button id="save">Save</button>
    `;
    root.querySelector('#save').addEventListener('click', () => {
      document.querySelector('#out').textContent = root.querySelector('#display').value;
    });
  }
});
"##
        .to_string(),
    ]);

    let shadow = act_instruction(&env, "Enter Shadow User into Display name and press Save.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "Shadow User");
    assert_eq!(shadow["plan"]["action"], "sequence");
    assert_eq!(shadow["plan"]["steps"][0]["action"], "type");
    assert_eq!(shadow["plan"]["steps"][1]["action"], "click");
    assert!(shadow["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["selector"] == "#display"
            && element["label"] == "Display name"
            && element["context"]["kind"] == "shadow-root"));
    assert_eq!(shadow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_shadow_host_metadata_for_inner_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-host-field-label");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <payment-field label="Security code" data-field-name="card verification code"></payment-field>
  <output id="out"></output>
`;
customElements.define('payment-field', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <input>
      <button>Continue</button>
    `;
    root.querySelector('button').addEventListener('click', () => {
      document.querySelector('#out').textContent = root.querySelector('input').value;
    });
  }
});
"##
        .to_string(),
    ]);

    let shadow = act_instruction(&env, "Enter 123456 into security code and press Continue.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "123456");
    assert_eq!(shadow["plan"]["action"], "sequence");
    assert_eq!(shadow["plan"]["steps"][0]["action"], "type");
    assert_eq!(shadow["plan"]["steps"][1]["action"], "click");
    assert!(shadow["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["tag"] == "input"
            && element["label"]
                .as_str()
                .unwrap_or("")
                .contains("Security code")
            && element["context"]["kind"] == "shadow-root"));
    assert_eq!(shadow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_external_shadow_host_labels_for_inner_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-external-shadow-host-label");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="contact-host">Billing contact</label>
  <contact-field id="contact-host"></contact-field>
  <output id="out"></output>
`;
customElements.define('contact-field', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <input>
      <button>Save</button>
    `;
    root.querySelector('button').addEventListener('click', () => {
      document.querySelector('#out').textContent = root.querySelector('input').value;
    });
  }
});
"##
        .to_string(),
    ]);

    let shadow = act_instruction(
        &env,
        "Enter Ada Lovelace into billing contact and press Save.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "Ada Lovelace");
    assert_eq!(shadow["plan"]["action"], "sequence");
    assert_eq!(shadow["plan"]["steps"][0]["action"], "type");
    assert_eq!(shadow["plan"]["steps"][1]["action"], "click");
    assert!(shadow["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["tag"] == "input"
            && element["label"]
                .as_str()
                .unwrap_or("")
                .contains("Billing contact")
            && element["context"]["kind"] == "shadow-root"));
    assert_eq!(shadow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_shadow_host_aria_references_for_inner_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-host-aria-reference-label");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <span id="contact-label">Support owner</span>
  <contact-field aria-labelledby="contact-label"></contact-field>
  <output id="out"></output>
`;
customElements.define('contact-field', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <input>
      <button>Save</button>
    `;
    root.querySelector('button').addEventListener('click', () => {
      document.querySelector('#out').textContent = root.querySelector('input').value;
    });
  }
});
"##
        .to_string(),
    ]);

    let shadow = act_instruction(
        &env,
        "Enter Linus Torvalds into support owner and press Save.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "Linus Torvalds");
    assert_eq!(shadow["plan"]["action"], "sequence");
    assert_eq!(shadow["plan"]["steps"][0]["action"], "type");
    assert_eq!(shadow["plan"]["steps"][1]["action"], "click");
    assert!(shadow["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["tag"] == "input"
            && element["label"]
                .as_str()
                .unwrap_or("")
                .contains("Support owner")
            && element["context"]["kind"] == "shadow-root"));
    assert_eq!(shadow["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_custom_element_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-element-value-host");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <account-field label="Account owner" data-field-name="owner"></account-field>
  <button id="save">Save</button>
  <output id="out"></output>
`;
customElements.define('account-field', class extends HTMLElement {
  constructor() {
    super();
    this._value = '';
  }
  connectedCallback() {
    this.style.display = 'inline-block';
    this.style.width = '180px';
    this.style.height = '32px';
    this.style.border = '1px solid rgb(80, 80, 80)';
  }
  get value() {
    return this._value;
  }
  set value(next) {
    this._value = String(next);
    this.textContent = this._value;
  }
});
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    document.querySelector('account-field').value;
});
"##
        .to_string(),
    ]);

    let result = act_instruction(
        &env,
        "Enter Ada Lovelace into account owner and press Save.",
    );
    let output = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(output["result"], "Ada Lovelace");
    assert_eq!(result["plan"]["action"], "sequence");
    assert_eq!(result["plan"]["steps"][0]["action"], "type");
    assert_eq!(result["plan"]["steps"][1]["action"], "click");
    assert!(result["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["tag"] == "account-field"
            && element["affordances"]["fillable"] == true
            && element["label"]
                .as_str()
                .unwrap_or("")
                .contains("Account owner")));
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_custom_element_checked_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-element-checked-host");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <notification-toggle label="Marketing emails" data-field-name="marketing_emails"></notification-toggle>
  <button id="save">Save</button>
  <output id="out"></output>
`;
customElements.define('notification-toggle', class extends HTMLElement {
  constructor() {
    super();
    this._checked = false;
  }
  connectedCallback() {
    this.style.display = 'inline-block';
    this.style.width = '180px';
    this.style.height = '32px';
    this.style.border = '1px solid rgb(80, 80, 80)';
    this.textContent = 'Off';
  }
  get checked() {
    return this._checked;
  }
  set checked(next) {
    this._checked = Boolean(next);
    this.textContent = this._checked ? 'On' : 'Off';
  }
});
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    String(document.querySelector('notification-toggle').checked);
});
"##
        .to_string(),
    ]);

    let result = act_instruction(&env, "Turn on marketing emails and press Save.");
    let output = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(output["result"], "true");
    assert_eq!(result["plan"]["action"], "sequence");
    assert_eq!(result["plan"]["steps"][0]["action"], "set_checked");
    assert_eq!(result["plan"]["steps"][1]["action"], "click");
    assert!(result["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["tag"] == "notification-toggle"
            && element["affordances"]["checkable"] == true
            && element["label"]
                .as_str()
                .unwrap_or("")
                .contains("Marketing emails")));
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_externally_labeled_custom_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-external-label-custom-value");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="custom-a">Project lead</label>
  <plain-field id="custom-a"></plain-field>
  <button id="save">Save</button>
  <output id="out"></output>
`;
customElements.define('plain-field', class extends HTMLElement {
  constructor() {
    super();
    this._value = '';
  }
  connectedCallback() {
    this.style.display = 'inline-block';
    this.style.width = '180px';
    this.style.height = '32px';
    this.style.border = '1px solid rgb(80, 80, 80)';
  }
  get value() {
    return this._value;
  }
  set value(next) {
    this._value = String(next);
    this.textContent = this._value;
  }
});
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    document.querySelector('plain-field').value;
});
"##
        .to_string(),
    ]);

    let result = act_instruction(&env, "Enter Grace Hopper into project lead and press Save.");
    let output = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(output["result"], "Grace Hopper");
    assert_eq!(result["plan"]["action"], "sequence");
    assert_eq!(result["plan"]["steps"][0]["action"], "type");
    assert_eq!(result["plan"]["steps"][1]["action"], "click");
    assert!(result["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["selector"] == "#custom-a"
            && element["affordances"]["fillable"] == true
            && element["label"]
                .as_str()
                .unwrap_or("")
                .contains("Project lead")));
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_externally_labeled_custom_checked_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-external-label-custom-checked");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="custom-b">Beta alerts</label>
  <plain-toggle id="custom-b"></plain-toggle>
  <button id="save">Save</button>
  <output id="out"></output>
`;
customElements.define('plain-toggle', class extends HTMLElement {
  constructor() {
    super();
    this._checked = false;
  }
  connectedCallback() {
    this.style.display = 'inline-block';
    this.style.width = '180px';
    this.style.height = '32px';
    this.style.border = '1px solid rgb(80, 80, 80)';
    this.textContent = 'Off';
  }
  get checked() {
    return this._checked;
  }
  set checked(next) {
    this._checked = Boolean(next);
    this.textContent = this._checked ? 'On' : 'Off';
  }
});
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    String(document.querySelector('plain-toggle').checked);
});
"##
        .to_string(),
    ]);

    let result = act_instruction(&env, "Turn on beta alerts and press Save.");
    let output = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(output["result"], "true");
    assert_eq!(result["plan"]["action"], "sequence");
    assert_eq!(result["plan"]["steps"][0]["action"], "set_checked");
    assert_eq!(result["plan"]["steps"][1]["action"], "click");
    assert!(result["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["selector"] == "#custom-b"
            && element["affordances"]["checkable"] == true
            && element["label"]
                .as_str()
                .unwrap_or("")
                .contains("Beta alerts")));
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_skips_disabled_custom_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-disabled-custom-value-host");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="locked-owner">Project lead</label>
  <plain-field id="locked-owner" disabled></plain-field>
  <label for="active-owner">Project lead</label>
  <plain-field id="active-owner"></plain-field>
  <button id="save">Save</button>
  <output id="out"></output>
`;
customElements.define('plain-field', class extends HTMLElement {
  constructor() {
    super();
    this._value = '';
  }
  connectedCallback() {
    this.style.display = 'inline-block';
    this.style.width = '180px';
    this.style.height = '32px';
    this.style.border = '1px solid rgb(80, 80, 80)';
  }
  get value() {
    return this._value;
  }
  set value(next) {
    this._value = String(next);
    this.textContent = this._value;
  }
});
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    locked: document.querySelector('#locked-owner').value,
    active: document.querySelector('#active-owner').value
  });
});
"##
        .to_string(),
    ]);

    let result = act_instruction(&env, "Enter Grace Hopper into project lead and press Save.");
    let output = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value = serde_json::from_str(output["result"].as_str().unwrap_or("{}"))
        .expect("parse custom value state");

    assert_eq!(state["locked"], "");
    assert_eq!(state["active"], "Grace Hopper");
    assert_eq!(result["plan"]["action"], "sequence");
    assert_eq!(result["plan"]["steps"][0]["action"], "type");
    assert_eq!(
        result["plan"]["steps"][0]["params"]["selector"],
        "#active-owner"
    );
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_skips_disabled_custom_checked_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-disabled-custom-checked-host");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="locked-alerts">Beta alerts</label>
  <plain-toggle id="locked-alerts" disabled></plain-toggle>
  <label for="active-alerts">Beta alerts</label>
  <plain-toggle id="active-alerts"></plain-toggle>
  <button id="save">Save</button>
  <output id="out"></output>
`;
customElements.define('plain-toggle', class extends HTMLElement {
  constructor() {
    super();
    this._checked = false;
  }
  connectedCallback() {
    this.style.display = 'inline-block';
    this.style.width = '180px';
    this.style.height = '32px';
    this.style.border = '1px solid rgb(80, 80, 80)';
    this.textContent = 'Off';
  }
  get checked() {
    return this._checked;
  }
  set checked(next) {
    this._checked = Boolean(next);
    this.textContent = this._checked ? 'On' : 'Off';
  }
});
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    locked: document.querySelector('#locked-alerts').checked,
    active: document.querySelector('#active-alerts').checked
  });
});
"##
        .to_string(),
    ]);

    let result = act_instruction(&env, "Turn on beta alerts and press Save.");
    let output = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value = serde_json::from_str(output["result"].as_str().unwrap_or("{}"))
        .expect("parse custom checked state");

    assert_eq!(state["locked"], false);
    assert_eq!(state["active"], true);
    assert_eq!(result["plan"]["action"], "sequence");
    assert_eq!(result["plan"]["steps"][0]["action"], "set_checked");
    assert_eq!(
        result["plan"]["steps"][0]["params"]["selector"],
        "#active-alerts"
    );
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_slotted_component_button_labels_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-slotted-button-label");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <action-button data-action="archive">
    <span slot="label">Archive</span>
  </action-button>
  <output id="out"></output>
`;
customElements.define('action-button', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `<button id="action"><slot name="label"></slot></button>`;
    root.querySelector('button').addEventListener('click', () => {
      document.querySelector('#out').textContent = this.dataset.action;
    });
  }
});
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click Archive.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "archive");
    assert!(click["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["selector"] == "#action"
            && element["text"].as_str().unwrap_or("").contains("Archive")
            && element["context"]["kind"] == "shadow-root"));
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_svg_symbol_title_buttons_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-svg-symbol-title-button");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg width="0" height="0" style="position:absolute">
    <defs>
      <symbol id="action-42" viewBox="0 0 16 16">
        <title>Archive</title>
        <path d="M1 3h14v10H1z"></path>
      </symbol>
    </defs>
  </svg>
  <button id="archive-button">
    <svg aria-hidden="true" width="16" height="16">
      <use href="#action-42"></use>
    </svg>
  </button>
  <output id="out"></output>
`;
document.querySelector('#archive-button').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'archive';
});
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click Archive.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "archive");
    assert!(click["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["selector"] == "#archive-button"
            && element["text"].as_str().unwrap_or("").contains("Archive")));
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_page_provided_semantic_metadata_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-page-semantic-metadata");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="billing" data-aliases="invoice payment receipt">Billing</button>
  <button id="support">Support</button>
  <output id="out"></output>
`;
document.querySelector('#billing').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'billing';
});
document.querySelector('#support').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'support';
});
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click the invoice action.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "billing");
    assert_eq!(click["plan"]["params"]["selector"], "#billing");
    assert!(click["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["selector"] == "#billing"
            && element["text"].as_str().unwrap_or("").contains("invoice")));
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_same_origin_iframe_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-iframe-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <iframe id="settings-frame" title="Settings"></iframe>
  <output id="out"></output>
`;
const frame = document.querySelector('#settings-frame');
const doc = frame.contentDocument;
doc.open();
doc.write(`
  <label for="display">Display name</label>
  <input id="display" name="display_name">
  <button id="save">Save</button>
`);
doc.close();
doc.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent = doc.querySelector('#display').value;
});
"##
        .to_string(),
    ]);

    let frame = act_instruction(&env, "Enter Frame User into Display name and press Save.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "Frame User");
    assert_eq!(frame["plan"]["action"], "sequence");
    assert_eq!(frame["plan"]["steps"][0]["action"], "type");
    assert_eq!(frame["plan"]["steps"][1]["action"], "click");
    assert!(frame["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["selector"] == "#display"
            && element["label"] == "Display name"
            && element["context"]["kind"] == "iframe"));
    assert_eq!(frame["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_accessible_names_for_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-accessible-control-names");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="settings">
    <span id="contact-label">Public contact</span>
    <input id="public-contact" name="field_a" aria-labelledby="contact-label">
    <span id="internal-label">Internal alias</span>
    <input id="internal-alias" name="field_b" aria-labelledby="internal-label">

    <span id="tier-label">Support tier</span>
    <span id="tier-help">Plan used for customer routing</span>
    <select id="support-tier" name="choice_a" aria-labelledby="tier-label" aria-describedby="tier-help">
      <option>Starter</option>
      <option>Enterprise</option>
    </select>
    <span id="billing-label">Billing tier</span>
    <select id="billing-tier" name="choice_b" aria-labelledby="billing-label">
      <option>Starter</option>
      <option>Enterprise</option>
    </select>

    <span id="receipts-label">Send receipts</span>
    <button id="send-receipts" type="button" role="switch" aria-checked="false" aria-labelledby="receipts-label">Off</button>
    <span id="digest-label">Send digest</span>
    <button id="send-digest" type="button" role="switch" aria-checked="false" aria-labelledby="digest-label">Off</button>
  </form>
`;
for (const button of document.querySelectorAll('[role=switch]')) {
  button.addEventListener('click', () => {
    button.setAttribute('aria-checked', 'true');
    button.textContent = 'On';
  });
}
"##
        .to_string(),
    ]);

    let fill = act_instruction(&env, "Fill Public contact with Ada Lovelace.");
    let select = act_instruction(&env, "Choose Enterprise from the Support tier dropdown.");
    let check = act_instruction(&env, "Check Send receipts switch.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({contact: document.querySelector('#public-contact').value, internal: document.querySelector('#internal-alias').value, support: document.querySelector('#support-tier').value, billing: document.querySelector('#billing-tier').value, receipts: document.querySelector('#send-receipts').getAttribute('aria-checked'), digest: document.querySelector('#send-digest').getAttribute('aria-checked')})".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse accessible control state");
    assert_eq!(state["contact"], "Ada Lovelace");
    assert_eq!(state["internal"], "");
    assert_eq!(state["support"], "Enterprise");
    assert_eq!(state["billing"], "Starter");
    assert_eq!(state["receipts"], "true");
    assert_eq!(state["digest"], "false");
    assert_eq!(fill["plan"]["params"]["selector"], "#public-contact");
    assert_eq!(select["plan"]["params"]["selector"], "#support-tier");
    assert_eq!(check["plan"]["params"]["selector"], "#send-receipts");
    assert!(fill["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["selector"] == "#public-contact"
            && element["label"] == "Public contact"));
    assert_eq!(fill["verification"]["status"], "observed");
    assert_eq!(select["verification"]["status"], "observed");
    assert_eq!(check["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_nearby_visible_text_as_field_labels_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-nearby-field-labels");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="profile">
    <div class="field-row"><span>Account owner</span><input id="owner" name="field_a"></div>
    <div class="field-row"><span>Project code</span><input id="code" name="field_b"></div>
    <div class="field-row">
      <span>Visibility</span>
      <select id="visibility" name="field_c">
        <option>Public</option>
        <option>Private</option>
      </select>
    </div>
    <button id="save" type="button">Save</button>
    <output id="out"></output>
  </form>
`;
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    owner: document.querySelector('#owner').value,
    code: document.querySelector('#code').value,
    visibility: document.querySelector('#visibility').value
  });
});
"##
        .to_string(),
    ]);

    let fill = act_instruction(
        &env,
        "Set Account owner to Priya; set Project code to PX-42; set Visibility to Private; click Save.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse nearby-label form state");
    assert_eq!(state["owner"], "Priya");
    assert_eq!(state["code"], "PX-42");
    assert_eq!(state["visibility"], "Private");
    assert_eq!(fill["plan"]["action"], "sequence");
    assert!(fill["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |step| step["action"] == "select_option" && step["params"]["selector"] == "#visibility"
        ));
    assert!(fill["pageModel"]["before"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["selector"] == "#owner"
            && element["label"]
                .as_str()
                .unwrap_or("")
                .contains("Account owner")));
    assert_eq!(fill["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_contenteditable_editors_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-contenteditable-editor");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="editor">Message body</label>
  <div id="editor" role="textbox" contenteditable="true" aria-label="Message body"></div>
  <button id="send">Send</button>
  <output id="out"></output>
`;
const editor = document.querySelector('#editor');
editor.addEventListener('input', () => {
  editor.dataset.inputSeen = 'true';
});
document.querySelector('#send').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    text: editor.textContent,
    inputSeen: editor.dataset.inputSeen === 'true'
  });
});
"##
        .to_string(),
    ]);

    let fill = act_instruction(
        &env,
        "Write \"Launch update approved\" into the Message body editor and press Send.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse editor state");
    assert_eq!(state["text"], "Launch update approved");
    assert_eq!(state["inputSeen"], true);
    assert_eq!(fill["plan"]["action"], "sequence");
    assert_eq!(fill["plan"]["steps"][0]["action"], "type");
    assert_eq!(
        fill["result"]["steps"][0]["typed"]["kind"],
        "contenteditable"
    );
    assert_eq!(fill["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_plaintext_only_contenteditable_editors_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-plaintext-contenteditable-editor");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="notes">Internal notes</label>
  <div id="notes" contenteditable="plaintext-only" tabindex="0"></div>
  <button id="save">Save</button>
  <output id="out"></output>
`;
const notes = document.querySelector('#notes');
notes.addEventListener('input', () => {
  notes.dataset.inputSeen = 'true';
});
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    text: notes.textContent,
    inputSeen: notes.dataset.inputSeen === 'true'
  });
});
"##
        .to_string(),
    ]);

    let fill = act_instruction(
        &env,
        "Enter \"Escalate after review\" into Internal notes and click Save.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse plaintext editor state");
    assert_eq!(state["text"], "Escalate after review");
    assert_eq!(state["inputSeen"], true);
    assert_eq!(fill["plan"]["action"], "sequence");
    assert_eq!(fill["plan"]["steps"][0]["action"], "type");
    assert_eq!(
        fill["result"]["steps"][0]["typed"]["kind"],
        "contenteditable"
    );
    assert_eq!(fill["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_aria_searchbox_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-aria-searchbox-field");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="site-search" role="searchbox" aria-label="Site search" tabindex="0"></div>
  <button id="submit">Search</button>
  <output id="out"></output>
`;
const search = document.querySelector('#site-search');
search.addEventListener('input', () => {
  search.dataset.inputSeen = 'true';
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    text: search.textContent,
    inputSeen: search.dataset.inputSeen === 'true'
  });
});
"##
        .to_string(),
    ]);

    let fill = act_instruction(
        &env,
        "Type \"release notes\" into the Site search field and click Search.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse searchbox state");
    assert_eq!(state["text"], "release notes");
    assert_eq!(state["inputSeen"], true);
    assert_eq!(fill["plan"]["action"], "form_workflow");
    assert_eq!(
        fill["result"]["formWorkflow"]["filled"][0]["selector"],
        "#site-search"
    );
    assert_eq!(
        fill["result"]["formWorkflow"]["submitted"]["selector"],
        "#submit"
    );
    assert_eq!(fill["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_custom_aria_combobox_text_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-aria-combobox-field");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="destination" role="combobox" aria-label="Destination" aria-autocomplete="list" tabindex="0" style="min-height: 20px; width: 220px; border: 1px solid #888;"></div>
  <button id="go">Go</button>
  <output id="out"></output>
`;
const destination = document.querySelector('#destination');
destination.addEventListener('input', () => {
  destination.dataset.inputSeen = 'true';
});
document.querySelector('#go').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    text: destination.textContent,
    inputSeen: destination.dataset.inputSeen === 'true'
  });
});
"##
        .to_string(),
    ]);

    let fill = act_instruction(
        &env,
        "Type \"Value Delta\" into the Destination combobox and click Go.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse combobox state");
    assert_eq!(state["text"], "Value Delta");
    assert_eq!(state["inputSeen"], true);
    match fill["plan"]["action"].as_str() {
        Some("sequence") => match fill["plan"]["steps"][0]["action"].as_str() {
            Some("type") => {
                assert_eq!(
                    fill["result"]["steps"][0]["typed"]["kind"],
                    "contenteditable"
                );
            }
            Some("autocomplete_select") => {
                assert_eq!(
                    fill["result"]["steps"][0]["autocomplete"]["selected"],
                    "Value Delta"
                );
            }
            other => panic!("unexpected first sequence action for custom combobox: {other:?}"),
        },
        Some("form_workflow") => {
            assert_eq!(
                fill["result"]["formWorkflow"]["filled"][0]["selector"],
                "#destination"
            );
            assert_eq!(
                fill["result"]["formWorkflow"]["submitted"]["selector"],
                "#go"
            );
        }
        other => panic!("unexpected plan action for custom combobox: {other:?}"),
    }
    assert_eq!(fill["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_custom_aria_spinbutton_values_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-aria-spinbutton");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="locked-quantity" role="spinbutton" aria-label="Quantity" aria-readonly="true" aria-valuemin="1" aria-valuemax="10" aria-valuenow="3" tabindex="0">3</div>
  <div id="quantity" role="spinbutton" aria-label="Quantity" aria-valuemin="1" aria-valuemax="10" aria-valuenow="1" tabindex="0">1</div>
  <button id="apply">Apply</button>
  <output id="out"></output>
`;
const quantity = document.querySelector('#quantity');
const lockedQuantity = document.querySelector('#locked-quantity');
quantity.addEventListener('input', () => {
  quantity.dataset.inputSeen = 'true';
});
document.querySelector('#apply').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    value: quantity.getAttribute('aria-valuenow'),
    lockedValue: lockedQuantity.getAttribute('aria-valuenow'),
    inputSeen: quantity.dataset.inputSeen === 'true'
  });
});
"##
        .to_string(),
    ]);

    let spin = act_instruction(&env, "Set 7 with the Quantity spinbutton and click Apply.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse spinbutton state");
    assert_eq!(state["value"], "7");
    assert_eq!(state["lockedValue"], "3");
    assert_eq!(state["inputSeen"], true);
    assert_eq!(spin["plan"]["action"], "sequence");
    assert_eq!(spin["plan"]["steps"][0]["action"], "type");
    assert_eq!(spin["plan"]["steps"][0]["params"]["selector"], "#quantity");
    assert_eq!(spin["result"]["steps"][0]["typed"]["kind"], "spinbutton");
    assert_eq!(spin["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_custom_value_host_steppers_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-value-stepper");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
customElements.define('quantity-stepper', class extends HTMLElement {
  constructor() {
    super();
    this._value = 1;
  }
  connectedCallback() {
    this.tabIndex = 0;
    this.style.display = 'inline-block';
    this.style.minWidth = '60px';
    this.style.padding = '4px';
    this.style.border = '1px solid #555';
    this.textContent = String(this._value);
  }
  get value() {
    return String(this._value);
  }
  set value(next) {
    const min = Number(this.getAttribute('min') || '0');
    const max = Number(this.getAttribute('max') || '99');
    const numeric = Math.max(min, Math.min(max, Number(next)));
    this._value = numeric;
    this.textContent = String(numeric);
    this.setAttribute('data-current-value', String(numeric));
  }
});
document.body.innerHTML = `
  <label id="quantity-label" for="quantity">Quantity stepper</label>
  <quantity-stepper id="quantity" aria-labelledby="quantity-label" data-field="quantity-stepper" min="1" max="10"></quantity-stepper>
  <button id="apply">Apply</button>
  <output id="out"></output>
`;
const quantity = document.querySelector('#quantity');
quantity.addEventListener('input', () => {
  quantity.dataset.inputSeen = 'true';
});
document.querySelector('#apply').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    value: quantity.value,
    text: quantity.textContent,
    currentValue: quantity.getAttribute('data-current-value'),
    inputSeen: quantity.dataset.inputSeen === 'true'
  });
});
"##
        .to_string(),
    ]);

    let spin = act_instruction(&env, "Set Quantity stepper to 7 and click Apply.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse custom value stepper state");
    assert_eq!(state["value"], "7");
    assert_eq!(state["text"], "7");
    assert_eq!(state["currentValue"], "7");
    assert_eq!(state["inputSeen"], true);
    assert_eq!(spin["plan"]["action"], "sequence");
    assert_eq!(spin["plan"]["steps"][0]["action"], "type");
    assert_eq!(spin["plan"]["steps"][0]["params"]["selector"], "#quantity");
    assert_eq!(
        spin["result"]["steps"][0]["typed"]["valueResult"]["kind"],
        "custom-number"
    );
    assert_eq!(spin["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_highlights_visible_text_blocks_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-select-text-block");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <article id="story">
    <p>Alpha first paragraph.</p>
    <p>Beta second paragraph.</p>
    <p>Gamma third paragraph.</p>
  </article>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = window.getSelection().toString();
});
"##
        .to_string(),
    ]);

    let highlight = act_instruction(
        &env,
        "Highlight the text in the 2nd paragraph and click submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "Beta second paragraph.");
    assert_eq!(highlight["plan"]["action"], "sequence");
    assert_eq!(highlight["plan"]["capability"]["name"], "text-selection");
    assert_eq!(highlight["plan"]["steps"][0]["action"], "select_text");
    assert_eq!(
        highlight["result"]["steps"][0]["selection"]["selected"],
        "Beta second paragraph."
    );

    env.stop();
}

#[test]
fn act_instruction_applies_editor_toolbar_styles_after_selecting_text_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-editor-toolbar-style");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div class="ql-editor" contenteditable="true">Launch notes ready</div>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const editor = document.querySelector('.ql-editor');
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    text: editor.textContent,
    html: editor.innerHTML
  });
});
"##
        .to_string(),
    ]);

    let style = act_instruction(
        &env,
        "Using the text editor, give everything the style bold and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse editor formatting state");

    assert_eq!(state["text"], "Launch notes ready");
    assert!(
        state["html"].as_str().unwrap_or("").contains("<b>")
            || state["html"].as_str().unwrap_or("").contains("<strong>")
    );
    assert_eq!(style["plan"]["action"], "sequence");
    assert_eq!(style["plan"]["capability"]["name"], "text-selection");
    assert_eq!(style["plan"]["steps"][0]["action"], "format_text");

    env.stop();
}

#[test]
fn act_instruction_uploads_files_to_file_inputs_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-file-upload");
    fs::create_dir_all(&env.home).expect("create temp home");
    let upload_path = env.home.join("resume.txt");
    fs::write(&upload_path, "agent upload proof").expect("write upload file");
    let upload_path = upload_path.to_string_lossy().to_string();

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="resume">Resume document</label>
  <input id="resume" name="resume_document" type="file" style="display:none">
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelector('#submit').addEventListener('click', () => {
  const file = document.querySelector('#resume').files[0];
  document.querySelector('#out').textContent = JSON.stringify({
    name: file ? file.name : '',
    count: document.querySelector('#resume').files.length
  });
});
"##
        .to_string(),
    ]);

    let upload = act_instruction(
        &env,
        &format!(
            "Upload \"{}\" to the Resume document field and press Submit.",
            upload_path
        ),
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse upload state");
    assert_eq!(state["name"], "resume.txt");
    assert_eq!(state["count"], 1);
    assert_eq!(upload["plan"]["action"], "sequence");
    assert_eq!(upload["plan"]["steps"][0]["action"], "upload_file");
    assert_eq!(upload["plan"]["capability"]["name"], "file-upload");
    assert_eq!(upload["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uploads_files_through_custom_dropzones_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-dropzone-upload");
    fs::create_dir_all(&env.home).expect("create temp home");
    let upload_path = env.home.join("avatar.txt");
    fs::write(&upload_path, "avatar upload proof").expect("write upload file");
    let upload_path = upload_path.to_string_lossy().to_string();

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="resume-card" data-dropzone="resume document upload">
    <p>Resume document drop zone</p>
    <input id="resume-file" type="file" hidden>
  </section>
  <section id="avatar-card" data-dropzone="avatar image upload">
    <p>Avatar image drop zone</p>
    <input id="avatar-file" type="file" hidden>
  </section>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelector('#submit').addEventListener('click', () => {
  const avatar = document.querySelector('#avatar-file').files[0];
  const resume = document.querySelector('#resume-file').files[0];
  document.querySelector('#out').textContent = JSON.stringify({
    avatarName: avatar ? avatar.name : '',
    avatarCount: document.querySelector('#avatar-file').files.length,
    resumeCount: document.querySelector('#resume-file').files.length
  });
});
"##
        .to_string(),
    ]);

    let upload = act_instruction(
        &env,
        &format!(
            "Upload \"{}\" to the Avatar image drop zone and press Submit.",
            upload_path
        ),
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse custom dropzone upload state");
    assert_eq!(state["avatarName"], "avatar.txt");
    assert_eq!(state["avatarCount"], 1);
    assert_eq!(state["resumeCount"], 0);
    assert_eq!(upload["plan"]["action"], "sequence");
    assert_eq!(upload["plan"]["steps"][0]["action"], "upload_file");
    assert_eq!(
        upload["plan"]["steps"][0]["params"]["selector"],
        "#avatar-file"
    );
    assert_eq!(upload["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn upload_file_resolves_proxy_dropzone_selectors_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("upload-file-proxy-dropzone");
    fs::create_dir_all(&env.home).expect("create temp home");
    let upload_path = env.home.join("photo.txt");
    fs::write(&upload_path, "proxy upload proof").expect("write upload file");
    let upload_path = upload_path.to_string_lossy().to_string();

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="photo-dropzone" data-dropzone="profile photo upload">
    <span>Profile photo drop zone</span>
    <input id="photo-file" type="file" hidden>
  </div>
  <output id="out"></output>
`;
document.querySelector('#photo-file').addEventListener('change', () => {
  const file = document.querySelector('#photo-file').files[0];
  document.querySelector('#out').textContent = JSON.stringify({
    name: file ? file.name : '',
    count: document.querySelector('#photo-file').files.length
  });
});
"##
        .to_string(),
    ]);

    let upload = env.json(&[
        "upload-file".to_string(),
        "#photo-dropzone".to_string(),
        upload_path,
    ]);
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse proxy dropzone upload state");
    assert_eq!(state["name"], "photo.txt");
    assert_eq!(state["count"], 1);
    assert_eq!(upload["uploaded"]["selector"], "#photo-dropzone");

    env.stop();
}

#[test]
fn act_instruction_exercises_mixed_field_control_matrix_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-field-control-matrix");
    fs::create_dir_all(&env.home).expect("create temp home");
    let upload_path = env.home.join("attachment.txt");
    fs::write(&upload_path, "matrix upload proof").expect("write upload file");
    let upload_path = upload_path.to_string_lossy().to_string();

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="matrix">
    <label>Display name <input id="display-name"></label>
    <label>Notes <textarea id="notes"></textarea></label>
    <div id="message-body" role="textbox" contenteditable="true" aria-label="Message body"></div>
    <label>Plan
      <select id="plan">
        <option>Basic</option>
        <option>Pro</option>
        <option>Enterprise</option>
      </select>
    </label>
    <label><input id="updates" type="checkbox"> Receive updates</label>
    <fieldset>
      <legend>Contact method</legend>
      <label><input id="contact-phone" type="radio" name="contact" value="Phone"> Phone</label>
      <label><input id="contact-email" type="radio" name="contact" value="Email"> Email</label>
    </fieldset>
    <button id="alerts" type="button" role="switch" aria-checked="false">Alerts</button>
    <label>Completion slider <input id="completion" type="range" min="0" max="100" value="10"></label>
    <div id="quantity" role="spinbutton" aria-label="Quantity" aria-valuemin="0" aria-valuemax="20" aria-valuenow="1" tabindex="0">1</div>
    <button id="routing" type="button" role="combobox" aria-haspopup="listbox" aria-controls="routing-options" aria-expanded="false">Routing: Automatic</button>
    <div id="routing-options" role="listbox" hidden>
      <div role="option" data-value="automatic">Automatic</div>
      <div role="option" data-value="manual">Manual</div>
    </div>
    <label for="attachment">Attachment file</label>
    <input id="attachment" type="file" style="display:none">
    <button id="submit" type="submit">Submit</button>
  </form>
  <output id="out"></output>
`;
document.querySelector('#alerts').addEventListener('click', event => {
  const button = event.currentTarget;
  button.setAttribute('aria-checked', String(button.getAttribute('aria-checked') !== 'true'));
});
document.querySelector('#quantity').addEventListener('input', event => {
  const value = event.currentTarget.getAttribute('aria-valuenow');
  event.currentTarget.textContent = value;
});
document.querySelector('#routing').addEventListener('click', () => {
  document.querySelector('#routing-options').hidden = false;
  routing.setAttribute('aria-expanded', 'true');
});
document.querySelectorAll('#routing-options [role=option]').forEach(option => {
  option.addEventListener('click', () => {
    document.querySelectorAll('#routing-options [role=option]').forEach(peer => peer.setAttribute('aria-selected', 'false'));
    option.setAttribute('aria-selected', 'true');
    routing.dataset.value = option.dataset.value;
    routing.textContent = 'Routing: ' + option.textContent;
    document.querySelector('#routing-options').hidden = true;
    routing.setAttribute('aria-expanded', 'false');
  });
});
matrix.addEventListener('submit', event => {
  event.preventDefault();
  const file = document.querySelector('#attachment').files[0];
  out.textContent = JSON.stringify({
    displayName: document.querySelector('#display-name').value,
    notes: document.querySelector('#notes').value,
    message: document.querySelector('#message-body').textContent,
    plan: document.querySelector('#plan').value,
    updates: document.querySelector('#updates').checked,
    contact: document.querySelector('input[name=contact]:checked')?.value || '',
    alerts: document.querySelector('#alerts').getAttribute('aria-checked'),
    completion: document.querySelector('#completion').value,
    quantity: document.querySelector('#quantity').getAttribute('aria-valuenow'),
    routing: document.querySelector('#routing').dataset.value || 'automatic',
    attachmentName: file ? file.name : '',
    attachmentCount: document.querySelector('#attachment').files.length
  });
});
"##
        .to_string(),
    ]);

    let display_name = act_instruction(&env, "Set Display name to Rowan.");
    let notes = act_instruction(&env, "Set Notes to Matrix note.");
    let message = act_instruction(&env, "Set Message body to Ready for review.");
    let plan = act_instruction(&env, "Choose Pro from the Plan dropdown.");
    let updates = act_instruction(&env, "Check Receive updates.");
    let contact = act_instruction(&env, "Choose Email from Contact method.");
    let alerts = act_instruction(&env, "Turn on Alerts switch.");
    let completion = act_instruction(&env, "Set Completion slider to 80.");
    let quantity = act_instruction(&env, "Set Quantity spinbutton to 4.");
    let routing = act_instruction(&env, "Choose Manual from the Routing listbox.");
    let upload = act_instruction(
        &env,
        &format!(
            "Upload \"{}\" to the Attachment file field and press Submit.",
            upload_path
        ),
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse mixed field control matrix state");
    assert_eq!(state["displayName"], "Rowan");
    assert_eq!(state["notes"], "Matrix note");
    assert_eq!(state["message"], "Ready for review");
    assert_eq!(state["plan"], "Pro");
    assert_eq!(state["updates"], true);
    assert_eq!(state["contact"], "Email");
    assert_eq!(state["alerts"], "true");
    assert_eq!(state["completion"], "80");
    assert_eq!(state["quantity"], "4");
    assert_eq!(state["routing"], "manual");
    assert_eq!(state["attachmentName"], "attachment.txt");
    assert_eq!(state["attachmentCount"], 1);

    for action in [
        display_name,
        notes,
        message,
        plan,
        updates,
        contact,
        alerts,
        completion,
        quantity,
        routing,
        upload,
    ] {
        assert_eq!(action["verification"]["status"], "observed");
    }

    env.stop();
}

#[test]
fn act_instruction_uploads_files_inside_scoped_multi_action_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-file-upload");
    fs::create_dir_all(&env.home).expect("create temp home");
    let billing_path = env.home.join("billing.pdf");
    let support_path = env.home.join("support.pdf");
    fs::write(&billing_path, "billing proof").expect("write billing upload file");
    fs::write(&support_path, "support proof").expect("write support upload file");
    let support_path = support_path.to_string_lossy().to_string();

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-section" aria-label="Billing">
      <h2>Billing</h2>
      <label for="billing-resume">Resume document</label>
      <input id="billing-resume" name="billing_resume" type="file" style="display:none">
      <button id="billing-save">Save</button>
    </section>
    <section id="support-section" aria-label="Support">
      <h2>Support</h2>
      <label for="support-resume">Resume document</label>
      <input id="support-resume" name="support_resume" type="file" style="display:none">
      <button id="support-save">Save</button>
    </section>
  </main>
  <button id="global-save">Save</button>
  <output id="out"></output>
`;
for (const section of document.querySelectorAll('section')) {
  const name = section.querySelector('h2').textContent;
  section.querySelector('button[id$="-save"]').addEventListener('click', () => {
    const file = section.querySelector('input[type=file]').files[0];
    document.querySelector('#out').textContent = JSON.stringify({
      section: name,
      name: file ? file.name : '',
      count: section.querySelector('input[type=file]').files.length
    });
  });
}
document.querySelector('#global-save').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'global saved';
});
"##
        .to_string(),
    ]);

    let action = act_instruction(
        &env,
        &format!("Support: Resume document: {}; Save.", support_path),
    );
    let result = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  billingCount: document.querySelector('#billing-resume').files.length,
  supportCount: document.querySelector('#support-resume').files.length,
  out: document.querySelector('#out').textContent
})
"##
        .to_string(),
    ]);
    let values: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped upload state");
    let out: Value =
        serde_json::from_str(values["out"].as_str().unwrap_or("{}")).expect("parse save output");

    assert_eq!(values["billingCount"], 0);
    assert_eq!(values["supportCount"], 1);
    assert_eq!(out["section"], "Support");
    assert_eq!(out["name"], "support.pdf");
    assert_eq!(out["count"], 1);
    assert_eq!(action["plan"]["capability"]["name"], "scoped-multi-action");
    assert_eq!(action["plan"]["steps"][0]["action"], "upload_file");
    assert_eq!(
        action["plan"]["steps"][0]["params"]["selector"],
        "#support-resume"
    );
    assert_eq!(
        action["plan"]["steps"][1]["params"]["selector"],
        "#support-save"
    );
    assert_eq!(action["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn upload_file_uses_shadow_dom_file_inputs_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("upload-file-shadow-input");
    fs::create_dir_all(&env.home).expect("create temp home");
    let upload_path = env.home.join("avatar.txt");
    fs::write(&upload_path, "shadow upload proof").expect("write upload file");
    let upload_path = upload_path.to_string_lossy().to_string();

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <input id="decoy-file" type="file">
  <avatar-uploader></avatar-uploader>
  <output id="out"></output>
`;
customElements.define('avatar-uploader', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <label for="avatar-file">Avatar file</label>
      <input id="avatar-file" type="file">
    `;
    root.querySelector('#avatar-file').addEventListener('change', () => {
      const file = root.querySelector('#avatar-file').files[0];
      document.querySelector('#out').textContent = JSON.stringify({
        name: file ? file.name : '',
        count: root.querySelector('#avatar-file').files.length,
        decoyCount: document.querySelector('#decoy-file').files.length
      });
    });
  }
});
"##
        .to_string(),
    ]);

    let upload = env.json(&[
        "upload-file".to_string(),
        "#avatar-file".to_string(),
        upload_path,
    ]);
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse shadow upload state");
    assert_eq!(state["name"], "avatar.txt");
    assert_eq!(state["count"], 1);
    assert_eq!(state["decoyCount"], 0);
    assert_eq!(upload["uploaded"]["selector"], "#avatar-file");

    env.stop();
}

#[test]
fn act_instruction_presses_keyboard_keys_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-key-press");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label>Search <input id="search" value="ready"></label>
  <output id="out"></output>
`;
const input = document.querySelector('#search');
input.focus();
input.addEventListener('keydown', event => {
  if (event.key === 'Enter') {
    document.querySelector('#out').textContent = `submitted:${input.value}`;
  }
});
"##
        .to_string(),
    ]);

    let press = act_instruction(&env, "Press Enter.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "submitted:ready");
    assert_eq!(press["plan"]["action"], "press");
    assert_eq!(press["result"]["pressed"], "Enter");
    assert_eq!(press["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_hovers_targets_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-hover");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <nav>
    <button id="account" aria-label="Account menu">Account</button>
    <div id="menu" hidden>Profile Settings</div>
  </nav>
  <output id="out"></output>
`;
const account = document.querySelector('#account');
account.addEventListener('mouseover', () => {
  document.querySelector('#menu').hidden = false;
  document.querySelector('#out').textContent = 'hovered';
});
"##
        .to_string(),
    ]);

    let hover = act_instruction(&env, "Hover over the Account menu.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({out: document.querySelector('#out').textContent, hidden: document.querySelector('#menu').hidden})".to_string(),
    ]);

    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse hover state");
    assert_eq!(state["out"], "hovered");
    assert_eq!(state["hidden"], false);
    assert_eq!(hover["plan"]["action"], "hover");
    assert_eq!(hover["plan"]["candidate"]["selector"], "#account");
    assert_eq!(hover["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clears_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-clear-field");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label>Search field <input id="search" value="old query"></label>
  <output id="out"></output>
`;
const search = document.querySelector('#search');
search.addEventListener('input', () => {
  document.querySelector('#out').textContent = search.value;
});
"##
        .to_string(),
    ]);

    let clear = act_instruction(&env, "Clear the Search field.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#search').value, out: document.querySelector('#out').textContent})".to_string(),
    ]);

    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse clear state");
    assert_eq!(state["value"], "");
    assert_eq!(state["out"], "");
    assert_eq!(clear["plan"]["action"], "type");
    assert_eq!(clear["plan"]["params"]["clear_first"], true);
    assert_eq!(clear["result"]["typed"]["actual"], "");
    assert_eq!(clear["result"]["typed"]["fill"]["expected"], "");
    assert_eq!(clear["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_appends_to_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-append-field");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label>Notes field <textarea id="notes">alpha </textarea></label>
  <output id="out"></output>
`;
const notes = document.querySelector('#notes');
notes.addEventListener('input', () => {
  document.querySelector('#out').textContent = notes.value;
});
"##
        .to_string(),
    ]);

    let append = act_instruction(&env, "Append beta to the Notes field.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#notes').value, out: document.querySelector('#out').textContent})".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse append state");
    assert_eq!(state["value"], "alpha beta");
    assert_eq!(state["out"], "alpha beta");
    assert_eq!(append["plan"]["action"], "type");
    assert_eq!(append["plan"]["params"]["clear_first"], false);
    assert_eq!(append["plan"]["params"]["slowly"], true);
    assert_eq!(append["result"]["typed"]["actual"], "alpha beta");
    assert_eq!(append["result"]["typed"]["fill"]["expected"], "alpha beta");
    assert_eq!(append["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_edits_child_fields_inside_matching_containers_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-field-edit");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="records">
    <div class="row" role="listitem" data-name="Alice">
      <span>Alice</span>
      <label>Notes <textarea id="alice-locked-notes" readonly>locked alpha</textarea></label>
      <label>Notes <textarea id="alice-notes">alpha</textarea></label>
    </div>
    <div class="row" role="listitem" data-name="Bob">
      <span>Bob</span>
      <fieldset disabled>
        <label>Notes <textarea id="bob-disabled-notes">disabled omega</textarea></label>
      </fieldset>
      <label>Notes <textarea id="bob-notes">omega </textarea></label>
    </div>
  </section>
  <output id="out"></output>
`;
function update() {
  document.querySelector('#out').textContent = JSON.stringify({
    aliceLocked: document.querySelector('#alice-locked-notes').value,
    alice: document.querySelector('#alice-notes').value,
    bobDisabled: document.querySelector('#bob-disabled-notes').value,
    bob: document.querySelector('#bob-notes').value
  });
}
document.querySelectorAll('textarea').forEach(textarea => textarea.addEventListener('input', update));
update();
"##
        .to_string(),
    ]);

    let clear = act_instruction(&env, "Clear the Notes field in the row containing Alice.");
    let append = act_instruction(
        &env,
        "Append done to the Notes field in the row containing Bob.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped edit state");
    assert_eq!(state["aliceLocked"], "locked alpha");
    assert_eq!(state["alice"], "");
    assert_eq!(state["bobDisabled"], "disabled omega");
    assert_eq!(state["bob"], "omega done");
    assert_eq!(clear["plan"]["capability"]["name"], "scoped-field-edit");
    assert_eq!(clear["plan"]["candidate"]["selector"], "#alice-notes");
    assert_eq!(clear["plan"]["evidence"]["mode"], "clear");
    assert_eq!(clear["plan"]["evidence"]["itemQuery"], "Alice");
    assert_eq!(clear["plan"]["evidence"]["fieldHint"], "Notes");
    assert_eq!(clear["verification"]["status"], "observed");
    assert_eq!(append["plan"]["capability"]["name"], "scoped-field-edit");
    assert_eq!(append["plan"]["candidate"]["selector"], "#bob-notes");
    assert_eq!(append["plan"]["evidence"]["mode"], "append");
    assert_eq!(append["plan"]["evidence"]["itemQuery"], "Bob");
    assert_eq!(append["plan"]["evidence"]["fieldHint"], "Notes");
    assert_eq!(append["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_edits_repeated_fields_inside_named_sections_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-named-section-field-edit");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="billing-panel" aria-label="Billing">
    <h2>Billing</h2>
    <label>Notes <textarea id="billing-locked-notes" readonly>locked billing</textarea></label>
    <label>Notes <textarea id="billing-notes">billing </textarea></label>
  </section>
  <section id="support-panel" aria-label="Support">
    <h2>Support</h2>
    <fieldset disabled>
      <label>Notes <textarea id="support-disabled-notes">disabled support</textarea></label>
    </fieldset>
    <label>Notes <textarea id="support-notes">support</textarea></label>
  </section>
  <output id="out"></output>
`;
function update() {
  document.querySelector('#out').textContent = JSON.stringify({
    billingLocked: document.querySelector('#billing-locked-notes').value,
    billing: document.querySelector('#billing-notes').value,
    supportDisabled: document.querySelector('#support-disabled-notes').value,
    support: document.querySelector('#support-notes').value
  });
}
document.querySelectorAll('textarea').forEach(textarea => textarea.addEventListener('input', update));
update();
"##
        .to_string(),
    ]);

    let clear = act_instruction(&env, "Clear Notes in the Support panel.");
    let append = act_instruction(&env, "Append resolved to Notes in the Billing section.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse named section scoped edit state");
    assert_eq!(state["billingLocked"], "locked billing");
    assert_eq!(state["billing"], "billing resolved");
    assert_eq!(state["supportDisabled"], "disabled support");
    assert_eq!(state["support"], "");
    assert_eq!(clear["plan"]["capability"]["name"], "scoped-field-edit");
    assert_eq!(clear["plan"]["candidate"]["selector"], "#support-notes");
    assert_eq!(clear["plan"]["evidence"]["mode"], "clear");
    assert_eq!(clear["plan"]["evidence"]["itemQuery"], "Support");
    assert_eq!(clear["plan"]["evidence"]["fieldHint"], "Notes");
    assert_eq!(clear["verification"]["status"], "observed");
    assert_eq!(append["plan"]["capability"]["name"], "scoped-field-edit");
    assert_eq!(append["plan"]["candidate"]["selector"], "#billing-notes");
    assert_eq!(append["plan"]["evidence"]["mode"], "append");
    assert_eq!(append["plan"]["evidence"]["itemQuery"], "Billing");
    assert_eq!(append["plan"]["evidence"]["fieldHint"], "Notes");
    assert_eq!(append["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_ordered_and_close_like_clicks_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-clicks");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="one">ONE</button>
  <button id="two">TWO</button>
  <button id="close" class="ui-dialog-titlebar-close" title="Close" aria-label="Close">
    <span>Close</span>
  </button>
  <output id="result"></output>
`;
const out = document.querySelector('#result');
document.querySelector('#one').addEventListener('click', () => out.textContent += '1');
document.querySelector('#two').addEventListener('click', () => out.textContent += '2');
document.querySelector('#close').addEventListener('click', () => out.textContent += 'x');
"##
        .to_string(),
    ]);

    let ordered = act_instruction(&env, "Click ONE, then click TWO.");
    let close = act_instruction(&env, "Click the button labeled \"x\".");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#result').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "12x");
    assert_eq!(ordered["plan"]["action"], "sequence");
    assert_eq!(ordered["verification"]["status"], "observed");
    assert_eq!(close["plan"]["candidate"]["selector"], "#close");
    assert_eq!(close["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_shadow_dom_numeric_targets_in_order_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-ordered-numbers");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <number-pad></number-pad>
  <output id="result"></output>
`;
customElements.define('number-pad', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <button data-value="3">3</button>
      <button data-value="1">1</button>
      <button data-value="2">2</button>
    `;
    for (const button of root.querySelectorAll('button')) {
      button.addEventListener('click', () => {
        document.querySelector('#result').textContent += button.textContent.trim();
      });
    }
  }
});
"##
        .to_string(),
    ]);

    let ordered = act_instruction(&env, "Click on the numbers in ascending order.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#result').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "123");
    assert_eq!(ordered["plan"]["action"], "click_ordered_values");
    assert_eq!(ordered["result"]["orderedValues"]["order"], "ascending");
    assert_eq!(ordered["result"]["orderedValues"]["count"], 3);
    assert_eq!(ordered["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_choice_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-choice-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label>State
    <select id="state">
      <option>Alabama</option>
      <option>California</option>
      <option>Ohio</option>
    </select>
  </label>
  <fieldset>
    <legend>Colors</legend>
    <label><input type="checkbox" id="red">Red</label>
    <label><input type="checkbox" id="green">Green</label>
    <label><input type="checkbox" id="blue">Blue</label>
  </fieldset>
  <fieldset>
    <legend>Size</legend>
    <label><input type="radio" name="size" id="small">Small</label>
    <label><input type="radio" name="size" id="large">Large</label>
  </fieldset>
  <button id="combo" role="combobox" aria-haspopup="listbox" aria-controls="fruit-list" aria-expanded="false">
    Fruit: Apple
  </button>
  <div id="fruit-list" role="listbox" hidden>
    <div role="option" data-value="apple">Apple</div>
    <div role="option" data-value="banana">Banana</div>
    <div role="option" data-value="cherry">Cherry</div>
  </div>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
function update() {
  document.querySelector('#out').textContent = JSON.stringify({
    state: state.value,
    red: red.checked,
    green: green.checked,
    blue: blue.checked,
    small: small.checked,
    large: large.checked,
    fruit: combo.dataset.value || 'apple'
  });
}
document.querySelectorAll('input,select').forEach(el => el.addEventListener('change', update));
submit.addEventListener('click', update);
combo.addEventListener('click', () => {
  document.querySelector('#fruit-list').hidden = false;
  combo.setAttribute('aria-expanded', 'true');
});
document.querySelectorAll('[role=option]').forEach(option => option.addEventListener('click', () => {
  combo.textContent = 'Fruit: ' + option.textContent;
  combo.dataset.value = option.dataset.value;
  document.querySelector('#fruit-list').hidden = true;
  combo.setAttribute('aria-expanded', 'false');
  update();
}));
update();
"##
        .to_string(),
    ]);

    let native_select = act_instruction(&env, "choose California from the State dropdown");
    let checkboxes = act_instruction(&env, "check Red and Blue");
    let radio = act_instruction(&env, "choose Large from Size");
    let custom_combo = act_instruction(&env, "choose Banana from the Fruit combo");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let selected: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse selected choice state");
    assert_eq!(selected["state"], "California");
    assert_eq!(selected["red"], true);
    assert_eq!(selected["green"], false);
    assert_eq!(selected["blue"], true);
    assert_eq!(selected["small"], false);
    assert_eq!(selected["large"], true);
    assert_eq!(selected["fruit"], "banana");

    assert_eq!(native_select["verification"]["status"], "observed");
    assert_eq!(checkboxes["verification"]["status"], "observed");
    assert_eq!(radio["verification"]["status"], "observed");
    assert_eq!(custom_combo["verification"]["status"], "observed");
    assert_eq!(native_select["result"]["selected"]["mode"], "native-select");
    assert_eq!(custom_combo["result"]["selected"]["mode"], "custom-option");

    env.stop();
}

#[test]
fn act_instruction_selects_custom_value_host_dropdowns_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-value-dropdown");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
customElements.define('plan-picker', class extends HTMLElement {
  constructor() {
    super();
    this._value = 'basic';
  }
  connectedCallback() {
    this.tabIndex = 0;
    this.style.display = 'inline-block';
    this.style.minWidth = '160px';
    this.style.minHeight = '24px';
    this.style.border = '1px solid #555';
    this.textContent = 'Plan: Basic';
  }
  get value() {
    return this._value;
  }
  set value(next) {
    this._value = String(next);
    this.setAttribute('data-current-value', this._value);
  }
});
document.body.innerHTML = `
  <label id="plan-label" for="plan">Plan dropdown</label>
  <plan-picker id="plan" aria-labelledby="plan-label" data-field="plan dropdown" data-control="select"></plan-picker>
  <div id="plan-options" data-options="plan options" hidden>
    <button type="button" data-value="basic">Basic</button>
    <button type="button" data-value="pro">Pro</button>
    <button type="button" data-value="enterprise">Enterprise</button>
  </div>
  <button id="apply">Apply</button>
  <output id="out"></output>
`;
const plan = document.querySelector('#plan');
plan.addEventListener('click', () => {
  document.querySelector('#plan-options').hidden = false;
});
document.querySelectorAll('#plan-options button').forEach(option => {
  option.addEventListener('click', () => {
    plan.value = option.dataset.value;
    plan.textContent = 'Plan: ' + option.textContent;
    plan.dataset.inputSeen = 'true';
    document.querySelector('#plan-options').hidden = true;
    plan.dispatchEvent(new Event('change', { bubbles: true }));
  });
});
document.querySelector('#apply').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    value: plan.value,
    text: plan.textContent,
    currentValue: plan.getAttribute('data-current-value'),
    inputSeen: plan.dataset.inputSeen === 'true'
  });
});
"##
        .to_string(),
    ]);

    let selection = act_instruction(
        &env,
        "Choose Enterprise from the Plan dropdown and click Apply.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse custom value dropdown state");
    assert_eq!(state["value"], "enterprise");
    assert_eq!(state["text"], "Plan: Enterprise");
    assert_eq!(state["currentValue"], "enterprise");
    assert_eq!(state["inputSeen"], true);
    assert_eq!(selection["plan"]["action"], "sequence");
    assert_eq!(selection["plan"]["steps"][0]["action"], "select_option");
    assert_eq!(selection["plan"]["steps"][0]["params"]["selector"], "#plan");
    assert_eq!(
        selection["result"]["steps"][0]["selected"]["mode"],
        "custom-option"
    );
    assert_eq!(selection["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_semantically_related_checkboxes_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-semantic-checkboxes");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <fieldset>
    <legend>Word choices</legend>
    <label><input type="checkbox" id="courageous">Courageous</label>
    <label><input type="checkbox" id="dislike">Dislike</label>
    <label><input type="checkbox" id="complete">Complete</label>
    <label><input type="checkbox" id="ordinary">Ordinary</label>
  </fieldset>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
submit.addEventListener('click', () => {
  const selected = Array.from(document.querySelectorAll('input:checked')).map(input => input.id);
  out.textContent = selected.join(',');
});
"##
        .to_string(),
    ]);

    let result = act_instruction(
        &env,
        "Select words similar to brave, hate, finish and click Submit.",
    );
    let state = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(state["result"], "courageous,dislike,complete");
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_completes_listed_checkbox_sequences_with_submit_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-checkbox-submit-fallback");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="choices">
    <label><input type="checkbox" id="alpha">Alpha</label>
    <label><input type="checkbox" id="beta">Beta</label>
    <label><input type="checkbox" id="gamma">Gamma</label>
    <label><input type="checkbox" id="delta">Delta</label>
    <label><input type="checkbox" id="epsilon">Epsilon</label>
    <button id="submit" type="submit">Submit</button>
  </form>
  <output id="out"></output>
`;
choices.addEventListener('submit', event => {
  event.preventDefault();
  const selected = Array.from(document.querySelectorAll('input:checked')).map(input => input.id);
  out.textContent = selected.join(',');
});
"##
        .to_string(),
    ]);

    let result = act_instruction(&env, "Select Alpha, Gamma, Epsilon and submit.");
    let state = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(state["result"], "alpha,gamma,epsilon");
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_ordinal_checked_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-ordinal-checked-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <fieldset id="checks">
    <legend>Repeated checkbox labels</legend>
    <label><input type="checkbox" id="check-a"> Option</label>
    <label><input type="checkbox" id="check-b"> Option</label>
    <label><input type="checkbox" id="check-c"> Option</label>
  </fieldset>
  <fieldset id="radios">
    <legend>Repeated radio labels</legend>
    <label><input type="radio" name="priority" id="radio-a"> Choice</label>
    <label><input type="radio" name="priority" id="radio-b"> Choice</label>
    <label><input type="radio" name="priority" id="radio-c"> Choice</label>
  </fieldset>
  <output id="out"></output>
`;
function update() {
  document.querySelector('#out').textContent = JSON.stringify({
    checkA: document.querySelector('#check-a').checked,
    checkB: document.querySelector('#check-b').checked,
    checkC: document.querySelector('#check-c').checked,
    radioA: document.querySelector('#radio-a').checked,
    radioB: document.querySelector('#radio-b').checked,
    radioC: document.querySelector('#radio-c').checked
  });
}
document.querySelectorAll('input').forEach(input => input.addEventListener('change', update));
update();
"##
        .to_string(),
    ]);

    let checkbox = act_instruction(&env, "Check the second checkbox.");
    let radio = act_instruction(&env, "Select the last radio button.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse ordinal checked state");
    assert_eq!(state["checkA"], false);
    assert_eq!(state["checkB"], true);
    assert_eq!(state["checkC"], false);
    assert_eq!(state["radioA"], false);
    assert_eq!(state["radioB"], false);
    assert_eq!(state["radioC"], true);
    assert_eq!(
        checkbox["plan"]["capability"]["name"],
        "ordinal-checked-control"
    );
    assert_eq!(checkbox["plan"]["evidence"]["controlKind"], "checkbox");
    assert_eq!(checkbox["plan"]["evidence"]["resolvedIndex"], 1);
    assert_eq!(checkbox["verification"]["status"], "observed");
    assert_eq!(
        radio["plan"]["capability"]["name"],
        "ordinal-checked-control"
    );
    assert_eq!(radio["plan"]["evidence"]["controlKind"], "radio");
    assert_eq!(radio["plan"]["evidence"]["resolvedIndex"], 2);
    assert_eq!(radio["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_compound_ordinal_radio_and_textbox_sequence_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-compound-ordinal-form-steps");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="form">
    <input type="radio" name="choice" id="radio-a" value="1">
    <input type="radio" name="choice" id="radio-b" value="2">
    <input type="radio" name="choice" id="radio-c" value="3">
    <input type="text" id="input-a">
    <input type="text" id="input-b">
    <input type="text" id="input-c">
    <button id="submit">Submit</button>
  </section>
  <output id="out"></output>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    radioA: document.querySelector('#radio-a').checked,
    radioB: document.querySelector('#radio-b').checked,
    radioC: document.querySelector('#radio-c').checked,
    inputA: document.querySelector('#input-a').value,
    inputB: document.querySelector('#input-b').value,
    inputC: document.querySelector('#input-c').value
  });
});
"##
        .to_string(),
    ]);

    let form = act_instruction(
        &env,
        "Check the 1st radio button and enter the number \"16\" into the 3rd textbox, then press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse form state");

    assert_eq!(state["radioA"], true);
    assert_eq!(state["radioB"], false);
    assert_eq!(state["radioC"], false);
    assert_eq!(state["inputA"], "");
    assert_eq!(state["inputB"], "");
    assert_eq!(state["inputC"], "16");
    assert_eq!(form["plan"]["action"], "sequence");
    assert_eq!(form["plan"]["capability"]["name"], "compound-form-steps");
    assert_eq!(form["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_completes_compound_form_sequence_with_obvious_submit_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-compound-form-obvious-submit");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="form">
    <input type="radio" name="choice" value="1">
    <input type="radio" name="choice" value="2">
    <input type="radio" name="choice" value="3">
    <input type="text" id="input-1">
    <input type="text" id="input-2">
    <input type="text" id="input-3">
    <button id="submit">Submit</button>
  </section>
  <output id="out"></output>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    selected: document.querySelector('input[type=radio]:checked')?.value || '',
    first: document.querySelector('#input-1').value,
    second: document.querySelector('#input-2').value,
    third: document.querySelector('#input-3').value
  });
});
"##
        .to_string(),
    ]);

    let form = act_instruction(
        &env,
        "Check the 1st radio button and enter the number \"16\" into the 3rd textbox.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse form state");

    assert_eq!(state["selected"], "1");
    assert_eq!(state["first"], "");
    assert_eq!(state["second"], "");
    assert_eq!(state["third"], "16");
    assert_eq!(form["plan"]["action"], "sequence");
    assert_eq!(form["plan"]["capability"]["name"], "compound-form-steps");
    assert_eq!(form["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_dropdown_then_labeled_button_sequence_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-dropdown-button-sequence");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <select id="height">
    <option>5ft 9in</option>
    <option>5ft 10in</option>
    <option>5ft 11in</option>
  </select>
  <button id="yes">Yes</button>
  <button id="no">No</button>
  <button id="maybe">Maybe</button>
  <output id="out"></output>
`;
for (const button of document.querySelectorAll('button')) {
  button.addEventListener('click', () => {
    document.querySelector('#out').textContent =
      document.querySelector('#height').value + '|' + button.textContent.trim();
  });
}
"##
        .to_string(),
    ]);

    let form = act_instruction(
        &env,
        "Choose 5ft 10in from the dropdown, then click the button labeled \"No\".",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "5ft 10in|No");
    assert_eq!(form["plan"]["action"], "sequence");
    assert_eq!(form["plan"]["capability"]["name"], "compound-form-steps");
    assert_eq!(form["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_scoped_checked_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-checked-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="records">
    <div class="row" role="listitem" data-name="Alice">
      <span>Alice</span>
      <label><input type="checkbox" id="alice-notify"> Notify</label>
    </div>
    <div class="row" role="listitem" data-name="Bob">
      <span>Bob</span>
      <label><input type="checkbox" id="bob-notify"> Notify</label>
    </div>
  </section>
  <output id="out"></output>
`;
function update() {
  document.querySelector('#out').textContent = JSON.stringify({
    alice: document.querySelector('#alice-notify').checked,
    bob: document.querySelector('#bob-notify').checked
  });
}
document.querySelectorAll('input').forEach(input => input.addEventListener('change', update));
update();
"##
        .to_string(),
    ]);

    let notify = act_instruction(
        &env,
        "Check the Notify checkbox in the row containing Alice.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse scoped checked state");
    assert_eq!(state["alice"], true);
    assert_eq!(state["bob"], false);
    assert_eq!(
        notify["plan"]["capability"]["name"],
        "scoped-checked-control"
    );
    assert_eq!(notify["plan"]["evidence"]["itemQuery"], "Alice");
    assert_eq!(notify["plan"]["evidence"]["controlHint"], "Notify");
    assert_eq!(notify["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_repeated_checked_controls_inside_named_sections_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-named-section-checked-control");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-panel" aria-label="Billing">
      <h2>Billing</h2>
      <button id="billing-notify" type="button" role="switch" aria-checked="false">Notify</button>
      <button id="billing-archive" type="button" role="switch" aria-checked="false">Archive</button>
    </section>
    <section id="support-panel" aria-label="Support">
      <h2>Support</h2>
      <button id="support-notify" type="button" role="switch" aria-checked="false">Notify</button>
      <button id="support-archive" type="button" role="switch" aria-checked="false">Archive</button>
    </section>
  </main>
`;
for (const button of document.querySelectorAll('[role=switch]')) {
  button.addEventListener('click', () => {
    const current = button.getAttribute('aria-checked') === 'true';
    button.setAttribute('aria-checked', String(!current));
  });
}
"##
        .to_string(),
    ]);

    let notify = act_instruction(&env, "Turn on Notify in the Support panel.");
    let archive = act_instruction(&env, "In the Billing section, enable Archive.");
    let state = env.json(&[
        "eval".to_string(),
        "JSON.stringify({billingNotify: document.querySelector('#billing-notify').getAttribute('aria-checked'), billingArchive: document.querySelector('#billing-archive').getAttribute('aria-checked'), supportNotify: document.querySelector('#support-notify').getAttribute('aria-checked'), supportArchive: document.querySelector('#support-archive').getAttribute('aria-checked')})".to_string(),
    ]);
    let values: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse named section checked-control state");

    assert_eq!(values["billingNotify"], "false");
    assert_eq!(values["billingArchive"], "true");
    assert_eq!(values["supportNotify"], "true");
    assert_eq!(values["supportArchive"], "false");
    assert_eq!(
        notify["plan"]["capability"]["name"],
        "scoped-checked-control"
    );
    assert_eq!(notify["plan"]["candidate"]["selector"], "#support-notify");
    assert_eq!(notify["plan"]["evidence"]["itemQuery"], "Support");
    assert_eq!(notify["plan"]["evidence"]["controlHint"], "Notify");
    assert_eq!(
        archive["plan"]["capability"]["name"],
        "scoped-checked-control"
    );
    assert_eq!(archive["plan"]["candidate"]["selector"], "#billing-archive");
    assert_eq!(archive["plan"]["evidence"]["itemQuery"], "Billing");
    assert_eq!(archive["plan"]["evidence"]["controlHint"], "Archive");
    assert_eq!(notify["verification"]["status"], "observed");
    assert_eq!(archive["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_aria_checked_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-aria-checked-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="email-switch" role="switch" aria-checked="false" aria-label="Email notifications">
    Email notifications
  </button>
  <div role="menu" aria-label="Editor settings">
    <div id="compact-mode" role="menuitemcheckbox" aria-checked="true" tabindex="0">
      Compact mode
    </div>
  </div>
  <output id="out"></output>
`;
function checked(el) {
  return el.getAttribute('aria-checked') === 'true';
}
function render() {
  out.textContent = JSON.stringify({
    email: checked(emailSwitch),
    compact: checked(compactMode)
  });
}
const emailSwitch = document.querySelector('#email-switch');
const compactMode = document.querySelector('#compact-mode');
for (const el of [emailSwitch, compactMode]) {
  el.addEventListener('click', () => {
    el.setAttribute('aria-checked', String(!checked(el)));
    render();
  });
  el.addEventListener('change', render);
  el.addEventListener('input', render);
}
render();
"##
        .to_string(),
    ]);

    let switch = act_instruction(&env, "turn on Email notifications");
    let menu_item = act_instruction(&env, "disable Compact mode");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");
    assert_eq!(state["email"], true);
    assert_eq!(state["compact"], false);
    assert_eq!(switch["plan"]["action"], "set_checked");
    assert_eq!(menu_item["plan"]["action"], "set_checked");
    assert_eq!(switch["result"]["checked"]["result"]["mode"], "switch");
    assert_eq!(
        menu_item["result"]["checked"]["result"]["mode"],
        "menuitemcheckbox"
    );
    assert_eq!(switch["verification"]["status"], "observed");
    assert_eq!(menu_item["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_aria_pressed_toggle_buttons_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-aria-pressed-toggle-buttons");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div role="toolbar" aria-label="Text formatting">
    <button id="bold" type="button" aria-pressed="false">Bold</button>
    <button id="compact" type="button" aria-pressed="true">Compact layout</button>
  </div>
  <output id="out"></output>
`;
function pressed(id) {
  return document.querySelector('#' + id).getAttribute('aria-pressed') === 'true';
}
function render() {
  out.textContent = JSON.stringify({
    bold: pressed('bold'),
    compact: pressed('compact')
  });
}
for (const button of document.querySelectorAll('[aria-pressed]')) {
  button.addEventListener('click', () => {
    const current = button.getAttribute('aria-pressed') === 'true';
    button.setAttribute('aria-pressed', String(!current));
    render();
  });
  button.addEventListener('change', render);
  button.addEventListener('input', render);
}
render();
"##
        .to_string(),
    ]);

    let bold = act_instruction(&env, "turn on Bold toggle");
    let compact = act_instruction(&env, "disable Compact layout toggle");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");
    assert_eq!(state["bold"], true);
    assert_eq!(state["compact"], false);
    assert_eq!(bold["plan"]["action"], "set_checked");
    assert_eq!(compact["plan"]["action"], "set_checked");
    assert_eq!(bold["result"]["checked"]["result"]["mode"], "pressed");
    assert_eq!(compact["result"]["checked"]["result"]["mode"], "pressed");
    assert_eq!(bold["verification"]["status"], "observed");
    assert_eq!(compact["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_custom_aria_radio_groups_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-aria-radio-groups");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div role="radiogroup" aria-label="Plan">
    <div id="plan-basic" role="radio" aria-checked="true" tabindex="0">Basic</div>
    <div id="plan-pro" role="radio" aria-checked="false" tabindex="0">Pro</div>
    <div id="plan-enterprise" role="radio" aria-checked="false" tabindex="0">Enterprise</div>
  </div>
  <div role="menu" aria-label="Sort order">
    <div id="sort-name" role="menuitemradio" aria-checked="true" tabindex="0">Name</div>
    <div id="sort-date" role="menuitemradio" aria-checked="false" tabindex="0">Date</div>
  </div>
  <output id="out"></output>
`;
function checked(id) {
  return document.querySelector('#' + id).getAttribute('aria-checked') === 'true';
}
function render() {
  out.textContent = JSON.stringify({
    basic: checked('plan-basic'),
    pro: checked('plan-pro'),
    enterprise: checked('plan-enterprise'),
    name: checked('sort-name'),
    date: checked('sort-date')
  });
}
for (const item of document.querySelectorAll('[role=radio], [role=menuitemradio]')) {
  item.addEventListener('click', () => {
    item.setAttribute('aria-checked', 'true');
    render();
  });
  item.addEventListener('change', render);
  item.addEventListener('input', render);
}
render();
"##
        .to_string(),
    ]);

    let plan = act_instruction(&env, "choose Pro from Plan");
    let sort = act_instruction(&env, "choose Date from Sort order");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");
    assert_eq!(state["basic"], false);
    assert_eq!(state["pro"], true);
    assert_eq!(state["enterprise"], false);
    assert_eq!(state["name"], false);
    assert_eq!(state["date"], true);
    assert_eq!(plan["plan"]["capability"]["name"], "grouped-choice-control");
    assert_eq!(sort["plan"]["capability"]["name"], "grouped-choice-control");
    assert_eq!(plan["result"]["checked"]["result"]["mode"], "radio");
    assert_eq!(sort["result"]["checked"]["result"]["mode"], "menuitemradio");
    assert_eq!(plan["result"]["checked"]["result"]["peerChanges"], 1);
    assert_eq!(sort["result"]["checked"]["result"]["peerChanges"], 1);
    assert_eq!(plan["verification"]["status"], "observed");
    assert_eq!(sort["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_custom_checkable_group_options_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-checkable-group-options");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section role="radiogroup" aria-label="Delivery speed">
    <choice-chip id="speed-standard" label="Standard" data-option="standard"></choice-chip>
    <choice-chip id="speed-express" label="Express" data-option="express"></choice-chip>
    <choice-chip id="speed-overnight" label="Overnight" data-option="overnight"></choice-chip>
  </section>
  <output id="out"></output>
`;
if (!customElements.get('choice-chip')) {
  customElements.define('choice-chip', class extends HTMLElement {
    constructor() {
      super();
      this._checked = false;
    }
    connectedCallback() {
      this.style.display = 'inline-flex';
      this.style.width = '110px';
      this.style.height = '30px';
      this.style.border = '1px solid #555';
      this.style.margin = '4px';
      this.style.alignItems = 'center';
      this.style.justifyContent = 'center';
      this.textContent = this.getAttribute('label') || '';
    }
    get checked() {
      return this._checked;
    }
    set checked(next) {
      this._checked = Boolean(next);
      this.setAttribute('aria-checked', String(this._checked));
      this.dataset.selected = String(this._checked);
    }
  });
}
function render() {
  out.textContent = JSON.stringify({
    standard: document.querySelector('#speed-standard').checked,
    express: document.querySelector('#speed-express').checked,
    overnight: document.querySelector('#speed-overnight').checked
  });
}
document.querySelector('#speed-standard').checked = true;
for (const chip of document.querySelectorAll('choice-chip')) {
  chip.addEventListener('input', render);
  chip.addEventListener('change', render);
}
render();
"##
        .to_string(),
    ]);

    let choice = act_instruction(&env, "choose Express from Delivery speed");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["express"], true);
    assert_eq!(state["standard"], false);
    assert_eq!(state["overnight"], false);
    assert_eq!(choice["plan"]["action"], "set_checked");
    assert_eq!(choice["plan"]["params"]["selector"], "#speed-express");
    assert_eq!(
        choice["plan"]["capability"]["name"],
        "grouped-choice-control"
    );
    assert_eq!(choice["result"]["checked"]["result"]["peerChanges"], 1);
    assert_eq!(choice["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_rendered_color_swatches_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-color-swatches");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    .swatch {
      display: inline-block;
      width: 18px;
      height: 18px;
      margin: 6px;
      border: 1px solid transparent;
    }
    .selected { border-color: black; }
  </style>
  <div id="palette">
    <span id="red-1" class="swatch" style="background-color:hsl(0, 70%, 45%)"></span>
    <span id="blue-1" class="swatch" style="background-color:hsl(225, 75%, 45%)"></span>
    <span id="red-2" class="swatch" style="background-color:hsl(8, 60%, 70%)"></span>
    <span id="green-1" class="swatch" style="background-color:hsl(120, 65%, 45%)"></span>
  </div>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelectorAll('.swatch').forEach(swatch => {
  swatch.addEventListener('click', () => swatch.classList.toggle('selected'));
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    Array.from(document.querySelectorAll('.selected')).map(el => el.id).sort().join(',');
});
"##
        .to_string(),
    ]);

    let swatches = act_instruction(&env, "Select all the shades of red and press Submit.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "red-1,red-2");
    assert_eq!(swatches["plan"]["action"], "sequence");
    assert_eq!(
        swatches["plan"]["capability"]["name"],
        "visual-color-selection"
    );
    assert_eq!(swatches["plan"]["evidence"]["matched"], 2);
    assert_eq!(swatches["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_text_backed_color_picker_inputs_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-color-picker-input");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label>Archived Color<br>
    <input id="locked-color-value" class="jscolor" data-jscolor="{width:101}" readonly value="ab2567">
  </label>
  <label>Color<br>
    <input id="color-value" class="jscolor" data-jscolor="{width:101}" value="ab2567">
  </label>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    value: document.querySelector('#color-value').value.toLowerCase(),
    lockedValue: document.querySelector('#locked-color-value').value.toLowerCase()
  });
});
"##
        .to_string(),
    ]);

    let color = act_instruction(&env, "Select blue with the color picker and hit Submit.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse color state");

    assert_eq!(state["value"], "0000ff");
    assert_eq!(state["lockedValue"], "ab2567");
    assert_eq!(color["plan"]["action"], "sequence");
    assert_eq!(color["plan"]["capability"]["name"], "color-picker-input");
    assert_eq!(color["plan"]["evidence"]["color"], "blue");
    assert_eq!(
        color["plan"]["steps"][0]["params"]["selector"],
        "#color-value"
    );
    assert_eq!(color["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_hex_color_picker_inputs_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-hex-color-picker-input");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label>Color<br>
    <input id="color-value" class="jscolor" data-jscolor="{width:101}" value="ab2567">
  </label>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#color-value').value.toLowerCase();
});
"##
        .to_string(),
    ]);

    let color = act_instruction(
        &env,
        "Select the following color #36ddb8 with the color picker and hit Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "36ddb8");
    assert_eq!(color["plan"]["action"], "sequence");
    assert_eq!(color["plan"]["capability"]["name"], "color-picker-input");
    assert_eq!(color["plan"]["evidence"]["hex"], "#36ddb8");
    assert_eq!(color["plan"]["evidence"]["typedValue"], "36ddb8");
    assert_eq!(color["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_custom_value_host_color_pickers_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-color-picker-value-host");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <color-value-box id="color-value" aria-label="Color picker" data-field="color" data-jscolor="{width:101}" tabindex="0" style="display:block; min-height: 24px; width: 180px; border: 1px solid #888;"></color-value-box>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
if (!customElements.get('color-value-box')) {
  customElements.define('color-value-box', class extends HTMLElement {
    constructor() {
      super();
      this._value = 'ab2567';
    }
    connectedCallback() {
      this.textContent = this._value;
    }
    get value() {
      return this._value || '';
    }
    set value(next) {
      this._value = String(next);
      this.textContent = this._value;
      this.setAttribute('data-current-value', this._value);
    }
  });
}
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#color-value').value.toLowerCase();
});
"##
        .to_string(),
    ]);

    let color = act_instruction(&env, "Select green with the color picker and hit Submit.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "008000");
    assert_eq!(color["plan"]["action"], "sequence");
    assert_eq!(color["plan"]["capability"]["name"], "color-picker-input");
    assert_eq!(color["plan"]["evidence"]["color"], "green");
    assert_eq!(
        color["plan"]["steps"][0]["params"]["selector"],
        "#color-value"
    );
    assert_eq!(color["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_unlabeled_visual_color_targets_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-single-color-target");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    .tile {
      display: inline-block;
      width: 22px;
      height: 22px;
      margin: 4px;
      border: 1px solid transparent;
    }
    .chosen { outline: 2px solid black; }
  </style>
  <div id="choices">
    <span id="red-tile" class="tile" style="background-color: rgb(220, 35, 35)"></span>
    <span id="blue-tile" class="tile" style="background-color: rgb(35, 91, 220)"></span>
    <span id="green-tile" class="tile" style="background-color: rgb(28, 150, 80)"></span>
  </div>
  <button id="save">Save</button>
  <output id="out"></output>
`;
document.querySelectorAll('.tile').forEach(tile => {
  tile.addEventListener('click', () => {
    document.querySelectorAll('.tile').forEach(other => other.classList.remove('chosen'));
    tile.classList.add('chosen');
  });
});
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    document.querySelector('.chosen')?.id || '';
});
"##
        .to_string(),
    ]);

    let tile = act_instruction(&env, "Click the blue tile and press Save.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "blue-tile");
    assert_eq!(tile["plan"]["action"], "sequence");
    assert_eq!(tile["plan"]["capability"]["name"], "visual-color-selection");
    assert_eq!(tile["plan"]["evidence"]["mode"], "single");
    assert_eq!(tile["plan"]["evidence"]["matched"], 1);
    assert_eq!(tile["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_infers_target_color_from_prompt_swatch_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-prompt-color-swatch");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    .sample, .box {
      display: inline-block;
      width: 18px;
      height: 18px;
      border: 1px solid black;
      margin: 4px;
    }
    .box { width: 34px; height: 34px; }
  </style>
  <div id="query">
    Click on the <span class="sample" style="background-color: olive"></span> colored box.
  </div>
  <div id="area">
    <span id="blue" class="box" data-color="blue" style="background-color: blue"></span>
    <span id="olive" class="box" data-color="olive" style="background-color: olive"></span>
    <span id="orange" class="box" data-color="orange" style="background-color: orange"></span>
  </div>
  <output id="out"></output>
`;
document.querySelectorAll('.box').forEach(box => {
  box.addEventListener('click', () => {
    document.querySelector('#out').textContent = box.id;
  });
});
"##
        .to_string(),
    ]);

    let tile = act_instruction(&env, "Click on the colored box.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "olive");
    let capability = tile["plan"]["capability"]["name"]
        .as_str()
        .unwrap_or_default();
    assert!(
        capability == "visual-color-selection" || capability == "visual-object-click",
        "unexpected capability: {capability}"
    );
    let evidence_color = tile["plan"]["evidence"]["color"].as_str().or_else(|| {
        tile["plan"]["steps"]
            .get(0)
            .and_then(|step| step["evidence"]["color"].as_str())
    });
    assert_eq!(evidence_color, Some("olive"));
    assert_eq!(tile["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_standard_named_color_targets_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-standard-color-target");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #wrap { width: 160px; height: 210px; }
    #query { height: 50px; background: yellow; }
    #area { text-align: center; margin: 20px 0; }
    .box {
      display: inline-block;
      width: 50px;
      height: 50px;
      margin: 5px;
    }
  </style>
  <div id="wrap">
    <div id="query"></div>
    <div id="area">
      <div class="box" data-color="olive" style="background-color: olive"></div>
      <div class="box" data-color="white" style="background-color: white"></div>
      <div class="box" data-color="yellow" style="background-color: yellow"></div>
      <div class="box" data-color="orange" style="background-color: orange"></div>
    </div>
  </div>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelectorAll('.box').forEach(box => {
  box.addEventListener('click', () => {
    document.querySelector('#out').dataset.selected = box.getAttribute('data-color');
  });
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    document.querySelector('#out').dataset.selected || '';
});
"##
        .to_string(),
    ]);

    let tile = act_instruction(&env, "Click on the olive colored box and press Submit.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "olive");
    assert_eq!(tile["plan"]["action"], "sequence");
    assert_eq!(tile["plan"]["capability"]["name"], "visual-color-selection");
    assert_eq!(tile["plan"]["evidence"]["color"], "olive");
    assert_eq!(tile["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_visible_shape_center_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-visible-shape-center");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" width="220" height="140" style="width:220px;height:140px;border:1px solid #ddd">
    <rect x="15" y="20" width="40" height="40" fill="#ccc"></rect>
    <circle cx="140" cy="70" r="28" fill="#48a868"></circle>
    <polygon points="80,110 105,70 130,110" fill="#444"></polygon>
  </svg>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelector('#surface').addEventListener('click', event => {
  const circle = document.querySelector('circle');
  const rect = circle.getBoundingClientRect();
  const centerX = rect.left + rect.width / 2;
  const centerY = rect.top + rect.height / 2;
  const distance = Math.hypot(event.clientX - centerX, event.clientY - centerY);
  document.querySelector('#out').dataset.pending = distance <= 4 ? 'hit' : 'miss';
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    document.querySelector('#out').dataset.pending || '';
});
"##
        .to_string(),
    ]);

    let click = act_instruction(
        &env,
        "Find and click on the center of the circle, then press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "hit");
    assert_eq!(click["plan"]["action"], "sequence");
    assert_eq!(click["plan"]["capability"]["name"], "visual-object-click");
    assert_eq!(click["plan"]["steps"][0]["evidence"]["shape"], "circle");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_visible_svg_shape_value_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-visual-geometry-selection");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" width="180" height="90" style="width:180px;height:90px;border:1px solid #ddd">
    <polygon points="35,70 70,15 105,70" fill="magenta"></polygon>
  </svg>
  <button>Rectangle</button>
  <button>Circle</button>
  <button id="triangle">Triangle</button>
  <button>Letter</button>
  <output id="out"></output>
`;
document.querySelectorAll('button').forEach(button => {
  button.addEventListener('click', () => {
    document.querySelector('#out').textContent = button.textContent.trim();
  });
});
"##
        .to_string(),
    ]);

    let click = act_instruction(
        &env,
        "Click the button that best describes the figure below.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "Triangle");
    assert_eq!(click["plan"]["action"], "click");
    assert_eq!(
        click["plan"]["capability"]["name"],
        "visual-geometry-selection"
    );
    assert_eq!(click["plan"]["evidence"]["value"], "triangle");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_visible_shape_side_count_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-visual-side-count-selection");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" width="180" height="110" style="width:180px;height:110px;border:1px solid #ddd">
    <polygon points="90,15 130,45 115,88 65,88 50,45" fill="#5f72e6"></polygon>
  </svg>
  <button>3</button>
  <button>4</button>
  <button id="answer">5</button>
  <button>6</button>
  <output id="out"></output>
`;
document.querySelectorAll('button').forEach(button => {
  button.addEventListener('click', () => {
    document.querySelector('#out').textContent = button.textContent.trim();
  });
});
"##
        .to_string(),
    ]);

    let click = act_instruction(
        &env,
        "Click the button that shows how many sides the figure has.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "5");
    assert_eq!(click["plan"]["action"], "click");
    assert_eq!(
        click["plan"]["capability"]["name"],
        "visual-geometry-selection"
    );
    assert_eq!(click["plan"]["evidence"]["value"], "5");
    assert_eq!(click["plan"]["evidence"]["mode"], "side_count");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_canvas_shape_side_count_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-canvas-side-count-selection");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <canvas id="surface" width="150" height="100" style="width:150px;height:100px"></canvas>
  <button>3</button>
  <button>4</button>
  <button>5</button>
  <button id="answer">6</button>
  <button>7</button>
  <output id="out"></output>
`;
const canvas = document.querySelector('#surface');
const ctx = canvas.getContext('2d');
ctx.translate(75, 50);
ctx.rotate(17 * Math.PI / 180);
ctx.beginPath();
const sides = 6;
const size = 35;
ctx.moveTo(size, 0);
for (let index = 1; index <= sides; index += 1) {
  ctx.lineTo(size * Math.cos(index * 2 * Math.PI / sides), size * Math.sin(index * 2 * Math.PI / sides));
}
ctx.strokeStyle = '#000000';
ctx.lineWidth = 3;
ctx.stroke();
document.querySelectorAll('button').forEach(button => {
  button.addEventListener('click', () => {
    document.querySelector('#out').textContent = button.textContent.trim();
  });
});
"##
        .to_string(),
    ]);

    let click = act_instruction(
        &env,
        "Press the button that correctly denotes how many sides the shape has.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "6");
    assert_eq!(click["plan"]["action"], "click");
    assert_eq!(
        click["plan"]["capability"]["name"],
        "visual-geometry-selection"
    );
    assert_eq!(click["plan"]["evidence"]["value"], "6");
    assert_eq!(click["plan"]["evidence"]["mode"], "side_count");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_midpoint_of_visible_line_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-visible-line-midpoint");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" aria-label="geometry surface" width="240" height="130"
    style="width:240px;height:130px;border:1px solid #ddd">
    <line id="segment" x1="40" y1="35" x2="190" y2="95" stroke="black" stroke-width="3"></line>
    <circle cx="40" cy="35" r="5" fill="black"></circle>
    <circle cx="190" cy="95" r="5" fill="black"></circle>
  </svg>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const svg = document.querySelector('#surface');
let clicked = null;
function localPoint(event) {
  const point = svg.createSVGPoint();
  point.x = event.clientX;
  point.y = event.clientY;
  return point.matrixTransform(svg.getScreenCTM().inverse());
}
svg.addEventListener('click', event => {
  clicked = localPoint(event);
});
document.querySelector('#submit').addEventListener('click', () => {
  if (!clicked) {
    document.querySelector('#out').textContent = 'missing';
    return;
  }
  const hit = Math.hypot(clicked.x - 115, clicked.y - 65) <= 5;
  document.querySelector('#out').textContent = hit ? 'midpoint' : JSON.stringify(clicked);
});
"##
        .to_string(),
    ]);

    let click = act_instruction(
        &env,
        "Click the midpoint of the visible line, then press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "midpoint");
    assert_eq!(click["plan"]["action"], "sequence");
    assert_eq!(click["plan"]["capability"]["name"], "visual-geometry-click");
    assert_eq!(click["plan"]["steps"][0]["evidence"]["source"], "line");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_midpoint_between_labeled_points_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-labeled-point-midpoint");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" aria-label="point diagram" width="240" height="130"
    style="width:240px;height:130px;border:1px solid #ddd">
    <circle data-label="A" cx="54" cy="100" r="6" fill="#2f6fed"></circle>
    <text x="45" y="122">A</text>
    <circle data-label="B" cx="186" cy="42" r="6" fill="#2f6fed"></circle>
    <text x="178" y="34">B</text>
    <circle data-label="C" cx="50" cy="40" r="6" fill="#aaa"></circle>
  </svg>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const svg = document.querySelector('#surface');
let clicked = null;
function localPoint(event) {
  const point = svg.createSVGPoint();
  point.x = event.clientX;
  point.y = event.clientY;
  return point.matrixTransform(svg.getScreenCTM().inverse());
}
svg.addEventListener('click', event => {
  clicked = localPoint(event);
});
document.querySelector('#submit').addEventListener('click', () => {
  if (!clicked) {
    document.querySelector('#out').textContent = 'missing';
    return;
  }
  const hit = Math.hypot(clicked.x - 120, clicked.y - 71) <= 5;
  document.querySelector('#out').textContent = hit ? 'between' : JSON.stringify(clicked);
});
"##
        .to_string(),
    ]);

    let click = act_instruction(
        &env,
        "Click halfway between point A and point B, then press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "between");
    assert_eq!(click["plan"]["action"], "sequence");
    assert_eq!(click["plan"]["capability"]["name"], "visual-geometry-click");
    assert_eq!(
        click["plan"]["steps"][0]["evidence"]["source"],
        "point_pair"
    );
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_visual_text_by_attributes_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-visual-text-attributes");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" width="240" height="130" style="width:240px;height:130px;border:1px solid #ddd">
    <text x="30" y="55" font-size="32" fill="black">2</text>
    <text x="95" y="55" font-size="18" fill="black">8</text>
    <text x="145" y="55" font-size="28" fill="red">8</text>
    <text x="190" y="55" font-size="16" fill="black">5</text>
  </svg>
  <output id="out"></output>
`;
for (const node of document.querySelectorAll('text')) {
  node.addEventListener('click', () => {
    document.querySelector('#out').textContent =
      node.textContent + ':' + node.getAttribute('font-size') + ':' + node.getAttribute('fill');
  });
}
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click the small black 8.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "8:18:black");
    assert_eq!(click["plan"]["capability"]["name"], "visual-object-click");
    assert_eq!(click["plan"]["evidence"]["color"], "black");
    assert_eq!(click["plan"]["evidence"]["text"], "8");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_narrow_svg_text_items_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-narrow-svg-text-item");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" width="260" height="130" style="width:260px;height:130px;border:1px solid #ddd">
    <polygon id="large-yellow-polygon" points="25,95 70,25 115,95" fill="yellow"></polygon>
    <text id="small-yellow-letter" x="165" y="78" font-size="18" fill="yellow">I</text>
    <circle id="small-blue-circle" cx="225" cy="68" r="11" fill="blue"></circle>
  </svg>
  <output id="out"></output>
`;
for (const node of document.querySelectorAll('polygon,text,circle')) {
  node.addEventListener('click', () => {
    document.querySelector('#out').textContent = node.id;
  });
}
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click on a small yellow item.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "small-yellow-letter");
    assert_eq!(click["plan"]["capability"]["name"], "visual-object-click");
    assert_eq!(click["plan"]["evidence"]["color"], "yellow");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_an_indefinite_colored_svg_item_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-indefinite-colored-svg-item");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" width="240" height="150" style="width:240px;height:150px;border:1px solid #ddd">
    <text id="red-digit" x="30" y="45" font-size="20" fill="red">7</text>
    <text id="green-letter-a" x="80" y="45" font-size="18" fill="green">x</text>
    <text id="green-letter-b" x="140" y="95" font-size="18" fill="green">m</text>
    <rect id="blue-square" x="180" y="35" width="22" height="22" fill="blue"></rect>
  </svg>
  <output id="out"></output>
`;
for (const node of document.querySelectorAll('text,rect')) {
  node.addEventListener('click', () => {
    document.querySelector('#out').textContent = node.id;
  });
}
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click on a green item.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert!(["green-letter-a", "green-letter-b"].contains(&result["result"].as_str().unwrap_or("")));
    assert_eq!(click["plan"]["capability"]["name"], "visual-object-click");
    assert_eq!(click["plan"]["evidence"]["color"], "green");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_color_qualified_svg_digit_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-color-qualified-svg-digit");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" width="240" height="150" style="width:240px;height:150px;border:1px solid #ddd">
    <text id="green-letter" x="45" y="45" font-size="18" fill="green">A</text>
    <text id="green-digit" x="115" y="75" font-size="26" fill="green">9</text>
    <text id="yellow-digit" x="175" y="45" font-size="18" fill="yellow">7</text>
  </svg>
  <output id="out"></output>
`;
for (const node of document.querySelectorAll('text')) {
  node.addEventListener('click', () => {
    document.querySelector('#out').textContent = node.id;
  });
}
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click on a green digit.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "green-digit");
    assert_eq!(click["plan"]["capability"]["name"], "visual-object-click");
    assert_eq!(click["plan"]["evidence"]["color"], "green");
    assert_eq!(click["plan"]["evidence"]["shape"], "digit");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_ignores_prompt_and_container_regions_for_visual_items() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-visual-item-region-filter");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #query {
      width: 240px;
      height: 42px;
      margin-bottom: 10px;
      background: yellow;
    }
    #visual-zone {
      display: inline-block;
      padding: 12px;
      border: 1px solid #ddd;
    }
  </style>
  <div id="query">Instruction text appears here.</div>
  <div id="visual-zone">
    <svg id="surface" width="260" height="130" style="width:260px;height:130px">
      <rect id="large-visual-item" x="25" y="25" width="95" height="82" fill="green"></rect>
      <circle id="small-visual-item" cx="205" cy="66" r="14" fill="purple"></circle>
    </svg>
  </div>
  <output id="out"></output>
`;
for (const node of document.querySelectorAll('#query,#visual-zone,rect,circle')) {
  node.addEventListener('click', event => {
    if (event.target !== node) return;
    document.querySelector('#out').textContent = node.id;
  });
}
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click on a large item.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "large-visual-item");
    assert_eq!(click["plan"]["capability"]["name"], "visual-object-click");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_child_targets_inside_matching_containers_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-child-click");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    .row { display: grid; grid-template-columns: 1fr auto auto; gap: 8px; padding: 6px; }
  </style>
  <section id="records">
    <div class="row" role="listitem" data-name="Alice">
      <span class="name">Alice</span>
      <button class="edit" aria-label="Edit record">Edit</button>
      <button class="delete" aria-label="Delete record">Delete</button>
    </div>
    <div class="row" role="listitem" data-name="Bob">
      <span class="name">Bob</span>
      <button class="edit" aria-label="Edit record">Edit</button>
      <button class="delete" aria-label="Delete record">Delete</button>
    </div>
  </section>
  <output id="out"></output>
`;
document.querySelectorAll('.row').forEach(row => {
  row.querySelector('.edit').addEventListener('click', () => {
    document.querySelector('#out').textContent = 'edit:' + row.dataset.name;
  });
  row.querySelector('.delete').addEventListener('click', () => {
    document.querySelector('#out').textContent = 'delete:' + row.dataset.name;
  });
});
"##
        .to_string(),
    ]);

    let scoped = act_instruction(&env, "Click the Delete button in the row containing Alice.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "delete:Alice");
    assert_eq!(scoped["plan"]["action"], "click");
    assert_eq!(scoped["plan"]["capability"]["name"], "scoped-child-click");
    assert_eq!(scoped["plan"]["evidence"]["itemQuery"], "Alice");
    assert_eq!(scoped["plan"]["evidence"]["actionHint"], "Delete");
    assert_eq!(scoped["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_repeated_buttons_inside_named_sections_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-named-section-child-click");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="billing-panel" aria-label="Billing">
      <h2>Billing</h2>
      <button class="save">Save</button>
      <button class="delete">Delete</button>
    </section>
    <section id="support-panel" aria-label="Support">
      <h2>Support</h2>
      <button class="save">Save</button>
      <button class="delete">Delete</button>
    </section>
  </main>
  <output id="out"></output>
`;
for (const section of document.querySelectorAll('section')) {
  const name = section.querySelector('h2').textContent;
  section.querySelector('.save').addEventListener('click', () => {
    document.querySelector('#out').textContent = `save:${name}`;
  });
  section.querySelector('.delete').addEventListener('click', () => {
    document.querySelector('#out').textContent = `delete:${name}`;
  });
}
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click the Delete button in the Support panel.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "delete:Support");
    assert_eq!(click["plan"]["action"], "click");
    assert_eq!(click["plan"]["capability"]["name"], "scoped-child-click");
    let candidate_selector = click["plan"]["candidate"]["selector"]
        .as_str()
        .expect("candidate selector");
    let candidate_summary = env.json(&[
        "eval".to_string(),
        format!(
            "(() => {{ const el = document.querySelector({candidate_selector:?}); return el ? el.parentElement.id + ':' + el.textContent.trim() : 'missing'; }})()"
        ),
    ]);
    assert_eq!(candidate_summary["result"], "support-panel:Delete");
    assert_eq!(click["plan"]["evidence"]["itemQuery"], "Support");
    assert_eq!(click["plan"]["evidence"]["actionHint"], "Delete");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_repeated_item_local_icon_actions_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-repeated-local-icon-actions");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #feed { height: 130px; overflow-y: auto; }
    .post { height: 48px; border: 1px solid #ddd; position: relative; }
    .controls { position: absolute; right: 4px; bottom: 3px; }
    .reply, .like, .share { display: inline-block; width: 16px; height: 16px; cursor: pointer; }
    .reply::before { content: "R"; }
    .like::before { content: "L"; }
    .share::before { content: "S"; }
    .active { outline: 2px solid green; }
  </style>
  <section id="feed">
    <article class="post"><span class="username">@ada</span><span>First post</span><div class="controls"><span class="reply"></span><span class="like"></span><span class="share"></span></div></article>
    <article class="post"><span class="username">@grace</span><span>Other post</span><div class="controls"><span class="reply"></span><span class="like"></span><span class="share"></span></div></article>
    <article class="post"><span class="username">@ada</span><span>Second post</span><div class="controls"><span class="reply"></span><span class="like"></span><span class="share"></span></div></article>
  </section>
  <button id="submit" type="button">Submit</button>
  <output id="out"></output>
`;
for (const control of document.querySelectorAll('.reply,.like,.share')) {
  control.addEventListener('click', () => control.classList.toggle('active'));
}
document.querySelector('#submit').addEventListener('click', () => {
  const adaLikes = Array.from(document.querySelectorAll('.post'))
    .filter(post => post.querySelector('.username').textContent === '@ada')
    .filter(post => post.querySelector('.like').classList.contains('active')).length;
  const wrong = Array.from(document.querySelectorAll('.post'))
    .filter(post => post.querySelector('.username').textContent !== '@ada')
    .some(post => post.querySelector('.like').classList.contains('active'));
  document.querySelector('#out').textContent = adaLikes === 2 && !wrong ? 'submitted' : `wrong:${adaLikes}:${wrong}`;
});
"##
        .to_string(),
    ]);

    let scoped = act_instruction(
        &env,
        "Click the \"Like\" button on 2 posts by @ada and then click Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "submitted");
    assert_eq!(scoped["plan"]["action"], "scoped_item_workflow");
    assert_eq!(scoped["plan"]["capability"]["name"], "scoped-item-workflow");
    assert_eq!(scoped["plan"]["evidence"]["itemQuery"], "@ada");
    assert_eq!(
        scoped["plan"]["evidence"]["actionHint"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase(),
        "like"
    );
    assert_eq!(scoped["plan"]["evidence"]["itemCount"], 2);
    assert_eq!(scoped["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_all_matching_item_local_actions_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-all-matching-local-actions");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #feed { height: 120px; overflow-y: auto; }
    .post { height: 46px; border: 1px solid #ddd; position: relative; }
    .controls { position: absolute; right: 4px; bottom: 3px; }
    .reply, .like, .share { display: inline-block; width: 16px; height: 16px; cursor: pointer; }
    .reply::before { content: "R"; }
    .like::before { content: "L"; }
    .share::before { content: "S"; }
    .active { outline: 2px solid green; }
  </style>
  <section id="feed">
    <article class="post"><span class="username">@ada</span><span>First post</span><div class="controls"><span class="reply"></span><span class="like"></span><span class="share"></span></div></article>
    <article class="post"><span class="username">@grace</span><span>Other post</span><div class="controls"><span class="reply"></span><span class="like"></span><span class="share"></span></div></article>
    <article class="post"><span class="username">@ada</span><span>Second post</span><div class="controls"><span class="reply"></span><span class="like"></span><span class="share"></span></div></article>
    <article class="post"><span class="username">@ada</span><span>Third post</span><div class="controls"><span class="reply"></span><span class="like"></span><span class="share"></span></div></article>
  </section>
  <button id="submit" type="button">Submit</button>
  <output id="out"></output>
`;
for (const control of document.querySelectorAll('.reply,.like,.share')) {
  control.addEventListener('click', () => control.classList.toggle('active'));
}
document.querySelector('#submit').addEventListener('click', () => {
  const adaLikes = Array.from(document.querySelectorAll('.post'))
    .filter(post => post.querySelector('.username').textContent === '@ada')
    .filter(post => post.querySelector('.like').classList.contains('active')).length;
  const wrong = Array.from(document.querySelectorAll('.post'))
    .filter(post => post.querySelector('.username').textContent !== '@ada')
    .some(post => post.querySelector('.like').classList.contains('active'));
  document.querySelector('#out').textContent = adaLikes === 3 && !wrong ? 'submitted' : `wrong:${adaLikes}:${wrong}`;
});
"##
        .to_string(),
    ]);

    let scoped = act_instruction(
        &env,
        "Click the \"Like\" button on all posts by @ada and then click Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "submitted");
    assert_eq!(scoped["plan"]["action"], "scoped_item_workflow");
    assert_eq!(scoped["plan"]["capability"]["name"], "scoped-item-workflow");
    assert_eq!(scoped["plan"]["evidence"]["itemQuery"], "@ada");
    assert_eq!(
        scoped["plan"]["evidence"]["actionHint"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase(),
        "like"
    );
    assert_eq!(scoped["plan"]["evidence"]["itemCountMode"], "all");
    assert_eq!(
        scoped["result"]["scopedWorkflow"]["itemCount"].as_u64(),
        Some(3)
    );
    assert_eq!(scoped["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_reveals_scoped_hidden_menu_actions_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-hidden-menu");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    .profile-card { display: grid; grid-template-columns: 1fr auto; gap: 8px; padding: 8px; }
    .menu.hide { display: none; }
    .more { cursor: pointer; padding: 4px; }
    li { cursor: pointer; }
  </style>
  <section id="profiles">
    <article class="profile-card" data-user="@alpha">
      <span class="username">@alpha</span>
      <span class="more" aria-haspopup="menu" aria-label="More actions">More</span>
      <ul class="menu hide">
        <li class="share">Share via DM</li>
        <li class="block">Block</li>
      </ul>
    </article>
    <article class="profile-card" data-user="@beta">
      <span class="username">@beta</span>
      <span class="more" aria-haspopup="menu" aria-label="More actions">More</span>
      <ul class="menu hide">
        <li class="share">Share via DM</li>
        <li class="block">Block</li>
      </ul>
    </article>
  </section>
  <output id="out"></output>
`;
document.querySelectorAll('.profile-card').forEach(card => {
  card.querySelector('.more').addEventListener('click', () => {
    card.querySelector('.menu').classList.remove('hide');
  });
  card.querySelectorAll('li').forEach(item => {
    item.addEventListener('click', () => {
      document.querySelector('#out').textContent = card.dataset.user + ':' + item.className;
    });
  });
});
"##
        .to_string(),
    ]);

    let scoped = act_instruction(
        &env,
        "For the user @beta, click on the \"Share via DM\" button.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "@beta:share");
    assert_eq!(scoped["plan"]["action"], "scoped_menu_click");
    assert_eq!(scoped["plan"]["capability"]["name"], "scoped-child-click");
    assert_eq!(scoped["plan"]["evidence"]["itemQuery"], "@beta");
    assert_eq!(scoped["plan"]["evidence"]["actionHint"], "Share via DM");
    assert_eq!(scoped["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_reveals_scoped_hidden_menu_actions_in_scrollable_regions_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-scroll-menu");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #feed { height: 95px; width: 220px; overflow-y: auto; border: 1px solid #999; }
    .card { height: 68px; border-bottom: 1px solid #ddd; position: relative; }
    .user { font-weight: bold; }
    .actions { position: absolute; right: 4px; bottom: 4px; }
    .menu { display: none; position: absolute; right: 0; bottom: 16px; background: white; border: 1px solid #333; }
    .menu.open { display: block; }
    .menu li { list-style: none; padding: 2px 4px; cursor: pointer; }
  </style>
  <div id="feed">
    <div class="card"><span class="user">@alpha</span><div class="actions"><button class="more">More</button><ul class="menu"><li>Share via DM</li></ul></div></div>
    <div class="card"><span class="user">@beta</span><div class="actions"><button class="more">More</button><ul class="menu"><li>Share via DM</li></ul></div></div>
    <div class="card"><span class="user">@gamma</span><div class="actions"><button class="more">More</button><ul class="menu"><li>Share via DM</li></ul></div></div>
    <div class="card"><span class="user">@target</span><div class="actions"><button class="more">More</button><ul class="menu"><li>Share via DM</li></ul></div></div>
  </div>
  <output id="out"></output>
`;
document.querySelectorAll('.more').forEach(button => {
  button.addEventListener('click', () => {
    document.querySelectorAll('.menu').forEach(menu => menu.classList.remove('open'));
    button.nextElementSibling.classList.add('open');
  });
});
document.querySelectorAll('.menu li').forEach(item => {
  item.addEventListener('click', () => {
    const user = item.closest('.card').querySelector('.user').textContent;
    document.querySelector('#out').textContent = `${user}:${item.textContent}`;
  });
});
"##
        .to_string(),
    ]);

    let scoped = act_instruction(
        &env,
        "For the user @target, click on the \"Share via DM\" button.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "@target:Share via DM");
    assert_eq!(scoped["analysis"]["kind"], "click");
    assert_eq!(scoped["plan"]["action"], "scoped_menu_click");
    assert_eq!(scoped["plan"]["capability"]["name"], "scoped-child-click");
    assert_eq!(scoped["plan"]["evidence"]["itemQuery"], "@target");
    assert_eq!(scoped["plan"]["evidence"]["actionHint"], "Share via DM");
    assert_eq!(scoped["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_item_reply_workflows_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-item-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    .mail-row { display: flex; gap: 10px; padding: 8px; cursor: pointer; }
    .mail-row button { cursor: pointer; }
    #detail[hidden], #composer[hidden] { display: none; }
  </style>
  <div id="inbox" role="list">
    <div class="mail-row" role="listitem" tabindex="0" data-sender="Anne">
      <button class="star" aria-label="Star important">☆</button>
      <span class="from">Anne</span>
      <span class="subject">Budget notes</span>
    </div>
    <div class="mail-row" role="listitem" tabindex="0" data-sender="Carlynne">
      <button class="star" aria-label="Star important">☆</button>
      <span class="from">Carlynne</span>
      <span class="subject">Launch copy</span>
    </div>
  </div>
  <section id="detail" hidden>
    <h2 id="opened"></h2>
    <button id="reply">Reply</button>
    <button id="forward">Forward</button>
    <button id="delete" aria-label="Delete message">Delete</button>
    <div id="composer" hidden>
      <label>Reply text <textarea id="readonly-reply" readonly>archived reply</textarea></label>
      <fieldset disabled>
        <label>Reply text <textarea id="disabled-reply">disabled reply</textarea></label>
      </fieldset>
      <label>Reply text <textarea id="reply-body"></textarea></label>
      <fieldset disabled>
        <label>Forward to <input id="disabled-forward-to" value="nobody@example.com"></label>
      </fieldset>
      <label>Forward to <input id="forward-to"></label>
      <div class="email-send clickable" role="toolbar" tabindex="0" style="cursor: pointer;">
        <span>Reply</span>
        <span>Forward</span>
        <span id="send-reply" class="icon-send" style="display: inline-block; width: 18px; height: 18px;" aria-label=""></span>
      </div>
    </div>
  </section>
  <output id="out"></output>
`;
let currentSender = '';
document.querySelectorAll('.mail-row').forEach(row => {
  row.addEventListener('click', event => {
    if (event.target.closest('.star')) return;
    currentSender = row.dataset.sender;
    document.querySelector('#detail').hidden = false;
    document.querySelector('#opened').textContent = currentSender;
  });
  row.querySelector('.star').addEventListener('click', () => {
    row.dataset.important = 'true';
    document.querySelector('#out').textContent = 'important:' + row.dataset.sender;
  });
});
document.querySelector('#reply').addEventListener('click', () => {
  document.querySelector('#composer').hidden = false;
  document.querySelector('#reply-body').focus();
});
document.querySelector('#forward').addEventListener('click', () => {
  document.querySelector('#composer').hidden = false;
  document.querySelector('#forward-to').focus();
});
document.querySelector('#send-reply').addEventListener('click', () => {
  const forwardTo = document.querySelector('#forward-to').value;
  document.querySelector('#out').textContent = forwardTo
    ? 'forward:' + currentSender + ':' + forwardTo
    : 'reply:' + currentSender + ':' + document.querySelector('#reply-body').value;
});
"##
        .to_string(),
    ]);

    let reply = act_instruction(
        &env,
        "Find the message by Carlynne and reply to them with the text \"Ornare commodo.\".",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({out: document.querySelector('#out').textContent, readonlyReply: document.querySelector('#readonly-reply').value, disabledReply: document.querySelector('#disabled-reply').value, replyBody: document.querySelector('#reply-body').value})".to_string(),
    ]);
    let result_state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse reply state");

    assert_eq!(result_state["out"], "reply:Carlynne:Ornare commodo.");
    assert_eq!(result_state["readonlyReply"], "archived reply");
    assert_eq!(result_state["disabledReply"], "disabled reply");
    assert_eq!(result_state["replyBody"], "Ornare commodo.");
    assert_eq!(reply["plan"]["action"], "scoped_item_workflow");
    assert_eq!(reply["plan"]["capability"]["name"], "scoped-item-workflow");
    assert_eq!(reply["plan"]["evidence"]["itemQuery"], "Carlynne");
    let reply_fill_target = reply["result"]["scopedWorkflow"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "fill_text")
        .and_then(|step| step["target"]["selector"].as_str());
    assert_eq!(reply_fill_target, Some("#reply-body"));
    assert_eq!(reply["verification"]["status"], "observed");

    let important = act_instruction(
        &env,
        "Find the message by Anne and click the star icon to mark it as important.",
    );
    let important_result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(important_result["result"], "important:Anne");
    assert_eq!(important["plan"]["action"], "scoped_item_workflow");
    assert_eq!(important["plan"]["evidence"]["itemQuery"], "Anne");
    assert_eq!(
        important["plan"]["params"]["completionHint"],
        serde_json::Value::Null
    );
    assert_eq!(important["verification"]["status"], "observed");

    let forward = act_instruction(&env, "Send the email from Carlynne to Anne.");
    let forward_result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({out: document.querySelector('#out').textContent, disabledForwardTo: document.querySelector('#disabled-forward-to').value, forwardTo: document.querySelector('#forward-to').value})".to_string(),
    ]);
    let forward_state: Value =
        serde_json::from_str(forward_result["result"].as_str().unwrap_or("{}"))
            .expect("parse forward state");

    assert_eq!(forward_state["out"], "forward:Carlynne:Anne");
    assert_eq!(forward_state["disabledForwardTo"], "nobody@example.com");
    assert_eq!(forward_state["forwardTo"], "Anne");
    assert_eq!(forward["analysis"]["kind"], "click");
    assert_eq!(forward["plan"]["action"], "scoped_item_workflow");
    assert_eq!(forward["plan"]["evidence"]["itemQuery"], "Carlynne");
    assert_eq!(forward["plan"]["evidence"]["actionHint"], "forward");
    let forward_fill_target = forward["result"]["scopedWorkflow"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "fill_text")
        .and_then(|step| step["target"]["selector"].as_str());
    assert_eq!(forward_fill_target, Some("#forward-to"));
    assert_eq!(forward["verification"]["status"], "observed");

    env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent = ''; document.querySelector('#forward-to').value = '';".to_string(),
    ]);
    let information_forward = act_instruction(
        &env,
        "Please forward the information from Carlynne to Anne.",
    );
    let information_forward_result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(
        information_forward_result["result"],
        "forward:Carlynne:Anne"
    );
    assert_eq!(
        information_forward["plan"]["action"],
        "scoped_item_workflow"
    );
    assert_eq!(
        information_forward["plan"]["evidence"]["itemQuery"],
        "Carlynne"
    );
    assert_eq!(
        information_forward["plan"]["evidence"]["actionHint"],
        "forward"
    );

    env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent = ''; document.querySelector('#reply-body').value = ''; document.querySelector('#forward-to').value = '';".to_string(),
    ]);
    let quoted_reply = act_instruction(
        &env,
        "Respond \"Vitae mattis dictum. Ut.\" to the message by Carlynne.",
    );
    let quoted_reply_result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(
        quoted_reply_result["result"],
        "reply:Carlynne:Vitae mattis dictum. Ut."
    );
    assert_eq!(quoted_reply["plan"]["action"], "scoped_item_workflow");
    assert_eq!(
        quoted_reply["plan"]["params"]["fillText"],
        "Vitae mattis dictum. Ut."
    );

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_item_workflows_with_writable_comboboxes_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-item-combobox-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="inbox" role="list">
    <div class="mail-row" role="listitem" tabindex="0" data-sender="Anne">
      <span class="from">Anne</span>
      <span class="subject">Budget notes</span>
    </div>
    <div class="mail-row" role="listitem" tabindex="0" data-sender="Carlynne">
      <span class="from">Carlynne</span>
      <span class="subject">Launch copy</span>
    </div>
  </div>
  <section id="detail" hidden>
    <h2 id="opened"></h2>
    <button id="forward">Forward</button>
    <div id="composer" hidden>
      <recipient-combo id="forward-to" role="combobox" aria-label="Forward to" tabindex="0"></recipient-combo>
      <button id="send">Send</button>
    </div>
  </section>
  <output id="out"></output>
`;
customElements.define('recipient-combo', class extends HTMLElement {
  constructor() {
    super();
    this._value = '';
  }
  connectedCallback() {
    this.style.display = 'inline-block';
    this.style.width = '180px';
    this.style.height = '28px';
    this.style.border = '1px solid rgb(80, 80, 80)';
  }
  get value() {
    return this._value;
  }
  set value(next) {
    this._value = String(next);
    this.textContent = this._value;
  }
});
let currentSender = '';
document.querySelectorAll('.mail-row').forEach(row => {
  row.addEventListener('click', () => {
    currentSender = row.dataset.sender;
    document.querySelector('#detail').hidden = false;
    document.querySelector('#opened').textContent = currentSender;
  });
});
document.querySelector('#forward').addEventListener('click', () => {
  document.querySelector('#composer').hidden = false;
  document.querySelector('#forward-to').focus();
});
document.querySelector('#send').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    'forward:' + currentSender + ':' + document.querySelector('#forward-to').value;
});
"##
        .to_string(),
    ]);

    let forward = act_instruction(&env, "Send the email from Carlynne to Anne.");
    let forward_result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({out: document.querySelector('#out').textContent, forwardTo: document.querySelector('#forward-to').value})".to_string(),
    ]);
    let forward_state: Value =
        serde_json::from_str(forward_result["result"].as_str().unwrap_or("{}"))
            .expect("parse combobox forward state");

    assert_eq!(forward_state["out"], "forward:Carlynne:Anne");
    assert_eq!(forward_state["forwardTo"], "Anne");
    assert_eq!(forward["plan"]["action"], "scoped_item_workflow");
    assert_eq!(forward["plan"]["evidence"]["itemQuery"], "Carlynne");
    assert_eq!(forward["plan"]["evidence"]["actionHint"], "forward");
    let forward_fill_target = forward["result"]["scopedWorkflow"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "fill_text")
        .and_then(|step| step["target"]["selector"].as_str());
    assert_eq!(forward_fill_target, Some("#forward-to"));
    assert_eq!(forward["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_item_workflows_with_custom_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-item-custom-value-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="inbox" role="list">
    <div class="mail-row" role="listitem" tabindex="0" data-sender="Anne">
      <span class="from">Anne</span>
      <span class="subject">Budget notes</span>
    </div>
    <div class="mail-row" role="listitem" tabindex="0" data-sender="Carlynne">
      <span class="from">Carlynne</span>
      <span class="subject">Launch copy</span>
    </div>
  </div>
  <section id="detail" hidden>
    <h2 id="opened"></h2>
    <button id="reply">Reply</button>
    <div id="composer" hidden>
      <message-field id="reply-body" label="Reply text"></message-field>
      <button id="send">Send</button>
    </div>
  </section>
  <output id="out"></output>
`;
customElements.define('message-field', class extends HTMLElement {
  constructor() {
    super();
    this._value = '';
  }
  connectedCallback() {
    this.style.display = 'block';
    this.style.width = '240px';
    this.style.height = '48px';
    this.style.border = '1px solid rgb(80, 80, 80)';
  }
  get value() {
    return this._value;
  }
  set value(next) {
    this._value = String(next);
    this.textContent = this._value;
  }
});
let currentSender = '';
document.querySelectorAll('.mail-row').forEach(row => {
  row.addEventListener('click', () => {
    currentSender = row.dataset.sender;
    document.querySelector('#detail').hidden = false;
    document.querySelector('#opened').textContent = currentSender;
  });
});
document.querySelector('#reply').addEventListener('click', () => {
  document.querySelector('#composer').hidden = false;
  document.querySelector('#reply-body').focus();
});
document.querySelector('#send').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    'reply:' + currentSender + ':' + document.querySelector('#reply-body').value;
});
"##
        .to_string(),
    ]);

    let reply = act_instruction(
        &env,
        "Find the message by Carlynne and reply to them with the text \"Custom fields work.\".",
    );
    let reply_result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({out: document.querySelector('#out').textContent, replyBody: document.querySelector('#reply-body').value})".to_string(),
    ]);
    let reply_state: Value = serde_json::from_str(reply_result["result"].as_str().unwrap_or("{}"))
        .expect("parse custom value reply state");

    assert_eq!(reply_state["out"], "reply:Carlynne:Custom fields work.");
    assert_eq!(reply_state["replyBody"], "Custom fields work.");
    assert_eq!(reply["plan"]["action"], "scoped_item_workflow");
    assert_eq!(reply["plan"]["evidence"]["itemQuery"], "Carlynne");
    assert_eq!(reply["plan"]["evidence"]["actionHint"], "reply");
    let reply_fill_target = reply["result"]["scopedWorkflow"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "fill_text")
        .and_then(|step| step["target"]["selector"].as_str());
    assert_eq!(reply_fill_target, Some("#reply-body"));
    assert_eq!(reply["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_item_workflows_with_custom_checkable_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-item-custom-checkable-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="inbox" role="list">
    <div class="mail-row" role="listitem" tabindex="0" data-sender="Anne">
      <span class="from">Anne</span>
      <span class="subject">Budget notes</span>
    </div>
    <div class="mail-row" role="listitem" tabindex="0" data-sender="Carlynne">
      <span class="from">Carlynne</span>
      <span class="subject">Launch copy</span>
    </div>
  </div>
  <section id="detail" hidden>
    <h2 id="opened"></h2>
    <notification-toggle id="alerts" label="Alerts"></notification-toggle>
  </section>
  <output id="out"></output>
`;
customElements.define('notification-toggle', class extends HTMLElement {
  constructor() {
    super();
    this._checked = false;
  }
  connectedCallback() {
    this.style.display = 'inline-block';
    this.style.width = '140px';
    this.style.height = '28px';
    this.style.border = '1px solid rgb(80, 80, 80)';
    this.textContent = 'Alerts off';
    this.addEventListener('click', () => {
      this.checked = !this.checked;
      document.querySelector('#out').textContent =
        'alerts:' + document.querySelector('#opened').textContent + ':' + String(this.checked);
    });
  }
  get checked() {
    return this._checked;
  }
  set checked(next) {
    this._checked = Boolean(next);
    this.textContent = this._checked ? 'Alerts on' : 'Alerts off';
  }
});
document.querySelectorAll('.mail-row').forEach(row => {
  row.addEventListener('click', () => {
    document.querySelector('#detail').hidden = false;
    document.querySelector('#opened').textContent = row.dataset.sender;
    document.querySelector('#alerts').checked = false;
    document.querySelector('#out').textContent = '';
  });
});
"##
        .to_string(),
    ]);

    let alerts = act_instruction(&env, "Find the message by Carlynne and turn on alerts.");
    let alerts_result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({out: document.querySelector('#out').textContent, checked: document.querySelector('#alerts').checked})".to_string(),
    ]);
    let alerts_state: Value =
        serde_json::from_str(alerts_result["result"].as_str().unwrap_or("{}"))
            .expect("parse custom checkable workflow state");

    assert_eq!(alerts_state["out"], "alerts:Carlynne:true");
    assert_eq!(alerts_state["checked"], true);
    assert_eq!(alerts["plan"]["action"], "scoped_item_workflow");
    assert_eq!(alerts["plan"]["evidence"]["itemQuery"], "Carlynne");
    assert_eq!(alerts["plan"]["evidence"]["actionHint"], "turn on alerts");
    let action_target = alerts["result"]["scopedWorkflow"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "click_action")
        .and_then(|step| step["target"]["selector"].as_str());
    assert_eq!(action_target, Some("#alerts"));
    assert_eq!(alerts["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_item_workflows_with_short_row_identifiers_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-short-row-id");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="inventory" role="list">
    <div class="product-row" role="listitem" data-product="Product A">
      <span class="name">Product A</span>
      <span class="sku">SKU-100</span>
      <button class="delete">Delete</button>
    </div>
    <div class="product-row" role="listitem" data-product="Product B">
      <span class="name">Product B</span>
      <span class="sku">SKU-200</span>
      <button class="delete">Delete</button>
    </div>
  </div>
  <output id="out"></output>
`;
document.querySelectorAll('.product-row').forEach(row => {
  row.querySelector('.delete').addEventListener('click', () => {
    document.querySelector('#out').textContent = 'delete:' + row.dataset.product;
  });
});
"##
        .to_string(),
    ]);

    let deleted = act_instruction(&env, "Find the item named Product B and delete it.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "delete:Product B");
    assert_eq!(deleted["plan"]["action"], "scoped_item_workflow");
    assert_eq!(
        deleted["plan"]["capability"]["name"],
        "scoped-item-workflow"
    );
    assert_eq!(deleted["plan"]["evidence"]["itemQuery"], "Product B");
    assert_eq!(deleted["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_item_workflows_with_accessible_names_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-accessible-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="labels" hidden>
    <span id="product-a-name">Product A</span>
    <span id="product-b-name">Product B</span>
    <span id="delete-label">Delete</span>
  </div>
  <div id="inventory" role="list">
    <div class="record" role="listitem" aria-labelledby="product-a-name">
      <span class="sku">SKU-100</span>
      <button class="icon-button" aria-labelledby="delete-label"></button>
    </div>
    <div class="record" role="listitem" aria-labelledby="product-b-name">
      <span class="sku">SKU-200</span>
      <button class="icon-button" aria-labelledby="delete-label"></button>
    </div>
  </div>
  <output id="out"></output>
`;
document.querySelectorAll('.record').forEach(row => {
  row.querySelector('button').addEventListener('click', () => {
    const label = document.getElementById(row.getAttribute('aria-labelledby')).textContent;
    document.querySelector('#out').textContent = 'delete:' + label;
  });
});
"##
        .to_string(),
    ]);

    let deleted = act_instruction(&env, "Find the item named Product B and delete it.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "delete:Product B");
    assert_eq!(deleted["plan"]["action"], "scoped_item_workflow");
    assert_eq!(
        deleted["plan"]["capability"]["name"],
        "scoped-item-workflow"
    );
    assert_eq!(deleted["plan"]["evidence"]["itemQuery"], "Product B");
    assert_eq!(deleted["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_item_workflows_with_slotted_component_text_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-slotted-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <inventory-row data-product="Product A">
    <span slot="name">Product A</span>
  </inventory-row>
  <inventory-row data-product="Product B">
    <span slot="name">Product B</span>
  </inventory-row>
  <output id="out"></output>
`;
customElements.define('inventory-row', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <div class="record" role="listitem">
        <slot name="name"></slot>
        <button class="delete" aria-label="Delete"></button>
      </div>
    `;
    root.querySelector('.delete').addEventListener('click', () => {
      document.querySelector('#out').textContent = 'delete:' + this.dataset.product;
    });
  }
});
"##
        .to_string(),
    ]);

    let deleted = act_instruction(&env, "Find the item named Product B and delete it.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "delete:Product B");
    assert_eq!(deleted["plan"]["action"], "scoped_item_workflow");
    assert_eq!(
        deleted["plan"]["capability"]["name"],
        "scoped-item-workflow"
    );
    assert_eq!(deleted["plan"]["evidence"]["itemQuery"], "Product B");
    assert_eq!(deleted["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_item_workflows_with_svg_symbol_action_titles_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-svg-symbol-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg width="0" height="0" style="position:absolute">
    <defs>
      <symbol id="action-99" viewBox="0 0 16 16">
        <title>Delete</title>
        <path d="M2 3h12v12H2z"></path>
      </symbol>
    </defs>
  </svg>
  <div id="inventory" role="list">
    <div class="product-row" role="listitem" data-product="Product A">
      <span class="name">Product A</span>
      <button class="icon-button"><svg aria-hidden="true"><use href="#action-99"></use></svg></button>
    </div>
    <div class="product-row" role="listitem" data-product="Product B">
      <span class="name">Product B</span>
      <button class="icon-button"><svg aria-hidden="true"><use href="#action-99"></use></svg></button>
    </div>
  </div>
  <output id="out"></output>
`;
document.querySelectorAll('.product-row').forEach(row => {
  row.querySelector('button').addEventListener('click', () => {
    document.querySelector('#out').textContent = 'delete:' + row.dataset.product;
  });
});
"##
        .to_string(),
    ]);

    let deleted = act_instruction(&env, "Find the item named Product B and delete it.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "delete:Product B");
    assert_eq!(deleted["plan"]["action"], "scoped_item_workflow");
    assert_eq!(
        deleted["plan"]["capability"]["name"],
        "scoped-item-workflow"
    );
    assert_eq!(deleted["plan"]["evidence"]["itemQuery"], "Product B");
    assert_eq!(deleted["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_item_workflows_in_open_shadow_roots_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-shadow-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <inventory-panel></inventory-panel>
  <output id="out"></output>
`;
customElements.define('inventory-panel', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <div id="inventory" role="list">
        <div class="product-row" role="listitem" data-product="Product A">
          <span class="name">Product A</span>
          <button class="delete">Delete</button>
        </div>
        <div class="product-row" role="listitem" data-product="Product B">
          <span class="name">Product B</span>
          <button class="delete">Delete</button>
        </div>
      </div>
    `;
    root.querySelectorAll('.product-row').forEach(row => {
      row.querySelector('.delete').addEventListener('click', () => {
        document.querySelector('#out').textContent = 'delete:' + row.dataset.product;
      });
    });
  }
});
"##
        .to_string(),
    ]);

    let deleted = act_instruction(&env, "Find the item named Product B and delete it.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "delete:Product B");
    assert_eq!(deleted["plan"]["action"], "scoped_item_workflow");
    assert_eq!(
        deleted["plan"]["capability"]["name"],
        "scoped-item-workflow"
    );
    assert_eq!(deleted["plan"]["evidence"]["itemQuery"], "Product B");
    assert_eq!(deleted["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_handles_scoped_item_workflows_in_same_origin_iframes_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scoped-iframe-workflow");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <iframe id="inventory-frame" title="Inventory"></iframe>
  <output id="out"></output>
`;
const frame = document.querySelector('#inventory-frame');
const doc = frame.contentDocument;
doc.open();
doc.write(`
  <div id="inventory" role="list">
    <div class="product-row" role="listitem" data-product="Product A">
      <span class="name">Product A</span>
      <button class="delete">Delete</button>
    </div>
    <div class="product-row" role="listitem" data-product="Product B">
      <span class="name">Product B</span>
      <button class="delete">Delete</button>
    </div>
  </div>
`);
doc.close();
doc.querySelectorAll('.product-row').forEach(row => {
  row.querySelector('.delete').addEventListener('click', () => {
    document.querySelector('#out').textContent = 'delete:' + row.dataset.product;
  });
});
"##
        .to_string(),
    ]);

    let deleted = act_instruction(&env, "Find the item named Product B and delete it.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "delete:Product B");
    assert_eq!(deleted["plan"]["action"], "scoped_item_workflow");
    assert_eq!(
        deleted["plan"]["capability"]["name"],
        "scoped-item-workflow"
    );
    assert_eq!(deleted["plan"]["evidence"]["itemQuery"], "Product B");
    assert_eq!(deleted["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_offscreen_scroll_list_options_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scroll-list");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="cities">Choices</label>
  <select id="cities" multiple size="4" style="width: 160px; height: 72px;">
    <option>Choice Alpha</option>
    <option>Choice Beta</option>
    <option>Choice Gamma</option>
    <option>Choice Delta</option>
    <option>Choice Epsilon</option>
    <option>Choice Zeta</option>
    <option>Choice Eta</option>
    <option>Choice Theta</option>
    <option>Choice Iota</option>
    <option>Choice Kappa</option>
  </select>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
submit.addEventListener('click', () => {
  out.textContent = Array.from(cities.selectedOptions).map(option => option.textContent).join(',');
});
"##
        .to_string(),
    ]);

    let list = act_instruction(
        &env,
        "Select Choice Theta and Choice Iota from the scroll list and click Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({selected: Array.from(document.querySelector('#cities').selectedOptions).map(option => option.textContent), out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse list state");

    assert_eq!(
        state["selected"],
        serde_json::json!(["Choice Theta", "Choice Iota"])
    );
    assert_eq!(state["out"], "Choice Theta,Choice Iota");
    assert_eq!(list["plan"]["action"], "sequence");
    assert_eq!(list["plan"]["steps"][0]["action"], "select_option");
    assert_eq!(list["plan"]["capability"]["name"], "list-option-selection");
    assert_eq!(
        list["result"]["steps"][0]["selected"]["mode"],
        "native-select"
    );
    assert_eq!(list["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_multiple_aria_listbox_options_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-aria-multiselect-listbox");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <div id="city-list" role="listbox" aria-label="City list" aria-multiselectable="true">
      <div role="option" data-value="berlin" aria-selected="false">Berlin</div>
      <div role="option" data-value="lima" aria-selected="false">Lima</div>
      <div role="option" data-value="oslo" aria-selected="false">Oslo</div>
      <div role="option" data-value="zurich" aria-selected="false">Zurich</div>
    </div>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelectorAll('[role=option]').forEach(option => {
  option.addEventListener('click', () => {
    const next = option.getAttribute('aria-selected') !== 'true';
    option.setAttribute('aria-selected', String(next));
  });
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = Array.from(
    document.querySelectorAll('[role=option][aria-selected=true]')
  ).map(option => option.textContent).join(',');
});
"##
        .to_string(),
    ]);

    let list = act_instruction(
        &env,
        "Select Lima and Oslo from the City list and click Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({selected: Array.from(document.querySelectorAll('[role=option][aria-selected=true]')).map(option => option.textContent), out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse aria listbox state");

    assert_eq!(state["selected"], serde_json::json!(["Lima", "Oslo"]));
    assert_eq!(state["out"], "Lima,Oslo");
    assert_eq!(list["plan"]["action"], "sequence");
    assert_eq!(list["plan"]["steps"][0]["action"], "select_option");
    assert_eq!(
        list["result"]["steps"][0]["selected"]["mode"],
        "custom-option"
    );
    assert_eq!(
        list["result"]["steps"][0]["selected"]["actual"]["options"]
            .as_array()
            .map(|options| options.len()),
        Some(2)
    );
    assert_eq!(list["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_accessibly_named_custom_options_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-accessibly-named-options");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <div id="city-list" role="listbox" aria-label="City list" aria-multiselectable="true">
      <div role="option" aria-label="Berlin" aria-selected="false" tabindex="0" style="height: 20px; width: 140px;"></div>
      <div role="option" aria-label="Lima" aria-selected="false" tabindex="0" style="height: 20px; width: 140px;"></div>
      <div role="option" aria-label="Oslo" aria-selected="false" tabindex="0" style="height: 20px; width: 140px;"></div>
      <div role="option" aria-label="Zurich" aria-selected="false" tabindex="0" style="height: 20px; width: 140px;"></div>
    </div>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelectorAll('[role=option]').forEach(option => {
  option.addEventListener('click', () => {
    const next = option.getAttribute('aria-selected') !== 'true';
    option.setAttribute('aria-selected', String(next));
  });
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = Array.from(
    document.querySelectorAll('[role=option][aria-selected=true]')
  ).map(option => option.getAttribute('aria-label')).join(',');
});
"##
        .to_string(),
    ]);

    let list = act_instruction(
        &env,
        "Select Lima and Oslo from the City list and click Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({selected: Array.from(document.querySelectorAll('[role=option][aria-selected=true]')).map(option => option.getAttribute('aria-label')), out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse accessible option state");

    assert_eq!(state["selected"], serde_json::json!(["Lima", "Oslo"]));
    assert_eq!(state["out"], "Lima,Oslo");
    assert_eq!(list["plan"]["action"], "sequence");
    assert_eq!(list["plan"]["steps"][0]["action"], "select_option");
    assert_eq!(
        list["result"]["steps"][0]["selected"]["actual"]["options"][0]["text"],
        "Lima"
    );
    assert_eq!(
        list["result"]["steps"][0]["selected"]["actual"]["options"][1]["text"],
        "Oslo"
    );
    assert_eq!(list["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_skips_disabled_custom_options_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-disabled-custom-options");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <div id="city-list" role="listbox" aria-label="City list">
      <div role="option" aria-label="Lima" aria-disabled="true" data-value="disabled" tabindex="0" style="height: 20px; width: 140px;"></div>
      <div role="option" aria-label="Lima" data-value="enabled" tabindex="0" style="height: 20px; width: 140px;"></div>
      <div role="option" aria-label="Oslo" data-value="oslo" tabindex="0" style="height: 20px; width: 140px;"></div>
    </div>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelectorAll('[role=option]').forEach(option => {
  option.addEventListener('click', () => {
    if (option.getAttribute('aria-disabled') === 'true') return;
    document.querySelectorAll('[role=option]').forEach(other => {
      other.setAttribute('aria-selected', 'false');
    });
    option.setAttribute('aria-selected', 'true');
  });
});
document.querySelector('#submit').addEventListener('click', () => {
  const selected = document.querySelector('[role=option][aria-selected=true]');
  document.querySelector('#out').textContent = selected ? selected.dataset.value : '';
});
"##
        .to_string(),
    ]);

    let list = act_instruction(&env, "Select Lima from the City list and click Submit.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({selected: document.querySelector('[role=option][aria-selected=true]')?.dataset.value || '', out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse disabled option state");

    assert_eq!(state["selected"], "enabled");
    assert_eq!(state["out"], "enabled");
    assert_eq!(list["plan"]["action"], "sequence");
    assert_eq!(list["plan"]["steps"][0]["action"], "select_option");
    assert_eq!(
        list["result"]["steps"][0]["selected"]["matched"][0]["value"],
        "enabled"
    );
    assert_eq!(list["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_aria_tree_items_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-aria-tree-selection");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <div id="department-tree" role="tree" aria-label="Department tree">
      <div role="treeitem" aria-selected="false" data-value="sales">Sales</div>
      <div role="treeitem" aria-selected="false" data-value="engineering">Engineering</div>
      <div role="treeitem" aria-selected="false" data-value="support">Support</div>
    </div>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelectorAll('[role=treeitem]').forEach(item => {
  item.addEventListener('click', () => {
    document.querySelectorAll('[role=treeitem]').forEach(other => {
      other.setAttribute('aria-selected', String(other === item));
    });
  });
});
document.querySelector('#submit').addEventListener('click', () => {
  const selected = document.querySelector('[role=treeitem][aria-selected=true]');
  document.querySelector('#out').textContent = selected ? selected.textContent : '';
});
"##
        .to_string(),
    ]);

    let tree = act_instruction(
        &env,
        "Select Engineering from the Department tree and click Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({selected: document.querySelector('[role=treeitem][aria-selected=true]')?.textContent || '', out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse aria tree state");

    assert_eq!(state["selected"], "Engineering");
    assert_eq!(state["out"], "Engineering");
    assert_eq!(tree["plan"]["action"], "sequence");
    assert_eq!(tree["plan"]["steps"][0]["action"], "select_option");
    assert_eq!(tree["plan"]["capability"]["name"], "list-option-selection");
    assert_eq!(
        tree["result"]["steps"][0]["selected"]["mode"],
        "custom-option"
    );
    assert_eq!(tree["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_shadow_dom_controlled_options_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-controlled-options");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <region-picker></region-picker>
  <output id="out"></output>
`;
customElements.define('region-picker', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <label for="region">Region</label>
      <input id="region" role="combobox" aria-controls="region-options" aria-haspopup="listbox" readonly>
      <div id="region-options" role="listbox">
        <div role="option" data-value="na" aria-selected="false">North America</div>
        <div role="option" data-value="emea" aria-selected="false">Europe, Middle East, and Africa</div>
        <div role="option" data-value="apac" aria-selected="false">Asia Pacific</div>
      </div>
      <button id="save">Save</button>
    `;
    const input = root.querySelector('#region');
    for (const option of root.querySelectorAll('[role=option]')) {
      option.addEventListener('click', () => {
        root.querySelectorAll('[role=option]').forEach(other => {
          other.setAttribute('aria-selected', String(other === option));
        });
        input.value = option.dataset.value;
        input.dispatchEvent(new Event('change', { bubbles: true, composed: true }));
      });
    }
    root.querySelector('#save').addEventListener('click', () => {
      document.querySelector('#out').textContent = input.value;
    });
  }
});
"##
        .to_string(),
    ]);

    let selection = act_instruction(&env, "Select Asia Pacific from Region and press Save.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "apac");
    assert_eq!(selection["plan"]["action"], "sequence");
    assert_eq!(selection["plan"]["steps"][0]["action"], "select_option");
    assert_eq!(selection["plan"]["steps"][1]["action"], "click");
    assert_eq!(
        selection["result"]["steps"][0]["selected"]["mode"],
        "custom-option"
    );
    assert_eq!(selection["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_aria_owned_options_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-aria-owned-options");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <button id="primary-region" role="combobox" aria-label="Region" aria-haspopup="listbox" aria-owns="primary-region-options">Primary region</button>
    <div id="primary-region-options" role="listbox" hidden>
      <div role="option" data-value="emea" aria-selected="false">Europe, Middle East, and Africa</div>
      <div role="option" data-value="latam" aria-selected="false">Latin America</div>
    </div>
    <button id="backup-region" role="combobox" aria-label="Region" aria-haspopup="listbox" aria-owns="backup-region-options">Backup region</button>
    <div id="backup-region-options" role="listbox" hidden>
      <div role="option" data-value="apac" aria-selected="false">Asia Pacific</div>
      <div role="option" data-value="na" aria-selected="false">North America</div>
    </div>
    <button id="save">Save</button>
    <output id="out"></output>
  </section>
`;
for (const option of document.querySelectorAll('[role=option]')) {
  option.addEventListener('click', () => {
    const owner = option.closest('[role=listbox]');
    owner.querySelectorAll('[role=option]').forEach(other => {
      other.setAttribute('aria-selected', String(other === option));
    });
    const combo = document.querySelector(`[aria-owns="${owner.id}"]`);
    combo.dataset.value = option.dataset.value;
    combo.textContent = option.textContent;
  });
}
for (const combo of document.querySelectorAll('[role=combobox]')) {
  combo.addEventListener('click', () => {
    const owner = document.getElementById(combo.getAttribute('aria-owns'));
    owner.hidden = false;
  });
}
document.querySelector('#save').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    document.querySelector('#backup-region').dataset.value || '';
});
"##
        .to_string(),
    ]);

    let selection = act_instruction(&env, "Select Asia Pacific from Region and press Save.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "apac");
    assert_eq!(selection["plan"]["action"], "sequence");
    assert_eq!(selection["plan"]["steps"][0]["action"], "select_option");
    assert_eq!(
        selection["plan"]["steps"][0]["params"]["selector"],
        "#backup-region"
    );
    assert_eq!(
        selection["result"]["steps"][0]["selected"]["mode"],
        "custom-option"
    );
    assert_eq!(selection["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_autocomplete_suggestions_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-autocomplete");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="country">Country</label>
  <input id="country" aria-autocomplete="list" autocomplete="off">
  <ul id="suggestions" role="listbox" hidden></ul>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const countries = ['Comoros', 'Colombia', 'Canada', 'Cambodia'];
const input = document.querySelector('#country');
const suggestions = document.querySelector('#suggestions');
const out = document.querySelector('#out');
input.addEventListener('input', () => {
  suggestions.innerHTML = '';
  const prefix = input.value.toLowerCase();
  if ('comoros'.startsWith(prefix)) {
    const disabledGroup = document.createElement('li');
    disabledGroup.setAttribute('aria-disabled', 'true');
    const disabledOption = document.createElement('span');
    disabledOption.setAttribute('role', 'option');
    disabledOption.textContent = 'Comoros';
    disabledOption.addEventListener('click', () => {
      input.value = 'disabled-choice';
    });
    disabledGroup.appendChild(disabledOption);
    suggestions.appendChild(disabledGroup);
  }
  countries
    .filter(country => country.toLowerCase().startsWith(prefix))
    .forEach(country => {
      const option = document.createElement('li');
      option.setAttribute('role', 'option');
      option.textContent = country;
      option.addEventListener('click', () => {
        input.value = country;
        input.dispatchEvent(new Event('change', { bubbles: true }));
        suggestions.hidden = true;
      });
      suggestions.appendChild(option);
    });
  suggestions.hidden = suggestions.children.length === 0;
});
submit.addEventListener('click', () => {
  out.textContent = input.value;
});
"##
        .to_string(),
    ]);

    let autocomplete = act_instruction(
        &env,
        "Enter an item that starts with \"Com\" and ends with \"os\".",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#country').value, out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse autocomplete state");

    assert_eq!(state["value"], "Comoros");
    assert_eq!(state["out"], "Comoros");
    assert_eq!(autocomplete["plan"]["action"], "sequence");
    assert_eq!(
        autocomplete["plan"]["steps"][0]["action"],
        "autocomplete_select"
    );
    assert_eq!(
        autocomplete["result"]["steps"][0]["autocomplete"]["selected"],
        "Comoros"
    );
    assert_eq!(autocomplete["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_autocomplete_suggestions_on_custom_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-host-autocomplete");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="city">City</label>
  <city-combobox
    id="city"
    role="combobox"
    aria-label="City"
    aria-autocomplete="list"
    aria-controls="suggestions"
    data-field="city"
    tabindex="0"
    style="display:inline-block; min-width: 10rem; min-height: 1.5rem; border: 1px solid #777;"></city-combobox>
  <ul id="suggestions" role="listbox" hidden></ul>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
if (!customElements.get('city-combobox')) {
  customElements.define('city-combobox', class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    get value() {
      return this._value || '';
    }
    set value(next) {
      this._value = String(next);
      this.textContent = this._value;
      this.setAttribute('data-current-value', this._value);
    }
  });
}
const cities = ['Comoros City', 'Colombo', 'Copenhagen'];
const input = document.querySelector('#city');
const suggestions = document.querySelector('#suggestions');
input.addEventListener('input', () => {
  suggestions.innerHTML = '';
  const prefix = input.value.toLowerCase();
  cities
    .filter(city => city.toLowerCase().startsWith(prefix))
    .forEach(city => {
      const option = document.createElement('li');
      option.setAttribute('role', 'option');
      option.textContent = city;
      option.addEventListener('click', () => {
        input.value = city;
        input.dispatchEvent(new Event('change', { bubbles: true }));
        suggestions.hidden = true;
      });
      suggestions.appendChild(option);
    });
  suggestions.hidden = suggestions.children.length === 0;
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = input.value;
});
"##
        .to_string(),
    ]);

    let autocomplete = act_instruction(
        &env,
        "Enter a city that starts with \"Com\" and ends with \"City\", then submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#city').value, out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse autocomplete state");

    assert_eq!(state["value"], "Comoros City");
    assert_eq!(state["out"], "Comoros City");
    assert_eq!(autocomplete["plan"]["action"], "sequence");
    assert_eq!(
        autocomplete["plan"]["steps"][0]["action"],
        "autocomplete_select"
    );
    assert_eq!(
        autocomplete["result"]["steps"][0]["autocomplete"]["selected"],
        "Comoros City"
    );
    assert_eq!(autocomplete["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_native_datalist_values_then_completes_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-datalist-autocomplete-submit");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <label>City <input id="city" list="cities" autocomplete="off"></label>
    <datalist id="cities">
      <option value="Paris"></option>
      <option value="Prague"></option>
      <option value="Lima"></option>
    </datalist>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#city').value;
});
"##
        .to_string(),
    ]);

    let autocomplete = act_instruction(&env, "Select Paris from the City list and submit.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#city').value, out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse datalist state");

    assert_eq!(state["value"], "Paris");
    assert_eq!(state["out"], "Paris");
    assert_eq!(autocomplete["plan"]["action"], "sequence");
    assert_eq!(
        autocomplete["plan"]["steps"][0]["action"],
        "autocomplete_select"
    );
    assert_eq!(autocomplete["plan"]["steps"][1]["action"], "click");
    assert_eq!(
        autocomplete["result"]["steps"][0]["autocomplete"]["mode"],
        "datalist"
    );
    assert_eq!(autocomplete["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_aria_activedescendant_combobox_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-aria-activedescendant-combobox");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label for="airport">Airport</label>
  <input id="airport"
    role="combobox"
    aria-autocomplete="list"
    aria-controls="airport-options"
    aria-expanded="false"
    autocomplete="off">
  <ul id="airport-options" role="listbox" hidden></ul>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const airports = ['Comodoro Rivadavia', 'Comoros International', 'Colorado Springs'];
const input = document.querySelector('#airport');
const options = document.querySelector('#airport-options');
const out = document.querySelector('#out');
function render() {
  options.innerHTML = '';
  const prefix = input.value.toLowerCase();
  airports
    .filter(airport => airport.toLowerCase().startsWith(prefix))
    .forEach((airport, index) => {
      const option = document.createElement('li');
      option.id = `airport-option-${index}`;
      option.setAttribute('role', 'option');
      option.textContent = airport;
      option.addEventListener('click', () => {
        input.value = airport;
        input.dispatchEvent(new Event('change', { bubbles: true }));
        options.hidden = true;
      });
      options.appendChild(option);
    });
  const first = options.querySelector('[role=option]');
  if (first) {
    input.setAttribute('aria-expanded', 'true');
    input.setAttribute('aria-activedescendant', first.id);
    first.setAttribute('aria-selected', 'true');
    options.hidden = false;
  }
}
input.addEventListener('input', render);
input.addEventListener('keydown', event => {
  if (event.key !== 'Enter') return;
  const active = document.getElementById(input.getAttribute('aria-activedescendant'));
  if (!active) return;
  input.value = active.textContent;
  input.dispatchEvent(new Event('change', { bubbles: true }));
  options.hidden = true;
});
submit.addEventListener('click', () => {
  out.textContent = input.value;
});
"##
        .to_string(),
    ]);

    let autocomplete = act_instruction(
        &env,
        "Enter an airport that starts with \"Com\" and submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#airport').value, out: document.querySelector('#out').textContent, active: document.querySelector('#airport').getAttribute('aria-activedescendant')})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse aria combobox state");

    assert_eq!(state["value"], "Comodoro Rivadavia");
    assert_eq!(state["out"], "Comodoro Rivadavia");
    assert_eq!(state["active"], "airport-option-0");
    assert_eq!(autocomplete["plan"]["action"], "sequence");
    assert_eq!(
        autocomplete["plan"]["steps"][0]["action"],
        "autocomplete_select"
    );
    assert_eq!(
        autocomplete["result"]["steps"][0]["autocomplete"]["mode"],
        "aria-activedescendant"
    );
    assert_eq!(autocomplete["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_shadow_dom_autocomplete_suggestions_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-autocomplete");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <country-picker></country-picker>
  <output id="out"></output>
`;
customElements.define('country-picker', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <label for="country">Country</label>
      <input id="country" aria-autocomplete="list" autocomplete="off">
      <ul id="suggestions" role="listbox" hidden></ul>
      <button id="submit">Submit</button>
    `;
    const countries = ['Comoros', 'Colombia', 'Canada', 'Cambodia'];
    const input = root.querySelector('#country');
    const suggestions = root.querySelector('#suggestions');
    input.addEventListener('input', () => {
      suggestions.innerHTML = '';
      const prefix = input.value.toLowerCase();
      countries
        .filter(country => country.toLowerCase().startsWith(prefix))
        .forEach(country => {
          const option = document.createElement('li');
          option.setAttribute('role', 'option');
          option.textContent = country;
          option.addEventListener('click', () => {
            input.value = country;
            input.dispatchEvent(new Event('change', { bubbles: true }));
            suggestions.hidden = true;
          });
          suggestions.appendChild(option);
        });
      suggestions.hidden = suggestions.children.length === 0;
    });
    root.querySelector('#submit').addEventListener('click', () => {
      document.querySelector('#out').textContent = input.value;
    });
  }
});
"##
        .to_string(),
    ]);

    let autocomplete = act_instruction(
        &env,
        "Enter an item that starts with \"Com\" and ends with \"os\".",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "Comoros");
    assert_eq!(autocomplete["plan"]["action"], "sequence");
    assert_eq!(
        autocomplete["plan"]["steps"][0]["action"],
        "autocomplete_select"
    );
    assert_eq!(
        autocomplete["result"]["steps"][0]["autocomplete"]["selected"],
        "Comoros"
    );
    assert_eq!(autocomplete["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_scrolls_shadow_dom_containers_before_filling_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-scroll-fill");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <scroll-form></scroll-form>
  <output id="out"></output>
`;
customElements.define('scroll-form', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <div id="panel" style="height: 50px; overflow: auto; border: 1px solid black">
        <div style="height: 220px">Scroll for details</div>
      </div>
      <label for="answer">Answer field</label>
      <input id="answer">
      <button id="submit">Submit</button>
    `;
    const panel = root.querySelector('#panel');
    const answer = root.querySelector('#answer');
    root.querySelector('#submit').addEventListener('click', () => {
      document.querySelector('#out').textContent = JSON.stringify({
        value: answer.value,
        scrolled: panel.scrollTop > 0
      });
    });
  }
});
"##
        .to_string(),
    ]);

    let fill = act_instruction(
        &env,
        "Scroll the panel, enter \"Ready\" into the Answer field, and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["value"], "Ready");
    assert_eq!(state["scrolled"], true);
    assert_eq!(fill["plan"]["action"], "sequence");
    assert_eq!(fill["plan"]["steps"][0]["action"], "scroll_element");
    assert_eq!(fill["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_scrolls_before_filling_custom_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scroll-fill-custom-value-host");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="panel" style="height: 50px; overflow: auto; border: 1px solid black">
    <div style="height: 220px">Scroll for details</div>
  </div>
  <scroll-answer-box id="answer" aria-label="Answer field" data-field="answer" tabindex="0" style="display:block; min-height: 24px; width: 180px; border: 1px solid #888;"></scroll-answer-box>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
if (!customElements.get('scroll-answer-box')) {
  customElements.define('scroll-answer-box', class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    get value() {
      return this._value || '';
    }
    set value(next) {
      this._value = String(next);
      this.textContent = this._value;
      this.setAttribute('data-current-value', this._value);
    }
  });
}
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    value: document.querySelector('#answer').value,
    scrolled: document.querySelector('#panel').scrollTop > 0
  });
});
"##
        .to_string(),
    ]);

    let fill = act_instruction(
        &env,
        "Scroll the panel, enter \"Ready\" into the Answer field, and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["value"], "Ready");
    assert_eq!(state["scrolled"], true);
    assert_eq!(fill["plan"]["action"], "sequence");
    assert_eq!(fill["plan"]["steps"][0]["action"], "scroll_element");
    assert_eq!(fill["plan"]["steps"][1]["params"]["selector"], "#answer");
    assert_eq!(fill["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_extracts_words_from_scrollable_text_sources_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scroll-word-extract");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <textarea id="source" style="height: 42px; width: 180px">alpha beta gamma omega</textarea>
  <input id="answer" aria-label="Answer field">
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    value: document.querySelector('#answer').value,
    scrolled: document.querySelector('#source').scrollTop > 0
  });
});
"##
        .to_string(),
    ]);

    let fill = act_instruction(
        &env,
        "Find the last word in the text area, enter it into the text field and hit Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["value"], "omega");
    assert_eq!(fill["plan"]["action"], "sequence");
    assert_eq!(
        fill["plan"]["capability"]["name"],
        "scrollable-text-extract"
    );
    assert_eq!(fill["plan"]["steps"][0]["action"], "scroll_text_extract");
    assert_eq!(fill["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_extracts_words_into_custom_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scroll-word-extract-custom-value-host");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <textarea id="source" style="height: 42px; width: 180px">alpha beta gamma omega</textarea>
  <word-answer-box id="answer" aria-label="Answer field" data-field="answer" tabindex="0" style="display:block; min-height: 24px; width: 180px; border: 1px solid #888;"></word-answer-box>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
if (!customElements.get('word-answer-box')) {
  customElements.define('word-answer-box', class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    get value() {
      return this._value || '';
    }
    set value(next) {
      this._value = String(next);
      this.textContent = this._value;
      this.setAttribute('data-current-value', this._value);
    }
  });
}
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    value: document.querySelector('#answer').value,
    scrolled: document.querySelector('#source').scrollTop > 0
  });
});
"##
        .to_string(),
    ]);

    let fill = act_instruction(
        &env,
        "Find the last word in the text area, enter it into the Answer field and hit Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["value"], "omega");
    assert_eq!(fill["plan"]["action"], "sequence");
    assert_eq!(
        fill["plan"]["capability"]["name"],
        "scrollable-text-extract"
    );
    assert_eq!(fill["plan"]["steps"][0]["action"], "scroll_text_extract");
    assert_eq!(fill["plan"]["steps"][0]["params"]["target"], "#answer");
    assert_eq!(fill["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_scrolls_textareas_to_requested_edge_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-scroll-edge");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <textarea id="notes" style="height: 42px; width: 180px">line 1
line 2
line 3
line 4
line 5
line 6
line 7
line 8</textarea>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const notes = document.querySelector('#notes');
notes.scrollTop = notes.scrollHeight;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = String(notes.scrollTop);
});
"##
        .to_string(),
    ]);

    let scroll = act_instruction(
        &env,
        "Scroll the textarea to the top of the text hit submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "0");
    assert_eq!(scroll["plan"]["action"], "sequence");
    assert_eq!(scroll["plan"]["steps"][0]["action"], "scroll_element");
    assert_eq!(scroll["plan"]["steps"][0]["params"]["direction"], "up");

    env.stop();
}

#[test]
fn act_instruction_resizes_textareas_by_dragging_edges_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-resize-edge");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <textarea id="notes" style="display:block; height: 52px; width: 150px; resize: both">alpha</textarea>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const notes = document.querySelector('#notes');
const initialHeight = notes.getBoundingClientRect().height;
let resizing = false;
notes.addEventListener('mousedown', event => {
  const rect = notes.getBoundingClientRect();
  if (event.clientX >= rect.right - 8 && event.clientY >= rect.bottom - 8) {
    resizing = true;
  }
});
document.addEventListener('mousemove', event => {
  if (!resizing) return;
  const rect = notes.getBoundingClientRect();
  notes.style.height = Math.max(20, event.clientY - rect.top) + 'px';
});
document.addEventListener('mouseup', () => { resizing = false; });
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    initialHeight,
    height: notes.getBoundingClientRect().height
  });
});
"##
        .to_string(),
    ]);

    let resize = act_instruction(
        &env,
        "Resize the textarea so that the height is larger than its initial size then press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert!(state["height"].as_f64().unwrap() > state["initialHeight"].as_f64().unwrap());
    assert_eq!(resize["plan"]["action"], "sequence");
    assert_eq!(resize["plan"]["capability"]["name"], "element-resize");
    assert_eq!(resize["plan"]["steps"][0]["action"], "drag");

    env.stop();
}

#[test]
fn act_instruction_copies_visible_text_into_target_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-copy-text-transfer");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <label>Source code <input id="source-code" readonly></label>
    <label>Verification field <input id="verification" autocomplete="off"></label>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelector('#source-code').value = 'ZX-4187 ';
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#verification').value;
});
"##
        .to_string(),
    ]);

    let transfer = act_instruction(
        &env,
        "Copy the code from the source field into the verification field and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "ZX-4187 ");
    assert_eq!(transfer["plan"]["action"], "sequence");
    assert_eq!(transfer["plan"]["capability"]["name"], "copy-text-transfer");
    assert_eq!(transfer["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_copies_ordinal_textarea_into_target_field_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-copy-ordinal-textarea");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="status" role="timer">Time left: 1000 / 1000 sec</div>
  <section>
    <textarea id="text-1"></textarea>
    <textarea id="text-2"></textarea>
    <textarea id="text-3"></textarea>
    <input id="answer-input" type="text" autocomplete="off">
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelector('#text-1').value = 'First value ';
document.querySelector('#text-2').value = 'Second value ';
document.querySelector('#text-3').value = 'Third exact value ';
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#answer-input').value;
});
"##
        .to_string(),
    ]);

    let transfer = act_instruction(
        &env,
        "Copy the text from the 3rd text area below and paste it into the text input, then press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "Third exact value ");
    assert_eq!(transfer["plan"]["action"], "sequence");
    assert_eq!(transfer["plan"]["capability"]["name"], "copy-text-transfer");
    assert_eq!(transfer["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_transcribes_visible_text_below_into_field_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-visible-text-transcription");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="query">Type the text below into the text field and press Submit.</div>
  <section id="area">
    <div id="sample-text" class="display-text">
      <span style="display:inline-block; transform: skewX(8deg)">q</span><span style="display:inline-block; transform: skewY(-18deg)">R</span><span style="display:inline-block; transform: rotate(7deg)">5</span><span style="display:inline-block; transform: skewX(-6deg)">k</span>
    </div>
    <label>Answer <input id="answer" autocomplete="off"></label>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#answer').value;
});
"##
        .to_string(),
    ]);

    let transcribed = act_instruction(
        &env,
        "Type the text below into the text field and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "({answer: document.querySelector('#answer').value, submitted: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["answer"], "qR5k");
    assert_eq!(state["submitted"], "qR5k");
    assert_eq!(transcribed["plan"]["action"], "sequence");
    assert_eq!(
        transcribed["plan"]["capability"]["name"],
        "visible-text-transcription"
    );
    assert_eq!(transcribed["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_derives_visible_arithmetic_values_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-derived-arithmetic");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <p id="problem">7 x 6 =</p>
    <label>Answer <input id="locked-answer" readonly value="locked"></label>
    <label>Answer <input id="answer" autocomplete="off"></label>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#answer').value;
});
"##
        .to_string(),
    ]);

    let solved = act_instruction(
        &env,
        "Solve the math problem and type your answer into the textbox. Press submit when done.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "({answer: document.querySelector('#answer').value, locked: document.querySelector('#locked-answer').value, submitted: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["answer"], "42");
    assert_eq!(state["locked"], "locked");
    assert_eq!(state["submitted"], "42");
    assert_eq!(solved["plan"]["action"], "derive_and_act");
    assert_eq!(
        solved["result"]["derivedValue"]["mode"],
        "arithmetic-visible-text"
    );
    assert_eq!(solved["result"]["derivedValue"]["target"], "#answer");
    assert_eq!(solved["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_derives_visible_arithmetic_into_custom_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-derived-custom-value-host");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <p id="problem">6 + 5 =</p>
    <answer-box id="answer" aria-label="Answer" data-field="answer" tabindex="0" style="display:inline-block; min-width: 4rem; min-height: 1.5rem; border: 1px solid black;"></answer-box>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
if (!customElements.get('answer-box')) {
  customElements.define('answer-box', class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    get value() {
      return this._value || '';
    }
    set value(next) {
      this._value = String(next);
      this.textContent = this._value;
      this.setAttribute('data-current-value', this._value);
    }
  });
}
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#answer').value;
});
"##
        .to_string(),
    ]);

    let solved = act_instruction(
        &env,
        "Solve the math problem and enter the answer. Press submit when done.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "({answer: document.querySelector('#answer').value, submitted: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["answer"], "11");
    assert_eq!(state["submitted"], "11");
    assert_eq!(solved["plan"]["action"], "derive_and_act");
    assert_eq!(
        solved["result"]["derivedValue"]["mode"],
        "arithmetic-visible-text"
    );
    assert_eq!(solved["result"]["derivedValue"]["target"], "#answer");
    assert_eq!(solved["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_derives_visible_single_variable_equations_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-derived-equation");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <p id="problem">x - 7 = 12</p>
    <label>x = <input id="answer" autocomplete="off"></label>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#answer').value;
});
"##
        .to_string(),
    ]);

    let solved = act_instruction(
        &env,
        "Solve for x and type your answer into the textbox. Press Submit when done.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "({answer: document.querySelector('#answer').value, submitted: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["answer"], "19");
    assert_eq!(state["submitted"], "19");
    assert_eq!(solved["plan"]["action"], "derive_and_act");
    assert_eq!(
        solved["result"]["derivedValue"]["mode"],
        "arithmetic-visible-text"
    );
    assert_eq!(solved["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_extracts_ordinal_visible_words_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-derived-ordinal-word");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <p id="paragraph">Alpha bravo charlie delta echo foxtrot.</p>
    <label>Answer <input id="answer" autocomplete="off"></label>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#answer').value;
});
"##
        .to_string(),
    ]);

    let solved = act_instruction(
        &env,
        "Find the 4th word in the paragraph, type that into the textbox and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "({answer: document.querySelector('#answer').value, submitted: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["answer"], "delta");
    assert_eq!(state["submitted"], "delta");
    assert_eq!(solved["plan"]["action"], "derive_and_act");
    assert_eq!(solved["plan"]["capability"]["name"], "derived-value");
    assert_eq!(
        solved["result"]["derivedValue"]["mode"],
        "ordinal-visible-word-to-field"
    );
    assert_eq!(solved["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_generates_visible_numbers_until_constraints_match_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-generate-constrained-number");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <div id="generated-number" class="number-display">-</div>
    <button id="generate">Generate</button>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
const values = [9, 8, 3];
let index = 0;
document.querySelector('#generate').addEventListener('click', () => {
  document.querySelector('#generated-number').textContent = values[Math.min(index, values.length - 1)];
  index += 1;
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    value: Number(document.querySelector('#generated-number').textContent),
    attempts: index
  });
});
"##
        .to_string(),
    ]);

    let generated = act_instruction(&env, "Generate a number less than 6, then press submit.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["value"], 3);
    assert_eq!(state["attempts"], 3);
    assert_eq!(generated["plan"]["action"], "generate_constrained_value");
    assert_eq!(
        generated["plan"]["capability"]["name"],
        "numeric-constraint-generation"
    );
    assert_eq!(
        generated["result"]["generatedValue"]["mode"],
        "generated-visible-value"
    );
    assert_eq!(generated["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_waits_for_visible_numeric_condition_before_action_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-conditional-visible-value-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <div>
      <label>Instrument:</label>
      <span id="symbol">ALP</span>
    </div>
    <div>
      <label>Live price:</label>
      <span id="live-price" class="price metric">$12.00</span>
    </div>
    <button id="buy">Buy</button>
    <output id="out"></output>
  </section>
`;
const values = [12.00, 11.20, 10.40, 9.50, 8.90];
let index = 0;
const price = document.querySelector('#live-price');
const timer = setInterval(() => {
  index = Math.min(index + 1, values.length - 1);
  price.textContent = '$' + values[index].toFixed(2);
  if (index === values.length - 1) clearInterval(timer);
}, 60);
document.querySelector('#buy').addEventListener('click', () => {
  const value = Number(price.textContent.replace('$', ''));
  document.querySelector('#out').textContent = JSON.stringify({ value, ok: value <= 9.50 });
});
"##
        .to_string(),
    ]);

    let conditional = act_instruction(&env, "Buy ALP when the price is less than $9.50.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["ok"], true);
    assert!(state["value"].as_f64().unwrap() <= 9.50);
    assert_eq!(conditional["plan"]["action"], "conditional_value_action");
    assert_eq!(
        conditional["plan"]["capability"]["name"],
        "conditional-value-action"
    );
    assert_eq!(
        conditional["result"]["conditionalAction"]["mode"],
        "conditional-visible-value-action"
    );
    assert_eq!(conditional["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_operates_visible_command_surfaces_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-command-surface-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <p id="task">Run ls in the terminal below.</p>
  <section id="command-surface" class="terminal shell">
    <div id="log">
      <div class="terminal-output">Welcome. Type help for commands.</div>
      <div class="terminal-line"><span class="prompt">user$</span> <span id="active-input"></span></div>
    </div>
    <input id="command-target" type="text" style="opacity:0;width:1px;height:1px">
  </section>
  <output id="out"></output>
`;
const files = ['alpha', 'report.txt', 'archive.tar.gz', 'build.js'];
let current = '';
function output(text) {
  const div = document.createElement('div');
  div.className = 'terminal-output';
  div.textContent = text;
  document.querySelector('#log').insertBefore(div, document.querySelector('.terminal-line'));
}
function commandLine(text) {
  const div = document.createElement('div');
  div.className = 'terminal-line';
  div.textContent = 'user$ ' + text;
  document.querySelector('#log').insertBefore(div, document.querySelector('.terminal-line'));
}
document.querySelector('#command-target').addEventListener('keydown', event => {
  if (event.key.length === 1) {
    current += event.key;
    document.querySelector('#active-input').textContent = current;
  } else if (event.key === 'Backspace') {
    current = current.slice(0, -1);
    document.querySelector('#active-input').textContent = current;
  } else if (event.key === 'Enter') {
    const command = current;
    commandLine(command);
    current = '';
    document.querySelector('#active-input').textContent = '';
    if (command === 'ls') {
      output(files.join(' '));
      document.querySelector('#out').textContent = JSON.stringify({
        command,
        listed: files
      });
    } else {
      output('unknown command');
    }
  }
});
document.querySelector('#command-surface').addEventListener('click', () => {
  document.querySelector('#command-target').focus();
});
document.querySelector('#command-target').focus();
"##
        .to_string(),
    ]);

    let command = act_instruction(&env, "Run `ls` in the terminal below.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["command"], "ls");
    assert!(state["listed"]
        .as_array()
        .expect("listed files")
        .iter()
        .any(|item| item == "alpha"));
    assert!(state["listed"]
        .as_array()
        .expect("listed files")
        .iter()
        .any(|item| item == "report.txt"));
    assert_eq!(command["plan"]["action"], "command_surface_action");
    assert_eq!(
        command["plan"]["capability"]["name"],
        "command-surface-action"
    );
    assert_eq!(
        command["result"]["commandSurfaceAction"]["mode"],
        "command-surface-run"
    );
    assert_eq!(command["result"]["commandSurfaceAction"]["command"], "ls");
    assert_eq!(command["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_operates_custom_value_host_command_surfaces_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-command-surface-action");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <p id="task">Run ls in the terminal below.</p>
  <section id="command-surface" class="terminal shell">
    <div id="log">
      <div class="terminal-output">Ready. Type a command.</div>
      <div class="terminal-line"><span class="prompt">agent$</span> <terminal-input id="command-target" aria-label="Command input" data-field="command" tabindex="0" style="display:inline-block; min-width: 8rem; min-height: 1.5rem;"></terminal-input></div>
    </div>
  </section>
  <output id="out"></output>
`;
if (!customElements.get('terminal-input')) {
  customElements.define('terminal-input', class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    get value() {
      return this._value || '';
    }
    set value(next) {
      this._value = String(next);
      this.textContent = this._value;
    }
  });
}
const files = ['alpha', 'report.txt', 'archive.tar.gz'];
const input = document.querySelector('#command-target');
function output(text) {
  const div = document.createElement('div');
  div.className = 'terminal-output';
  div.textContent = text;
  document.querySelector('#log').appendChild(div);
}
function commandLine(text) {
  const div = document.createElement('div');
  div.className = 'terminal-line';
  div.textContent = 'agent$ ' + text;
  document.querySelector('#log').appendChild(div);
}
input.addEventListener('keydown', event => {
  if (event.key.length === 1) {
    input.value = input.value + event.key;
  } else if (event.key === 'Backspace') {
    input.value = input.value.slice(0, -1);
  } else if (event.key === 'Enter') {
    const command = input.value;
    commandLine(command);
    input.value = '';
    if (command === 'ls') {
      output(files.join(' '));
      document.querySelector('#out').textContent = JSON.stringify({
        command,
        listed: files
      });
    } else {
      output('unknown command');
    }
  }
});
document.querySelector('#command-surface').addEventListener('click', () => {
  input.focus();
});
input.focus();
"##
        .to_string(),
    ]);

    let command = act_instruction(&env, "Run `ls` in the terminal below.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["command"], "ls");
    assert!(state["listed"]
        .as_array()
        .expect("listed files")
        .iter()
        .any(|item| item == "alpha"));
    assert_eq!(command["plan"]["action"], "command_surface_action");
    assert_eq!(
        command["plan"]["capability"]["name"],
        "command-surface-action"
    );
    assert_eq!(
        command["result"]["commandSurfaceAction"]["mode"],
        "command-surface-run"
    );
    assert_eq!(command["result"]["commandSurfaceAction"]["command"], "ls");
    assert_eq!(command["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_enters_deterministic_numbers_for_constraints_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-fill-constrained-number");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <label>Number <input id="locked-number" type="number" readonly value="99"></label>
    <label>Number <input id="answer" type="number" autocomplete="off"></label>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#answer').value;
});
"##
        .to_string(),
    ]);

    let generated = act_instruction(
        &env,
        "Enter an even number greater than 7 and press submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({answer: document.querySelector('#answer').value, locked: document.querySelector('#locked-number').value, submitted: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["answer"], "8");
    assert_eq!(state["locked"], "99");
    assert_eq!(state["submitted"], "8");
    assert_eq!(generated["plan"]["action"], "generate_constrained_value");
    assert_eq!(
        generated["result"]["generatedValue"]["mode"],
        "deterministic-value-to-field"
    );
    assert_eq!(generated["result"]["generatedValue"]["target"], "#answer");
    assert_eq!(generated["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_enters_deterministic_numbers_into_custom_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-fill-constrained-custom-number");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
class NumericEntry extends HTMLElement {
  constructor() {
    super();
    this._value = '';
  }
  connectedCallback() {
    this.style.display = 'inline-block';
    this.style.minWidth = '80px';
    this.style.minHeight = '24px';
    this.style.border = '1px solid #777';
    this.textContent = this._value;
  }
  get value() {
    return this._value;
  }
  set value(next) {
    this._value = String(next);
    this.textContent = this._value;
  }
}
customElements.define('numeric-entry', NumericEntry);
document.body.innerHTML = `
  <section>
    <numeric-entry id="answer" aria-label="Number" data-field="number" tabindex="0"></numeric-entry>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#answer').value;
});
"##
        .to_string(),
    ]);

    let generated = act_instruction(
        &env,
        "Enter an even number greater than 7 and press submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({answer: document.querySelector('#answer').value, text: document.querySelector('#answer').textContent, submitted: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["answer"], "8");
    assert_eq!(state["text"], "8");
    assert_eq!(state["submitted"], "8");
    assert_eq!(generated["plan"]["action"], "generate_constrained_value");
    assert_eq!(
        generated["result"]["generatedValue"]["mode"],
        "deterministic-value-to-field"
    );
    assert_eq!(generated["result"]["generatedValue"]["target"], "#answer");
    assert_eq!(generated["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_adjusts_numeric_entry_from_visible_feedback_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-feedback-number-search");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <label>Answer <input id="locked-answer" type="number" readonly value="0"></label>
    <label>Answer <input id="answer" type="number" min="1" max="100" autocomplete="off"></label>
    <button id="submit">Check</button>
    <output id="feedback" role="status" aria-live="polite"></output>
  </section>
`;
const target = 37;
let attempts = 0;
document.querySelector('#submit').addEventListener('click', () => {
  attempts += 1;
  const value = Number(document.querySelector('#answer').value);
  let message = '';
  if (value === target) message = `Correct in ${attempts}`;
  else if (value > target) message = 'Too high';
  else message = 'Too low';
  document.querySelector('#feedback').textContent = message;
});
"##
        .to_string(),
    ]);

    let solved = act_instruction(&env, "Guess the hidden number between 1 and 100.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#answer').value, locked: document.querySelector('#locked-answer').value, feedback: document.querySelector('#feedback').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["value"], "37");
    assert_eq!(state["locked"], "0");
    assert!(state["feedback"]
        .as_str()
        .unwrap_or("")
        .starts_with("Correct"));
    assert_eq!(solved["plan"]["action"], "feedback_loop_value");
    assert_eq!(solved["plan"]["capability"]["name"], "feedback-loop-value");
    assert_eq!(
        solved["result"]["generatedValue"]["mode"],
        "feedback-loop-value"
    );
    assert_eq!(solved["result"]["generatedValue"]["target"], "#answer");
    assert_eq!(solved["result"]["generatedValue"]["value"], 37);
    assert_eq!(solved["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_adjusts_custom_numeric_value_hosts_from_visible_feedback_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-feedback-custom-number-search");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
class NumericEntry extends HTMLElement {
  constructor() {
    super();
    this._value = '';
  }
  connectedCallback() {
    this.style.display = 'inline-block';
    this.style.minWidth = '80px';
    this.style.minHeight = '24px';
    this.style.border = '1px solid #777';
    this.textContent = this._value;
  }
  get value() {
    return this._value;
  }
  set value(next) {
    this._value = String(next);
    this.textContent = this._value;
  }
}
customElements.define('numeric-entry', NumericEntry);
document.body.innerHTML = `
  <section>
    <numeric-entry id="answer" aria-label="Answer" data-field="answer" tabindex="0"></numeric-entry>
    <button id="submit">Check</button>
    <output id="feedback" role="status" aria-live="polite"></output>
  </section>
`;
const target = 37;
let attempts = 0;
document.querySelector('#submit').addEventListener('click', () => {
  attempts += 1;
  const value = Number(document.querySelector('#answer').value);
  let message = '';
  if (value === target) message = `Correct in ${attempts}`;
  else if (value > target) message = 'Too high';
  else message = 'Too low';
  document.querySelector('#feedback').textContent = message;
});
"##
        .to_string(),
    ]);

    let solved = act_instruction(&env, "Guess the hidden number between 1 and 100.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#answer').value, text: document.querySelector('#answer').textContent, feedback: document.querySelector('#feedback').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["value"], "37");
    assert_eq!(state["text"], "37");
    assert!(state["feedback"]
        .as_str()
        .unwrap_or("")
        .starts_with("Correct"));
    assert_eq!(solved["plan"]["action"], "feedback_loop_value");
    assert_eq!(solved["plan"]["capability"]["name"], "feedback-loop-value");
    assert_eq!(
        solved["result"]["generatedValue"]["mode"],
        "feedback-loop-value"
    );
    assert_eq!(solved["result"]["generatedValue"]["target"], "#answer");
    assert_eq!(solved["result"]["generatedValue"]["value"], 37);
    assert_eq!(solved["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_parses_hyphenated_numeric_feedback_ranges_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-feedback-hyphen-range");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <label>Answer <input id="answer" type="number" autocomplete="off"></label>
    <button id="submit">Submit</button>
    <div id="feedback">
      <div id="waiting">Waiting for your guess...</div>
      <div id="correct" style="display:none">Correct!</div>
      <div id="lower" style="display:none">The number is lower than <span></span>.</div>
      <div id="higher" style="display:none">The number is higher than <span></span>.</div>
    </div>
  </section>
`;
const target = 2;
document.querySelector('#submit').addEventListener('click', event => {
  event.preventDefault();
  const value = Number(document.querySelector('#answer').value);
  for (const id of ['waiting', 'correct', 'lower', 'higher']) {
    document.querySelector('#' + id).style.display = 'none';
  }
  if (value === target) {
    document.querySelector('#correct').style.display = '';
  } else if (value < target) {
    document.querySelector('#higher span').textContent = value;
    document.querySelector('#higher').style.display = '';
  } else {
    document.querySelector('#lower span').textContent = value;
    document.querySelector('#lower').style.display = '';
  }
});
"##
        .to_string(),
    ]);

    let solved = act_instruction(
        &env,
        "Guess the number between 0-9 and press Submit. Use the feedback below to find the right number.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#answer').value, feedback: document.querySelector('#feedback').innerText})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["value"], "2");
    assert!(state["feedback"].as_str().unwrap_or("").contains("Correct"));
    assert_eq!(solved["plan"]["action"], "feedback_loop_value");
    assert_eq!(solved["plan"]["capability"]["name"], "feedback-loop-value");
    assert_eq!(solved["plan"]["evidence"]["min"], 0);
    assert_eq!(solved["plan"]["evidence"]["max"], 9);
    assert_eq!(solved["result"]["generatedValue"]["value"], 2);
    assert_eq!(solved["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_completes_numeric_feedback_loop_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-feedback-loop-number");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <label>Answer <input id="answer" type="number" min="1" max="100" autocomplete="off"></label>
    <button id="check">Check</button>
    <output id="feedback" role="status" aria-live="polite"></output>
    <output id="out"></output>
  </section>
`;
const target = 73;
let attempts = 0;
document.querySelector('#check').addEventListener('click', () => {
  attempts += 1;
  const value = Number(document.querySelector('#answer').value);
  const feedback = document.querySelector('#feedback');
  if (value === target) {
    feedback.textContent = 'Correct';
    document.querySelector('#out').textContent = JSON.stringify({ value, attempts });
  } else if (value > target) {
    feedback.textContent = 'Too high, try lower';
  } else {
    feedback.textContent = 'Too low, try higher';
  }
});
"##
        .to_string(),
    ]);

    let solved = act_instruction(&env, "Find the hidden number from 1 to 100.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["value"], 73);
    assert!(state["attempts"].as_u64().unwrap_or(0) <= 8);
    assert_eq!(solved["plan"]["action"], "feedback_loop_value");
    assert_eq!(solved["plan"]["capability"]["name"], "feedback-loop-value");
    assert_eq!(
        solved["result"]["generatedValue"]["mode"],
        "feedback-loop-value"
    );
    assert_eq!(solved["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_counts_visible_visual_items_into_fields_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-visual-count-value");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <div id="visual-group">
      <span class="metric-block"></span>
      <span class="metric-block"></span>
      <span class="metric-block"></span>
      <span class="metric-block"></span>
      <span class="metric-block"></span>
      <span class="metric-block"></span>
      <span class="metric-block"></span>
    </div>
    <label>Answer <input id="answer" type="text" autocomplete="off"></label>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
document.querySelectorAll('.metric-block').forEach(block => {
  block.style.display = 'inline-block';
  block.style.width = '12px';
  block.style.height = '12px';
  block.style.margin = '1px';
  block.style.background = '#7fb0ff';
  block.style.border = '1px solid black';
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#answer').value;
});
"##
        .to_string(),
    ]);

    let counted = act_instruction(
        &env,
        "Type the total number of blocks into the textbox and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "7");
    assert_eq!(counted["plan"]["action"], "sequence");
    assert_eq!(counted["plan"]["capability"]["name"], "visible-count-value");
    assert_eq!(counted["plan"]["steps"][0]["value"], 7);
    assert_eq!(counted["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_counts_visible_visual_items_into_custom_value_hosts_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-visual-count-custom-value");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section>
    <div id="visual-group">
      <span class="metric-dot"></span>
      <span class="metric-dot"></span>
      <span class="metric-dot"></span>
      <span class="metric-dot"></span>
    </div>
    <answer-box id="answer" aria-label="Answer" data-field="answer" tabindex="0" style="display:inline-block; min-width: 4rem; min-height: 1.5rem; border: 1px solid black;"></answer-box>
    <button id="submit">Submit</button>
    <output id="out"></output>
  </section>
`;
if (!customElements.get('answer-box')) {
  customElements.define('answer-box', class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    get value() {
      return this._value || '';
    }
    set value(next) {
      this._value = String(next);
      this.textContent = this._value;
      this.setAttribute('data-current-value', this._value);
    }
  });
}
document.querySelectorAll('.metric-dot').forEach(dot => {
  dot.style.display = 'inline-block';
  dot.style.width = '12px';
  dot.style.height = '12px';
  dot.style.margin = '1px';
  dot.style.background = '#7fb0ff';
  dot.style.border = '1px solid black';
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = document.querySelector('#answer').value;
});
"##
        .to_string(),
    ]);

    let counted = act_instruction(
        &env,
        "Type the total number of dots into the answer field and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "4");
    assert_eq!(counted["plan"]["action"], "sequence");
    assert_eq!(counted["plan"]["capability"]["name"], "visible-count-value");
    assert_eq!(counted["plan"]["steps"][0]["value"], 4);
    assert_eq!(counted["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_extreme_numeric_item_ignoring_status_numbers() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-extreme-numeric-card");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="hud" role="timer">Time left: 1000 / 1000 sec</div>
  <section id="choices">
    <div class="card hidden" data-index="0"><span>0</span></div>
    <div class="card hidden" data-index="1"><span>10</span></div>
    <div class="card hidden" data-index="2"><span>5</span></div>
  </section>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelectorAll('.card').forEach(card => {
  card.addEventListener('click', () => {
    document.querySelectorAll('.card').forEach(other => other.classList.remove('selected'));
    card.classList.add('selected');
  });
});
document.querySelector('#submit').addEventListener('click', () => {
  const selected = document.querySelector('.card.selected');
  document.querySelector('#out').textContent = selected ? selected.textContent.trim() : '';
});
"##
        .to_string(),
    ]);

    let picked = act_instruction(
        &env,
        "Find and pick the card with the greatest number, then press submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "10");
    assert_eq!(picked["plan"]["action"], "sequence");
    assert_eq!(picked["plan"]["capability"]["name"], "extreme-click");
    assert_eq!(picked["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_reveals_hidden_targets_before_clicking_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-disclosure-click");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="section" class="accordion-header" aria-expanded="false" aria-controls="panel">
    Section
  </button>
  <div id="panel" hidden>
    <button id="submit">Submit</button>
  </div>
  <output id="out"></output>
`;
const section = document.querySelector('#section');
const panel = document.querySelector('#panel');
const out = document.querySelector('#out');
section.addEventListener('click', () => {
  section.setAttribute('aria-expanded', 'true');
  setTimeout(() => {
    panel.hidden = false;
  }, 180);
});
document.querySelector('#submit').addEventListener('click', () => {
  out.textContent = panel.hidden ? 'clicked-too-early' : 'submitted-open';
});
"##
        .to_string(),
    ]);

    let reveal = act_instruction(&env, "Expand the section below and click submit.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({expanded: document.querySelector('#section').getAttribute('aria-expanded'), hidden: document.querySelector('#panel').hidden, out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse disclosure state");

    assert_eq!(state["expanded"], "true");
    assert_eq!(state["hidden"], false);
    assert_eq!(state["out"], "submitted-open");
    assert_eq!(reveal["plan"]["action"], "discover_click");
    assert_eq!(reveal["result"]["discoverClick"]["mode"], "discover-click");
    assert_eq!(reveal["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_custom_search_field_before_discovering_click_target_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-discover-click-custom-searchbox");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="lookup" role="searchbox" aria-label="Lookup" tabindex="0" style="min-height: 24px; width: 220px; border: 1px solid #777;"></div>
  <button id="search">Search</button>
  <section id="results"></section>
  <output id="out"></output>
`;
const lookup = document.querySelector('#lookup');
const results = document.querySelector('#results');
const out = document.querySelector('#out');
document.querySelector('#search').addEventListener('click', () => {
  const value = String(lookup.value || lookup.textContent || '').trim();
  results.innerHTML = value === 'Mira'
    ? '<a id="open" href="#">Open</a>'
    : '<span>No results</span>';
  const open = document.querySelector('#open');
  if (open) {
    open.addEventListener('click', event => {
      event.preventDefault();
      out.textContent = 'opened:' + value;
    });
  }
});
"##
        .to_string(),
    ]);

    let reveal = act_instruction(&env, r#"Find "Mira" and click the Open link."#);
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#lookup').value || document.querySelector('#lookup').textContent || '', out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse search state");

    assert_eq!(state["value"], "Mira");
    assert_eq!(state["out"], "opened:Mira");
    assert_eq!(reveal["plan"]["action"], "discover_click");
    assert_eq!(reveal["result"]["discoverClick"]["clicked"], "#open");
    assert_eq!(
        reveal["result"]["discoverClick"]["filledTrigger"]["value"],
        "Mira"
    );
    assert_eq!(reveal["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_named_result_after_custom_search_workflow_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-search-workflow-named-result");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="lookup" role="searchbox" aria-label="Lookup" tabindex="0" style="min-height: 24px; width: 220px; border: 1px solid #777;"></div>
  <button id="search">Search</button>
  <section id="results"></section>
  <output id="out"></output>
`;
const lookup = document.querySelector('#lookup');
const results = document.querySelector('#results');
const out = document.querySelector('#out');
document.querySelector('#search').addEventListener('click', () => {
  const value = String(lookup.value || lookup.textContent || '').trim();
  results.innerHTML = value === 'Mira'
    ? '<article class="result"><a id="details" href="#">Details</a></article>'
    : '<span>No results</span>';
  const details = document.querySelector('#details');
  if (details) {
    details.addEventListener('click', event => {
      event.preventDefault();
      out.textContent = 'opened:' + value;
    });
  }
});
"##
        .to_string(),
    ]);

    let result = act_instruction(&env, r#"Search for "Mira" and click Details."#);
    let state = env.json(&[
        "eval".to_string(),
        "JSON.stringify({value: document.querySelector('#lookup').value || document.querySelector('#lookup').textContent || '', out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(state["result"].as_str().unwrap_or("{}")).expect("parse search state");

    assert_eq!(state["value"], "Mira");
    assert_eq!(state["out"], "opened:Mira");
    assert_eq!(result["plan"]["action"], "form_workflow");
    assert_eq!(
        result["result"]["formWorkflow"]["namedResult"]["selector"],
        "#details"
    );
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_collapses_expanded_disclosures_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-disclosure-collapse");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="advanced" class="accordion-header" aria-expanded="true" aria-controls="advanced-panel">
    Advanced settings
  </button>
  <div id="advanced-panel">
    <label>Token <input id="token" value="ready"></label>
  </div>
  <output id="out"></output>
`;
const advanced = document.querySelector('#advanced');
const panel = document.querySelector('#advanced-panel');
const out = document.querySelector('#out');
function render() {
  out.textContent = JSON.stringify({
    expanded: advanced.getAttribute('aria-expanded'),
    hidden: panel.hidden
  });
}
advanced.addEventListener('click', () => {
  const next = advanced.getAttribute('aria-expanded') !== 'true';
  advanced.setAttribute('aria-expanded', String(next));
  panel.hidden = !next;
  render();
});
render();
"##
        .to_string(),
    ]);

    let collapse = act_instruction(&env, "Collapse Advanced settings.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse state");

    assert_eq!(state["expanded"], "false");
    assert_eq!(state["hidden"], true);
    assert_eq!(collapse["plan"]["action"], "click");
    assert_eq!(collapse["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_exact_link_revealed_inside_expanded_section_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-disclosure-exact-link");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="first" class="accordion-header" aria-expanded="false" aria-controls="panel-a">
    First section
  </button>
  <div id="panel-a" hidden>
    <span id="wrong" class="alink">inside</span>
  </div>
  <button id="second" class="accordion-header" aria-expanded="false" aria-controls="panel-b">
    Second section
  </button>
  <div id="panel-b" hidden>
    <span id="right" class="alink">in</span>
  </div>
  <output id="out"></output>
`;
for (const button of document.querySelectorAll('.accordion-header')) {
  const panel = document.querySelector('#' + button.getAttribute('aria-controls'));
  button.addEventListener('click', () => {
    button.setAttribute('aria-expanded', 'true');
    panel.hidden = false;
  });
}
document.querySelector('#wrong').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'wrong';
});
document.querySelector('#right').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'right';
});
"##
        .to_string(),
    ]);

    let reveal = act_instruction(
        &env,
        "Expand the sections below, to find and click on the link \"in\".",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "right");
    assert_eq!(reveal["plan"]["action"], "discover_click");
    assert_eq!(reveal["result"]["discoverClick"]["clicked"], "#right");
    assert_eq!(reveal["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_discovers_targets_inside_nested_menus_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-discover-nested-menu");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    .ui-menu { display: none; list-style: none; margin: 0; padding: 0; }
    .ui-menu.open { display: block; }
    .ui-menu-item-wrapper { cursor: pointer; padding: 2px 6px; }
  </style>
  <button id="open-menu">Menu</button>
  <ul id="menu" class="ui-menu" role="menu">
    <li class="ui-menu-item"><div id="save" class="ui-menu-item-wrapper" role="menuitem">Save</div></li>
    <li class="ui-menu-item">
      <div id="playback" class="ui-menu-item-wrapper" role="menuitem" aria-haspopup="menu">Playback</div>
      <ul id="playback-menu" class="ui-menu" role="menu">
        <li class="ui-menu-item"><div id="prev" class="ui-menu-item-wrapper" role="menuitem">Prev</div></li>
        <li class="ui-menu-item"><div id="play" class="ui-menu-item-wrapper" role="menuitem">Play</div></li>
      </ul>
    </li>
  </ul>
  <output id="out"></output>
`;
document.querySelector('#open-menu').addEventListener('click', () => {
  document.querySelector('#menu').classList.add('open');
});
document.querySelector('#playback').addEventListener('mouseover', () => {
  document.querySelector('#playback-menu').classList.add('open');
});
document.querySelector('#prev').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'prev';
});
document.querySelector('#play').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'play';
});
"##
        .to_string(),
    ]);

    let reveal = act_instruction(
        &env,
        r#"Click the "Menu" button, and then find and click on the item labeled "Play"."#,
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "play");
    assert_eq!(reveal["plan"]["action"], "discover_click");
    assert_eq!(reveal["result"]["discoverClick"]["clicked"], "#play");
    assert_eq!(reveal["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_prefers_case_exact_revealed_link_across_sections_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-disclosure-case-exact-link");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="first" class="accordion-header" aria-expanded="false" aria-controls="panel-a">
    First section
  </button>
  <div id="panel-a" hidden>
    <span id="wrong" class="alink">Proin</span>
  </div>
  <button id="second" class="accordion-header" aria-expanded="false" aria-controls="panel-b">
    Second section
  </button>
  <div id="panel-b" hidden>
    <span id="right" class="alink">proin</span>
  </div>
  <output id="out"></output>
`;
for (const button of document.querySelectorAll('.accordion-header')) {
  const panel = document.querySelector('#' + button.getAttribute('aria-controls'));
  button.addEventListener('click', () => {
    for (const other of document.querySelectorAll('.accordion-header')) {
      const otherPanel = document.querySelector('#' + other.getAttribute('aria-controls'));
      other.setAttribute('aria-expanded', String(other === button));
      otherPanel.hidden = other !== button;
    }
  });
}
document.querySelector('#wrong').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'wrong';
});
document.querySelector('#right').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'right';
});
"##
        .to_string(),
    ]);

    let reveal = act_instruction(
        &env,
        "Expand the sections below, to find and click on the link \"proin\".",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "right");
    assert_eq!(reveal["plan"]["action"], "discover_click");
    assert_eq!(reveal["result"]["discoverClick"]["clicked"], "#right");
    assert_eq!(reveal["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_discovers_shadow_dom_targets_by_page_semantics_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-semantic-discovery");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <settings-menu></settings-menu>
  <output id="out"></output>
`;
customElements.define('settings-menu', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <button id="settings" aria-expanded="false" aria-controls="panel">Settings</button>
      <div id="panel" hidden>
        <button id="billing" data-aliases="invoice payment receipt">Billing</button>
      </div>
    `;
    const settings = root.querySelector('#settings');
    const panel = root.querySelector('#panel');
    settings.addEventListener('click', () => {
      settings.setAttribute('aria-expanded', 'true');
      setTimeout(() => {
        panel.hidden = false;
      }, 120);
    });
    root.querySelector('#billing').addEventListener('click', () => {
      document.querySelector('#out').textContent = 'billing';
    });
  }
});
"##
        .to_string(),
    ]);

    let discover = act_instruction(
        &env,
        "Open the settings panel and click the invoice action.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "billing");
    assert_eq!(discover["plan"]["action"], "discover_click");
    assert_eq!(
        discover["result"]["discoverClick"]["mode"],
        "discover-click"
    );
    assert!(discover["result"]["discoverClick"]["clicked"]
        .as_str()
        .unwrap_or("")
        .contains("billing"));
    assert_eq!(discover["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_searches_tabs_to_click_revealed_target_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-tab-discovery");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <ul role="tablist" class="ui-tabs-nav">
    <li role="tab" class="ui-tabs-tab" id="tab-a" aria-controls="panel-a"><a class="ui-tabs-anchor" href="#panel-a">Alpha</a></li>
    <li role="tab" class="ui-tabs-tab" id="tab-b" aria-controls="panel-b"><a class="ui-tabs-anchor" href="#panel-b">Beta</a></li>
    <li role="tab" class="ui-tabs-tab" id="tab-c" aria-controls="panel-c"><a class="ui-tabs-anchor" href="#panel-c">Gamma</a></li>
  </ul>
  <section id="panel-a" role="tabpanel">No target here.</section>
  <section id="panel-b" role="tabpanel" hidden><span id="target-link" class="alink">vel</span></section>
  <section id="panel-c" role="tabpanel" hidden>Still no target.</section>
  <output id="out"></output>
`;
function activate(id) {
  for (const panel of document.querySelectorAll('[role=tabpanel]')) {
    panel.hidden = panel.id !== id;
  }
}
document.querySelectorAll('.ui-tabs-anchor').forEach(anchor => {
  anchor.addEventListener('click', event => {
    event.preventDefault();
    const id = anchor.getAttribute('href')?.slice(1);
    if (id) activate(id);
  });
});
document.querySelector('#target-link').addEventListener('click', event => {
  event.preventDefault();
  document.querySelector('#out').textContent = 'clicked';
});
"##
        .to_string(),
    ]);

    let tabs = act_instruction(
        &env,
        "Switch between the tabs to find and click on the link \"vel\".",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "clicked");
    assert_eq!(tabs["plan"]["action"], "discover_click");
    assert_eq!(tabs["result"]["discoverClick"]["target"], "vel");
    assert_eq!(tabs["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_aria_tabs_by_label_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-aria-tab-selection");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div role="tablist" aria-label="Account sections">
    <button id="overview-tab" role="tab" aria-selected="true" aria-controls="overview-panel">Overview</button>
    <button id="billing-tab" role="tab" aria-selected="false" aria-controls="billing-panel">Billing</button>
    <button id="support-tab" role="tab" aria-selected="false" aria-controls="support-panel">Support</button>
  </div>
  <section id="overview-panel" role="tabpanel">Overview content</section>
  <section id="billing-panel" role="tabpanel" hidden>Billing content</section>
  <section id="support-panel" role="tabpanel" hidden>Support content</section>
  <output id="out"></output>
`;
for (const tab of document.querySelectorAll('[role=tab]')) {
  tab.addEventListener('click', () => {
    for (const other of document.querySelectorAll('[role=tab]')) {
      const selected = other === tab;
      other.setAttribute('aria-selected', String(selected));
      document.querySelector('#' + other.getAttribute('aria-controls')).hidden = !selected;
    }
    document.querySelector('#out').textContent = tab.textContent.trim();
  });
}
"##
        .to_string(),
    ]);

    let selected = act_instruction(&env, "Switch to the Billing tab.");
    let state = env.json(&[
        "eval".to_string(),
        "JSON.stringify({selected: document.querySelector('[role=tab][aria-selected=true]').textContent.trim(), billingHidden: document.querySelector('#billing-panel').hidden, out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: serde_json::Value =
        serde_json::from_str(state["result"].as_str().unwrap()).expect("parse tab state");

    assert_eq!(state["selected"], "Billing");
    assert_eq!(state["billingHidden"], false);
    assert_eq!(state["out"], "Billing");
    assert_eq!(selected["plan"]["action"], "click");
    assert_eq!(selected["plan"]["capability"]["name"], "tab-selection");
    assert_eq!(selected["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_child_anchors_for_jquery_style_tabs_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-anchor-tab-selection");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <ul role="tablist" class="ui-tabs-nav">
    <li role="tab" class="ui-tabs-tab" id="tab-alpha" aria-controls="panel-alpha"><a class="ui-tabs-anchor" href="#panel-alpha">Alpha</a></li>
    <li role="tab" class="ui-tabs-tab" id="tab-gamma" aria-controls="panel-gamma"><a class="ui-tabs-anchor" href="#panel-gamma">Gamma</a></li>
  </ul>
  <section id="panel-alpha" role="tabpanel">Alpha content</section>
  <section id="panel-gamma" role="tabpanel" hidden>Gamma content</section>
  <output id="out"></output>
`;
function activate(id, label) {
  for (const panel of document.querySelectorAll('[role=tabpanel]')) {
    panel.hidden = panel.id !== id;
  }
  document.querySelector('#out').textContent = label;
}
document.querySelectorAll('.ui-tabs-anchor').forEach(anchor => {
  anchor.addEventListener('click', event => {
    event.preventDefault();
    activate(anchor.getAttribute('href').slice(1), anchor.textContent.trim());
  });
});
"##
        .to_string(),
    ]);

    let selected = act_instruction(&env, "Open the Gamma tab.");
    let state = env.json(&[
        "eval".to_string(),
        "JSON.stringify({gammaHidden: document.querySelector('#panel-gamma').hidden, out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: serde_json::Value =
        serde_json::from_str(state["result"].as_str().unwrap()).expect("parse anchor tab state");

    assert_eq!(state["gammaHidden"], false);
    assert_eq!(state["out"], "Gamma");
    assert_eq!(selected["plan"]["action"], "click");
    assert_eq!(selected["plan"]["capability"]["name"], "tab-selection");
    assert_eq!(selected["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_reports_exhausted_discovery_actions_as_structured_result_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-tab-discovery-exhausted");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <ul role="tablist" class="ui-tabs-nav">
    <li role="tab" class="ui-tabs-tab" id="tab-a" aria-controls="panel-a"><a class="ui-tabs-anchor" href="#panel-a">Alpha</a></li>
    <li role="tab" class="ui-tabs-tab" id="tab-b" aria-controls="panel-b"><a class="ui-tabs-anchor" href="#panel-b">Beta</a></li>
  </ul>
  <section id="panel-a" role="tabpanel">No matching target here.</section>
  <section id="panel-b" role="tabpanel" hidden>The requested link is no longer visible.</section>
  <output id="out"></output>
`;
function activate(id) {
  for (const panel of document.querySelectorAll('[role=tabpanel]')) {
    panel.hidden = panel.id !== id;
  }
}
document.querySelectorAll('.ui-tabs-anchor').forEach(anchor => {
  anchor.addEventListener('click', event => {
    event.preventDefault();
    const id = anchor.getAttribute('href')?.slice(1);
    if (id) activate(id);
  });
});
document.querySelector('#tab-b').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'tab-effect';
});
"##
        .to_string(),
    ]);

    let tabs = act_instruction(
        &env,
        "Switch between the tabs to find and click on the link \"vel\".",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "tab-effect");
    assert_eq!(tabs["plan"]["action"], "discover_click");
    assert_eq!(
        tabs["result"]["discoverClick"]["mode"],
        "discovery-actions-exhausted"
    );
    assert_eq!(tabs["result"]["discoverClick"]["partial"], true);
    assert_eq!(tabs["result"]["discoverClick"]["targetFound"], false);

    env.stop();
}

#[test]
fn act_instruction_clicks_cartesian_svg_grid_coordinate_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-coordinate-grid");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="grid" width="150" height="150" style="width:150px;height:150px;border:1px solid #ccc"></svg>
  <output id="out"></output>
`;
const svg = document.querySelector('#grid');
const xmlns = 'http://www.w3.org/2000/svg';
const coords = new WeakMap();
for (let x = -2; x <= 2; x++) {
  for (let y = -2; y <= 2; y++) {
    const circle = document.createElementNS(xmlns, 'circle');
    circle.setAttribute('cx', String((x + 2) * 30 + 15));
    circle.setAttribute('cy', String((2 - y) * 30 + 15));
    circle.setAttribute('r', '5');
    circle.setAttribute('fill', 'blue');
    coords.set(circle, `${x},${y}`);
    circle.addEventListener('click', () => {
      document.querySelector('#out').textContent = coords.get(circle);
    });
    svg.appendChild(circle);
  }
}
"##
        .to_string(),
    ]);

    let grid = act_instruction(&env, "Click on the grid coordinate (-1,0).");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "-1,0");
    assert_eq!(grid["plan"]["action"], "click");
    assert_eq!(grid["plan"]["capability"]["name"], "coordinate-grid-click");
    assert_eq!(grid["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_nested_menu_paths_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-menu-path");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <nav>
    <ul role="menubar" id="menu">
      <li role="menuitem" id="file" tabindex="0">File
        <ul role="menu" id="filemenu" hidden>
          <li role="menuitem" id="new">New</li>
          <li role="menuitem" id="openItem">Open</li>
        </ul>
      </li>
      <li role="menuitem" id="edit" tabindex="0">Edit
        <ul role="menu" id="editmenu" hidden>
          <li role="menuitem" id="copy">Copy</li>
          <li role="menuitem" id="paste">Paste</li>
        </ul>
      </li>
    </ul>
  </nav>
  <output id="out"></output>
`;
const fileEl = document.querySelector('#file');
const editEl = document.querySelector('#edit');
const fileMenu = document.querySelector('#filemenu');
const editMenu = document.querySelector('#editmenu');
const out = document.querySelector('#out');
fileEl.addEventListener('mouseover', () => fileMenu.hidden = false);
editEl.addEventListener('mouseover', () => editMenu.hidden = false);
document.querySelector('#openItem').addEventListener('click', () => out.textContent = 'open');
document.querySelector('#paste').addEventListener('click', () => out.textContent = 'paste');
"##
        .to_string(),
    ]);

    let open = act_instruction(&env, "Select File > Open");
    let paste = act_instruction(&env, "Select Edit > Paste");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "paste");
    assert_eq!(open["plan"]["action"], "select_menu_path");
    assert_eq!(paste["plan"]["action"], "select_menu_path");
    assert_eq!(open["verification"]["status"], "observed");
    assert_eq!(paste["verification"]["status"], "observed");
    assert_eq!(open["result"]["menuPath"]["path"][0], "File");
    assert_eq!(open["result"]["menuPath"]["path"][1], "Open");
    assert_eq!(paste["result"]["menuPath"]["path"][0], "Edit");
    assert_eq!(paste["result"]["menuPath"]["path"][1], "Paste");

    env.stop();
}

#[test]
fn act_instruction_selects_shadow_dom_nested_menu_paths_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-menu-path");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <app-menu></app-menu>
  <output id="out"></output>
`;
customElements.define('app-menu', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <nav>
        <ul role="menubar" id="menu">
          <li role="menuitem" id="file" tabindex="0">File
            <ul role="menu" id="filemenu" hidden>
              <li role="menuitem" id="exportItem" data-aliases="download save">Export</li>
              <li role="menuitem" id="print">Print</li>
            </ul>
          </li>
          <li role="menuitem" id="edit" tabindex="0">Edit
            <ul role="menu" id="editmenu" hidden>
              <li role="menuitem" id="copy">Copy</li>
            </ul>
          </li>
        </ul>
      </nav>
    `;
    const fileEl = root.querySelector('#file');
    const fileMenu = root.querySelector('#filemenu');
    fileEl.addEventListener('mouseover', () => fileMenu.hidden = false);
    root.querySelector('#exportItem').addEventListener('click', () => {
      document.querySelector('#out').textContent = 'export';
    });
  }
});
"##
        .to_string(),
    ]);

    let export = act_instruction(&env, "Select File > Export");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "export");
    assert_eq!(export["plan"]["action"], "select_menu_path");
    assert_eq!(export["result"]["menuPath"]["path"][0], "File");
    assert_eq!(export["result"]["menuPath"]["path"][1], "Export");
    assert_eq!(export["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_discovers_menu_item_by_icon_metadata_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-icon-menu");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="open-menu">Menu</button>
  <ul id="menu" role="menu" hidden>
    <li class="ui-menu-item" role="none">
      <div id="save" class="ui-menu-item-wrapper" role="menuitem">
        <span class="ui-icon ui-icon-disk" aria-hidden="true"></span>
        Save
      </div>
    </li>
    <li class="ui-menu-item" role="none">
      <div id="play" class="ui-menu-item-wrapper" role="menuitem">
        <span class="ui-icon ui-icon-play" aria-hidden="true"></span>
        Play
      </div>
    </li>
  </ul>
  <output id="out"></output>
`;
const menu = document.querySelector('#menu');
document.querySelector('#open-menu').addEventListener('click', () => {
  menu.hidden = false;
});
document.querySelectorAll('[role=menuitem]').forEach(item => {
  item.addEventListener('click', () => {
    document.querySelector('#out').textContent = item.textContent.trim();
  });
});
"##
        .to_string(),
    ]);

    let icon_menu = act_instruction(
        &env,
        "Click the \"Menu\" button, and then find and click on the item with the \"ui-icon-play\" icon.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "Play");
    assert_eq!(icon_menu["intent"]["followUpClickHint"], "ui-icon-play");
    assert_eq!(icon_menu["plan"]["action"], "discover_click");
    assert_eq!(
        icon_menu["result"]["discoverClick"]["target"],
        "ui-icon-play"
    );
    assert_eq!(icon_menu["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_renders_digit_pattern_into_checkbox_grid_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-checkbox-grid");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #grid {
      display: grid;
      grid-template-columns: repeat(4, 22px);
      gap: 4px;
      width: max-content;
      margin: 20px;
    }
    #grid input {
      width: 18px;
      height: 18px;
    }
  </style>
  <div id="grid" aria-label="checkbox pattern grid"></div>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const grid = document.querySelector('#grid');
for (let i = 0; i < 28; i++) {
  const input = document.createElement('input');
  input.type = 'checkbox';
  input.id = 'cell-' + i;
  grid.appendChild(input);
}
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'submitted';
});
"##
        .to_string(),
    ]);

    let render = act_instruction(
        &env,
        "Draw the number \"2\" in the checkboxes and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        r##"
JSON.stringify({
  submitted: document.querySelector('#out').textContent,
  checked: Array.from(document.querySelectorAll('#grid input')).map((input, index) => input.checked ? index : null).filter(index => index !== null)
})
"##
        .to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse checkbox grid state");

    assert_eq!(state["submitted"], "submitted");
    assert_eq!(
        state["checked"],
        serde_json::json!([1, 2, 4, 7, 8, 11, 14, 17, 20, 24, 25, 26, 27])
    );
    assert_eq!(render["analysis"]["kind"], "render_pattern");
    assert_eq!(render["plan"]["action"], "sequence");
    assert_eq!(render["plan"]["steps"][0]["action"], "set_checkbox_grid");
    assert_eq!(
        render["result"]["steps"][0]["checkboxGrid"]["mode"],
        "digit-glyph-checkbox-grid"
    );
    assert_eq!(render["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_renders_digit_pattern_in_shadow_dom_checkbox_grids_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-checkbox-grid");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <pattern-board></pattern-board>
  <output id="out"></output>
`;
customElements.define('pattern-board', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <style>
        #grid {
          display: grid;
          grid-template-columns: repeat(4, 22px);
          gap: 4px;
          width: max-content;
          margin: 20px;
        }
        #grid input {
          width: 18px;
          height: 18px;
        }
      </style>
      <div id="grid" aria-label="checkbox pattern grid"></div>
      <button id="submit">Submit</button>
    `;
    const grid = root.querySelector('#grid');
    for (let i = 0; i < 28; i++) {
      const input = document.createElement('input');
      input.type = 'checkbox';
      input.id = 'cell-' + i;
      grid.appendChild(input);
    }
    root.querySelector('#submit').addEventListener('click', () => {
      document.querySelector('#out').textContent = JSON.stringify({
        submitted: true,
        checked: Array.from(root.querySelectorAll('#grid input'))
          .map((input, index) => input.checked ? index : null)
          .filter(index => index !== null)
      });
    });
  }
});
"##
        .to_string(),
    ]);

    let render = act_instruction(
        &env,
        "Draw the number \"2\" in the checkboxes and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse shadow checkbox grid state");

    assert_eq!(state["submitted"], true);
    assert_eq!(
        state["checked"],
        serde_json::json!([1, 2, 4, 7, 8, 11, 14, 17, 20, 24, 25, 26, 27])
    );
    assert_eq!(render["analysis"]["kind"], "render_pattern");
    assert_eq!(render["plan"]["action"], "sequence");
    assert_eq!(render["plan"]["steps"][0]["action"], "set_checkbox_grid");
    assert_eq!(
        render["result"]["steps"][0]["checkboxGrid"]["mode"],
        "digit-glyph-checkbox-grid"
    );
    assert_eq!(render["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_renders_digit_pattern_into_custom_checkable_grids_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-custom-checkable-grid");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #grid {
      display: grid;
      grid-template-columns: repeat(4, 22px);
      gap: 4px;
      width: max-content;
      margin: 20px;
    }
    check-cell {
      display: inline-flex;
      width: 18px;
      height: 18px;
      border: 1px solid #444;
      align-items: center;
      justify-content: center;
    }
  </style>
  <div id="grid" aria-label="checkbox pattern grid"></div>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
if (!customElements.get('check-cell')) {
  customElements.define('check-cell', class extends HTMLElement {
    constructor() {
      super();
      this._checked = false;
    }
    connectedCallback() {
      if (!this.hasAttribute('role')) this.setAttribute('role', 'checkbox');
      if (!this.hasAttribute('tabindex')) this.setAttribute('tabindex', '0');
      this.render();
    }
    get checked() {
      return this._checked;
    }
    set checked(next) {
      this._checked = Boolean(next);
      this.render();
    }
    render() {
      this.setAttribute('aria-checked', String(this._checked));
      this.textContent = this._checked ? 'x' : '';
    }
  });
}
const grid = document.querySelector('#grid');
for (let i = 0; i < 28; i++) {
  const cell = document.createElement('check-cell');
  cell.id = 'cell-' + i;
  cell.setAttribute('aria-label', 'Pattern cell ' + (i + 1));
  grid.appendChild(cell);
}
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent = JSON.stringify({
    submitted: true,
    checked: Array.from(document.querySelectorAll('#grid check-cell'))
      .map((cell, index) => cell.checked ? index : null)
      .filter(index => index !== null)
  });
});
"##
        .to_string(),
    ]);

    let render = act_instruction(
        &env,
        "Draw the number \"2\" in the checkboxes and press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse custom grid");

    assert_eq!(state["submitted"], true);
    assert_eq!(
        state["checked"],
        serde_json::json!([1, 2, 4, 7, 8, 11, 14, 17, 20, 24, 25, 26, 27])
    );
    assert_eq!(render["analysis"]["kind"], "render_pattern");
    assert_eq!(render["plan"]["action"], "sequence");
    assert_eq!(render["plan"]["steps"][0]["action"], "set_checkbox_grid");
    assert_eq!(
        render["result"]["steps"][0]["checkboxGrid"]["mode"],
        "digit-glyph-checkbox-grid"
    );
    assert_eq!(render["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_drags_visible_content_to_drop_zone_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-drag");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #decoy-source, #decoy-target {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 120px;
      height: 80px;
      margin: 20px;
      border: 1px solid #333;
    }
    #source { background: #def; }
    #target { background: #fed; }
  </style>
  <div id="source" draggable="true">Token</div>
  <div id="target" data-dropzone="true">Drop Zone</div>
  <output id="out"></output>
`;
const target = document.querySelector('#target');
const out = document.querySelector('#out');
target.addEventListener('mousemove', () => {
  out.textContent = 'entered';
});
target.addEventListener('mouseup', () => {
  out.textContent = 'mouseup';
  target.textContent = 'Token moved';
});
"##
        .to_string(),
    ]);

    let drag = act_instruction(&env, "drag Token to Drop Zone");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({out: document.querySelector('#out').textContent, target: document.querySelector('#target').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse drag state");

    assert_eq!(state["out"], "mouseup");
    assert_eq!(state["target"], "Token moved");
    assert_eq!(drag["plan"]["action"], "drag");
    assert_eq!(drag["plan"]["candidate"]["source"]["selector"], "#source");
    assert_eq!(drag["plan"]["candidate"]["target"]["selector"], "#target");
    assert_eq!(drag["pageModel"]["before"]["summary"]["draggable"], 1);
    assert_eq!(drag["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn drag_command_uses_shadow_dom_selector_targets_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("drag-shadow-selector-targets");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #source, #target {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 120px;
      height: 80px;
      margin: 20px;
      border: 1px solid #900;
    }
  </style>
  <div id="decoy-source">Decoy Source</div>
  <div id="decoy-target">Decoy Target</div>
  <drag-board></drag-board>
  <output id="out"></output>
`;
customElements.define('drag-board', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <style>
        #source, #target {
          display: inline-flex;
          align-items: center;
          justify-content: center;
          width: 120px;
          height: 80px;
          margin: 20px;
          border: 1px solid #333;
        }
        #source { background: #def; }
        #target { background: #fed; }
      </style>
      <div id="source">Token</div>
      <div id="target">Drop Zone</div>
    `;
    let started = false;
    root.querySelector('#source').addEventListener('mousedown', () => {
      started = true;
    });
    root.querySelector('#target').addEventListener('mousemove', () => {
      if (started) document.querySelector('#out').textContent = 'entered';
    });
    root.querySelector('#target').addEventListener('mouseup', () => {
      if (!started) return;
      root.querySelector('#target').textContent = 'Token moved';
      document.querySelector('#out').textContent = 'shadow-drop';
    });
  }
});
"##
        .to_string(),
    ]);

    let drag = env.json(&[
        "drag".to_string(),
        "#source".to_string(),
        "#target".to_string(),
        "--steps".to_string(),
        "5".to_string(),
    ]);
    let result = env.json(&[
        "eval".to_string(),
        r#"JSON.stringify({
  out: document.querySelector('#out').textContent,
  lightTarget: document.querySelector('#decoy-target').textContent,
  shadowTarget: document.querySelector('drag-board').shadowRoot.querySelector('#target').textContent
})"#
        .to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse drag state");

    assert_eq!(state["out"], "shadow-drop");
    assert_eq!(state["lightTarget"], "Decoy Target");
    assert_eq!(state["shadowTarget"], "Token moved");
    assert_eq!(drag["dragged"]["source"], "#source");
    assert_eq!(drag["dragged"]["target"], "#target");

    env.stop();
}

#[test]
fn act_instruction_drags_visible_object_in_requested_direction_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-directional-drag");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #token {
      position: absolute;
      left: 40px;
      top: 60px;
      width: 44px;
      height: 44px;
      background: #2f6fed;
      color: white;
      user-select: none;
    }
    #target {
      position: absolute;
      left: 160px;
      top: 55px;
      width: 70px;
      height: 54px;
      border: 2px dashed #333;
    }
  </style>
  <div id="token" draggable="true">Token</div>
  <div id="target" data-dropzone="true">Drop zone</div>
  <output id="out"></output>
`;
let start = null;
document.querySelector('#token').addEventListener('mousedown', event => {
  start = { x: event.clientX, y: event.clientY };
});
document.addEventListener('mouseup', event => {
  if (!start) return;
  const movedRight = event.clientX - start.x > 80 && Math.abs(event.clientY - start.y) < 40;
  document.querySelector('#out').textContent = movedRight ? 'right' : 'miss';
});
"##
        .to_string(),
    ]);

    let drag = act_instruction(&env, "Drag the Token right.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "right");
    assert_eq!(drag["analysis"]["kind"], "drag");
    assert_eq!(drag["plan"]["action"], "drag");
    assert_eq!(drag["plan"]["evidence"]["direction"], "right");
    assert_eq!(drag["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_draws_line_on_visible_surface_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-line-draw");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <canvas id="surface" aria-label="drawing surface" width="220" height="120"
    style="width:220px;height:120px;border:1px solid #333"></canvas>
  <output id="out"></output>
`;
const canvas = document.querySelector('#surface');
const points = [];
canvas.addEventListener('mousedown', event => {
  points.length = 0;
  points.push({ type: 'down', x: event.offsetX, y: event.offsetY });
});
canvas.addEventListener('mousemove', event => {
  if (points.length) points.push({ type: 'move', x: event.offsetX, y: event.offsetY });
});
canvas.addEventListener('mouseup', event => {
  points.push({ type: 'up', x: event.offsetX, y: event.offsetY });
  const first = points[0];
  const last = points[points.length - 1];
  const horizontal = Math.abs(last.y - first.y) < 10 && last.x - first.x > 120 && points.length >= 4;
  document.querySelector('#out').textContent = horizontal ? 'line' : 'miss';
});
"##
        .to_string(),
    ]);

    let draw = act_instruction(&env, "Draw a horizontal line on the drawing surface.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "line");
    assert_eq!(draw["analysis"]["kind"], "drag");
    assert_eq!(draw["analysis"]["value"], "line");
    assert_eq!(draw["plan"]["action"], "drag");
    assert_eq!(draw["plan"]["evidence"]["explicitCoordinates"], false);
    assert_eq!(draw["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_bisects_visible_svg_angles_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-angle-bisector");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" aria-label="geometry drawing surface" width="180" height="140"
    style="width:180px;height:140px;border:1px solid #333">
    <line x1="42" y1="102" x2="138" y2="26" stroke="black"></line>
    <line x1="42" y1="102" x2="145" y2="118" stroke="black"></line>
    <circle class="vertex blue" cx="42" cy="102" r="4" fill="blue"></circle>
    <circle class="endpoint black" cx="138" cy="26" r="4" fill="black"></circle>
    <circle class="endpoint black" cx="145" cy="118" r="4" fill="black"></circle>
  </svg>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const svg = document.querySelector('#surface');
let clicked = null;
function localPoint(event) {
  const point = svg.createSVGPoint();
  point.x = event.clientX;
  point.y = event.clientY;
  const matrix = svg.getScreenCTM().inverse();
  return point.matrixTransform(matrix);
}
function angle(a, b, v) {
  const av = Math.hypot(a.x - v.x, a.y - v.y);
  const bv = Math.hypot(b.x - v.x, b.y - v.y);
  const ab = Math.hypot(b.x - a.x, b.y - a.y);
  return Math.acos((bv * bv + av * av - ab * ab) / (2 * bv * av)) * 180 / Math.PI;
}
svg.addEventListener('click', event => {
  clicked = localPoint(event);
});
document.querySelector('#submit').addEventListener('click', () => {
  const vertex = { x: 42, y: 102 };
  const a = { x: 138, y: 26 };
  const b = { x: 145, y: 118 };
  if (!clicked) {
    document.querySelector('#out').textContent = 'missing';
    return;
  }
  const delta = Math.abs(angle(a, clicked, vertex) - angle(b, clicked, vertex));
  document.querySelector('#out').textContent = delta < 4 ? 'bisected' : `miss:${delta.toFixed(2)}`;
});
"##
        .to_string(),
    ]);

    let draw = act_instruction(
        &env,
        "Create a line that bisects the angle evenly in two, then press submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "bisected");
    assert_eq!(draw["analysis"]["kind"], "drag");
    assert_eq!(draw["analysis"]["value"], "line");
    assert_eq!(draw["plan"]["action"], "sequence");
    assert_eq!(draw["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_bisects_svg_angles_in_container_event_coordinates_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-angle-bisector-container-coords");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    body { margin: 0; }
    #query { height: 50px; }
    #area { position: relative; width: 160px; height: 160px; }
    #surface { width: 150px; height: 130px; margin: 2px; }
  </style>
  <div id="query">Create a line that bisects the angle evenly in two, then press submit.</div>
  <div id="area">
    <svg id="surface" aria-label="geometry drawing surface">
      <line x1="122" y1="16" x2="8" y2="72" stroke="black"></line>
      <line x1="122" y1="16" x2="62" y2="48" stroke="black"></line>
      <circle class="vertex blue" cx="122" cy="16" r="3.5" fill="blue"></circle>
      <circle class="endpoint black" cx="8" cy="72" r="3.5" fill="black"></circle>
      <circle class="endpoint black" cx="62" cy="48" r="3.5" fill="black"></circle>
    </svg>
    <button id="submit">Submit</button>
  </div>
  <output id="out"></output>
`;
const svg = document.querySelector('#surface');
const area = document.querySelector('#area');
let clicked = null;
function angle(a, b, v) {
  const av = Math.hypot(a.x - v.x, a.y - v.y);
  const bv = Math.hypot(b.x - v.x, b.y - v.y);
  const ab = Math.hypot(b.x - a.x, b.y - a.y);
  return Math.acos((bv * bv + av * av - ab * ab) / (2 * bv * av)) * 180 / Math.PI;
}
svg.addEventListener('click', event => {
  const rect = area.getBoundingClientRect();
  clicked = { x: event.pageX - rect.left, y: event.pageY - rect.top };
});
document.querySelector('#submit').addEventListener('click', () => {
  const vertex = { x: 122, y: 16 };
  const a = { x: 8, y: 72 };
  const b = { x: 62, y: 48 };
  if (!clicked) {
    document.querySelector('#out').textContent = 'missing';
    return;
  }
  const delta = Math.abs(angle(a, clicked, vertex) - angle(b, clicked, vertex));
  document.querySelector('#out').textContent = delta < 0.4 ? 'bisected' : `miss:${delta.toFixed(2)}`;
});
"##
        .to_string(),
    ]);

    let draw = act_instruction(
        &env,
        "Create a line that bisects the angle evenly in two, then press submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "bisected");
    assert_eq!(draw["analysis"]["kind"], "drag");
    assert_eq!(draw["analysis"]["value"], "line");
    assert_eq!(draw["plan"]["action"], "sequence");
    assert_eq!(
        draw["plan"]["steps"][0]["evidence"]["coordinateMode"],
        "container-local-event"
    );
    assert_eq!(draw["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_constructs_perpendicular_point_from_visible_segment_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-perpendicular-point-construction");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" aria-label="geometry drawing surface" width="180" height="140"
    style="width:180px;height:140px;border:1px solid #333">
    <circle id="fixed-point" class="endpoint" cx="35" cy="54" r="4" fill="black"></circle>
    <circle id="vertex-point" class="vertex active" cx="122" cy="106" r="4" fill="blue"></circle>
    <line x1="35" y1="54" x2="122" y2="106" stroke="black"></line>
  </svg>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const svg = document.querySelector('#surface');
let clicked = null;
function localPoint(event) {
  const point = svg.createSVGPoint();
  point.x = event.clientX;
  point.y = event.clientY;
  return point.matrixTransform(svg.getScreenCTM().inverse());
}
function angle(a, b, v) {
  const av = Math.hypot(a.x - v.x, a.y - v.y);
  const bv = Math.hypot(b.x - v.x, b.y - v.y);
  const ab = Math.hypot(b.x - a.x, b.y - a.y);
  return Math.acos((bv * bv + av * av - ab * ab) / (2 * bv * av)) * 180 / Math.PI;
}
svg.addEventListener('click', event => {
  clicked = localPoint(event);
});
document.querySelector('#submit').addEventListener('click', () => {
  const vertex = { x: 122, y: 106 };
  const fixed = { x: 35, y: 54 };
  if (!clicked) {
    out.textContent = 'missing';
    return;
  }
  const delta = Math.abs(90 - angle(fixed, clicked, vertex));
  out.textContent = delta < 3 ? 'perpendicular' : `miss:${delta.toFixed(2)}`;
});
"##
        .to_string(),
    ]);

    let result = act_instruction(
        &env,
        "Add a third point to create a right angle, then press submit.",
    );
    let state = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(state["result"], "perpendicular");
    assert_eq!(
        result["plan"]["capability"]["name"],
        "perpendicular-point-construction"
    );
    assert_eq!(result["plan"]["action"], "sequence");
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_draws_oriented_line_through_visible_marker_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-line-through-marker");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" aria-label="drawing surface" width="180" height="140"
    style="width:180px;height:140px;border:1px solid #333">
    <circle id="dot" cx="72" cy="82" r="5" fill="purple"></circle>
  </svg>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const svg = document.querySelector('#surface');
const submit = document.querySelector('#submit');
const out = document.querySelector('#out');
const points = [];
function localPoint(event) {
  const rect = svg.getBoundingClientRect();
  return { x: event.clientX - rect.left, y: event.clientY - rect.top };
}
svg.addEventListener('mousedown', event => {
  points.length = 0;
  points.push(localPoint(event));
});
svg.addEventListener('mousemove', event => {
  if (points.length) points.push(localPoint(event));
});
svg.addEventListener('mouseup', event => {
  if (points.length) points.push(localPoint(event));
});
submit.addEventListener('click', () => {
  const first = points[0] || { x: 0, y: 0 };
  const last = points[points.length - 1] || { x: 999, y: 999 };
  const crossesDot = Math.abs(first.x - 72) < 8 && Math.abs(last.x - 72) < 8;
  const vertical = Math.abs(last.y - first.y) > 80 && Math.abs(last.x - first.x) < 8;
  out.textContent = crossesDot && vertical ? 'done' : JSON.stringify({ first, last, count: points.length });
});
"##
        .to_string(),
    ]);

    let draw = act_instruction(
        &env,
        "Draw a vertical line that runs through the dot, then press submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "done");
    assert_eq!(draw["analysis"]["kind"], "drag");
    assert_eq!(draw["plan"]["action"], "sequence");
    assert_eq!(draw["plan"]["steps"][0]["action"], "drag");
    assert_eq!(draw["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_draws_circle_around_visible_marker_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-circle-path-draw");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="surface" aria-label="drawing surface" width="180" height="150"
    style="width:180px;height:150px;border:1px solid #333">
    <circle id="marker" cx="82" cy="74" r="4" fill="black"></circle>
  </svg>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const svg = document.querySelector('#surface');
const submit = document.querySelector('#submit');
const out = document.querySelector('#out');
const points = [];
function localPoint(event) {
  const rect = svg.getBoundingClientRect();
  return { x: event.clientX - rect.left, y: event.clientY - rect.top };
}
svg.addEventListener('mousedown', event => {
  points.length = 0;
  points.push(localPoint(event));
});
svg.addEventListener('mousemove', event => {
  if (points.length) points.push(localPoint(event));
});
svg.addEventListener('mouseup', event => {
  if (points.length) points.push(localPoint(event));
});
submit.addEventListener('click', () => {
  if (points.length < 20) {
    out.textContent = `too-few:${points.length}`;
    return;
  }
  const xs = points.map(point => point.x);
  const ys = points.map(point => point.y);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  const surroundsMarker = minX < 82 && maxX > 82 && minY < 74 && maxY > 74;
  const hasWidth = maxX - minX > 35;
  const hasHeight = maxY - minY > 35;
  const closes = Math.hypot(points[0].x - points[points.length - 1].x, points[0].y - points[points.length - 1].y) < 12;
  out.textContent = surroundsMarker && hasWidth && hasHeight && closes ? 'circle' :
    JSON.stringify({ count: points.length, minX, maxX, minY, maxY, closes });
});
"##
        .to_string(),
    ]);

    let draw = act_instruction(
        &env,
        "Draw a circle centered around the marked point, then press submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "circle");
    assert_eq!(draw["analysis"]["kind"], "drag");
    assert_eq!(draw["analysis"]["value"], "circle");
    assert_eq!(draw["plan"]["action"], "sequence");
    assert_eq!(draw["plan"]["steps"][0]["action"], "draw_path");
    assert_eq!(draw["plan"]["steps"][0]["evidence"]["shape"], "circle");
    assert_eq!(draw["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_reorders_visible_list_items_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-list-reorder-drag");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #items { width: 180px; margin: 0; padding: 0; }
    #items li {
      list-style: none;
      height: 34px;
      line-height: 34px;
      border: 1px solid #777;
      margin: 2px 0;
      padding-left: 8px;
      user-select: none;
    }
  </style>
  <ul id="items" role="list">
    <li draggable="true">Alpha</li>
    <li draggable="true">Beta</li>
    <li draggable="true">Gamma</li>
    <li draggable="true">Delta</li>
  </ul>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const list = document.querySelector('#items');
let dragging = null;
list.querySelectorAll('li').forEach(item => {
  item.addEventListener('mousedown', () => {
    dragging = item;
  });
});
document.addEventListener('mouseup', event => {
  if (!dragging) return;
  const items = Array.from(list.querySelectorAll('li'));
  const fromIndex = items.indexOf(dragging);
  let targetIndex = fromIndex;
  for (const [index, item] of items.entries()) {
    if (item === dragging) continue;
    const rect = item.getBoundingClientRect();
    if (event.clientY >= rect.top && event.clientY <= rect.bottom) {
      targetIndex = index;
    }
  }
  if (targetIndex > fromIndex) {
    list.insertBefore(dragging, items[targetIndex].nextSibling);
  } else if (targetIndex < fromIndex) {
    list.insertBefore(dragging, items[targetIndex]);
  }
  dragging = null;
});
document.querySelector('#submit').addEventListener('click', () => {
  out.textContent = Array.from(list.querySelectorAll('li')).map(item => item.textContent.trim()).join(',');
});
"##
        .to_string(),
    ]);

    let drag = act_instruction(&env, "Move Beta down by one position, then press Submit.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "Alpha,Gamma,Beta,Delta");
    assert_eq!(drag["analysis"]["kind"], "drag");
    assert_eq!(drag["plan"]["action"], "sequence");
    assert_eq!(drag["plan"]["steps"][0]["action"], "drag");
    assert_eq!(drag["plan"]["steps"][0]["evidence"]["direction"], "down");
    assert_eq!(drag["verification"]["status"], "observed");

    let ordinal_drag = act_instruction(&env, "Move Delta to the 2nd position, then press Submit.");
    let ordinal_result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(ordinal_result["result"], "Alpha,Delta,Gamma,Beta");
    assert_eq!(ordinal_drag["analysis"]["kind"], "drag");
    assert_eq!(ordinal_drag["plan"]["action"], "sequence");
    assert_eq!(ordinal_drag["plan"]["steps"][0]["action"], "drag");
    assert_eq!(
        ordinal_drag["plan"]["steps"][0]["evidence"]["requestedPosition"],
        2
    );
    assert!(["observed", "executed_unverified"].contains(
        &ordinal_drag["verification"]["status"]
            .as_str()
            .unwrap_or_default()
    ));

    env.stop();
}

#[test]
fn act_instruction_drags_visible_item_to_grid_slot_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-grid-slot-drag");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #grid {
      display: grid;
      grid-template-columns: repeat(3, 58px);
      gap: 6px;
      width: 186px;
      user-select: none;
    }
    .cell {
      height: 38px;
      border: 1px solid #777;
      display: flex;
      align-items: center;
      justify-content: center;
    }
  </style>
  <div id="grid" role="grid">
    <div class="cell" draggable="true">Ada</div>
    <div class="cell" draggable="true">Bea</div>
    <div class="cell" draggable="true">Cory</div>
    <div class="cell" draggable="true">Drew</div>
    <div class="cell" draggable="true">Eli</div>
    <div class="cell" draggable="true">Finn</div>
    <div class="cell" draggable="true">Gail</div>
    <div class="cell" draggable="true">Hana</div>
    <div class="cell" draggable="true">Ira</div>
  </div>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const grid = document.querySelector('#grid');
let dragging = null;
grid.querySelectorAll('.cell').forEach(cell => {
  cell.addEventListener('mousedown', () => {
    dragging = cell;
  });
});
document.addEventListener('mouseup', event => {
  if (!dragging) return;
  const cells = Array.from(grid.querySelectorAll('.cell'));
  let target = dragging;
  let best = Infinity;
  for (const cell of cells) {
    const rect = cell.getBoundingClientRect();
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height / 2;
    const distance = Math.hypot(event.clientX - centerX, event.clientY - centerY);
    if (distance < best) {
      best = distance;
      target = cell;
    }
  }
  const fromText = dragging.textContent;
  dragging.textContent = target.textContent;
  target.textContent = fromText;
  dragging = null;
});
document.querySelector('#submit').addEventListener('click', () => {
  const cells = Array.from(grid.querySelectorAll('.cell'));
  document.querySelector('#out').textContent = cells.indexOf(cells.find(cell => cell.textContent.trim() === 'Bea'));
});
"##
        .to_string(),
    ]);

    let drag = act_instruction(&env, "Drag Bea to the bottom center, then press Submit.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "7");
    assert_eq!(drag["analysis"]["kind"], "drag");
    assert_eq!(drag["plan"]["action"], "sequence");
    assert_eq!(drag["plan"]["steps"][0]["action"], "drag");
    assert_eq!(drag["plan"]["steps"][0]["evidence"]["rows"], 3);
    assert_eq!(drag["plan"]["steps"][0]["evidence"]["cols"], 3);
    assert_eq!(drag["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sorts_visible_numeric_list_by_dragging_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-numeric-sort-drag");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #numbers { width: 130px; margin: 0; padding: 0; }
    #numbers li {
      list-style: none;
      height: 32px;
      line-height: 32px;
      border: 1px solid #777;
      margin: 2px 0;
      padding-left: 8px;
      user-select: none;
    }
  </style>
  <ul id="numbers" class="sortable">
    <li draggable="true">81</li>
    <li draggable="true">-12</li>
    <li draggable="true">43</li>
    <li draggable="true">7</li>
  </ul>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const list = document.querySelector('#numbers');
let dragging = null;
list.querySelectorAll('li').forEach(item => {
  item.addEventListener('mousedown', () => {
    dragging = item;
  });
});
document.addEventListener('mouseup', event => {
  if (!dragging) return;
  const items = Array.from(list.querySelectorAll('li'));
  const fromIndex = items.indexOf(dragging);
  let targetIndex = fromIndex;
  for (const [index, item] of items.entries()) {
    if (item === dragging) continue;
    const rect = item.getBoundingClientRect();
    if (event.clientY >= rect.top && event.clientY <= rect.bottom) {
      targetIndex = index;
    }
  }
  if (targetIndex > fromIndex) {
    list.insertBefore(dragging, items[targetIndex].nextSibling);
  } else if (targetIndex < fromIndex) {
    list.insertBefore(dragging, items[targetIndex]);
  }
  dragging = null;
});
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    Array.from(list.querySelectorAll('li')).map(item => item.textContent.trim()).join(',');
});
"##
        .to_string(),
    ]);

    let drag = act_instruction(
        &env,
        "Sort the numbers from lowest to highest, then press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "-12,7,43,81");
    assert_eq!(drag["analysis"]["kind"], "drag");
    assert_eq!(drag["plan"]["action"], "sequence");
    assert_eq!(
        drag["plan"]["evidence"]["sortedValues"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(drag["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_completes_when_numeric_list_is_already_sorted_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-numeric-sort-already-done");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <ul id="numbers" class="sortable">
    <li>-59</li>
    <li>-14</li>
    <li>70</li>
    <li>70</li>
  </ul>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
document.querySelector('#submit').addEventListener('click', () => {
  document.querySelector('#out').textContent =
    Array.from(document.querySelectorAll('#numbers li')).map(item => item.textContent.trim()).join(',');
});
"##
        .to_string(),
    ]);

    let drag = act_instruction(
        &env,
        "Sort the numbers in increasing order, starting with the lowest number at the top of the list, then press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "-59,-14,70,70");
    assert_eq!(drag["analysis"]["kind"], "drag");
    assert_eq!(drag["plan"]["action"], "sequence");
    assert_eq!(drag["plan"]["evidence"]["moves"], 0);
    assert_eq!(drag["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_partitions_visible_svg_shapes_by_dragging_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shape-partition-drag");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <svg id="stage" width="260" height="140" style="width:260px;height:140px;border:1px solid #ddd">
    <rect id="blackBox" x="170" y="20" width="70" height="90" fill="none" stroke="black" stroke-width="3"></rect>
    <polygon id="triA" class="shape" points="30,30 45,60 15,60" fill="blue"></polygon>
    <circle id="circleA" class="shape" cx="75" cy="45" r="16" fill="green"></circle>
    <polygon id="triB" class="shape" points="110,35 128,70 92,70" fill="magenta"></polygon>
    <rect id="squareA" class="shape" x="42" y="86" width="26" height="26" fill="orange"></rect>
  </svg>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
const box = document.querySelector('#blackBox');
let dragging = null;
document.querySelectorAll('.shape').forEach(shape => {
  shape.addEventListener('mousedown', () => {
    dragging = shape;
  });
});
document.addEventListener('mouseup', event => {
  if (!dragging) return;
  const rect = box.getBoundingClientRect();
  if (event.clientX > rect.left && event.clientX < rect.right &&
      event.clientY > rect.top && event.clientY < rect.bottom) {
    dragging.dataset.target = 'black';
  }
  dragging = null;
});
document.querySelector('#submit').addEventListener('click', () => {
  const triangles = Array.from(document.querySelectorAll('polygon.shape'));
  const nonTriangles = Array.from(document.querySelectorAll('.shape:not(polygon)'));
  const ok = triangles.every(shape => shape.dataset.target === 'black') &&
    nonTriangles.every(shape => shape.dataset.target !== 'black');
  document.querySelector('#out').textContent = ok ? 'partitioned' : 'wrong';
});
"##
        .to_string(),
    ]);

    let drag = act_instruction(
        &env,
        "Drag all triangles into the black box, then press Submit.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "partitioned");
    assert_eq!(drag["analysis"]["kind"], "drag");
    assert_eq!(drag["plan"]["action"], "sequence");
    assert_eq!(
        drag["plan"]["reason"],
        "planned visual shape partitioning by generic SVG/DOM geometry and attributes"
    );
    assert_eq!(drag["plan"]["evidence"]["primaryCount"], 2);
    assert_eq!(drag["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_drags_smaller_box_inside_larger_box_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-geometry-drag");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #stage {
      position: relative;
      width: 220px;
      height: 140px;
      border: 1px solid #bbb;
    }
    #smallBox {
      position: absolute;
      left: 10px;
      top: 15px;
      width: 24px;
      height: 24px;
      border: 1px solid #111;
      background: red;
    }
    #largeBox {
      position: absolute;
      left: 120px;
      top: 60px;
      width: 72px;
      height: 72px;
      border: 1px solid #111;
      background: blue;
    }
  </style>
  <div id="stage">
    <div id="smallBox" draggable="true"></div>
    <div id="largeBox"></div>
  </div>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
let droppedInside = false;
const small = document.querySelector('#smallBox');
const large = document.querySelector('#largeBox');
large.addEventListener('mouseup', event => {
  const bounds = large.getBoundingClientRect();
  droppedInside = event.clientX > bounds.left && event.clientX < bounds.right &&
    event.clientY > bounds.top && event.clientY < bounds.bottom;
});
submit.addEventListener('click', () => {
  out.textContent = droppedInside ? 'inside-submitted' : 'not-inside';
});
"##
        .to_string(),
    ]);

    let drag = act_instruction(
        &env,
        "Drag the smaller box so that it is completely inside the larger box.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "inside-submitted");
    assert_eq!(drag["plan"]["action"], "sequence");
    assert_eq!(drag["plan"]["steps"][0]["action"], "drag");
    assert_eq!(
        drag["plan"]["steps"][0]["candidate"]["source"]["selector"],
        "#smallBox"
    );
    assert_eq!(
        drag["plan"]["steps"][0]["candidate"]["target"]["selector"],
        "#largeBox"
    );
    assert_eq!(drag["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_orients_visual_surface_to_requested_active_face_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-orient-visual");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #rotator {
      position: relative;
      width: 160px;
      height: 120px;
      border: 1px solid #444;
      user-select: none;
    }
    .face {
      position: absolute;
      inset: 20px 40px;
      display: none;
      align-items: center;
      justify-content: center;
      border: 1px solid #888;
      font: 24px sans-serif;
    }
    .face.active {
      display: flex;
      background: #eef;
    }
  </style>
  <div id="rotator" class="rotatable viewport" aria-roledescription="rotatable visual surface">
    <div class="face active">1</div>
    <div class="face">2</div>
    <div class="face">3</div>
    <div class="face">4</div>
  </div>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
let activeIndex = 0;
let start = null;
const faces = Array.from(document.querySelectorAll('.face'));
function setActive(index) {
  activeIndex = (index + faces.length) % faces.length;
  faces.forEach((face, faceIndex) => face.classList.toggle('active', faceIndex === activeIndex));
}
rotator.addEventListener('mousedown', event => {
  start = { x: event.clientX, y: event.clientY };
});
document.addEventListener('mouseup', event => {
  if (!start) return;
  const dx = event.clientX - start.x;
  const dy = event.clientY - start.y;
  if (Math.abs(dx) >= Math.abs(dy)) {
    setActive(activeIndex + (dx >= 0 ? 1 : 1));
  } else {
    setActive(activeIndex + (dy >= 0 ? 1 : 1));
  }
  start = null;
});
submit.addEventListener('click', () => {
  out.textContent = document.querySelector('.face.active').textContent.trim() === '3'
    ? 'oriented-submitted'
    : 'wrong-face';
});
"##
        .to_string(),
    ]);

    let oriented = act_instruction(
        &env,
        "Move the visual object around so that \"3\" is the active side facing the user.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "oriented-submitted");
    assert_eq!(oriented["analysis"]["kind"], "drag");
    assert_eq!(oriented["plan"]["action"], "sequence");
    assert_eq!(oriented["plan"]["steps"][0]["action"], "orient_visual");
    assert_eq!(oriented["plan"]["steps"][0]["evidence"]["targetText"], "3");
    assert_eq!(oriented["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_orients_kinetic_visual_surface_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-kinetic-orient-visual");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    #viewport {
      position: relative;
      width: 150px;
      height: 96px;
      border: 1px solid #444;
      user-select: none;
    }
    .face {
      position: absolute;
      inset: 18px 35px;
      display: none;
      align-items: center;
      justify-content: center;
      border: 1px solid #777;
      font: 18px sans-serif;
    }
    .face.active {
      display: flex;
      background: #eef;
    }
  </style>
  <div id="viewport" class="rotatable visual surface" aria-roledescription="rotatable visual surface">
    <div class="face active">Red</div>
    <div class="face">Blue</div>
    <div class="face">Green</div>
  </div>
  <button id="submit">Submit</button>
  <output id="out"></output>
`;
let start = null;
let moves = 0;
const viewportEl = document.querySelector('#viewport');
const faces = Array.from(document.querySelectorAll('.face'));
function setActive(index) {
  faces.forEach((face, faceIndex) => face.classList.toggle('active', faceIndex === index));
}
viewportEl.addEventListener('mousedown', event => {
  start = { x: event.clientX, y: event.clientY };
  moves = 0;
});
document.addEventListener('mousemove', () => {
  if (start) moves += 1;
});
document.addEventListener('mouseup', event => {
  if (!start) return;
  const dx = event.clientX - start.x;
  const enoughContinuousMotion = moves >= 24;
  start = null;
  if (!enoughContinuousMotion) return;
  setActive(dx < 0 ? 1 : 2);
});
submit.addEventListener('click', () => {
  out.textContent = document.querySelector('.face.active').textContent.trim() === 'Blue'
    ? 'oriented-submitted'
    : 'wrong-face';
});
"##
        .to_string(),
    ]);

    let oriented = act_instruction(
        &env,
        "Move the visual surface around so that \"Blue\" is the active face.",
    );
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "oriented-submitted");
    assert_eq!(oriented["analysis"]["kind"], "drag");
    assert_eq!(oriented["plan"]["action"], "sequence");
    assert_eq!(oriented["plan"]["steps"][0]["action"], "orient_visual");
    assert_eq!(
        oriented["plan"]["steps"][0]["evidence"]["targetText"],
        "Blue"
    );
    assert_eq!(oriented["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_focuses_textbox_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-focus");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <label>Comment <input id="comment" type="text"></label>
  <output id="out"></output>
`;
document.querySelector('#comment').addEventListener('focus', () => {
  document.querySelector('#out').textContent = 'focused';
});
"##
        .to_string(),
    ]);

    let focus = act_instruction(&env, "Focus into the textbox.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({active: document.activeElement && document.activeElement.id, out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse focus state");

    assert_eq!(state["active"], "comment");
    assert_eq!(state["out"], "focused");
    assert_eq!(focus["plan"]["action"], "focus");
    assert_eq!(focus["plan"]["candidate"]["selector"], "#comment");
    assert_eq!(focus["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_focuses_ordinal_textbox_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-ordinal-focus");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <input id="first" type="text">
  <input id="second" type="text">
  <input id="third" type="text">
  <output id="out"></output>
`;
document.querySelectorAll('input').forEach(input => {
  input.addEventListener('focus', () => {
    document.querySelector('#out').textContent = input.id;
  });
});
"##
        .to_string(),
    ]);

    let focus = act_instruction(&env, "Focus into the 3rd input textbox.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({active: document.activeElement && document.activeElement.id, out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse focus state");

    assert_eq!(state["active"], "third");
    assert_eq!(state["out"], "third");
    assert_eq!(focus["plan"]["action"], "focus");
    assert_eq!(focus["plan"]["candidate"]["selector"], "#third");
    assert_eq!(focus["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_uses_click_styles_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-click-styles");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="preview">Preview card</button>
  <button id="actions">Actions button</button>
  <output id="out"></output>
`;
window.clickStyleEvents = { doubleClicks: 0, contextMenus: 0 };
function render() {
  document.querySelector('#out').textContent =
    `double=${window.clickStyleEvents.doubleClicks};context=${window.clickStyleEvents.contextMenus}`;
}
document.querySelector('#preview').addEventListener('dblclick', () => {
  window.clickStyleEvents.doubleClicks += 1;
  render();
});
document.querySelector('#actions').addEventListener('contextmenu', event => {
  event.preventDefault();
  window.clickStyleEvents.contextMenus += 1;
  render();
});
render();
"##
        .to_string(),
    ]);

    let double_click = act_instruction(&env, "Double-click the Preview card.");
    let right_click = act_instruction(&env, "Right click the Actions button.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({events: window.clickStyleEvents, out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse click style state");

    assert_eq!(state["events"]["doubleClicks"], 1);
    assert_eq!(state["events"]["contextMenus"], 1);
    assert_eq!(state["out"], "double=1;context=1");
    assert_eq!(double_click["plan"]["action"], "click");
    assert_eq!(double_click["plan"]["params"]["click_count"], 2);
    assert_eq!(double_click["plan"]["candidate"]["selector"], "#preview");
    assert_eq!(right_click["plan"]["action"], "click");
    assert_eq!(right_click["plan"]["params"]["button"], "right");
    assert_eq!(right_click["plan"]["candidate"]["selector"], "#actions");

    env.stop();
}

#[test]
fn act_instruction_clicks_the_only_visible_button_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-single-button-click");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="subbtn">Click Me!</button>
  <output id="out"></output>
`;
subbtn.addEventListener('click', () => {
  out.textContent = 'clicked';
});
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click the button.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "clicked");
    assert_eq!(click["plan"]["action"], "click");
    assert_eq!(click["plan"]["candidate"]["selector"], "#subbtn");
    assert_eq!(click["plan"]["evidence"]["onlyVisibleClickable"], true);
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_unicode_visible_button_labels_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-unicode-button-label");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <button id="cancel">Cancél</button>
  <button id="confirm">确定</button>
  <button id="hearts">♥♥♥</button>
  <output id="out"></output>
`;
for (const button of document.querySelectorAll('button')) {
  button.addEventListener('click', () => {
    out.textContent = button.id;
  });
}
"##
        .to_string(),
    ]);

    let clicked = act_instruction(&env, "Click on the \"确定\" button.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "confirm");
    assert_eq!(clicked["plan"]["action"], "click");
    assert!(
        clicked["plan"]["confidence"].as_f64().unwrap_or_default() >= 0.9,
        "plan payload: {clicked}"
    );
    assert_eq!(clicked["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_skips_disabled_custom_click_targets_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-disabled-custom-click-targets");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="disabled-save" role="button" aria-label="Save" aria-disabled="true" tabindex="0">Save</div>
  <div id="enabled-save" role="button" aria-label="Save" tabindex="0">Save</div>
  <output id="out"></output>
`;
document.querySelector('#disabled-save').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'disabled';
});
document.querySelector('#enabled-save').addEventListener('click', () => {
  document.querySelector('#out').textContent = 'enabled';
});
"##
        .to_string(),
    ]);

    let click = act_instruction(&env, "Click Save.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "enabled");
    assert_eq!(click["plan"]["action"], "click");
    assert_eq!(click["plan"]["candidate"]["selector"], "#enabled-save");
    assert_eq!(click["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_ordinal_visible_targets_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-ordinal-click");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    .row { display: block; padding: 6px; cursor: pointer; }
  </style>
  <section id="records">
    <div id="row-a" class="row" role="listitem" tabindex="0">Open record</div>
    <div id="row-b" class="row" role="listitem" tabindex="0">Open record</div>
    <div id="row-c" class="row" role="listitem" tabindex="0">Open record</div>
  </section>
  <output id="out"></output>
`;
document.querySelectorAll('.row').forEach(row => {
  row.addEventListener('click', () => {
    document.querySelector('#out').textContent = row.id;
  });
});
"##
        .to_string(),
    ]);

    let row = act_instruction(&env, "Click the second row.");
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    assert_eq!(result["result"], "row-b");
    assert_eq!(row["plan"]["action"], "click");
    assert_eq!(row["plan"]["capability"]["name"], "ordinal-click-target");
    assert_eq!(row["plan"]["evidence"]["targetKind"], "row");
    assert_eq!(row["plan"]["evidence"]["resolvedIndex"], 1);
    assert_eq!(row["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_waits_for_page_conditions_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-wait-conditions");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <div id="spinner">Loading</div>
  <section id="results" hidden></section>
  <output id="out">pending</output>
`;
setTimeout(() => {
  document.querySelector('#results').hidden = false;
  document.querySelector('#results').textContent = 'Results ready';
  document.querySelector('#out').textContent = 'results-visible';
}, 180);
setTimeout(() => {
  document.querySelector('#spinner').hidden = true;
  document.querySelector('#out').textContent = 'spinner-hidden';
}, 320);
"##
        .to_string(),
    ]);

    let visible = act_instruction(&env, "Wait for Results ready to appear within 2 seconds.");
    let hidden = act_instruction(&env, "Wait until #spinner is hidden within 2 seconds.");
    let result = env.json(&[
        "eval".to_string(),
        "JSON.stringify({results: document.querySelector('#results').textContent, spinnerHidden: document.querySelector('#spinner').hidden, out: document.querySelector('#out').textContent})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(result["result"].as_str().unwrap_or("{}")).expect("parse wait state");

    assert_eq!(state["results"], "Results ready");
    assert_eq!(state["spinnerHidden"], true);
    assert_eq!(state["out"], "spinner-hidden");
    assert_eq!(visible["plan"]["action"], "wait_for");
    assert_eq!(visible["plan"]["params"]["condition"], "text_visible");
    assert_eq!(visible["plan"]["params"]["value"], "Results ready");
    assert_eq!(visible["plan"]["params"]["timeout"], 2000);
    assert_eq!(visible["verification"]["status"], "observed");
    assert_eq!(hidden["plan"]["action"], "wait_for");
    assert_eq!(hidden["plan"]["params"]["condition"], "selector_hidden");
    assert_eq!(hidden["plan"]["params"]["value"], "#spinner");
    assert_eq!(hidden["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_navigates_browser_history_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-navigation");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);

    let first_url =
        "data:text/html,%3Ctitle%3EAgent%20Nav%20First%3C%2Ftitle%3E%3Ch1%3EFirst%3C%2Fh1%3E";
    let second_url =
        "data:text/html,%3Ctitle%3EAgent%20Nav%20Second%3C%2Ftitle%3E%3Ch1%3ESecond%3C%2Fh1%3E";
    let first = act_instruction(&env, &format!("Open {first_url}."));
    let second = act_instruction(&env, &format!("Open {second_url}."));
    let back = act_instruction(&env, "Go back.");
    let after_back = env.json(&[
        "eval".to_string(),
        "JSON.stringify({url: location.href, title: document.title})".to_string(),
    ]);
    let after_back_state: Value =
        serde_json::from_str(after_back["result"].as_str().unwrap_or("{}"))
            .expect("parse back navigation state");

    let forward = act_instruction(&env, "Go forward.");
    let after_forward = env.json(&[
        "eval".to_string(),
        "JSON.stringify({url: location.href, title: document.title})".to_string(),
    ]);
    let after_forward_state: Value =
        serde_json::from_str(after_forward["result"].as_str().unwrap_or("{}"))
            .expect("parse forward navigation state");

    let reload = act_instruction(&env, "Reload the page.");

    assert_eq!(first["plan"]["action"], "navigate");
    assert_eq!(first["plan"]["params"]["url"], first_url);
    assert_eq!(first["verification"]["status"], "observed");
    assert_eq!(second["plan"]["action"], "navigate");
    assert_eq!(second["plan"]["params"]["url"], second_url);
    assert_eq!(back["plan"]["action"], "back");
    assert_eq!(after_back_state["title"], "Agent Nav First");
    assert_eq!(back["verification"]["status"], "observed");
    assert_eq!(forward["plan"]["action"], "forward");
    assert_eq!(after_forward_state["title"], "Agent Nav Second");
    assert_eq!(forward["verification"]["status"], "observed");
    assert_eq!(reload["plan"]["action"], "reload");
    assert_eq!(reload["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_asserts_page_conditions_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-assertions");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
history.replaceState(null, '', 'about:blank#done');
document.body.innerHTML = `
  <main>
    <h1>Success message</h1>
    <div id="spinner" hidden>Loading</div>
    <label>Email <input id="email" value="alice@example.com"></label>
    <label>Archived email <input id="archived-email" readonly value="archived@example.com"></label>
    <fieldset disabled>
      <label>Locked code <input id="locked-code" value="ZX-9"></label>
    </fieldset>
    <label><input id="newsletter" type="checkbox" checked> Newsletter checkbox</label>
    <label><input id="beta" type="checkbox"> Beta access checkbox</label>
    <button id="alerts" role="switch" aria-checked="true">Email alerts switch</button>
  </main>
`;
"##
        .to_string(),
    ]);

    let visible = act_instruction(&env, "Verify that Success message is visible.");
    let hidden = act_instruction(&env, "Check that #spinner is hidden.");
    let url = act_instruction(&env, "Expect URL contains #done.");
    let no_console = act_instruction(&env, "Ensure no console errors.");
    let value = act_instruction(&env, "Verify that Email value equals alice@example.com.");
    let readonly_value = act_instruction(
        &env,
        "Verify that Archived email value equals archived@example.com.",
    );
    let disabled_value = act_instruction(&env, "Verify that Locked code value equals ZX-9.");
    let checked = act_instruction(&env, "Ensure Newsletter checkbox is checked.");
    let unchecked = act_instruction(&env, "Expect Beta access checkbox is not checked.");
    let aria_checked = act_instruction(&env, "Verify that Email alerts switch is checked.");

    assert_eq!(visible["analysis"]["kind"], "assert");
    assert_eq!(visible["plan"]["action"], "assert");
    assert_eq!(
        visible["plan"]["params"]["checks"][0]["kind"],
        "text_visible"
    );
    assert_eq!(visible["verification"]["status"], "observed");
    assert_eq!(
        hidden["plan"]["params"]["checks"][0]["kind"],
        "selector_hidden"
    );
    assert_eq!(
        hidden["plan"]["params"]["checks"][0]["selector"],
        "#spinner"
    );
    assert_eq!(hidden["verification"]["status"], "observed");
    assert_eq!(url["plan"]["params"]["checks"][0]["kind"], "url_contains");
    assert_eq!(url["plan"]["params"]["checks"][0]["text"], "#done");
    assert_eq!(url["verification"]["status"], "observed");
    assert_eq!(
        no_console["plan"]["params"]["checks"][0]["kind"],
        "no_console_errors"
    );
    assert_eq!(no_console["verification"]["status"], "observed");
    assert_eq!(value["plan"]["params"]["checks"][0]["kind"], "value_equals");
    assert_eq!(value["plan"]["params"]["checks"][0]["selector"], "#email");
    assert_eq!(
        value["plan"]["params"]["checks"][0]["value"],
        "alice@example.com"
    );
    assert_eq!(value["verification"]["status"], "observed");
    assert_eq!(
        readonly_value["plan"]["params"]["checks"][0]["kind"],
        "value_equals"
    );
    assert_eq!(
        readonly_value["plan"]["params"]["checks"][0]["selector"],
        "#archived-email"
    );
    assert_eq!(
        readonly_value["plan"]["params"]["checks"][0]["value"],
        "archived@example.com"
    );
    assert_eq!(readonly_value["verification"]["status"], "observed");
    assert_eq!(
        disabled_value["plan"]["params"]["checks"][0]["kind"],
        "value_equals"
    );
    assert_eq!(
        disabled_value["plan"]["params"]["checks"][0]["selector"],
        "#locked-code"
    );
    assert_eq!(
        disabled_value["plan"]["params"]["checks"][0]["value"],
        "ZX-9"
    );
    assert_eq!(disabled_value["verification"]["status"], "observed");
    assert_eq!(checked["plan"]["params"]["checks"][0]["kind"], "checked");
    assert_eq!(
        checked["plan"]["params"]["checks"][0]["selector"],
        "#newsletter"
    );
    assert_eq!(checked["plan"]["params"]["checks"][0]["checked"], true);
    assert_eq!(checked["verification"]["status"], "observed");
    assert_eq!(unchecked["plan"]["params"]["checks"][0]["kind"], "checked");
    assert_eq!(
        unchecked["plan"]["params"]["checks"][0]["selector"],
        "#beta"
    );
    assert_eq!(unchecked["plan"]["params"]["checks"][0]["checked"], false);
    assert_eq!(unchecked["verification"]["status"], "observed");
    assert_eq!(
        aria_checked["plan"]["params"]["checks"][0]["selector"],
        "#alerts"
    );
    assert_eq!(aria_checked["plan"]["params"]["checks"][0]["checked"], true);
    assert_eq!(aria_checked["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_captures_screenshots_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-screenshots");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <style>
    body { margin: 0; min-height: 1400px; font-family: sans-serif; }
    #profile { margin: 32px; width: 240px; height: 120px; background: #2f6fed; color: white; padding: 16px; }
    #panel { margin: 32px; width: 180px; height: 80px; background: #f6c453; }
  </style>
  <section id="profile" class="card" aria-label="Profile card">
    <h1>Profile card</h1>
    <p>Alice Example</p>
  </section>
  <section id="panel">Status panel</section>
`;
"##
        .to_string(),
    ]);

    let page = act_instruction(&env, "Take a full page screenshot.");
    let selector = act_instruction(&env, "Capture a screenshot of #panel.");
    let semantic = act_instruction(&env, "Capture a screenshot of the Profile card.");

    assert_eq!(page["analysis"]["kind"], "screenshot");
    assert_eq!(page["plan"]["action"], "screenshot");
    assert_eq!(page["plan"]["params"]["full_page"], true);
    assert_eq!(page["result"]["scope"], "fullPage");
    assert!(page["result"]["byteLength"].as_u64().unwrap_or(0) > 0);
    assert_eq!(page["verification"]["status"], "observed");

    assert_eq!(selector["plan"]["action"], "screenshot");
    assert_eq!(selector["plan"]["params"]["selector"], "#panel");
    assert_eq!(selector["result"]["scope"], "element");
    assert_eq!(selector["result"]["mimeType"], "image/png");
    assert!(selector["result"]["byteLength"].as_u64().unwrap_or(0) > 0);
    assert_eq!(selector["verification"]["status"], "observed");

    assert_eq!(semantic["plan"]["action"], "screenshot");
    assert_eq!(semantic["plan"]["params"]["selector"], "#profile");
    assert_eq!(semantic["result"]["scope"], "element");
    assert!(semantic["result"]["byteLength"].as_u64().unwrap_or(0) > 0);
    assert_eq!(semantic["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_sets_viewport_and_device_emulation_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-viewport-device");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r#"document.head.innerHTML = '<meta name="viewport" content="width=device-width, initial-scale=1">'"#
            .to_string(),
    ]);

    let mobile = act_instruction(&env, "Set viewport to mobile.");
    let mobile_state: Value = serde_json::from_str(
        env.json(&[
            "eval".to_string(),
            "JSON.stringify({width: window.innerWidth, height: window.innerHeight})".to_string(),
        ])["result"]
            .as_str()
            .unwrap_or("{}"),
    )
    .expect("parse mobile viewport state");

    let custom = act_instruction(&env, "Resize browser to 390x844.");
    let custom_state: Value = serde_json::from_str(
        env.json(&[
            "eval".to_string(),
            "JSON.stringify({width: window.innerWidth, height: window.innerHeight})".to_string(),
        ])["result"]
            .as_str()
            .unwrap_or("{}"),
    )
    .expect("parse custom viewport state");

    let device = act_instruction(&env, "Emulate iPhone 15.");
    let device_state: Value = serde_json::from_str(
        env.json(&[
            "eval".to_string(),
            "JSON.stringify({width: window.innerWidth, height: window.innerHeight, touch: navigator.maxTouchPoints, ua: navigator.userAgent})".to_string(),
        ])["result"]
            .as_str()
            .unwrap_or("{}"),
    )
    .expect("parse device emulation state");

    assert_eq!(mobile["analysis"]["kind"], "set_viewport");
    assert_eq!(mobile["plan"]["action"], "set_viewport");
    assert_eq!(mobile["plan"]["params"]["preset"], "mobile");
    assert_eq!(mobile["result"]["width"], 375);
    assert_eq!(mobile["result"]["height"], 667);
    assert_eq!(mobile["verification"]["status"], "observed");
    assert_eq!(mobile_state["width"], 375);
    assert_eq!(mobile_state["height"], 667);

    assert_eq!(custom["analysis"]["kind"], "set_viewport");
    assert_eq!(custom["plan"]["action"], "set_viewport");
    assert_eq!(custom["plan"]["params"]["width"], 390);
    assert_eq!(custom["plan"]["params"]["height"], 844);
    assert_eq!(custom["verification"]["status"], "observed");
    assert_eq!(custom_state["width"], 390);
    assert_eq!(custom_state["height"], 844);

    assert_eq!(device["analysis"]["kind"], "emulate_device");
    assert_eq!(device["plan"]["action"], "emulate_device");
    assert_eq!(device["plan"]["params"]["device"], "iPhone 15");
    assert_eq!(device["result"]["device"], "iPhone 15");
    assert_eq!(device["result"]["width"], 393);
    assert_eq!(device["result"]["height"], 852);
    assert_eq!(device["result"]["mobile"], true);
    assert_eq!(device["verification"]["status"], "observed");
    assert_eq!(device_state["width"], 393);
    assert_eq!(device_state["height"], 852);
    assert_eq!(device_state["touch"], 5);
    assert!(device_state["ua"]
        .as_str()
        .unwrap_or_default()
        .contains("iPhone"));

    env.stop();
}

#[test]
fn act_instruction_reads_visible_text_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-read-text");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <main>
    <section id="profile" class="card" aria-label="Profile card">
      <h2>Profile card</h2>
      <p>Alice Example</p>
      <p>Plan: Pro account</p>
    </section>
    <aside id="status-panel" aria-label="Status panel">
      <strong>Status panel</strong>
      <span>Build ready</span>
    </aside>
    <fieldset disabled>
      <label>Reference code <input id="reference-code" value="ZX-902"></label>
    </fieldset>
    <section aria-hidden="true">Hidden leak should not be read</section>
  </main>
`;
"##
        .to_string(),
    ]);

    let page = act_instruction(&env, "Read the page.");
    let selector = act_instruction(&env, "Extract text from #status-panel.");
    let semantic = act_instruction(&env, "Read the Profile card.");

    assert_eq!(page["analysis"]["kind"], "read_text");
    assert_eq!(page["plan"]["action"], "read_text");
    assert!(page["result"]["text"]
        .as_str()
        .unwrap_or_default()
        .contains("Build ready"));
    assert!(page["result"]["text"]
        .as_str()
        .unwrap_or_default()
        .contains("ZX-902"));
    assert!(!page["result"]["text"]
        .as_str()
        .unwrap_or_default()
        .contains("Hidden leak"));
    assert_eq!(page["verification"]["status"], "observed");

    assert_eq!(selector["plan"]["action"], "read_text");
    assert_eq!(selector["plan"]["params"]["selector"], "#status-panel");
    assert_eq!(selector["result"]["selector"], "#status-panel");
    assert!(selector["result"]["text"]
        .as_str()
        .unwrap_or_default()
        .contains("Build ready"));
    assert_eq!(selector["verification"]["status"], "observed");

    assert_eq!(semantic["plan"]["action"], "read_text");
    assert_eq!(semantic["plan"]["params"]["selector"], "#profile");
    assert!(semantic["result"]["text"]
        .as_str()
        .unwrap_or_default()
        .contains("Alice Example"));
    assert_eq!(semantic["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_analyzes_forms_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-analyze-form");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="search" aria-label="Search form">
    <label for="q">Search</label>
    <input id="q" name="q">
    <button>Find</button>
  </form>
  <form id="shipping" aria-label="Shipping form">
    <h2>Shipping form</h2>
    <label for="full-name">Full name</label>
    <input id="full-name" name="full_name" required>
    <label for="state">State</label>
    <select id="state" name="state">
      <option value="ca">California</option>
      <option value="ny">New York</option>
    </select>
    <label><input type="checkbox" name="residential"> Residential address</label>
    <button type="submit">Continue</button>
  </form>
`;
"##
        .to_string(),
    ]);

    let semantic = act_instruction(&env, "Analyze the Shipping form.");
    let selector = act_instruction(&env, "Inspect form fields in #shipping.");

    assert_eq!(semantic["analysis"]["kind"], "analyze_form");
    assert_eq!(semantic["plan"]["action"], "analyze_form");
    assert_eq!(semantic["plan"]["params"]["selector"], "#shipping");
    assert_eq!(semantic["result"]["formSelector"], "#shipping");
    assert_eq!(semantic["result"]["fieldCount"], 3);
    assert_eq!(semantic["result"]["submitButtons"][0]["text"], "Continue");
    assert_eq!(semantic["verification"]["status"], "observed");

    let fields = semantic["result"]["fields"]
        .as_array()
        .expect("fields array");
    assert!(fields.iter().any(|field| field["label"] == "Full name"));
    assert!(fields.iter().any(|field| field["label"] == "State"));
    assert!(fields
        .iter()
        .any(|field| field["label"] == "Residential address"));

    assert_eq!(selector["plan"]["action"], "analyze_form");
    assert_eq!(selector["plan"]["params"]["selector"], "#shipping");
    assert_eq!(selector["result"]["formSelector"], "#shipping");
    assert_eq!(selector["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_analyzes_shadow_dom_forms_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-shadow-analyze-form");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="search" aria-label="Search form">
    <label for="q">Search</label>
    <input id="q" name="q">
    <button>Find</button>
  </form>
  <account-form id="account-form" role="form" aria-label="Account form"></account-form>
`;
customElements.define('account-form', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <h2>Account form</h2>
      <label for="display-name">Display name</label>
      <input id="display-name" name="display_name" required>
      <span id="plan-label">Plan tier</span>
      <select id="plan" name="plan" aria-labelledby="plan-label">
        <option value="starter">Starter</option>
        <option value="pro">Pro</option>
      </select>
      <label><input type="checkbox" name="alerts"> Email alerts</label>
      <button type="submit">Save</button>
    `;
  }
});
"##
        .to_string(),
    ]);

    let semantic = act_instruction(&env, "Analyze the Account form.");

    assert_eq!(semantic["analysis"]["kind"], "analyze_form");
    assert_eq!(semantic["plan"]["action"], "analyze_form");
    assert_eq!(semantic["plan"]["params"]["selector"], "#account-form");
    assert_eq!(semantic["result"]["formSelector"], "#account-form");
    assert_eq!(semantic["result"]["fieldCount"], 3);
    assert_eq!(semantic["result"]["submitButtons"][0]["text"], "Save");
    assert_eq!(semantic["verification"]["status"], "observed");

    let fields = semantic["result"]["fields"]
        .as_array()
        .expect("fields array");
    assert!(fields
        .iter()
        .any(|field| field["label"] == "Display name"
            && field["required"].as_bool().unwrap_or(false)));
    assert!(fields
        .iter()
        .any(|field| field["label"] == "Plan tier" && field["type"] == "select"));
    assert!(fields
        .iter()
        .any(|field| field["label"] == "Email alerts" && field["type"] == "checkbox"));

    env.stop();
}

#[test]
fn fill_form_fills_shadow_dom_forms_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("fill-form-shadow-form");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="decoy" aria-label="Search form">
    <label for="q">Search</label>
    <input id="q" name="q">
    <button type="submit">Find</button>
  </form>
  <account-form id="account-form" role="form" aria-label="Account form"></account-form>
  <output id="out"></output>
`;
customElements.define('account-form', class extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <h2>Account form</h2>
      <label for="display-name">Display name</label>
      <input id="display-name" name="display_name" required>
      <span id="plan-label">Plan tier</span>
      <select id="plan" name="plan" aria-labelledby="plan-label">
        <option value="starter">Starter</option>
        <option value="pro">Pro</option>
      </select>
      <label><input type="checkbox" name="alerts"> Email alerts</label>
      <button type="submit">Save</button>
    `;
    root.querySelector('button[type=submit]').addEventListener('click', () => {
      document.querySelector('#out').textContent = JSON.stringify({
        name: root.querySelector('#display-name').value,
        plan: root.querySelector('#plan').value,
        alerts: root.querySelector('input[name=alerts]').checked,
      });
    });
  }
});
"##
        .to_string(),
    ]);

    let filled = env.json(&[
        "fill-form".to_string(),
        "--selector".to_string(),
        "#account-form".to_string(),
        "--values".to_string(),
        r#"{"Display name":"Ada Lovelace","Plan tier":"Pro","Email alerts":true}"#.to_string(),
        "--submit".to_string(),
    ]);
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse submitted shadow form state");
    assert_eq!(state["name"], "Ada Lovelace");
    assert_eq!(state["plan"], "pro");
    assert_eq!(state["alerts"], true);
    assert_eq!(filled["fieldCount"], 3);
    assert_eq!(filled["filled"].as_array().unwrap().len(), 3);
    assert!(filled["errors"].as_array().unwrap().is_empty());
    assert!(filled["unresolved"].as_array().unwrap().is_empty());
    assert_eq!(filled["submitted"], true);

    env.stop();
}

#[test]
fn analyze_form_reports_custom_form_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("analyze-form-custom-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <text-box id="display-name" data-field="outside-name" style="display:block; min-height:20px; width:220px;">Outside</text-box>
  <form id="profile">
    <label for="display-name">Display name</label>
    <text-box id="display-name" data-field="display-name" style="display:block; min-height:20px; width:220px;"></text-box>
    <label for="updates">Product updates</label>
    <toggle-box id="updates" data-field="updates" style="display:block; min-height:20px; width:40px;"></toggle-box>
    <status-chip></status-chip>
    <button type="submit">Save</button>
  </form>
`;
if (!customElements.get('text-box')) {
  customElements.define('text-box', class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    get value() { return this._value; }
    set value(next) { this._value = String(next); this.textContent = this._value; }
  });
}
if (!customElements.get('toggle-box')) {
  customElements.define('toggle-box', class extends HTMLElement {
    constructor() {
      super();
      this._checked = false;
    }
    get checked() { return this._checked; }
    set checked(next) { this._checked = !!next; this.setAttribute('aria-checked', String(this._checked)); }
  });
}
if (!customElements.get('status-chip')) {
  customElements.define('status-chip', class extends HTMLElement {
    constructor() {
      super();
      this.value = 'decorative';
      this.textContent = this.value;
    }
  });
}
"##
        .to_string(),
    ]);

    let analyzed = env.json(&[
        "analyze-form".to_string(),
        "--selector".to_string(),
        "#profile".to_string(),
    ]);
    let fields = analyzed["fields"].as_array().expect("fields array");
    let display_name_selector = fields
        .iter()
        .find(|field| field["label"] == "Display name")
        .and_then(|field| field["selector"].as_str())
        .expect("display name selector")
        .to_string();

    assert_eq!(analyzed["fieldCount"], 2);
    assert!(fields.iter().any(|field| {
        field["label"] == "Display name"
            && field["type"] == "text"
            && field["selector"]
                .as_str()
                .unwrap_or_default()
                .starts_with("[data-gsd-browser-form-ref=")
    }));
    assert!(fields.iter().any(|field| {
        field["selector"] == "#updates"
            && field["label"] == "Product updates"
            && field["type"] == "checkbox"
            && field["checked"] == false
    }));

    env.json(&[
        "type".to_string(),
        display_name_selector,
        "Grace Hopper".to_string(),
    ]);
    let state = env.json(&[
        "eval".to_string(),
        "JSON.stringify({outside: document.querySelector('text-box[data-field=outside-name]').value, inside: document.querySelector('form text-box[data-field=display-name]').value})".to_string(),
    ]);
    let values: Value =
        serde_json::from_str(state["result"].as_str().unwrap_or("{}")).expect("parse values");
    assert_eq!(values["outside"], "");
    assert_eq!(values["inside"], "Grace Hopper");

    env.stop();
}

#[test]
fn fill_form_fills_custom_form_controls_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("fill-form-custom-controls");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="profile">
    <label for="display-name">Display name</label>
    <text-box id="display-name" data-field="display-name" style="display:block; min-height:20px; width:220px;"></text-box>
    <label for="updates">Product updates</label>
    <toggle-box id="updates" data-field="updates" style="display:block; min-height:20px; width:40px;"></toggle-box>
    <button type="submit">Save</button>
  </form>
  <output id="out"></output>
`;
if (!customElements.get('text-box')) {
  customElements.define('text-box', class extends HTMLElement {
    constructor() {
      super();
      this._value = '';
    }
    get value() { return this._value; }
    set value(next) { this._value = String(next); this.textContent = this._value; }
  });
}
if (!customElements.get('toggle-box')) {
  customElements.define('toggle-box', class extends HTMLElement {
    constructor() {
      super();
      this._checked = false;
      this.setAttribute('role', 'checkbox');
      this.tabIndex = 0;
    }
    get checked() { return this._checked; }
    set checked(next) { this._checked = !!next; this.setAttribute('aria-checked', String(this._checked)); }
  });
}
document.querySelector('#profile').addEventListener('submit', event => {
  event.preventDefault();
  out.textContent = JSON.stringify({
    name: document.querySelector('#display-name').value,
    updates: document.querySelector('#updates').checked
  });
});
"##
        .to_string(),
    ]);

    let filled = env.json(&[
        "fill-form".to_string(),
        "--selector".to_string(),
        "#profile".to_string(),
        "--values".to_string(),
        r#"{"Display name":"Ada Lovelace","Product updates":true}"#.to_string(),
        "--submit".to_string(),
    ]);
    let result = env.json(&[
        "eval".to_string(),
        "document.querySelector('#out').textContent".to_string(),
    ]);

    let state: Value = serde_json::from_str(result["result"].as_str().unwrap_or("{}"))
        .expect("parse custom form state");
    assert_eq!(state["name"], "Ada Lovelace");
    assert_eq!(state["updates"], true);
    assert_eq!(filled["fieldCount"], 2);
    assert_eq!(filled["filled"].as_array().unwrap().len(), 2);
    assert!(filled["errors"].as_array().unwrap().is_empty());
    assert!(filled["unresolved"].as_array().unwrap().is_empty());
    assert_eq!(filled["submitted"], true);

    env.stop();
}

#[test]
fn act_instruction_reads_accessibility_trees_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-accessibility-tree");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <nav id="nav" aria-label="Primary navigation">
    <a href="#home">Home</a>
    <button aria-label="Open settings">Settings</button>
  </nav>
  <section id="profile" class="card" aria-label="Profile card">
    <h2>Profile card</h2>
    <label for="nickname">Nickname</label>
    <input id="nickname" value="Ada">
    <button>Save profile</button>
  </section>
`;
"##
        .to_string(),
    ]);

    let page = act_instruction(&env, "Show accessibility tree.");
    let selector = act_instruction(&env, "Show accessibility tree for #nav.");
    let semantic = act_instruction(&env, "List roles in the Profile card.");

    assert_eq!(page["analysis"]["kind"], "accessibility_tree");
    assert_eq!(page["plan"]["action"], "accessibility_tree");
    assert!(page["result"]["tree"]
        .as_str()
        .unwrap_or_default()
        .contains("Primary navigation"));
    assert_eq!(page["verification"]["status"], "observed");

    assert_eq!(selector["plan"]["action"], "accessibility_tree");
    assert_eq!(selector["plan"]["params"]["selector"], "#nav");
    assert!(selector["result"]["tree"]
        .as_str()
        .unwrap_or_default()
        .contains("Open settings"));
    assert_eq!(selector["verification"]["status"], "observed");

    assert_eq!(semantic["plan"]["action"], "accessibility_tree");
    assert_eq!(semantic["plan"]["params"]["selector"], "#profile");
    assert!(semantic["result"]["tree"]
        .as_str()
        .unwrap_or_default()
        .contains("Save profile"));
    assert_eq!(semantic["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_finds_elements_without_acting_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-find-elements");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
window.clicked = 0;
document.body.innerHTML = `
  <section id="profile" aria-label="Profile card">
    <h2>Account status</h2>
    <button id="save" onclick="window.clicked += 1">Save profile</button>
    <a href="#billing">Billing link</a>
  </section>
`;
"##
        .to_string(),
    ]);

    let by_role = act_instruction(&env, "Find buttons named Save profile.");
    let by_selector = act_instruction(&env, "Locate #profile.");
    let by_text = act_instruction(&env, "Search for Account status.");
    let clicked = env.json(&["eval".to_string(), "String(window.clicked)".to_string()]);

    assert_eq!(by_role["analysis"]["kind"], "find_elements");
    assert_eq!(by_role["plan"]["action"], "find");
    assert_eq!(by_role["plan"]["params"]["role"], "button");
    assert_eq!(by_role["plan"]["params"]["text"], "Save profile");
    assert_eq!(by_role["result"]["count"], 1);
    assert_eq!(by_role["result"]["elements"][0]["name"], "Save profile");
    assert_eq!(by_role["verification"]["status"], "observed");

    assert_eq!(by_selector["plan"]["action"], "find");
    assert_eq!(by_selector["plan"]["params"]["selector"], "#profile");
    assert_eq!(by_selector["result"]["count"], 1);
    assert_eq!(
        by_selector["result"]["elements"][0]["selector_hint"],
        "#profile"
    );
    assert_eq!(by_selector["verification"]["status"], "observed");

    assert_eq!(by_text["plan"]["action"], "find");
    assert_eq!(by_text["plan"]["params"]["text"], "Account status");
    assert!(by_text["result"]["count"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(by_text["verification"]["status"], "observed");
    assert_eq!(clicked["result"], "0");

    env.stop();
}

#[test]
fn act_instruction_searches_pointer_feedback_surfaces_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-pointer-feedback-surface");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
window.feedbackClicks = [];
document.body.innerHTML = `
  <p id="task">Use the feedback display to find the requested area.</p>
  <div id="target-area" class="feedback-area" style="width: 160px; height: 120px; border: 1px solid #444; cursor: crosshair;"></div>
  <output id="status" role="status">ICE COLD</output>
  <output id="clicked"></output>
`;
const surface = document.querySelector('#target-area');
const status = document.querySelector('#status');
const clicked = document.querySelector('#clicked');
const hidden = { x: 93, y: 47 };
function feedback(event) {
  const rect = surface.getBoundingClientRect();
  const dx = event.clientX - rect.left - hidden.x;
  const dy = event.clientY - rect.top - hidden.y;
  const distance = Math.hypot(dx, dy);
  status.textContent = distance <= 5 ? 'HOT' : distance <= 12 ? 'WARM' : distance <= 42 ? 'COLD' : 'ICE COLD';
}
surface.addEventListener('mousemove', feedback);
surface.addEventListener('click', event => {
  feedback(event);
  window.feedbackClicks.push(status.textContent);
  clicked.textContent = status.textContent === 'HOT' ? 'clicked:hot' : 'clicked:' + status.textContent.toLowerCase();
});
"##
        .to_string(),
    ]);

    let result = act_instruction(&env, "Find and click on the HOT area.");
    let state = env.json(&[
        "eval".to_string(),
        "({clicked: document.querySelector('#clicked').textContent, status: document.querySelector('#status').textContent, clicks: window.feedbackClicks})".to_string(),
    ]);
    let state: Value = serde_json::from_str(state["result"].as_str().unwrap_or("{}"))
        .expect("parse feedback state");

    assert_eq!(
        result["plan"]["capability"]["name"],
        "visual-feedback-search"
    );
    assert_eq!(result["plan"]["action"], "visual_feedback_search");
    assert_eq!(result["result"]["visualFeedbackSearch"]["feedback"], "hot");
    assert_eq!(state["clicked"], "clicked:hot");
    assert_eq!(state["status"], "HOT");
    assert_eq!(state["clicks"].as_array().unwrap().len(), 1);
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_preserves_executable_alternate_plans_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-executable-alternate-plans");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <p>Use feedback to find the requested area, or use the visible action if the area is unavailable.</p>
  <div id="target-area" class="feedback-area" style="width: 160px; height: 120px; border: 1px solid #444;"></div>
  <output role="status">ICE COLD</output>
  <button id="details">Details</button>
`;
"##
        .to_string(),
    ]);

    let plan = act_instruction_dry_run(&env, "Find and click on the HOT area.");
    let alternates = plan["plan"]["alternates"]
        .as_array()
        .expect("alternate plan summaries");
    let alternate_plans = plan["plan"]["alternatePlans"]
        .as_array()
        .expect("executable alternate plans");

    assert!(!alternates.is_empty());
    assert_eq!(alternate_plans.len(), alternates.len());
    assert!(alternate_plans.iter().all(|alternate| {
        alternate
            .get("action")
            .and_then(|value| value.as_str())
            .is_some()
            && alternate.get("params").is_some()
            && alternate.get("capability").is_some()
            && alternate.get("alternatePlans").is_none()
    }));
    assert_eq!(plan["dryRun"], true);

    env.stop();
}

#[test]
fn act_instruction_expands_tree_to_click_named_items_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-hierarchical-tree-search");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
window.treeClicks = [];
document.body.innerHTML = `
  <ul id="resource-tree" role="tree" aria-label="Resource tree">
    <li role="treeitem" aria-expanded="false">
      <button class="hitarea" type="button" aria-label="Expand Reports">+</button>
      <span class="folder">Reports</span>
      <ul role="group" hidden>
        <li role="treeitem"><span class="file">Summary</span></li>
        <li role="treeitem"><span class="file">Ledger</span></li>
      </ul>
    </li>
    <li role="treeitem" aria-expanded="false">
      <button class="hitarea" type="button" aria-label="Expand Archive">+</button>
      <span class="folder">Archive</span>
      <ul role="group" hidden>
        <li role="treeitem"><span class="file">Northstar</span></li>
      </ul>
    </li>
  </ul>
  <output id="selected"></output>
`;
for (const row of document.querySelectorAll('[role=treeitem]')) {
  row.addEventListener('click', event => {
    const label = row.querySelector(':scope > span')?.textContent?.trim();
    if (event.target.matches('.hitarea')) {
      const group = row.querySelector(':scope > [role=group]');
      row.setAttribute('aria-expanded', 'true');
      if (group) group.hidden = false;
      event.stopPropagation();
      return;
    }
    window.treeClicks.push(label);
    selected.textContent = label;
    event.stopPropagation();
  });
}
"##
        .to_string(),
    ]);

    let result = act_instruction(
        &env,
        "Navigate through the resource tree and click the folder or file named \"Northstar\".",
    );
    let state = env.json(&[
        "eval".to_string(),
        "({selected: document.querySelector('#selected').textContent, clicks: window.treeClicks, archiveExpanded: document.querySelectorAll('[aria-expanded=true]').length})".to_string(),
    ]);
    let state: Value =
        serde_json::from_str(state["result"].as_str().unwrap_or("{}")).expect("parse tree state");

    assert_eq!(
        result["plan"]["capability"]["name"],
        "hierarchical-tree-search"
    );
    assert_eq!(result["plan"]["action"], "tree_search_click");
    assert_eq!(result["result"]["treeSearchClick"]["target"], "Northstar");
    assert_eq!(state["selected"], "Northstar");
    assert_eq!(state["clicks"].as_array().unwrap().len(), 1);
    assert!(state["archiveExpanded"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_selects_requested_item_quantities_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-item-quantity-selection");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="menu">
    <div class="menu-item" data-item="Roasted carrots">
      <span class="item-name">Roasted carrots</span>
      <button class="add" type="button">+</button>
      <output class="quantity"></output>
    </div>
    <div class="menu-item" data-item="Miso soup">
      <span class="item-name">Miso soup</span>
      <button class="add" type="button">+</button>
      <output class="quantity"></output>
    </div>
    <div class="menu-item" data-item="Berry tart">
      <span class="item-name">Berry tart</span>
      <button class="add" type="button">+</button>
      <output class="quantity"></output>
    </div>
  </section>
  <button id="order">Order</button>
  <output id="receipt"></output>
`;
for (const row of document.querySelectorAll('.menu-item')) {
  row.dataset.quantity = '0';
  row.querySelector('.add').addEventListener('click', () => {
    row.dataset.quantity = String(Number(row.dataset.quantity) + 1);
    row.querySelector('.quantity').textContent = row.dataset.quantity;
  });
}
order.addEventListener('click', () => {
  receipt.textContent = Array.from(document.querySelectorAll('.menu-item'))
    .filter(row => row.dataset.quantity !== '0')
    .map(row => row.dataset.item + '=' + row.dataset.quantity)
    .join(';');
});
"##
        .to_string(),
    ]);

    let result = act_instruction(&env, "Order one of each item: Miso soup, Berry tart.");
    let state = env.json(&[
        "eval".to_string(),
        "document.querySelector('#receipt').textContent".to_string(),
    ]);

    assert_eq!(
        result["plan"]["capability"]["name"],
        "item-quantity-selection"
    );
    assert_eq!(result["plan"]["action"], "sequence");
    assert_eq!(state["result"], "Miso soup=1;Berry tart=1");
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_classifies_rows_with_binary_options_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-binary-row-classification");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <section id="records">
    <div class="record-row">
      <span class="record-value">7</span>
      <button type="button" class="odd">Odd</button>
      <button type="button" class="even">Even</button>
    </div>
    <div class="record-row">
      <span class="record-value">-2</span>
      <button type="button" class="odd">Odd</button>
      <button type="button" class="even">Even</button>
    </div>
    <div class="record-row">
      <span class="record-value">10</span>
      <button type="button" class="odd">Odd</button>
      <button type="button" class="even">Even</button>
    </div>
  </section>
  <button id="submit">Submit</button>
  <output id="result"></output>
`;
for (const row of document.querySelectorAll('.record-row')) {
  for (const button of row.querySelectorAll('button')) {
    button.addEventListener('click', () => {
      row.dataset.selected = button.textContent;
      for (const sibling of row.querySelectorAll('button')) sibling.classList.remove('selected');
      button.classList.add('selected');
    });
  }
}
submit.addEventListener('click', () => {
  result.textContent = Array.from(document.querySelectorAll('.record-row'))
    .map(row => row.querySelector('.record-value').textContent + '=' + row.dataset.selected)
    .join(';');
});
"##
        .to_string(),
    ]);

    let result = act_instruction(
        &env,
        "Mark the numbers below as odd or even and press submit.",
    );
    let state = env.json(&[
        "eval".to_string(),
        "document.querySelector('#result').textContent".to_string(),
    ]);

    assert_eq!(
        result["plan"]["capability"]["name"],
        "binary-row-classification"
    );
    assert_eq!(result["plan"]["action"], "sequence");
    assert_eq!(state["result"], "7=Odd;-2=Even;10=Even");
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_fills_labeled_form_from_table_values_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-table-to-form-fill");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <table id="profile-data">
    <tr><td>Department</td><td>Engineering</td></tr>
    <tr><td>Region</td><td>West</td></tr>
    <tr><td>Manager</td><td>Ada</td></tr>
  </table>
  <form id="profile-form">
    <div><label id="region-label" for="region">Region:</label><input id="region"></div>
    <div><label id="department-label" for="department">Department:</label><input id="department"></div>
    <button id="submit" type="submit">Submit</button>
  </form>
  <output id="submitted"></output>
`;
document.querySelector('#profile-form').addEventListener('submit', event => {
  event.preventDefault();
  submitted.textContent = [
    'Region=' + document.querySelector('#region').value,
    'Department=' + document.querySelector('#department').value,
  ].join(';');
});
"##
        .to_string(),
    ]);

    let result = act_instruction(
        &env,
        "Enter the value that corresponds with each label into the form and submit when done.",
    );
    let state = env.json(&[
        "eval".to_string(),
        "document.querySelector('#submitted').textContent".to_string(),
    ]);

    assert_eq!(result["plan"]["capability"]["name"], "table-to-form-fill");
    assert_eq!(result["plan"]["action"], "sequence");
    assert_eq!(state["result"], "Region=West;Department=Engineering");
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_finds_paginated_record_and_clicks_property_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-record-property-pagination");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
window.clickedProperty = '';
document.body.innerHTML = `
  <section id="directory">
    <div id="record" class="contact-card"></div>
    <button id="next" type="button">Next</button>
  </section>
  <output id="selected"></output>
`;
const records = [
  { name: 'Grace', phone: '111-111-1111', email: 'grace@example.test' },
  { name: 'Ada', phone: '222-222-2222', email: 'ada@example.test' },
];
let index = 0;
function render() {
  const record = records[index];
  document.querySelector('#record').innerHTML = `
    <h2 class="name">${record.name}</h2>
    <div class="property"><span class="property-name">Phone:</span> <a class="phone">${record.phone}</a></div>
    <div class="property"><span class="property-name">Email:</span> <a class="email">${record.email}</a></div>
  `;
  for (const link of document.querySelectorAll('#record a')) {
    link.addEventListener('click', () => {
      window.clickedProperty = link.className + ':' + link.textContent;
      selected.textContent = window.clickedProperty;
    });
  }
}
next.addEventListener('click', () => {
  index = Math.min(records.length - 1, index + 1);
  render();
});
render();
"##
        .to_string(),
    ]);

    let result = act_instruction(
        &env,
        "Find Ada in the directory and click on their phone number.",
    );
    let state = env.json(&[
        "eval".to_string(),
        "document.querySelector('#selected').textContent".to_string(),
    ]);

    assert_eq!(
        result["plan"]["capability"]["name"],
        "record-property-lookup"
    );
    assert_eq!(result["plan"]["action"], "record_property_click");
    assert_eq!(
        state["result"], "phone:222-222-2222",
        "result payload: {result}"
    );
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}

#[test]
fn act_instruction_clicks_ordinal_result_after_form_submission_generically() {
    let _guard = browser_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let env = BrowserTestEnv::new("instruction-form-ordinal-result-pagination");

    env.json(&["navigate".to_string(), "about:blank".to_string()]);
    env.json(&[
        "eval".to_string(),
        r##"
document.body.innerHTML = `
  <form id="lookup">
    <label>Search term <input id="search-text" type="text" aria-label="Search term"></label>
    <button id="search" type="submit">Search</button>
  </form>
  <main id="page-content"></main>
  <nav id="pagination" aria-label="Pagination"></nav>
  <output id="selected"></output>
`;
const results = [
  'Ada overview',
  'Ada profile',
  'Ada notes',
  'Ada target result',
  'Ada archive',
  'Ada references',
];
let page = 0;
function renderResults() {
  const start = page * 3;
  const resultContainer = document.querySelector('#page-content');
  const paginationContainer = document.querySelector('#pagination');
  resultContainer.innerHTML = results.slice(start, start + 3).map((title, offset) => {
    const index = start + offset;
    return `<article class="result-card"><a href="#" class="result-title" data-result="${index}">${title}</a></article>`;
  }).join('');
  paginationContainer.innerHTML = page < 1 ? `<a href="#" class="next" aria-label="Next page">&gt;</a>` : '';
  for (const link of resultContainer.querySelectorAll('a[data-result]')) {
    link.addEventListener('click', event => {
      event.preventDefault();
      selected.textContent = link.dataset.result + ':' + link.textContent;
    });
  }
  const next = paginationContainer.querySelector('.next');
  if (next) {
    next.addEventListener('click', event => {
      event.preventDefault();
      page += 1;
      renderResults();
    });
  }
}
lookup.addEventListener('submit', event => {
  event.preventDefault();
  page = 0;
  renderResults();
});
"##
        .to_string(),
    ]);

    let result = act_instruction(
        &env,
        "Use the textbox to enter \"Ada\" and press Search, then find and click the 4th search result.",
    );
    let state = env.json(&[
        "eval".to_string(),
        "document.querySelector('#selected').textContent".to_string(),
    ]);

    assert_eq!(result["plan"]["capability"]["name"], "form-result-workflow");
    assert_eq!(result["plan"]["action"], "form_workflow");
    assert_eq!(state["result"], "3:Ada target result");
    assert_eq!(result["verification"]["status"], "observed");

    env.stop();
}
