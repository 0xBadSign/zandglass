//! Blocking client for the keeper's blind mailbox API (PLAN section 5.5).
//!
//! The keeper only ever sees public kits. Tokens are random and are not keys
//! to any funds.

use std::time::Duration;

use serde::Deserialize;

/// Largest kit the keeper accepts, in bytes.
pub const MAX_KIT_BYTES: usize = 64 * 1024;

/// A mailbox failure. `code` is one of the shared codes (PLAN 5.10) when the
/// keeper sent one.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct MailboxError {
    /// Shared error code, such as `mailbox_not_found`, or `mailbox_unreachable`.
    pub code: String,
    /// Plain-English explanation.
    pub message: String,
}

impl MailboxError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Most mailboxes one batch request may create.
pub const MAX_BATCH: usize = 120;

/// What the keeper returns when a mailbox is created.
#[derive(Clone, Debug, Deserialize)]
pub struct Created {
    /// Mailbox id (128-bit hex).
    pub id: String,
    /// Token that may replace or delete the kit.
    pub write_token: String,
    /// Token that may read the kit; this goes to the heir in the claim link.
    pub read_token: String,
}

/// A claim link `<keeper>/claim#m=<id>&r=<read_token>` (the heir page). The
/// older `/claim.html` form is still accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimLink {
    /// Keeper base URL without a trailing slash.
    pub base: String,
    /// Mailbox id.
    pub id: String,
    /// Read token.
    pub read_token: String,
}

impl ClaimLink {
    /// Builds the link the owner gives to the heir.
    pub fn new(base: &str, created: &Created) -> Self {
        Self {
            base: normalize_base(base),
            id: created.id.clone(),
            read_token: created.read_token.clone(),
        }
    }

    /// Parses a claim link; `None` if the text is not one.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if !(text.starts_with("http://") || text.starts_with("https://")) {
            return None;
        }
        let (head, fragment) = text.split_once('#')?;
        let head = head.trim_end_matches('/');
        let base = head
            .strip_suffix("/claim.html")
            .or_else(|| head.strip_suffix("/claim"))
            .unwrap_or(head);
        let (mut id, mut token) = (None, None);
        for pair in fragment.split('&') {
            match pair.split_once('=') {
                Some(("m", v)) => id = Some(v),
                Some(("r", v)) => token = Some(v),
                _ => {}
            }
        }
        let (id, token) = (id?, token?);
        let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
        if !ok(id) || !ok(token) || base.is_empty() {
            return None;
        }
        Some(Self {
            base: base.to_string(),
            id: id.into(),
            read_token: token.into(),
        })
    }
}

impl std::fmt::Display for ClaimLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/claim#m={}&r={}", self.base, self.id, self.read_token)
    }
}

/// Removes trailing slashes from a keeper URL.
pub fn normalize_base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn parse_batch(body: &str, n: usize) -> Result<Vec<Created>, MailboxError> {
    #[derive(Deserialize)]
    struct Batch {
        mailboxes: Vec<Created>,
    }
    let bad =
        |e: String| MailboxError::new("mailbox_unreachable", format!("Bad keeper reply: {e}"));
    let batch: Batch = serde_json::from_str(body).map_err(|e| bad(e.to_string()))?;
    if batch.mailboxes.len() != n {
        return Err(bad(format!(
            "asked for {n} mailboxes, got {}",
            batch.mailboxes.len()
        )));
    }
    Ok(batch.mailboxes)
}

/// A keeper to talk to.
pub struct MailboxClient {
    base: String,
    agent: ureq::Agent,
}

