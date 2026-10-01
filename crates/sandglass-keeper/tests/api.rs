//! Keeper API integration tests: a real server on a random port, a temp DB.

use sandglass_core::heir::{generate_phrase, identity_from_phrase};
use sandglass_core::kit::{BuildParams, Kind, Network, Payload};
use sandglass_core::mailbox::MailboxClient;
use sandglass_core::{drand, Kit};
use sandglass_keeper::{spawn_background, Config};

const SEED: &str = "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth title";

fn kit_json(epoch: u32) -> String {
    let heir = identity_from_phrase(&generate_phrase()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let payload = Payload {
        vault_seed: SEED.into(),
        birthday_height: 1234,
        network: Network::Test,
        plan: "00112233445566778899aabbccddeeff".into(),
        epoch,
        message: None,
        restore_hints: vec![],
    };
    Kit::build(
        &payload,
        &BuildParams {
            kind: Kind::Legacy,
            sequence: None,
            now,
            unlock_round: drand::unlock_round(now + 3600),
            heirs: &[heir.code()],
        },
    )
    .unwrap()
    .to_json()
    .unwrap()
}

fn start() -> (
    String,
    tempfile::TempDir,
    std::sync::Arc<sandglass_keeper::AppState>,
) {
    let dir = tempfile::tempdir().unwrap();
    let (addr, state) = spawn_background(&dir.path().join("k.sqlite"), Config::default()).unwrap();
    (format!("http://{addr}"), dir, state)
}

fn raw(
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> (u16, String, ureq::http::HeaderMap) {
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
    let headers = resp.headers().clone();
    (status, resp.body_mut().read_to_string().unwrap(), headers)
}

fn error_code(body: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    assert!(v["message"].as_str().is_some_and(|m| !m.is_empty()));
    v["error"].as_str().unwrap().to_string()
}

#[test]
fn health() {
    let (base, _d, _s) = start();
    let (status, body, _) = raw("GET", &format!("{base}/v1/health"), None, None);
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["ok"], true);
    assert!(v["version"].is_string());
}

#[test]
fn create_put_get_replace_delete() {
    let (base, _d, _s) = start();
    let c = MailboxClient::new(&base);
    let m = c.create().unwrap();
    assert_eq!(m.id.len(), 32);
    assert_eq!(m.write_token.len(), 64);
    assert_eq!(m.read_token.len(), 64);
    assert_ne!(m.write_token, m.read_token);

    // Nothing uploaded yet.
    assert_eq!(
        c.get(&m.id, &m.read_token).unwrap_err().code,
        "mailbox_not_found"
    );

    let k1 = kit_json(1);
    c.put(&m.id, &m.write_token, &k1).unwrap();
    assert_eq!(c.get(&m.id, &m.read_token).unwrap(), k1);

    let k2 = kit_json(2);
    c.put(&m.id, &m.write_token, &k2).unwrap();
    assert_eq!(c.get(&m.id, &m.read_token).unwrap(), k2);

    c.delete(&m.id, &m.write_token).unwrap();
    assert_eq!(
        c.get(&m.id, &m.read_token).unwrap_err().code,
        "mailbox_not_found"
    );
    assert_eq!(
        c.delete(&m.id, &m.write_token).unwrap_err().code,
        "mailbox_not_found"
    );
}

#[test]
fn tokens_are_enforced() {
    let (base, _d, _s) = start();
    let c = MailboxClient::new(&base);
    let m = c.create().unwrap();
    let k = kit_json(1);
    let url = format!("{base}/v1/mailboxes/{}/kit", m.id);

    // The read token cannot write; the write token cannot read.
    assert_eq!(
        c.put(&m.id, &m.read_token, &k).unwrap_err().code,
        "unauthorized"
    );
    c.put(&m.id, &m.write_token, &k).unwrap();
    assert_eq!(
        c.get(&m.id, &m.write_token).unwrap_err().code,
        "unauthorized"
    );
    assert_eq!(
        c.delete(&m.id, &m.read_token).unwrap_err().code,
        "unauthorized"
    );
    let (status, body, _) = raw("GET", &url, None, None);
    assert_eq!((status, error_code(&body).as_str()), (401, "unauthorized"));
    let (status, body, _) = raw("GET", &url, Some("nope"), None);
    assert_eq!((status, error_code(&body).as_str()), (401, "unauthorized"));

    // Unknown mailbox.
    let (status, body, _) = raw(
        "GET",
        &format!("{base}/v1/mailboxes/{}/kit", "0".repeat(32)),
        Some(&m.read_token),
        None,
    );
    assert_eq!(
        (status, error_code(&body).as_str()),
        (404, "mailbox_not_found")
    );

    // Still intact after the failed attempts.
    assert_eq!(c.get(&m.id, &m.read_token).unwrap(), k);
}

#[test]
fn rejects_oversized_and_invalid_kits() {
    let (base, _d, _s) = start();
    let c = MailboxClient::new(&base);
    let m = c.create().unwrap();
    let url = format!("{base}/v1/mailboxes/{}/kit", m.id);
    let good = kit_json(1);

    let big = "x".repeat(64 * 1024 + 1);
    let (status, body, _) = raw("PUT", &url, Some(&m.write_token), Some(&big));
    assert_eq!((status, error_code(&body).as_str()), (413, "kit_invalid"));

    let (status, body, _) = raw("PUT", &url, Some(&m.write_token), Some("not json"));
    assert_eq!((status, error_code(&body).as_str()), (400, "kit_invalid"));

    let v2 = good.replacen("\"version\": 1", "\"version\": 2", 1);
    assert_ne!(v2, good);
    let (status, body, _) = raw("PUT", &url, Some(&m.write_token), Some(&v2));
    assert_eq!(
        (status, error_code(&body).as_str()),
        (400, "kit_version_unsupported")
    );

    // A different round than the sealed part is locked to.
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["unlock"]["round"] = serde_json::json!(value["unlock"]["round"].as_u64().unwrap() + 1);
    let (status, body, _) = raw("PUT", &url, Some(&m.write_token), Some(&value.to_string()));
    assert_eq!((status, error_code(&body).as_str()), (400, "kit_invalid"));

    // Nothing was stored.
    assert_eq!(
        c.get(&m.id, &m.read_token).unwrap_err().code,
        "mailbox_not_found"
    );
}

#[test]
fn cors_allows_get_from_any_origin() {
    let (base, _d, _s) = start();
    let c = MailboxClient::new(&base);
    let m = c.create().unwrap();
    c.put(&m.id, &m.write_token, &kit_json(1)).unwrap();
    let url = format!("{base}/v1/mailboxes/{}/kit", m.id);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let resp = agent
        .get(&url)
        .header("Origin", "https://heir.example")
        .header("Authorization", format!("Bearer {}", m.read_token))
        .call()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.headers().get("access-control-allow-origin").unwrap(),
        "*"
    );
    assert!(resp
        .headers()
        .get("access-control-allow-credentials")
        .is_none());

    let req = ureq::http::Request::builder()
        .method("OPTIONS")
        .uri(&url)
        .header("Origin", "https://heir.example")
        .header("Access-Control-Request-Method", "GET")
        .header("Access-Control-Request-Headers", "authorization")
        .body(String::new())
        .unwrap();
    let pre = agent.run(req).unwrap();
    assert!(pre.status().is_success());
    let methods = pre
        .headers()
        .get("access-control-allow-methods")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(methods.contains("GET") && !methods.contains("PUT"));
    assert!(pre
        .headers()
        .get("access-control-allow-headers")
        .unwrap()
        .to_str()
        .unwrap()
        .to_lowercase()
        .contains("authorization"));
}

