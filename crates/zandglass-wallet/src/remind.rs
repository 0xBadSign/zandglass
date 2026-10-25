//! Calendar reminders (iCalendar, RFC 5545) for the owner's own calendar.
//!
//! The file is made locally and imported by the owner: no server learns the
//! schedule. It names the plan by its short id only: no amounts, heirs or
//! addresses.

use zandglass_core::kit::format_time;

use crate::store::{Plan, PlanStatus};

const DAY: u64 = 86_400;

/// How long before the unlock the flip reminder fires: a quarter of the
/// period, at least one minute and at most seven days.
pub fn reminder_lead(period_secs: u64) -> u64 {
    (period_secs / 4).clamp(60, 7 * DAY)
}

/// An iCalendar file with reminders for one plan.
///
/// Legacy plans get a repeating "flip" reminder (a quarter period, at most a
/// week, before the current unlock time; it repeats every period when the
/// period is a whole number of days) and a one-off "unlocks" event.
/// Installment plans get one event per installment unlock.
pub fn plan_ics(plan: &Plan, now: u64) -> String {
    let short = plan.short_id();
    let mut events = Vec::new();
    if plan.status == PlanStatus::Active {
        if let Some(set) = &plan.installments {
            for item in &set.items {
                events.push(Event {
                    uid: random_uid(),
                    start: item.unlock_time,
                    summary: format!(
                        "Zandglass: installment {} of {} unlocks (plan {short})",
                        item.index, set.count
                    ),
                    description: "From now on the payee can claim this installment. To stop the schedule, cancel the plan before the next installment unlocks.".into(),
                    rrule: None,
                    alarm: false,
                });
            }
        } else if let (Some(_), Ok(rec)) = (&plan.lock, plan.current()) {
            // A savings lock is never flipped: one event when it opens.
            events.push(Event {
                uid: random_uid(),
                start: rec.unlock_time,
                summary: format!("Zandglass: savings lock {short} opens"),
                description: "The lock has ended. Open the kit with your heir words on zandglass.cash/claim or with `zandglass claim`.".into(),
                rrule: None,
                alarm: true,
            });
        } else if let Ok(rec) = plan.current() {
            let unlock = rec.unlock_time;
            let lead = reminder_lead(plan.period_secs);
            let rrule = (plan.period_secs >= DAY && plan.period_secs % DAY == 0)
                .then(|| format!("FREQ=DAILY;INTERVAL={}", plan.period_secs / DAY));
            events.push(Event {
                uid: random_uid(),
                start: unlock.saturating_sub(lead).max(now.min(unlock)),
                summary: format!("Zandglass: flip plan {short}"),
                description: format!(
                    "Flip before the kit unlocks at {}: press Flip in `zandglass ui`, or run `zandglass flip {short}`. \
                     If you do not, your heirs can open the kit after that time. \
                     After a flip, download this calendar file again for the new date.",
                    format_time(unlock)
                ),
                rrule,
                alarm: true,
            });
            events.push(Event {
                uid: random_uid(),
                start: unlock,
                summary: format!("Zandglass: kit of plan {short} unlocks"),
                description: "From now on your heirs can open the kit. A flip before this time keeps the funds yours.".into(),
                rrule: None,
                alarm: false,
            });
        }
    }
    let mut out = String::new();
    line(&mut out, "BEGIN:VCALENDAR");
    line(&mut out, "VERSION:2.0");
    line(&mut out, "PRODID:-//Zandglass//Reminders//EN");
    line(&mut out, "CALSCALE:GREGORIAN");
    for e in &events {
        line(&mut out, "BEGIN:VEVENT");
        line(&mut out, &format!("UID:{}", e.uid));
        line(&mut out, &format!("DTSTAMP:{}", ics_time(now)));
        line(&mut out, &format!("DTSTART:{}", ics_time(e.start)));
        line(&mut out, &format!("DTEND:{}", ics_time(e.start + 15 * 60)));
        if let Some(r) = &e.rrule {
            line(&mut out, &format!("RRULE:{r}"));
        }
        line(&mut out, &format!("SUMMARY:{}", escape(&e.summary)));
        line(&mut out, &format!("DESCRIPTION:{}", escape(&e.description)));
        if e.alarm {
            line(&mut out, "BEGIN:VALARM");
            line(&mut out, "ACTION:DISPLAY");
            line(&mut out, &format!("DESCRIPTION:{}", escape(&e.summary)));
            line(&mut out, "TRIGGER:PT0S");
            line(&mut out, "END:VALARM");
        }
        line(&mut out, "END:VEVENT");
    }
    line(&mut out, "END:VCALENDAR");
    out
}

