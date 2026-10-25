//! Keeper API integration tests: a real server on a random port, a temp DB.

use zandglass_core::heir::{generate_phrase, identity_from_phrase};
use zandglass_core::kit::{BuildParams, Kind, Network, Payload};
use zandglass_core::mailbox::MailboxClient;
use zandglass_core::{drand, Kit};
use zandglass_keeper::{spawn_background, Config};

const SEED: &str = "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth title";

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn kit_json(epoch: u32) -> String {
    kit_json_unlocking(epoch, now_secs() + 3600)
}

/// A kit that unlocks at (about) `unlock_at`, unix seconds.
fn kit_json_unlocking(epoch: u32, unlock_at: u64) -> String {
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
            unlock_round: drand::unlock_round(unlock_at),
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
    std::sync::Arc<zandglass_keeper::AppState>,
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

const DAY: u64 = 86_400;

#[test]
fn retention_deletes_old_mailboxes() {
    let (base, _d, state) = start();
    let c = MailboxClient::new(&base);
    let old = c.create().unwrap();
    let fresh = c.create().unwrap();
    c.put(&old.id, &old.write_token, &kit_json(1)).unwrap();
    c.put(&fresh.id, &fresh.write_token, &kit_json(1)).unwrap();
    let now = now_secs();
    state
        .db()
        .set_updated(&old.id, now - zandglass_keeper::RETENTION_SECS - 10)
        .unwrap();
    // Its kit unlocked long ago, so nothing keeps it.
    state
        .db()
        .set_unlock_time(
            &old.id,
            Some(now - zandglass_keeper::UNLOCK_GRACE_SECS - 10),
        )
        .unwrap();
    assert_eq!(state.db().purge_stale(now).unwrap(), 1);
    assert_eq!(
        c.get(&old.id, &old.read_token).unwrap_err().code,
        "mailbox_not_found"
    );
    assert!(c.get(&fresh.id, &fresh.read_token).is_ok());
}

#[test]
fn a_kit_that_has_not_unlocked_is_not_purged() {
    let (base, _d, state) = start();
    let c = MailboxClient::new(&base);
    let now = now_secs();
    // Stored today, unlocks in 300 days (a legacy plan with a long flip
    // period, or a late installment).
    let m = c.create().unwrap();
    let kit = kit_json_unlocking(1, now + 300 * DAY);
    c.put(&m.id, &m.write_token, &kit).unwrap();
    let unlock = Kit::from_json(&kit).unwrap().unlock_time();
    assert_eq!(state.db().unlock_time(&m.id).unwrap(), Some(unlock));

    // A purge run 200 days after the store: the old rule would delete it.
    assert_eq!(state.db().purge_stale(now + 200 * DAY).unwrap(), 0);
    assert!(c.get(&m.id, &m.read_token).is_ok());
    // Still there on the unlock day and 179 days after it ...
    assert_eq!(state.db().purge_stale(unlock + 179 * DAY).unwrap(), 0);
    assert!(c.get(&m.id, &m.read_token).is_ok());
    // ... gone once the grace after the unlock has passed.
    assert_eq!(state.db().purge_stale(unlock + 181 * DAY).unwrap(), 1);
    assert_eq!(
        c.get(&m.id, &m.read_token).unwrap_err().code,
        "mailbox_not_found"
    );
}

#[test]
fn a_kit_unlocked_long_ago_is_purged_when_old() {
    let (base, _d, state) = start();
    let c = MailboxClient::new(&base);
    let now = now_secs();
    let m = c.create().unwrap();
    // The server accepts a kit whose unlock time is already past.
    c.put(
        &m.id,
        &m.write_token,
        &kit_json_unlocking(1, now - 200 * DAY),
    )
    .unwrap();
    // Recently updated: kept whatever the unlock time says.
    assert_eq!(state.db().purge_stale(now).unwrap(), 0);
    // Not updated for more than the retention, unlocked 200 days ago: purged.
    state
        .db()
        .set_updated(&m.id, now - zandglass_keeper::RETENTION_SECS - 10)
        .unwrap();
    assert_eq!(state.db().purge_stale(now).unwrap(), 1);
    assert_eq!(
        c.get(&m.id, &m.read_token).unwrap_err().code,
        "mailbox_not_found"
    );
}

#[test]
fn a_kit_unlocking_far_in_the_future_is_refused() {
    let (base, _d, _s) = start();
    let c = MailboxClient::new(&base);
    let m = c.create().unwrap();
    let far = kit_json_unlocking(1, now_secs() + 32 * 365 * DAY);
    let err = c.put(&m.id, &m.write_token, &far).unwrap_err();
    assert_eq!(err.code, "kit_invalid");
    // Thirty years is fine.
    c.put(
        &m.id,
        &m.write_token,
        &kit_json_unlocking(1, now_secs() + 30 * 365 * DAY),
    )
    .unwrap();
}

#[test]
fn an_old_database_without_unlock_times_is_migrated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.sqlite");
    let now = now_secs();
    let long_kit = Kit::from_json(&kit_json_unlocking(1, now + 300 * DAY)).unwrap();
    let long_json = long_kit.to_json().unwrap();
    {
        // The schema an older keeper created, with no `unlock_time` column.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE mailboxes (
                 id TEXT PRIMARY KEY,
                 write_hash BLOB NOT NULL,
                 read_hash BLOB NOT NULL,
                 kit TEXT,
                 created INTEGER NOT NULL,
                 updated INTEGER NOT NULL
             );
             CREATE INDEX mailboxes_updated ON mailboxes (updated);",
        )
        .unwrap();
        let old = (now - 250 * DAY) as i64;
        for (id, kit) in [
            ("longkit", Some(long_json.as_str())),
            ("garbage", Some("not a kit")),
            ("empty", None),
        ] {
            conn.execute(
                "INSERT INTO mailboxes (id, write_hash, read_hash, kit, created, updated)
                 VALUES (?1, x'00', x'01', ?2, ?3, ?3)",
                rusqlite::params![id, kit, old],
            )
            .unwrap();
        }
    }
    for round in 0..2 {
        // The second open checks that the migration is repeatable.
        let db = zandglass_keeper::Db::open(&path).unwrap();
        assert_eq!(
            db.unlock_time("longkit").unwrap(),
            Some(long_kit.unlock_time()),
            "round {round}"
        );
        if round == 0 {
            assert_eq!(db.unlock_time("garbage").unwrap(), None);
            assert_eq!(db.unlock_time("empty").unwrap(), None);
            // Updated 250 days ago: the unparsable kit follows the old rule,
            // the long kit is kept until its unlock.
            assert_eq!(db.purge_stale(now).unwrap(), 2);
        }
    }
    // The data of the mailbox that stayed is intact.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let kept: String = conn
        .query_row("SELECT kit FROM mailboxes WHERE id = 'longkit'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(kept, long_json);
}

