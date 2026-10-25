//! Session storage for the playground (its own SQLite file).
//!
//! A row holds the heir code (public), timestamps, the claim link and the
//! state machine fields. It never holds a client address, IP or user agent:
//! the destination of a "send" is kept only until the sweep is done.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};

/// Session states (see `docs/PLAYGROUND_API.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Funding,
    Sealed,
    Flipping,
    Unlocked,
    Settling,
    Returned,
    Sent,
    Kept,
    Failed,
}

impl State {
    /// The API spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Funding => "funding",
            State::Sealed => "sealed",
            State::Flipping => "flipping",
            State::Unlocked => "unlocked",
            State::Settling => "settling",
            State::Returned => "returned",
            State::Sent => "sent",
            State::Kept => "kept",
            State::Failed => "failed",
        }
    }

    fn parse(s: &str) -> State {
        match s {
            "funding" => State::Funding,
            "sealed" => State::Sealed,
            "flipping" => State::Flipping,
            "unlocked" => State::Unlocked,
            "settling" => State::Settling,
            "returned" => State::Returned,
            "sent" => State::Sent,
            "kept" => State::Kept,
            _ => State::Failed,
        }
    }

    /// Counts against the concurrent session cap.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            State::Funding | State::Sealed | State::Flipping | State::Unlocked | State::Settling
        )
    }
}

/// One playground session.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub token_hash: Vec<u8>,
    pub heir_code: String,
    pub minutes: u32,
    pub state: State,
    pub epoch: u32,
    pub flips: u32,
    pub created: u64,
    pub unlock_round: u64,
    pub unlock_time: u64,
    /// Zatoshi the float sent to the vault in the funding transaction.
    pub funded_zat: u64,
    pub vault_address: String,
    pub balance_zat: u64,
    pub confirmed: bool,
    pub mailbox_id: String,
    pub claim_link: String,
    pub message: String,
    pub error: Option<String>,
    pub settled_at: Option<u64>,
    /// `flip`, `return` or `send` while the state is flipping or settling.
    pub op: Option<String>,
    /// Destination of a `send`; cleared when the sweep is done.
    pub op_to: Option<String>,
    pub attempts: u32,
    /// A kept vault has been swept back (or found empty) after 24 hours.
    pub kept_swept: bool,
    /// A flip whose sweep may or may not have reached the network (JSON,
    /// see the worker). The new epoch is tracked until the outcome is known.
    pub pending: Option<String>,
}

/// A transaction of a session.
#[derive(Clone, Debug)]
pub struct TxRow {
    pub kind: String,
    pub txid: String,
}

const COLS: &str = "id, token_hash, heir_code, minutes, state, epoch, flips, created, \
    unlock_round, unlock_time, funded_zat, vault_address, balance_zat, confirmed, mailbox_id, \
    claim_link, message, error, settled_at, op, op_to, attempts, kept_swept, pending";

fn from_row(r: &Row<'_>) -> rusqlite::Result<Session> {
    let u = |i: usize| -> rusqlite::Result<u64> { Ok(r.get::<_, i64>(i)?.max(0) as u64) };
    Ok(Session {
        id: r.get(0)?,
        token_hash: r.get(1)?,
        heir_code: r.get(2)?,
        minutes: u(3)? as u32,
        state: State::parse(&r.get::<_, String>(4)?),
        epoch: u(5)? as u32,
        flips: u(6)? as u32,
        created: u(7)?,
        unlock_round: u(8)?,
        unlock_time: u(9)?,
        funded_zat: u(10)?,
        vault_address: r.get(11)?,
        balance_zat: u(12)?,
        confirmed: r.get::<_, i64>(13)? != 0,
        mailbox_id: r.get(14)?,
        claim_link: r.get(15)?,
        message: r.get(16)?,
        error: r.get(17)?,
        settled_at: r.get::<_, Option<i64>>(18)?.map(|v| v.max(0) as u64),
        op: r.get(19)?,
        op_to: r.get(20)?,
        attempts: u(21)? as u32,
        kept_swept: r.get::<_, i64>(22)? != 0,
        pending: r.get(23)?,
    })
}

/// The session database.
pub struct SessionDb {
    conn: Mutex<Connection>,
}

/// UTC day number of a unix time.
pub fn day_of(t: u64) -> u64 {
    t / 86_400
}