#[test]
fn retention_deletes_old_mailboxes() {
    let (base, _d, state) = start();
    let c = MailboxClient::new(&base);
    let old = c.create().unwrap();
    let fresh = c.create().unwrap();
    c.put(&old.id, &old.write_token, &kit_json(1)).unwrap();
    c.put(&fresh.id, &fresh.write_token, &kit_json(1)).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    state
        .db()
        .set_updated(&old.id, now - sandglass_keeper::RETENTION_SECS - 10)
        .unwrap();
    assert_eq!(
        state
            .db()
            .purge_before(now - sandglass_keeper::RETENTION_SECS)
            .unwrap(),
        1
    );
    assert_eq!(
        c.get(&old.id, &old.read_token).unwrap_err().code,
        "mailbox_not_found"
    );
    assert!(c.get(&fresh.id, &fresh.read_token).is_ok());
}

#[test]
fn creation_is_rate_limited() {
    let (base, _d, _s) = start();
    let c = MailboxClient::new(&base);
    let mut limited = false;
    for _ in 0..15 {
        if let Err(e) = c.create() {
            assert_eq!(e.code, "rate_limited");
            limited = true;
            break;
        }
    }
    assert!(limited, "the creation limit must kick in");
}

#[test]
fn security_headers_on_api_and_errors() {
    let (base, _d, _s) = start();
    let csp = "default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; \
img-src 'self' data:; connect-src 'self' https://api.drand.sh https://api2.drand.sh \
https://drand.cloudflare.com; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
    for path in ["/v1/health", "/v1/nope", "/v1/mailboxes/zzzz/kit"] {
        let (_, _, h) = raw("GET", &format!("{base}{path}"), None, None);
        assert_eq!(h["content-security-policy"], csp, "{path}");
        assert_eq!(h["referrer-policy"], "no-referrer", "{path}");
        assert_eq!(h["x-content-type-options"], "nosniff", "{path}");
        assert_eq!(h["cache-control"], "no-store", "{path}");
    }
}

