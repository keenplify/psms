//! One-time codes from new texts. When a text with a verification code arrives, pSMS shows a
//! native notification with **Copy code** and **Type code**. While the code is fresh (one
//! minute), pressing Tab in any app types it instead of moving focus. Codes are kept in memory
//! only and never logged.
use std::{
    collections::HashSet,
    sync::Mutex,
    time::{Duration, Instant},
};

/// How long Tab types the latest code.
const FRESH: Duration = Duration::from_secs(60);
/// Texts older than this when pSMS sees them are not announced.
pub const RECENT_SECS: i64 = 300;

static ACTIVE: Mutex<Option<(String, Instant)>> = Mutex::new(None);
static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Finds a one-time code: a 4 to 8 digit number (spaces or a dash allowed in the middle) in a
/// text that mentions a code, or the `@site #123456` autofill format.
pub fn code(text: &str) -> Option<String> {
    if let Some(bound) = text.lines().last().and_then(|l| l.trim().rsplit_once('#'))
        && bound.0.trim_start().starts_with('@')
    {
        let digits: String = bound.1.chars().take_while(|c| c.is_ascii_digit()).collect();
        if (4..=8).contains(&digits.len()) {
            return Some(digits);
        }
    }
    let lower = text.to_lowercase();
    const WORDS: [&str; 12] = [
        "code",
        "otp",
        "passcode",
        "verification",
        "verify",
        "pin",
        "one-time",
        "one time",
        "2fa",
        "token",
        "验证码",
        "認証",
    ];
    if !WORDS.iter().any(|w| lower.contains(w)) {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut best: Option<String> = None;
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let mut digits = String::new();
            let mut j = i;
            while j < chars.len() {
                if chars[j].is_ascii_digit() {
                    digits.push(chars[j]);
                    j += 1;
                } else if (chars[j] == ' ' || chars[j] == '-')
                    && j + 1 < chars.len()
                    && chars[j + 1].is_ascii_digit()
                    && digits.len() <= 4
                {
                    j += 1;
                } else {
                    break;
                }
            }
            let ends_clean = j >= chars.len() || !chars[j].is_ascii_alphanumeric();
            // Skip amounts, times and phone numbers next to symbols that suggest them.
            let before = if i > 0 { chars[i - 1] } else { ' ' };
            let money_or_time = matches!(before, '$' | '₱' | '€' | '£' | '+')
                || (j < chars.len() && matches!(chars[j], ':' | '%'))
                || (j + 1 < chars.len()
                    && matches!(chars[j], '.' | ',')
                    && chars[j + 1].is_ascii_digit());
            if ends_clean && !money_or_time && (4..=8).contains(&digits.len()) && best.is_none() {
                best = Some(digits);
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    best
}

/// Groups a code for reading, such as `123 456`.
pub fn spaced(code: &str) -> String {
    let half = code.len().div_ceil(2);
    if code.len() >= 6 {
        format!("{} {}", &code[..half], &code[half..])
    } else {
        code.to_string()
    }
}

/// Announces a code once per message: a notification, and Tab-to-type for a minute.
pub fn announce(message_id: &str, sender: &str, code: &str) {
    let Ok(mut seen) = SEEN.lock() else { return };
    if !seen
        .get_or_insert_with(HashSet::new)
        .insert(message_id.to_string())
    {
        return;
    }
    drop(seen);
    if let Ok(mut active) = ACTIVE.lock() {
        *active = Some((code.to_string(), Instant::now()));
    }
    platform::notify(message_id, sender, code);
}

/// The latest code if it is still fresh, taken so it is typed only once.
fn take_fresh() -> Option<String> {
    let mut active = ACTIVE.lock().ok()?;
    match active.take() {
        Some((code, at)) if at.elapsed() < FRESH => Some(code),
        _ => None,
    }
}
fn is_fresh() -> bool {
    ACTIVE
        .lock()
        .ok()
        .and_then(|a| a.as_ref().map(|(_, at)| at.elapsed() < FRESH))
        .unwrap_or(false)
}

/// A plain notification for a new text: the sender, then the message.
pub fn alert(id: &str, title: &str, body: &str) {
    platform::alert(id, title, body);
}

/// Whether new texts notify, on unless turned off in Settings.
pub fn alerts_enabled(db: &rusqlite::Connection) -> bool {
    db.query_row("SELECT enabled FROM desktop_alerts WHERE id=1", [], |r| {
        r.get::<_, bool>(0)
    })
    .unwrap_or(true)
}
pub fn set_alerts(db: &rusqlite::Connection, on: bool) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS desktop_alerts(id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL)",
    )?;
    db.execute(
        "INSERT INTO desktop_alerts(id,enabled) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled",
        [on],
    )
    .map(|_| ())
}

