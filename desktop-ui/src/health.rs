//! Whether the iPhone is still sending, from the inbox's `/health`. iOS sometimes turns
//! automations off without saying so; these rules decide when to say something, from most to
//! least certain. The web inbox applies the same rules.
use serde_json::Value;

/// A day and a bit: a daily check-in that's later than this was missed.
const CHECKIN_LATE: i64 = 26 * 3600;
/// Quiet this long, from a phone that usually sends a couple of texts a day, is worth a nudge.
const QUIET: i64 = 36 * 3600;

pub struct Phone {
    pub last_message_at: Option<i64>,
    pub checked_in_at: Option<i64>,
}
pub fn phones(health: &Value) -> Vec<Phone> {
    health["phones"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| Phone {
            last_message_at: p["last_message_at"].as_i64(),
            checked_in_at: p["checked_in_at"].as_i64(),
        })
        .collect()
}
/// What's wrong, if anything: a headline and what to do. `None` when all looks well.
pub fn warning(health: &Value, now: i64) -> Option<(String, String)> {
    let phones = phones(health);
    let newest_text = phones.iter().filter_map(|p| p.last_message_at).max();
    let fix = "On your iPhone, open Shortcuts → Automation, turn pSMS Capture back on and set it to Run Immediately.".to_string();
    // 1. The Mac saw a text that no phone sent afterwards.
    if let Some(missed) = health["missed_at"].as_i64()
        && newest_text.is_none_or(|t| t < missed)
    {
        return Some((
            "Your iPhone stopped sending texts to pSMS".into(),
            format!("A text reached this Mac but not pSMS. {fix}"),
        ));
    }
    // 2. A phone that checks in daily missed its check-in.
    if let Some(late) = phones
        .iter()
        .filter_map(|p| p.checked_in_at)
        .filter(|t| now - t > CHECKIN_LATE)
        .max()
    {
        return Some((
            format!(
                "Your iPhone hasn't checked in for {} hours",
                (now - late) / 3600
            ),
            format!("iOS may have turned its automations off. {fix}"),
        ));
    }
    // 3. Unusually quiet for this inbox.
    let usual = health["recent_messages_7d"].as_i64().unwrap_or(0);
    if let Some(last) = newest_text
        && usual >= 14
        && now - last > QUIET
    {
        return Some((
            format!("No texts from your iPhone in {} hours", (now - last) / 3600),
            format!(
                "That's unusual for you. If you've had texts since, check the automation. {fix}"
            ),
        ));
    }
    None
}
/// A short line for Settings: when the phone last sent a text and checked in.
pub fn summary(health: &Value, now: i64) -> String {
    let ago = |t: i64| {
        let m = (now - t).max(0) / 60;
        match m {
            0..=1 => "just now".to_string(),
            2..=59 => format!("{m} min ago"),
            60..=2879 => format!("{} h ago", m / 60),
            _ => format!("{} days ago", m / 1440),
        }
    };
    let phones = phones(health);
    if phones.is_empty() {
        return "No phone connected".into();
    }
    let text = phones.iter().filter_map(|p| p.last_message_at).max();
    let check = phones.iter().filter_map(|p| p.checked_in_at).max();
    match (text, check) {
        (Some(t), Some(c)) => format!("Last text {} · checked in {}", ago(t), ago(c)),
        (Some(t), None) => format!("Last text {} · no daily check-in yet", ago(t)),
        (None, Some(c)) => format!("No texts yet · checked in {}", ago(c)),
        (None, None) => "Waiting for the first text".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn warns_from_most_to_least_certain() {
        let now = 1_000_000;
        let ok = json!({"phones":[{"last_message_at": now - 600, "checked_in_at": now - 3600}], "recent_messages_7d": 40});
        assert!(warning(&ok, now).is_none());
        let missed = json!({"phones":[{"last_message_at": now - 3000, "checked_in_at": now - 3600}], "missed_at": now - 100, "recent_messages_7d": 40});
        assert!(warning(&missed, now).unwrap().0.contains("stopped sending"));
        // A text after the miss means it's working again.
        let recovered = json!({"phones":[{"last_message_at": now - 10}], "missed_at": now - 100});
        assert!(warning(&recovered, now).is_none());
        let late =
            json!({"phones":[{"last_message_at": now - 3600, "checked_in_at": now - 30 * 3600}]});
        assert!(
            warning(&late, now)
                .unwrap()
                .0
                .contains("checked in for 30 hours")
        );
        let quiet =
            json!({"phones":[{"last_message_at": now - 40 * 3600}], "recent_messages_7d": 30});
        assert!(warning(&quiet, now).unwrap().0.contains("40 hours"));
        let rare =
            json!({"phones":[{"last_message_at": now - 40 * 3600}], "recent_messages_7d": 3});
        assert!(warning(&rare, now).is_none());
        assert_eq!(
            summary(&ok, now),
            "Last text 10 min ago · checked in 1 h ago"
        );
    }
}
