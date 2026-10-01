//! Playground API tests: a real HTTP server with the offline mock chain.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sandglass_core::heir::{generate_phrase, identity_from_phrase};
use sandglass_keeper::playground::chain::MockChain;
use sandglass_keeper::playground::{Playground, PlaygroundConfig};
use sandglass_keeper::{spawn_background, AppState, Config};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const JUDGE: &str = "api-test-judge-code";

struct Rig {
    base: String,
    state: Arc<AppState>,
    chain: Arc<MockChain>,
    rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn config() -> PlaygroundConfig {
    PlaygroundConfig {
        network: "test",
        amount_zat: 20_000,
        max_active: 10,
        max_per_day: 60,
        max_keeps_per_day: 5,
        max_flips: 3,
        judge_hash: Sha256::digest(JUDGE.as_bytes()).to_vec(),
        public_origin: "https://sandglass.example".into(),
        explorer_tx_url: Some("https://explorer.example/tx/{txid}".into()),
        allowed_minutes: vec![3, 5, 10],
    }
}

fn start(enabled: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let (addr, state) = spawn_background(&dir.path().join("k.sqlite"), Config::default()).unwrap();
    let chain = Arc::new(MockChain::new());
    if enabled {
        let pg = Playground::new(
            config(),
            chain.clone(),
            &dir.path().join("p.sqlite"),
            state.db_arc(),
        )
        .unwrap();
        state.set_playground(pg);
    }
    Rig {
        base: format!("http://{addr}"),
        state,
        chain,
        rt: tokio::runtime::Runtime::new().unwrap(),
        _dir: dir,
    }
}

impl Rig {
    fn tick(&self, at: u64) {
        let pg = self.state.playground().unwrap().clone();
        self.rt.block_on(pg.tick(at));
    }
}

fn call(method: &str, url: &str, token: Option<&str>, body: Option<&str>) -> (u16, Value) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut req = ureq::http::Request::builder().method(method).uri(url);
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let req = req.body(body.unwrap_or("").to_string()).unwrap();
    let mut resp = agent.run(req).unwrap();
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap();
    let value = serde_json::from_str(&text).unwrap_or(Value::Null);
    (status, value)
}

fn heir() -> String {
    identity_from_phrase(&generate_phrase())
        .unwrap()
        .code()
        .to_string()
}

fn create_body(judge: &str, heir: &str, minutes: u32) -> String {
    json!({ "judge_code": judge, "heir_code": heir, "minutes": minutes }).to_string()
}

fn expect_error(v: &Value, code: &str) {
    assert_eq!(v["error"], code, "{v}");
    assert!(v["message"].as_str().is_some_and(|m| !m.is_empty()));
}

fn new_session(r: &Rig) -> (String, String, Value) {
    let (status, v) = call(
        "POST",
        &format!("{}/v1/playground/sessions", r.base),
        None,
        Some(&create_body(JUDGE, &heir(), 3)),
    );
    assert_eq!(status, 201, "{v}");
    (
        v["id"].as_str().unwrap().to_string(),
        v["token"].as_str().unwrap().to_string(),
        v,
    )
}

fn get(r: &Rig, id: &str, token: &str) -> Value {
    let (status, v) = call(
        "GET",
        &format!("{}/v1/playground/sessions/{id}", r.base),
        Some(token),
        None,
    );
    assert_eq!(status, 200, "{v}");
    v
}