/// Sets up notifications and the Tab key. Call once at launch.
pub fn start() {
    platform::start();
}

/// The setting, on unless turned off in Settings.
pub fn enabled(db: &rusqlite::Connection) -> bool {
    db.query_row("SELECT enabled FROM desktop_codes WHERE id=1", [], |r| {
        r.get::<_, bool>(0)
    })
    .unwrap_or(true)
}
pub fn set_enabled(db: &rusqlite::Connection, on: bool) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS desktop_codes(id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL)")?;
    db.execute(
        "INSERT INTO desktop_codes(id,enabled) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled",
        [on],
    )
    .map(drop)
}

fn copy(code: &str) {
    use std::io::Write;
    #[cfg(target_os = "macos")]
    let program = ("/usr/bin/pbcopy", Vec::<&str>::new());
    #[cfg(target_os = "windows")]
    let program = ("clip", Vec::<&str>::new());
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let program = ("wl-copy", Vec::<&str>::new());
    if let Ok(mut child) = std::process::Command::new(program.0)
        .args(program.1)
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(code.as_bytes());
        }
        let _ = child.wait();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use block2::RcBlock;
    use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
    use core_graphics::event::{
        CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventType, EventField,
    };
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{MainThreadOnly, define_class, msg_send};
    use objc2_foundation::{NSArray, NSBundle, NSError, NSSet, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
        UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };
    use std::sync::OnceLock;

    const CATEGORY: &str = "psms.code";
    const TAB: i64 = 48;
    static CODES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    static NATIVE: OnceLock<bool> = OnceLock::new();

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PsmsCodeNotifications"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                handler.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                handler: &block2::DynBlock<dyn Fn()>,
            ) {
                let id = response.notification().request().identifier().to_string();
                let action = response.actionIdentifier().to_string();
                let code = CODES.lock().ok().and_then(|c| {
                    c.iter()
                        .find(|(i, _)| *i == id)
                        .map(|(_, code)| code.clone())
                });
                if let Some(code) = code {
                    match action.as_str() {
                        "copy" => copy(&code),
                        "type" => {
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_millis(250));
                                type_code(&code);
                            });
                        }
                        _ => {}
                    }
                }
                handler.call(());
            }
        }
    );

    /// Native notifications need an app bundle; development builds fall back to a plain banner.
    fn native() -> bool {
        *NATIVE.get_or_init(|| NSBundle::mainBundle().bundleIdentifier().is_some())
    }

    pub fn start() {
        if native()
            && let Some(mtm) = objc2::MainThreadMarker::new()
        {
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            // The center keeps a weak reference; the delegate lives as long as the app.
            std::mem::forget(delegate);
            let copy = UNNotificationAction::actionWithIdentifier_title_options(
                &NSString::from_str("copy"),
                &NSString::from_str("Copy code"),
                UNNotificationActionOptions::empty(),
            );
            let type_it = UNNotificationAction::actionWithIdentifier_title_options(
                &NSString::from_str("type"),
                &NSString::from_str("Type code"),
                UNNotificationActionOptions::empty(),
            );
            let category =
                UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
                    &NSString::from_str(CATEGORY),
                    &NSArray::from_retained_slice(&[type_it, copy]),
                    &NSArray::new(),
                    UNNotificationCategoryOptions::empty(),
                );
            center.setNotificationCategories(&NSSet::from_retained_slice(&[category]));
            let done = RcBlock::new(|_granted: Bool, _error: *mut NSError| {});
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &done,
            );
        }
        std::thread::spawn(watch_tab);
    }

    pub fn notify(id: &str, sender: &str, code: &str) {
        let title = format!("{} · code from {sender}", spaced(code));
        let body = "Press Tab to type it, or choose Copy code.";
        if !native() {
            let _ = std::process::Command::new("/usr/bin/osascript")
                .args([
                    "-e",
                    "on run argv\ndisplay notification (item 2 of argv) with title (item 1 of argv)\nend run",
                    "--",
                    &title,
                    body,
                ])
                .output();
            return;
        }
        if let Ok(mut codes) = CODES.lock() {
            codes.push((id.to_string(), code.to_string()));
            let excess = codes.len().saturating_sub(20);
            codes.drain(..excess);
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&title));
        content.setBody(&NSString::from_str(body));
        content.setCategoryIdentifier(&NSString::from_str(CATEGORY));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(id),
            &content,
            None,
        );
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, None);
    }

    pub fn alert(id: &str, title: &str, body: &str) {
        if !native() {
            let _ = std::process::Command::new("/usr/bin/osascript")
                .args([
                    "-e",
                    "on run argv\ndisplay notification (item 2 of argv) with title (item 1 of argv)\nend run",
                    "--",
                    title,
                    body,
                ])
                .output();
            return;
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&format!("text-{id}")),
            &content,
            None,
        );
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, None);
    }

    /// Types the code into the app in front, one character at a time.
    pub fn type_code(code: &str) {
        let Ok(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else {
            return;
        };
        for ch in code.chars() {
            for down in [true, false] {
                if let Ok(event) = CGEvent::new_keyboard_event(source.clone(), 0, down) {
                    event.set_string(&ch.to_string());
                    event.post(CGEventTapLocation::HID);
                }
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    }

    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    /// Watches the keyboard for Tab while a code is fresh. Needs Accessibility permission;
    /// without it, the notification buttons still work.
    fn watch_tab() {
        loop {
            if unsafe { AXIsProcessTrusted() } {
                break;
            }
            std::thread::sleep(Duration::from_secs(10));
        }
        let tap = CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            vec![CGEventType::KeyDown],
            |_proxy, _kind, event| {
                let plain = !event.get_flags().intersects(
                    CGEventFlags::CGEventFlagCommand
                        | CGEventFlags::CGEventFlagAlternate
                        | CGEventFlags::CGEventFlagControl
                        | CGEventFlags::CGEventFlagShift,
                );
                if plain
                    && event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) == TAB
                    && event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT) == 0
                    && is_fresh()
                    && let Some(code) = take_fresh()
                {
                    std::thread::spawn(move || type_code(&code));
                    return None;
                }
                Some(event.to_owned())
            },
        );
        let Ok(tap) = tap else { return };
        let Ok(source) = tap.mach_port.create_runloop_source(0) else {
            return;
        };
        let run_loop = CFRunLoop::get_current();
        unsafe { run_loop.add_source(&source, kCFRunLoopCommonModes) };
        tap.enable();
        CFRunLoop::run_current();
    }

    /// Asks macOS for Accessibility permission, which typing with Tab needs.
    pub fn request_typing_permission() {
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
    pub fn can_type() -> bool {
        unsafe { AXIsProcessTrusted() }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;
    pub fn start() {}
    /// A plain system notification with the code. Tab-to-type and buttons are macOS only.
    pub fn notify(_id: &str, sender: &str, code: &str) {
        let title = format!("{} · code from {sender}", spaced(code));
        #[cfg(target_os = "windows")]
        {
            let script = "param($t) Add-Type -AssemblyName System.Windows.Forms; $n = New-Object System.Windows.Forms.NotifyIcon; $n.Icon = [System.Drawing.SystemIcons]::Information; $n.Visible = $true; $n.ShowBalloonTip(15000, $t, 'Copied to the clipboard.', 'Info'); Start-Sleep 16; $n.Dispose()";
            let _ = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-WindowStyle",
                    "Hidden",
                    "-Command",
                    script,
                    &title,
                ])
                .spawn();
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = std::process::Command::new("notify-send")
                .args([title.as_str(), "Copied to the clipboard."])
                .spawn();
        }
        copy(code);
    }
    pub fn alert(_id: &str, title: &str, body: &str) {
        #[cfg(target_os = "windows")]
        {
            let script = "param($t, $b) Add-Type -AssemblyName System.Windows.Forms; $n = New-Object System.Windows.Forms.NotifyIcon; $n.Icon = [System.Drawing.SystemIcons]::Information; $n.Visible = $true; $n.ShowBalloonTip(10000, $t, $b, 'Info'); Start-Sleep 11; $n.Dispose()";
            let _ = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-WindowStyle",
                    "Hidden",
                    "-Command",
                    script,
                    title,
                    body,
                ])
                .spawn();
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = std::process::Command::new("notify-send")
                .args(["--app-name=pSMS", title, body])
                .spawn();
        }
    }
    pub fn request_typing_permission() {}
    pub fn can_type() -> bool {
        false
    }
}

pub use platform::{can_type, request_typing_permission};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_codes_and_skips_amounts_times_and_numbers() {
        for (text, want) in [
            (
                "Your GCash verification code is 482913. Do not share it.",
                Some("482913"),
            ),
            ("123456 is your Google verification code", Some("123456")),
            ("Use code 492-118 to sign in", Some("492118")),
            ("Your OTP: 7781", Some("7781")),
            ("Sign in to Example\n@example.com #602913", Some("602913")),
            ("您的验证码是 552031", Some("552031")),
            ("Your code expires at 10:45. Code: 30219", Some("30219")),
            ("Your balance is $1500 as of today", None),
            ("Call me at +639605659389 later", None),
            ("Meet at 1800 hours", None),
            ("Your verification amount is 1,500.00", None),
        ] {
            assert_eq!(code(text).as_deref(), want, "{text}");
        }
        assert_eq!(spaced("123456"), "123 456");
        assert_eq!(spaced("7781"), "7781");
    }
}