impl MailboxClient {
    /// Client for the keeper at `base_url` (for example `https://keeper.example`).
    pub fn new(base_url: &str) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .into();
        Self {
            base: normalize_base(base_url),
            agent,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn finish(
        &self,
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<String, MailboxError> {
        let mut resp = result.map_err(|e| {
            MailboxError::new(
                "mailbox_unreachable",
                format!("Could not reach the keeper at {}: {e}", self.base),
            )
        })?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        if (200..300).contains(&status) {
            return Ok(body);
        }
        let parsed: Option<serde_json::Value> = serde_json::from_str(&body).ok();
        let field = |k: &str| {
            parsed
                .as_ref()
                .and_then(|v| v.get(k))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        Err(MailboxError::new(
            &field("error").unwrap_or_else(|| format!("http_{status}")),
            field("message")
                .unwrap_or_else(|| format!("The keeper answered with status {status}.")),
        ))
    }

    /// Creates a mailbox.
    pub fn create(&self) -> Result<Created, MailboxError> {
        let body = self.finish(self.agent.post(self.url("/v1/mailboxes")).send_empty())?;
        serde_json::from_str(&body)
            .map_err(|e| MailboxError::new("mailbox_unreachable", format!("Bad keeper reply: {e}")))
    }

    /// Creates `n` mailboxes in one request (1 to [`MAX_BATCH`]). One request
    /// counts once against the keeper's per-address limiter.
    pub fn create_many(&self, n: usize) -> Result<Vec<Created>, MailboxError> {
        if n == 0 || n > MAX_BATCH {
            return Err(MailboxError::new(
                "invalid_request",
                format!("A batch creates 1 to {MAX_BATCH} mailboxes."),
            ));
        }
        if n == 1 {
            return Ok(vec![self.create()?]);
        }
        let body = self.finish(
            self.agent
                .post(self.url("/v1/mailboxes"))
                .header("Content-Type", "application/json")
                .send(format!("{{\"count\":{n}}}")),
        )?;
        parse_batch(&body, n)
    }

    /// Replaces the mailbox's kit.
    pub fn put(&self, id: &str, write_token: &str, kit_json: &str) -> Result<(), MailboxError> {
        self.finish(
            self.agent
                .put(self.url(&format!("/v1/mailboxes/{id}/kit")))
                .header("Authorization", format!("Bearer {write_token}"))
                .header("Content-Type", "application/json")
                .send(kit_json),
        )
        .map(|_| ())
    }

    /// Fetches the latest kit JSON.
    pub fn get(&self, id: &str, read_token: &str) -> Result<String, MailboxError> {
        self.finish(
            self.agent
                .get(self.url(&format!("/v1/mailboxes/{id}/kit")))
                .header("Authorization", format!("Bearer {read_token}"))
                .call(),
        )
    }

    /// Deletes the mailbox.
    pub fn delete(&self, id: &str, write_token: &str) -> Result<(), MailboxError> {
        self.finish(
            self.agent
                .delete(self.url(&format!("/v1/mailboxes/{id}")))
                .header("Authorization", format!("Bearer {write_token}"))
                .call(),
        )
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_reply_parsing() {
        let one = r#"{"id":"a","write_token":"w","read_token":"r"}"#;
        let body = format!(r#"{{"mailboxes":[{one},{one}]}}"#);
        assert_eq!(parse_batch(&body, 2).unwrap().len(), 2);
        assert_eq!(
            parse_batch(&body, 3).unwrap_err().code,
            "mailbox_unreachable"
        );
        assert!(parse_batch(one, 1).is_err());
        let c = MailboxClient::new("http://127.0.0.1:1");
        assert_eq!(c.create_many(0).unwrap_err().code, "invalid_request");
        assert_eq!(c.create_many(121).unwrap_err().code, "invalid_request");
    }

    #[test]
    fn link_round_trip() {
        let c = Created {
            id: "ab12".into(),
            write_token: "w".into(),
            read_token: "cd34".into(),
        };
        let l = ClaimLink::new("http://127.0.0.1:8787/", &c);
        let text = l.to_string();
        assert_eq!(text, "http://127.0.0.1:8787/claim#m=ab12&r=cd34");
        assert_eq!(ClaimLink::parse(&text), Some(l));
        assert_eq!(ClaimLink::parse("kit.zandglass"), None);
        assert_eq!(ClaimLink::parse("http://x/claim#m=a"), None);
        assert_eq!(ClaimLink::parse("http://x/claim#m=a&r=b/../c"), None);
    }
}