impl SessionDb {
    /// Opens (and if needed creates) the database.
    pub fn open(path: &Path) -> Result<Self> {
        zandglass_wallet::fsutil::touch_private(path)?;
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA secure_delete = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS sessions (
                 id TEXT PRIMARY KEY,
                 token_hash BLOB NOT NULL,
                 heir_code TEXT NOT NULL,
                 minutes INTEGER NOT NULL,
                 state TEXT NOT NULL,
                 epoch INTEGER NOT NULL,
                 flips INTEGER NOT NULL DEFAULT 0,
                 created INTEGER NOT NULL,
                 unlock_round INTEGER NOT NULL,
                 unlock_time INTEGER NOT NULL,
                 funded_zat INTEGER NOT NULL,
                 vault_address TEXT NOT NULL,
                 balance_zat INTEGER NOT NULL DEFAULT 0,
                 confirmed INTEGER NOT NULL DEFAULT 0,
                 mailbox_id TEXT NOT NULL,
                 claim_link TEXT NOT NULL,
                 message TEXT NOT NULL DEFAULT '',
                 error TEXT,
                 settled_at INTEGER,
                 op TEXT,
                 op_to TEXT,
                 attempts INTEGER NOT NULL DEFAULT 0,
                 kept_swept INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS txs (
                 seq INTEGER PRIMARY KEY AUTOINCREMENT,
                 session TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 txid TEXT NOT NULL,
                 at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS txs_session ON txs (session);",
        )?;
        // Databases from before the pending-flip column.
        let has_pending: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'pending'",
            [],
            |r| Ok(r.get::<_, i64>(0)? > 0),
        )?;
        if !has_pending {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN pending TEXT;")?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Inserts a new session row.
    pub fn insert(&self, s: &Session) -> Result<()> {
        self.conn().execute(
            &format!(
                "INSERT INTO sessions ({COLS}) VALUES \
                 (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)"
            ),
            params![
                s.id,
                s.token_hash,
                s.heir_code,
                s.minutes,
                s.state.as_str(),
                s.epoch,
                s.flips,
                s.created as i64,
                s.unlock_round as i64,
                s.unlock_time as i64,
                s.funded_zat as i64,
                s.vault_address,
                s.balance_zat as i64,
                i64::from(s.confirmed),
                s.mailbox_id,
                s.claim_link,
                s.message,
                s.error,
                s.settled_at.map(|v| v as i64),
                s.op,
                s.op_to,
                s.attempts,
                i64::from(s.kept_swept),
                s.pending,
            ],
        )?;
        Ok(())
    }

    /// Writes every mutable column. Only for states the worker owns
    /// (funding, flipping, settling); handlers use the guarded updates below.
    pub fn save(&self, s: &Session) -> Result<()> {
        self.conn().execute(
            "UPDATE sessions SET state=?2, epoch=?3, flips=?4, unlock_round=?5, unlock_time=?6, \
             funded_zat=?7, vault_address=?8, balance_zat=?9, confirmed=?10, message=?11, \
             error=?12, settled_at=?13, op=?14, op_to=?15, attempts=?16, kept_swept=?17, \
             pending=?18 WHERE id=?1",
            params![
                s.id,
                s.state.as_str(),
                s.epoch,
                s.flips,
                s.unlock_round as i64,
                s.unlock_time as i64,
                s.funded_zat as i64,
                s.vault_address,
                s.balance_zat as i64,
                i64::from(s.confirmed),
                s.message,
                s.error,
                s.settled_at.map(|v| v as i64),
                s.op,
                s.op_to,
                s.attempts,
                i64::from(s.kept_swept),
                s.pending,
            ],
        )?;
        Ok(())
    }

    /// Records the intent to fund a session, before the float is asked to
    /// pay: `op = 'fund'` on a `funding` row that has no intent yet. Returns
    /// whether this call set it. A row with the intent is never funded
    /// again unless [`SessionDb::release_fund`] clears it.
    pub fn claim_fund(&self, id: &str) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE sessions SET op = 'fund' WHERE id = ?1 AND state = 'funding' AND op IS NULL",
            [id],
        )?;
        Ok(n == 1)
    }