/// A random event UID. It must not contain the plan id: the id is also in
/// every kit's public part, and a cloud calendar would otherwise link the
/// owner to a kit or mailbox.
fn random_uid() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    format!("{}@zandglass", hex::encode(b))
}

struct Event {
    uid: String,
    start: u64,
    summary: String,
    description: String,
    rrule: Option<String>,
    alarm: bool,
}

/// `2026-10-01T12:00:00Z` -> `20261001T120000Z`.
fn ics_time(unix: u64) -> String {
    format_time(unix).replace(['-', ':'], "")
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

/// Appends one content line, folded at 75 octets (RFC 5545 3.1), with CRLF.
fn line(out: &mut String, text: &str) {
    let mut width = 0;
    for ch in text.chars() {
        let n = ch.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += n;
    }
    out.push_str("\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::EpochRecord;

    fn plan(period: u64, unlock: u64) -> Plan {
        Plan {
            id: "0123456789abcdef0123456789abcdef".into(),
            kind: "legacy".into(),
            network: "main".into(),
            heirs: vec!["age1secretheircode".into()],
            period_secs: period,
            message: Some("a private message".into()),
            epoch: 2,
            status: PlanStatus::Active,
            created: 0,
            mailbox_url: None,
            mailbox_id: None,
            mailbox_write_token: None,
            epochs: vec![EpochRecord {
                epoch: 2,
                dir: "v".into(),
                birthday_height: 1,
                unlock_round: 1,
                unlock_time: unlock,
                created: 0,
            }],
            installments: None,
            pending: None,
            archived: Vec::new(),
            lock: None,
        }
    }

    #[test]
    fn legacy_reminders() {
        let unlock = 1_800_000_000;
        let ics = plan_ics(&plan(90 * DAY, unlock), unlock - 30 * DAY);
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        // Seven days before the unlock, repeating every 90 days.
        assert!(ics.contains(&format!("DTSTART:{}", ics_time(unlock - 7 * DAY))));
        assert!(ics.contains("RRULE:FREQ=DAILY;INTERVAL=90"));
        assert!(ics.contains(&format!("DTSTART:{}", ics_time(unlock))));
        assert!(ics.contains("BEGIN:VALARM"));
        // Nothing private: no heir code, message or amount.
        assert!(!ics.contains("age1") && !ics.contains("private message"));
        // The full plan id (public in every kit) is not in the file; UIDs are random.
        assert!(!ics.contains("0123456789abcdef0123456789abcdef"));
        let uids: Vec<&str> = ics.lines().filter(|l| l.starts_with("UID:")).collect();
        assert_eq!(uids.len(), 2);
        assert_ne!(uids[0], uids[1]);
        assert!(uids
            .iter()
            .all(|u| u.len() == 4 + 32 + 10 && u.ends_with("@zandglass")));
        // Every physical line fits in 75 octets.
        assert!(ics.split("\r\n").all(|l| l.len() <= 75), "{ics}");
    }

    #[test]
    fn short_demo_period_has_no_repeat() {
        let ics = plan_ics(&plan(600, 1_000_000), 999_000);
        assert!(!ics.contains("RRULE"));
        assert_eq!(reminder_lead(600), 150);
        assert_eq!(reminder_lead(10), 60);
        assert_eq!(reminder_lead(400 * DAY), 7 * DAY);
    }

    #[test]
    fn a_savings_lock_has_one_unlock_event_and_no_flip_reminder() {
        let mut p = plan(90 * DAY, 1_800_000_000);
        p.kind = "lock".into();
        p.lock = Some(crate::store::LockInfo {
            address: "u1secretaddress".into(),
        });
        let ics = plan_ics(&p, 1_700_000_000);
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 1);
        assert!(ics.contains(&format!("DTSTART:{}", ics_time(1_800_000_000))));
        assert!(!ics.contains("RRULE") && !ics.to_lowercase().contains("flip"));
        assert!(!ics.contains("u1secretaddress") && !ics.contains("age1"));
    }

    #[test]
    fn cancelled_plan_has_no_events() {
        let mut p = plan(DAY, 1_000_000);
        p.status = PlanStatus::Cancelled;
        assert!(!plan_ics(&p, 0).contains("VEVENT"));
    }

    #[test]
    fn escaping() {
        assert_eq!(escape("a,b;c\\d\ne"), "a\\,b\\;c\\\\d\\ne");
    }
}