#[test]
fn security_headers_on_static_files() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(web.path().join("index.html"), "<p>hi</p>").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        web_dir: Some(web.path().to_path_buf()),
        ..Config::default()
    };
    let (addr, _s) = spawn_background(&dir.path().join("k.sqlite"), cfg).unwrap();
    let (status, body, h) = raw("GET", &format!("http://{addr}/"), None, None);
    assert_eq!(status, 200);
    assert!(body.contains("hi"));
    assert!(h["content-security-policy"]
        .to_str()
        .unwrap()
        .starts_with("default-src 'none'; script-src 'self'"));
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h["x-content-type-options"], "nosniff");
    // Static files are not API responses.
    assert!(h.get("cache-control").is_none_or(|v| v != "no-store"));
}

#[test]
fn clean_page_addresses() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(web.path().join("index.html"), "<p>home</p>").unwrap();
    std::fs::write(web.path().join("claim.html"), "<p>claim</p>").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        web_dir: Some(web.path().to_path_buf()),
        ..Config::default()
    };
    let (addr, _s) = spawn_background(&dir.path().join("k.sqlite"), cfg).unwrap();

    // The clean address serves the page.
    let (status, body, _) = raw("GET", &format!("http://{addr}/claim"), None, None);
    assert_eq!(status, 200);
    assert!(body.contains("claim"));

    // Old addresses redirect permanently, keeping the query.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();
    let loc = |path: &str| {
        let resp = agent.get(&format!("http://{addr}{path}")).call().unwrap();
        (
            resp.status().as_u16(),
            resp.headers()
                .get("location")
                .map(|v| v.to_str().unwrap().to_string()),
        )
    };
    assert_eq!(loc("/claim.html?api=x"), (301, Some("/claim?api=x".into())));
    assert_eq!(loc("/index.html"), (301, Some("/".into())));
    // Other files are untouched.
    assert_eq!(loc("/nothing.html").0, 404);
}

#[test]
fn global_creation_caps() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        max_creates_per_hour: 3,
        ..Config::default()
    };
    let (addr, _s) = spawn_background(&dir.path().join("k.sqlite"), cfg).unwrap();
    let c = MailboxClient::new(&format!("http://{addr}"));
    for _ in 0..3 {
        c.create().unwrap();
    }
    assert_eq!(c.create().unwrap_err().code, "rate_limited");

    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        max_mailboxes: 2,
        ..Config::default()
    };
    let (addr, _s) = spawn_background(&dir.path().join("k.sqlite"), cfg).unwrap();
    let c = MailboxClient::new(&format!("http://{addr}"));
    let a = c.create().unwrap();
    c.create().unwrap();
    assert_eq!(c.create().unwrap_err().code, "rate_limited");
    // Deleting one frees a slot.
    c.delete(&a.id, &a.write_token).unwrap();
    c.create().unwrap();
}

#[test]
fn stored_kit_is_canonical_and_unknown_fields_are_refused() {
    let (base, _d, _s) = start();
    let c = MailboxClient::new(&base);
    let m = c.create().unwrap();
    let good = kit_json(1);

    // A smuggled top-level field is refused.
    let mut v: serde_json::Value = serde_json::from_str(&good).unwrap();
    v["extra"] = serde_json::json!("parked data");
    let (status, body, _) = raw(
        "PUT",
        &format!("{base}/v1/mailboxes/{}/kit", m.id),
        Some(&m.write_token),
        Some(&v.to_string()),
    );
    assert_eq!(status, 400);
    assert_eq!(error_code(&body), "kit_invalid");

    // Unknown nested fields are dropped: the stored kit is the re-serialised
    // parse, not the submitted text.
    let mut v: serde_json::Value = serde_json::from_str(&good).unwrap();
    v["unlock"]["junk"] = serde_json::json!("x".repeat(500));
    c.put(&m.id, &m.write_token, &v.to_string()).unwrap();
    let stored = c.get(&m.id, &m.read_token).unwrap();
    assert!(!stored.contains("junk"));
    assert_eq!(stored, good);
}

#[test]
fn database_is_secure_delete_and_private() {
    let (_base, dir, s) = start();
    assert!(s.db().secure_delete_enabled());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("k.sqlite"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let _ = dir;
}