#[test]
fn batch_is_charged_n_against_the_per_address_hourly_quota() {
    // A legitimate installment batch of 120 passes for a fresh address ...
    let (base, _d, _s) = start();
    let c = MailboxClient::new(&base);
    assert_eq!(c.create_many(120).unwrap().len(), 120);
    assert_eq!(c.create_many(120).unwrap().len(), 120);
    // ... but the default 300 an hour runs out: a third batch needs 360.
    assert_eq!(c.create_many(120).unwrap_err().code, "rate_limited");
    // The refused batch took nothing: 60 still fit, then singles do not.
    assert_eq!(c.create_many(60).unwrap().len(), 60);
    assert_eq!(c.create().unwrap_err().code, "rate_limited");

    // A small quota: batches count N, singles count 1.
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        max_creates_per_ip_per_hour: 5,
        ..Config::default()
    };
    let (addr, state) = spawn_background(&dir.path().join("k.sqlite"), cfg).unwrap();
    let c = MailboxClient::new(&format!("http://{addr}"));
    c.create_many(3).unwrap();
    assert_eq!(c.create_many(3).unwrap_err().code, "rate_limited");
    c.create_many(2).unwrap();
    assert_eq!(c.create().unwrap_err().code, "rate_limited");
    assert_eq!(state.db().count().unwrap(), 5);
}