#[test]
fn disabled_playground_is_closed() {
    let r = start(false);
    let (status, v) = call(
        "GET",
        &format!("{}/v1/playground/status", r.base),
        None,
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(v["enabled"], false);
    let (status, v) = call(
        "POST",
        &format!("{}/v1/playground/sessions", r.base),
        None,
        Some(&create_body(JUDGE, &heir(), 3)),
    );
    assert_eq!(status, 503);
    expect_error(&v, "playground_closed");
    let (status, v) = call(
        "GET",
        &format!("{}/v1/playground/sessions/{}", r.base, "a".repeat(32)),
        Some("x"),
        None,
    );
    assert_eq!(status, 503);
    expect_error(&v, "playground_closed");
}

#[test]
fn status_reports_caps_and_counts() {
    let r = start(true);
    let (_, _, _) = new_session(&r);
    let (status, v) = call(
        "GET",
        &format!("{}/v1/playground/status", r.base),
        None,
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(v["enabled"], true);
    assert_eq!(v["network"], "test");
    assert_eq!(v["amount_zat"], 20_000);
    assert_eq!(v["active"], 1);
    assert_eq!(v["max_active"], 10);
    assert_eq!(v["today"], 1);
    assert_eq!(v["max_per_day"], 60);
    assert_eq!(v["float_ready"], true);
}

#[test]
fn creation_checks_the_judge_code_and_the_input() {
    let r = start(true);
    let url = format!("{}/v1/playground/sessions", r.base);
    let (s, v) = call("POST", &url, None, Some(&create_body("wrong", &heir(), 3)));
    assert_eq!(s, 403);
    expect_error(&v, "judge_code_invalid");
    let (s, v) = call("POST", &url, None, Some(&create_body(JUDGE, "age1nope", 3)));
    assert_eq!(s, 400);
    expect_error(&v, "heir_code_invalid");
    let (s, v) = call("POST", &url, None, Some(&create_body(JUDGE, &heir(), 4)));
    assert_eq!(s, 400);
    expect_error(&v, "invalid_request");
    let (s, v) = call("POST", &url, None, Some("{not json"));
    assert_eq!(s, 400);
    expect_error(&v, "invalid_request");
    let (s, v) = call("POST", &url, None, Some(&"x".repeat(10_000)));
    assert_eq!(s, 400);
    expect_error(&v, "invalid_request");
    // Nothing was created by the failures.
    let (_, status) = call(
        "GET",
        &format!("{}/v1/playground/status", r.base),
        None,
        None,
    );
    assert_eq!(status["active"], 0);
}

#[test]
fn capped_creation_has_clear_errors() {
    let r = start(true);
    r.chain.set_float_ready(false);
    let url = format!("{}/v1/playground/sessions", r.base);
    let (s, v) = call("POST", &url, None, Some(&create_body(JUDGE, &heir(), 3)));
    assert_eq!(s, 503);
    expect_error(&v, "playground_busy");
}

#[test]
fn sessions_need_their_own_token() {
    let r = start(true);
    let (id, token, _) = new_session(&r);
    let (other_id, other_token, _) = new_session(&r);
    let url = format!("{}/v1/playground/sessions/{id}", r.base);

    let (s, v) = call("GET", &url, None, None);
    assert_eq!(s, 401);
    expect_error(&v, "unauthorized");
    let (s, v) = call("GET", &url, Some("wrong"), None);
    assert_eq!(s, 401);
    expect_error(&v, "unauthorized");
    let (s, v) = call("GET", &url, Some(&other_token), None);
    assert_eq!(s, 401);
    expect_error(&v, "unauthorized");
    let (s, _) = call("GET", &url, Some(&token), None);
    assert_eq!(s, 200);

    for path in ["flip", "settle"] {
        let (s, v) = call(
            "POST",
            &format!("{url}/{path}"),
            Some(&other_token),
            Some("{\"action\":\"return\"}"),
        );
        assert_eq!(s, 401, "{path}");
        expect_error(&v, "unauthorized");
    }
    let _ = other_id;

    let (s, v) = call(
        "GET",
        &format!("{}/v1/playground/sessions/{}", r.base, "0".repeat(32)),
        Some(&token),
        None,
    );
    assert_eq!(s, 404);
    expect_error(&v, "session_not_found");
    let (s, v) = call(
        "GET",
        &format!("{}/v1/playground/sessions/not-an-id", r.base),
        Some(&token),
        None,
    );
    assert_eq!(s, 404);
    expect_error(&v, "session_not_found");
}

#[test]
fn full_session_over_http() {
    let r = start(true);
    let t = now();
    let (id, token, created) = new_session(&r);
    assert_eq!(created["network"], "test");
    assert_eq!(created["amount_zat"], 20_000);
    assert_eq!(created["minutes"], 3);
    let link = created["claim_link"].as_str().unwrap();
    assert!(link.starts_with("https://sandglass.example/claim.html#m="));
    assert!(link.contains("&r="));

    let v = get(&r, &id, &token);
    assert_eq!(v["state"], "funding");
    assert_eq!(v["epoch"], 1);
    assert_eq!(v["confirmed"], false);
    assert_eq!(v["error"], Value::Null);
    assert!(v["unlock_round"].as_u64().unwrap() > 0);
    assert!(v["unlock_time"].as_str().unwrap().ends_with('Z'));
    assert_eq!(v["claim_link"], created["claim_link"]);

    // Too early for flip and settle.
    let (s, e) = call(
        "POST",
        &format!("{}/v1/playground/sessions/{id}/flip", r.base),
        Some(&token),
        None,
    );
    assert_eq!(s, 409);
    expect_error(&e, "invalid_state");
    let (s, e) = call(
        "POST",
        &format!("{}/v1/playground/sessions/{id}/settle", r.base),
        Some(&token),
        Some("{\"action\":\"return\"}"),
    );
    assert_eq!(s, 409);
    expect_error(&e, "invalid_state");

    // The kit is in the mailbox and readable with the claim link's token.
    let fragment = link.split_once('#').unwrap().1;
    let m = fragment
        .split('&')
        .find_map(|p| p.strip_prefix("m="))
        .unwrap();
    let rt = fragment
        .split('&')
        .find_map(|p| p.strip_prefix("r="))
        .unwrap();
    let kit = ureq::get(format!("{}/v1/mailboxes/{m}/kit", r.base))
        .header("Authorization", format!("Bearer {rt}"))
        .call()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    let kit: Value = serde_json::from_str(&kit).unwrap();
    assert_eq!(kit["format"], "sandglass-kit");
    assert_eq!(kit["epoch"], 1);
    assert!(kit.to_string().find("mock seed words").is_none());

    r.tick(t);
    r.tick(t + 11);
    let v = get(&r, &id, &token);
    assert_eq!(v["state"], "sealed");
    assert_eq!(v["confirmed"], true);
    assert_eq!(v["balance_zat"], 60_000);
    let txs = v["txs"].as_array().unwrap();
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0]["kind"], "fund");
    let txid = txs[0]["txid"].as_str().unwrap();
    assert_eq!(
        txs[0]["explorer"],
        format!("https://explorer.example/tx/{txid}")
    );

    // Flip.
    let (s, v) = call(
        "POST",
        &format!("{}/v1/playground/sessions/{id}/flip", r.base),
        Some(&token),
        None,
    );
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["state"], "flipping");
    r.tick(t + 20);
    let v = get(&r, &id, &token);
    assert_eq!(v["state"], "sealed");
    assert_eq!(v["epoch"], 2);
    assert_eq!(v["txs"].as_array().unwrap().len(), 2);
    // The claim link stays the same; the kit behind it changed.
    assert_eq!(v["claim_link"], created["claim_link"]);
    let kit2 = ureq::get(format!("{}/v1/mailboxes/{m}/kit", r.base))
        .header("Authorization", format!("Bearer {rt}"))
        .call()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    let kit2: Value = serde_json::from_str(&kit2).unwrap();
    assert_eq!(kit2["epoch"], 2);

    // Unlock, bad settle input, then return.
    r.tick(t + 40);
    r.tick(t + 400);
    assert_eq!(get(&r, &id, &token)["state"], "unlocked");
    let settle = format!("{}/v1/playground/sessions/{id}/settle", r.base);
    let (s, e) = call("POST", &settle, Some(&token), Some("{\"action\":\"burn\"}"));
    assert_eq!(s, 400);
    expect_error(&e, "invalid_request");
    let (s, e) = call(
        "POST",
        &settle,
        Some(&token),
        Some("{\"action\":\"send\",\"to\":\"nope\"}"),
    );
    assert_eq!(s, 400);
    expect_error(&e, "network_mismatch");
    let (s, v) = call(
        "POST",
        &settle,
        Some(&token),
        Some("{\"action\":\"return\"}"),
    );
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["state"], "settling");
    r.tick(t + 410);
    let v = get(&r, &id, &token);
    assert_eq!(v["state"], "returned");
    let kinds: Vec<_> = v["txs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["kind"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(kinds, ["fund", "flip", "return"]);
    // Settling twice is refused.
    let (s, e) = call(
        "POST",
        &settle,
        Some(&token),
        Some("{\"action\":\"return\"}"),
    );
    assert_eq!(s, 409);
    expect_error(&e, "invalid_state");
}

#[test]
fn keep_cap_over_http() {
    let r = start(true);
    let t = now();
    // The test config allows 5 keeps; use the pg handle to lower nothing:
    // create six sessions and keep until the cap answers.
    let mut sessions = Vec::new();
    for _ in 0..6 {
        sessions.push(new_session(&r));
    }
    r.tick(t);
    r.tick(t + 11);
    r.tick(t + 400);
    let mut capped = 0;
    for (id, token, _) in &sessions {
        let (s, v) = call(
            "POST",
            &format!("{}/v1/playground/sessions/{id}/settle", r.base),
            Some(token),
            Some("{\"action\":\"keep\"}"),
        );
        if s == 429 {
            expect_error(&v, "daily_cap_reached");
            capped += 1;
        } else {
            assert_eq!(s, 200, "{v}");
            assert_eq!(v["state"], "kept");
        }
    }
    assert_eq!(capped, 1);
}