    /// Clears the funding intent after a funding attempt that certainly sent
    /// nothing, so the worker may try again.
    pub fn release_fund(&self, id: &str, attempts: u32, message: &str) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE sessions SET op = NULL, attempts = ?2, message = ?3 \
             WHERE id = ?1 AND state = 'funding' AND op = 'fund'",
            params![id, attempts, message],
        )?;
        Ok(n == 1)
    }

    /// One session by id.
    pub fn get(&self, id: &str) -> Result<Option<Session>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {COLS} FROM sessions WHERE id = ?1"),
                [id],
                from_row,
            )
            .optional()?)
    }

    /// Sessions the worker still has to look at.
    pub fn live(&self) -> Result<Vec<Session>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM sessions WHERE state IN \
             ('funding','sealed','flipping','unlocked','settling') \
             OR (state = 'kept' AND kept_swept = 0) ORDER BY created"
        ))?;
        let rows = stmt.query_map([], from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Number of sessions that count against the concurrent cap.
    pub fn count_active(&self) -> Result<u32> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM sessions WHERE state IN \
             ('funding','sealed','flipping','unlocked','settling')",
            [],
            |r| r.get(0),
        )?)
    }

    /// Sessions created at or after `since`.
    pub fn count_created_since(&self, since: u64) -> Result<u32> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM sessions WHERE created >= ?1",
            [since as i64],
            |r| r.get(0),
        )?)
    }

    /// Sessions kept by the visitor at or after `since`.
    pub fn count_kept_since(&self, since: u64) -> Result<u32> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM sessions WHERE state = 'kept' AND settled_at >= ?1",
            [since as i64],
            |r| r.get(0),
        )?)
    }

    /// Records a transaction.
    pub fn add_tx(&self, id: &str, kind: &str, txid: &str, at: u64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO txs (session, kind, txid, at) VALUES (?1, ?2, ?3, ?4)",
            params![id, kind, txid, at as i64],
        )?;
        Ok(())
    }

    /// The transactions of a session, oldest first.
    pub fn txs(&self, id: &str) -> Result<Vec<TxRow>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT kind, txid FROM txs WHERE session = ?1 ORDER BY seq")?;
        let rows = stmt.query_map([id], |r| {
            Ok(TxRow {
                kind: r.get(0)?,
                txid: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Whether a transaction of this kind is already recorded.
    pub fn has_tx(&self, id: &str, kind: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM txs WHERE session = ?1 AND kind = ?2",
            params![id, kind],
            |r| r.get::<_, i64>(0),
        )? > 0)
    }

    /// Guarded transition used by handlers and the worker for states that
    /// both may touch. Returns whether the row was in `from`.
    pub fn transition(
        &self,
        id: &str,
        from: State,
        to: State,
        op: Option<&str>,
        op_to: Option<&str>,
        message: &str,
    ) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE sessions SET state = ?3, op = ?4, op_to = ?5, attempts = 0, message = ?6 \
             WHERE id = ?1 AND state = ?2",
            params![id, from.as_str(), to.as_str(), op, op_to, message],
        )?;
        Ok(n == 1)
    }

    /// Marks a session as kept (guarded on `unlocked`).
    pub fn mark_kept(&self, id: &str, now: u64, message: &str) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE sessions SET state = 'kept', settled_at = ?2, message = ?3 \
             WHERE id = ?1 AND state = 'unlocked'",
            params![id, now as i64, message],
        )?;
        Ok(n == 1)
    }

    /// Updates only the balance columns (safe while a handler changes the state).
    pub fn set_balance(&self, id: &str, total: u64, confirmed: bool) -> Result<()> {
        self.conn().execute(
            "UPDATE sessions SET balance_zat = ?2, confirmed = ?3 WHERE id = ?1",
            params![id, total as i64, i64::from(confirmed)],
        )?;
        Ok(())
    }

    /// Marks a kept vault as swept back.
    pub fn set_kept_swept(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("UPDATE sessions SET kept_swept = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Sessions settled before `cutoff` that may be purged once their
    /// vaults are known to be empty: `(id, mailbox_id)`, oldest first. A kept
    /// vault still waiting for its 24-hour return is not a candidate.
    pub fn purge_candidates(&self, cutoff: u64) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, mailbox_id FROM sessions WHERE settled_at IS NOT NULL AND settled_at < ?1 \
             AND (state IN ('returned','sent','failed') OR (state = 'kept' AND kept_swept = 1)) \
             ORDER BY settled_at",
        )?;
        let it = stmt.query_map([cutoff as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(it.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Deletes a session row and its transactions.
    pub fn delete(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM txs WHERE session = ?1", [id])?;
        tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Test helper: run raw SQL (to break the database on purpose).
    #[cfg(test)]
    pub fn exec_for_tests(&self, sql: &str) -> Result<()> {
        self.conn().execute_batch(sql)?;
        Ok(())
    }

    /// Test helper: backdate the settlement time.
    pub fn set_settled_at(&self, id: &str, t: u64) -> Result<()> {
        self.conn().execute(
            "UPDATE sessions SET settled_at = ?2 WHERE id = ?1",
            params![id, t as i64],
        )?;
        Ok(())
    }
}