#[test]
fn a_client_over_its_quota_cannot_use_up_the_global_hourly_cap() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        max_creates_per_ip_per_hour: 4,
        max_creates_per_hour: 6,
        trust_proxy: true,
        ..Config::default()
    };
    let (addr, _s) = spawn_background(&dir.path().join("k.sqlite"), cfg).unwrap();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let post = |cf: &str, count: usize| {
        let req = ureq::http::Request::builder()
            .method("POST")
            .uri(format!("http://{addr}/v1/mailboxes"))
            .header("CF-Connecting-IP", cf)
            .body(format!("{{\"count\":{count}}}"))
            .unwrap();
        agent.run(req).unwrap().status().as_u16()
    };
    assert_eq!(post("198.51.100.1", 4), 201);
    // The first client is over its quota; refused attempts book nothing.
    for _ in 0..5 {
        assert_eq!(post("198.51.100.1", 2), 429);
    }
    // Another client can still use the rest of the global cap (2 of 6 left).
    assert_eq!(post("198.51.100.2", 2), 201);
    assert_eq!(post("198.51.100.2", 1), 429);
}

#[test]
fn never_filled_mailboxes_have_their_own_smaller_cap() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        max_empty_mailboxes: 3,
        ..Config::default()
    };
    let (addr, state) = spawn_background(&dir.path().join("k.sqlite"), cfg).unwrap();
    let c = MailboxClient::new(&format!("http://{addr}"));
    let made = c.create_many(3).unwrap();
    // The cap on empty ones is reached, although the keeper is far from full.
    assert_eq!(c.create().unwrap_err().code, "rate_limited");
    assert_eq!(c.create_many(2).unwrap_err().code, "rate_limited");
    // Filling one frees a slot for a new empty one; the filled one stays.
    c.put(&made[0].id, &made[0].write_token, &kit_json(1))
        .unwrap();
    c.create().unwrap();
    assert_eq!(c.create().unwrap_err().code, "rate_limited");
    // Empty ones still expire after a day.
    let now = now_secs();
    for m in &made[1..] {
        state
            .db()
            .set_created(&m.id, now - zandglass_keeper::EMPTY_RETENTION_SECS - 10)
            .unwrap();
    }
    assert_eq!(
        state
            .db()
            .purge_empty_before(now - zandglass_keeper::EMPTY_RETENTION_SECS)
            .unwrap(),
        2
    );
    assert!(c.get(&made[0].id, &made[0].read_token).is_ok());
    c.create_many(2).unwrap();
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
img-src 'self' data:; media-src 'self'; connect-src 'self' https://api.drand.sh https://api2.drand.sh \
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
    assert_eq!(h["strict-transport-security"], "max-age=31536000");
    assert_eq!(h["x-content-type-options"], "nosniff");
    // Static files are not API responses, and must not be rewritten by a CDN.
    assert_eq!(h["cache-control"], "no-transform");
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
fn signed_kits_are_stored_with_their_owner_fields_and_bad_signatures_refused() {
    let (base, _d, _s) = start();
    let c = MailboxClient::new(&base);
    let m = c.create().unwrap();

    let mut kit = Kit::from_json(&kit_json(1)).unwrap();
    kit.sign_owner(&[3u8; 32]);
    let signed = kit.to_json().unwrap();
    assert!(signed.contains("owner_key") && signed.contains("owner_sig"));
    c.put(&m.id, &m.write_token, &signed).unwrap();
    assert_eq!(c.get(&m.id, &m.read_token).unwrap(), signed);
    assert!(signed.len() < zandglass_core::mailbox::MAX_KIT_BYTES);

    // A signed kit changed after signing is refused as invalid.
    let mut v: serde_json::Value = serde_json::from_str(&signed).unwrap();
    v["epoch"] = serde_json::json!(2);
    let (status, body, _) = raw(
        "PUT",
        &format!("{base}/v1/mailboxes/{}/kit", m.id),
        Some(&m.write_token),
        Some(&v.to_string()),
    );
    assert_eq!(status, 400);
    assert_eq!(error_code(&body), "kit_invalid");
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

#[test]
fn hsts_on_every_response() {
    let (base, _d, _s) = start();
    for path in ["/v1/health", "/v1/nope"] {
        let (_, _, h) = raw("GET", &format!("{base}{path}"), None, None);
        assert_eq!(h["strict-transport-security"], "max-age=31536000", "{path}");
    }
}

#[test]
fn empty_mailboxes_are_purged_after_a_day() {
    let (base, _d, state) = start();
    let c = MailboxClient::new(&base);
    let empty_old = c.create().unwrap();
    let empty_new = c.create().unwrap();
    let filled_old = c.create().unwrap();
    c.put(&filled_old.id, &filled_old.write_token, &kit_json(1))
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let old = now - zandglass_keeper::EMPTY_RETENTION_SECS - 10;
    state.db().set_created(&empty_old.id, old).unwrap();
    state.db().set_created(&filled_old.id, old).unwrap();
    let cutoff = now - zandglass_keeper::EMPTY_RETENTION_SECS;
    assert_eq!(state.db().purge_empty_before(cutoff).unwrap(), 1);
    assert_eq!(
        c.delete(&empty_old.id, &empty_old.write_token)
            .unwrap_err()
            .code,
        "mailbox_not_found"
    );
    // A young empty mailbox and an old filled one both stay.
    c.delete(&empty_new.id, &empty_new.write_token).unwrap();
    assert!(c.get(&filled_old.id, &filled_old.read_token).is_ok());
}

#[test]
fn batch_create_returns_many_and_counts_against_the_global_caps() {
    let (base, _d, _s) = start();
    // The single shape is unchanged, with or without count 1.
    let url = format!("{base}/v1/mailboxes");
    let (s, body, _) = raw("POST", &url, None, Some("{\"count\":1}"));
    assert_eq!(s, 201);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["id"].is_string() && v.get("mailboxes").is_none());

    // 30 in one request pass a per-address limiter that allows a burst of 10.
    let c = MailboxClient::new(&base);
    let many = c.create_many(30).unwrap();
    assert_eq!(many.len(), 30);
    let ids: std::collections::HashSet<_> = many.iter().map(|m| m.id.clone()).collect();
    assert_eq!(ids.len(), 30);
    c.put(&many[7].id, &many[7].write_token, &kit_json(1))
        .unwrap();
    assert!(c.get(&many[7].id, &many[7].read_token).is_ok());

    // Bad counts are refused.
    for body in [
        "{\"count\":0}",
        "{\"count\":121}",
        "{\"count\":\"x\"}",
        "nope",
    ] {
        let (s, b, _) = raw("POST", &url, None, Some(body));
        assert_eq!(s, 400, "{body}");
        assert_eq!(error_code(&b), "invalid_request");
    }

    // Mailbox cap: the whole batch is refused if it does not fit.
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        max_mailboxes: 5,
        ..Config::default()
    };
    let (addr, _s) = spawn_background(&dir.path().join("k.sqlite"), cfg).unwrap();
    let c = MailboxClient::new(&format!("http://{addr}"));
    c.create_many(3).unwrap();
    assert_eq!(c.create_many(3).unwrap_err().code, "rate_limited");
    // Nothing was half-created: two more still fit.
    assert_eq!(c.create_many(2).unwrap().len(), 2);

    // Hourly cap: a batch counts N.
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        max_creates_per_hour: 4,
        ..Config::default()
    };
    let (addr, _s) = spawn_background(&dir.path().join("k.sqlite"), cfg).unwrap();
    let c = MailboxClient::new(&format!("http://{addr}"));
    c.create_many(3).unwrap();
    assert_eq!(c.create_many(2).unwrap_err().code, "rate_limited");
    c.create_many(1).unwrap();
}

#[test]
fn cf_connecting_ip_is_the_client_behind_a_trusted_proxy() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        trust_proxy: true,
        ..Config::default()
    };
    let (addr, _s) = spawn_background(&dir.path().join("k.sqlite"), cfg).unwrap();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let post = |cf: &str| {
        let req = ureq::http::Request::builder()
            .method("POST")
            .uri(format!("http://{addr}/v1/mailboxes"))
            .header("CF-Connecting-IP", cf)
            .header("X-Forwarded-For", "203.0.113.9")
            .body(String::new())
            .unwrap();
        agent.run(req).unwrap().status().as_u16()
    };
    // The same CF client is limited after the burst of 10 ...
    for _ in 0..10 {
        assert_eq!(post("198.51.100.1"), 201);
    }
    assert_eq!(post("198.51.100.1"), 429);
    // ... while another CF client (same proxy, same X-Forwarded-For) is not.
    assert_eq!(post("198.51.100.2"), 201);
}
