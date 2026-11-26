#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod access;
#[path = "../../src/app_tokens.rs"]
#[allow(dead_code)]
mod app_tokens;
mod archive;
mod connect;
#[path = "../../src/hook_store.rs"]
mod hook_store;
mod hosted_login;
#[path = "../../src/onboard.rs"]
mod onboard;
#[path = "../../src/recovery.rs"]
mod recovery;
mod setup;
use archive::Thread;
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        input::{Input, InputState},
        *,
    },
    *,
};
use rusqlite::{Connection, OpenFlags};
use std::{path::PathBuf, sync::Arc, time::Duration};

const INK: u32 = 0x20232b;
const MUTED: u32 = 0x586373;
const BLUE: u32 = 0x2366c9;
const GUIDE: &str = "https://www.papamo.dev/docs/papamo-sms";
#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Inbox,
    Settings,
    Setup,
}
struct Dashboard {
    update: Option<String>,
    updating: bool,
    checking: bool,
    update_status: String,
    checked_at: Option<String>,
    db: Connection,
    path: PathBuf,
    external: bool,
    storage_engine: Option<&'static str>,
    storage_file: Entity<InputState>,
    migrating: bool,
    external_page: Option<(i64, bool, Vec<archive::Record>)>,
    external_newest: i64,
    screen: Screen,
    dark: bool,
    starred: bool,
    page: i64,
    older: bool,
    threads: Vec<Thread>,
    selected: Option<String>,
    search: Entity<InputState>,
    key: Entity<InputState>,
    endpoint: Entity<InputState>,
    form: connect::Form,
    delivery_url: Entity<InputState>,
    expiry: Option<u32>,
    hook_url: Entity<InputState>,
    hook_token: Entity<InputState>,
    session_key: Option<String>,
    unlock_box: bool,
    notice: String,
    step: u8,
    reached: u8,
    folder: Option<PathBuf>,
    baseline: i64,
    logo: Arc<Image>,
    data_version: i64,
}
fn button(id: &'static str, label: &'static str) -> Button {
    let b = Button::new(id).label(label);
    let icon = match id {
        "refresh" | "check-updates" => IconName::RefreshCw,
        "disconnect" => IconName::Ban,
        "config" => IconName::Settings,
        "prev" | "back" => IconName::ChevronLeft,
        "next" | "verify" => IconName::ChevronRight,
        "copy-secret" | "command" => IconName::Copy,
        "connect" | "start" | "setup" | "prepare" | "save-hook" => IconName::Network,
        "activate" | "done" | "token-done" => IconName::Check,
        "filter" => IconName::Star,
        "unlock-switch" | "unlock" => {
            if label == "Lock archive" {
                IconName::EyeOff
            } else {
                IconName::Eye
            }
        }
        "files" => IconName::FolderOpen,
        "help" | "api-docs" => IconName::BookOpen,
        "return" => IconName::Inbox,
        "add-destination" => IconName::Plus,
        "direct" | "cf-login" | "ngrok-account" => IconName::Globe,
        "appearance" => {
            if label == "Use light mode" {
                IconName::Sun
            } else {
                IconName::Moon
            }
        }
        _ => return b,
    };
    b.icon(icon)
}
impl Dashboard {
    fn check_updates(&mut self, cx: &mut Context<Self>) {
        if self.checking || self.updating {
            return;
        }
        self.checking = true;
        self.update_status = "Checking for updates…".into();
        let path = self.path.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let out = setup::cli(&path)
                        .and_then(|mut cmd| cmd.args(["update", "--prepare"]).output())
                        .map_err(|e| e.to_string())?;
                    if !out.status.success() {
                        return Err(String::from_utf8_lossy(&out.stderr)
                            .trim()
                            .trim_start_matches("Error: ")
                            .to_string());
                    }
                    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.checking = false;
                s.checked_at =
                    s.db.query_row(
                        "SELECT strftime('%Y-%m-%d %H:%M','now','localtime')",
                        [],
                        |r| r.get(0),
                    )
                    .ok();
                match result {
                    Ok(v) if v.is_empty() => {
                        s.update = None;
                        s.update_status = "You’re up to date.".into();
                    }
                    Ok(v) => {
                        s.update_status = format!(
                            "Version {v} is ready. Restart now or reopen pSMS to apply it."
                        );
                        s.update = Some(v);
                    }
                    Err(e) => s.update_status = format!("Update check failed. {e}"),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn color(&self, light: u32) -> Rgba {
        let dark = match light {
            INK => 0xf3f4f7,
            MUTED => 0xb5bdca,
            0xffffff => 0x191b20,
            0xf1f3f6 | 0xf0f3f8 => 0x25282f,
            0xf9fafb | 0xf3f4f6 => 0x22252b,
            0xdce0e5 | 0xd3d8df | 0xd9dfe7 => 0x41454e,
            0xd7dce3 | 0xdce2ea => 0x414854,
            0xe9eaee => 0x32363d,
            0xe8f0ff => 0x24334a,
            other => other,
        };
        rgb(if self.dark { dark } else { light })
    }
    fn heading(&self, text: impl Into<SharedString>) -> Div {
        div()
            .text_2xl()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(self.color(INK))
            .child(text.into())
    }
    fn caption(&self, text: impl Into<SharedString>) -> Div {
        div()
            .text_sm()
            .text_color(self.color(MUTED))
            .child(text.into())
    }

    fn refresh(&mut self) {
        let page = if self.external {
            let records = self
                .external_page
                .as_ref()
                .filter(|(page, star, _)| *page == self.page && *star == self.starred)
                .map(|(_, _, items)| items.clone())
                .unwrap_or_default();
            Ok(archive::group(records, self.session_key.as_deref()))
        } else {
            archive::read(
                &self.db,
                self.page,
                self.starred,
                self.session_key.as_deref(),
            )
        };
        match page {
            Ok((threads, older)) => {
                if !threads
                    .iter()
                    .any(|t| Some(&t.key) == self.selected.as_ref())
                {
                    self.selected = threads.first().map(|t| t.key.clone())
                }
                self.threads = threads;
                self.older = older;
            }
            Err(_) => self.notice = "Could not read this archive. Refresh to try again.".into(),
        }
    }
    fn show(&mut self, screen: Screen, cx: &mut Context<Self>) {
        self.screen = screen;
        self.notice.clear();
        self.unlock_box = false;
        self.refresh();
        cx.notify();
    }
    fn toggle_filter(&mut self, cx: &mut Context<Self>) {
        let next = !self.starred;
        match self.db.execute("INSERT INTO desktop_preferences(id,starred_only) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET starred_only=excluded.starred_only",[next]){
            Ok(_)=>{self.starred=next;self.page=0;self.refresh()},Err(_)=>self.notice="Could not save inbox preference.".into(),
        }
        cx.notify();
    }
    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut list = div()
            .id("thread-list")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .gap_1();
        let mut shown = 0;
        for (i, t) in self.threads.iter().enumerate() {
            if !query.is_empty()
                && !t.title.to_lowercase().contains(&query)
                && !t
                    .items
                    .iter()
                    .any(|m| !m.locked && m.text.to_lowercase().contains(&query))
            {
                continue;
            }
            shown += 1;
            let m = &t.items[0];
            let selected = self.selected.as_ref() == Some(&t.key);
            let initials = if m.locked {
                "•".into()
            } else {
                t.title
                    .split_whitespace()
                    .take(2)
                    .filter_map(|s| s.chars().next())
                    .collect::<String>()
            };
            let preview: String = if m.locked {
                "Unlock to read".into()
            } else {
                m.text
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(70)
                    .collect()
            };
            let key = t.key.clone();
            list = list.child(
                div()
                    .id(("conversation", i))
                    .h_flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .cursor_pointer()
                    .bg(self.color(if selected { BLUE } else { 0xf1f3f6 }))
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.selected = Some(key.clone());
                        s.show(Screen::Inbox, cx)
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(self.color(if selected { 0x467ed3 } else { 0xd7dce3 }))
                            .text_color(self.color(if selected { 0xfffffe } else { MUTED }))
                            .child(initials),
                    )
                    .child(
                        div()
                            .v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(self.color(if selected { 0xfffffe } else { INK }))
                                    .child(t.title.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .text_color(self.color(if selected { 0xe6efff } else { MUTED }))
                                    .child(preview),
                            ),
                    ),
            );
        }
        if shown == 0 {
            list = list.child(div().p_3().child(self.caption(if query.is_empty() {
                "Your conversations will appear here."
            } else {
                "No matches on this page."
            })))
        }
        div()
            .v_flex()
            .w(px(278.))
            .h_full()
            .flex_shrink_0()
            .bg(self.color(0xf1f3f6))
            .border_r_1()
            .border_color(self.color(0xd3d8df))
            .child(
                div()
                    .v_flex()
                    .p_4()
                    .gap_3()
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .child(img(self.logo.clone()).size(px(30.)))
                            .child(self.heading("Messages")),
                    )
                    .child(Input::new(&self.search))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                button(
                                    "filter",
                                    if self.starred {
                                        "Starred only"
                                    } else {
                                        "All messages"
                                    },
                                )
                                .on_click(cx.listener(|s, _, _, cx| s.toggle_filter(cx))),
                            )
                            .child(button("refresh", "Refresh").ghost().on_click(cx.listener(
                                |s, _, _, cx| {
                                    s.refresh();
                                    cx.notify()
                                },
                            ))),
                    ),
            )
            .child(list)
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .child(
                        button("prev", "Newer")
                            .ghost()
                            .disabled(self.page == 0)
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.page = (s.page - 1).max(0);
                                s.refresh();
                                cx.notify()
                            })),
                    )
                    .child(
                        button("next", "Older")
                            .ghost()
                            .disabled(!self.older)
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.page += 1;
                                s.refresh();
                                cx.notify()
                            })),
                    ),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(self.color(0xd3d8df))
                    .p_3()
                    .child(if self.screen == Screen::Settings {
                        button("return", "Back to inbox")
                            .on_click(cx.listener(|s, _, _, cx| s.show(Screen::Inbox, cx)))
                    } else {
                        button("config", "Settings")
                            .ghost()
                            .on_click(cx.listener(|s, _, _, cx| s.show(Screen::Settings, cx)))
                    }),
            )
    }
    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.key.read(cx).value().to_string();
        let Ok(key) = recovery::key_from_backup(&key) else {
            self.notice =
                "Enter the 12 words from your recovery kit or your original archive key.".into();
            cx.notify();
            return;
        };
        let valid=self.db.query_row("SELECT body FROM messages WHERE sender='papamo:encrypted:v1' ORDER BY rowid DESC LIMIT 1",[],|r|r.get::<_,String>(0));
        if valid.is_ok()
            && !self
                .threads
                .iter()
                .flat_map(|t| &t.items)
                .any(|m| m.locked && !m.pgp && archive::decrypt(&key, &m.text).is_some())
            && valid.is_ok_and(|v| archive::decrypt(&key, &v).is_none())
        {
            self.notice = "This key could not unlock a message on this page.".into();
            cx.notify();
            return;
        }
        self.session_key = Some(key);
        self.key.update(cx, |i, cx| i.set_value("", window, cx));
        self.unlock_box = false;
        self.notice = "Archive key loaded for this window.".into();
        self.refresh();
        cx.notify();
    }
    fn inbox(&self, cx: &mut Context<Self>) -> Div {
        let thread = self
            .threads
            .iter()
            .find(|t| Some(&t.key) == self.selected.as_ref());
        let mut pane = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(self.color(0xffffff));
        let title = thread
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "Inbox".into());
        pane = pane.child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .px_5()
                .py_3()
                .bg(self.color(0xf9fafb))
                .border_b_1()
                .border_color(self.color(0xdce0e5))
                .child(
                    div()
                        .text_lg()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    button(
                        "unlock-switch",
                        if self.session_key.is_some() {
                            "Lock archive"
                        } else {
                            "Unlock archive"
                        },
                    )
                    .on_click(cx.listener(|s, _, window, cx| {
                        if s.session_key.take().is_some() {
                            s.key.update(cx, |i, cx| i.set_value("", window, cx));
                            s.refresh();
                            s.notice.clear()
                        } else {
                            s.unlock_box = !s.unlock_box
                        }
                        cx.notify()
                    })),
                ),
        );
        if self.unlock_box {
            pane = pane.child(
                div()
                    .p_4()
                    .gap_2()
                    .v_flex()
                    .child(
                        self.caption("Enter your Actions recovery key. It stays in this window."),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.key)))
                            .child(
                                button("unlock", "Unlock")
                                    .primary()
                                    .on_click(cx.listener(|s, _, w, cx| s.unlock(w, cx))),
                            ),
                    ),
            )
        }
        if let Some(t) = thread {
            let mut feed = div()
                .id("reading-pane")
                .v_flex()
                .min_h_0()
                .flex_1()
                .overflow_y_scroll()
                .p_6()
                .gap_4();
            for (i, m) in t.items.iter().rev().enumerate() {
                let id = m.id.clone();
                let starred = m.starred;
                let text = if m.locked {
                    if m.pgp {
                        "Encrypted with OpenPGP".into()
                    } else {
                        "This message is encrypted.".into()
                    }
                } else {
                    m.text.clone()
                };
                let mut actions=div().h_flex().items_center().gap_2().child(self.caption(m.time.clone()))
                    .child(Button::new(("star",i)).icon(if starred{IconName::StarOff}else{IconName::Star}).ghost().label(if starred{"Unstar"}else{"Star"}).on_click(cx.listener(move|s,_,_,cx|{
                        if !(if s.external { setup::cli(&s.path).and_then(|mut cmd|cmd.args(["storage","desktop-star",&id,&(!starred).to_string()]).output()).is_ok_and(|out|out.status.success()) } else {s.db.execute("INSERT INTO message_flags(message_id,starred) VALUES(?1,?2) ON CONFLICT(message_id) DO UPDATE SET starred=excluded.starred",(&id,!starred)).is_ok()}) {s.notice="Could not change this star.".into()}s.refresh();cx.notify()
                    })));
                if m.pgp {
                    let text = m.text.clone();
                    actions = actions.child(
                        Button::new(("copy", i))
                            .icon(IconName::Copy)
                            .ghost()
                            .label("Copy ciphertext")
                            .on_click(cx.listener(move |s, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                s.notice = "Ciphertext copied.".into();
                                cx.notify()
                            })),
                    )
                }
                feed = feed.child(
                    div().v_flex().items_start().gap_1().child(actions).child(
                        div()
                            .max_w(px(520.))
                            .px_4()
                            .py_2()
                            .rounded_xl()
                            .bg(self.color(0xe9eaee))
                            .child(text),
                    ),
                );
            }
            pane = pane.child(feed);
        } else {
            pane = pane.child(
                div()
                    .v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .p_5()
                    .child(img(self.logo.clone()).size(px(60.)))
                    .child(self.heading(if self.starred {
                        "No starred messages"
                    } else {
                        "Your inbox is ready"
                    }))
                    .child(self.caption(if self.starred {
                        "Star a message to find it here."
                    } else {
                        "Messages appear here when a connected device sends them."
                    }))
                    .child(
                        button("connect", "Connect a phone")
                            .on_click(cx.listener(|s, _, _, cx| s.open_setup(cx))),
                    ),
            );
        }
        pane.child(
            div()
                .border_t_1()
                .border_color(self.color(0xdce0e5))
                .p_3()
                .child(
                    div()
                        .rounded_full()
                        .bg(self.color(0xf3f4f6))
                        .px_4()
                        .py_2()
                        .text_color(self.color(MUTED))
                        .child("Reply is a upcoming feature"),
                ),
        )
    }
}
impl Render for Dashboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = match self.screen {
            Screen::Setup => self.setup_view(cx),
            Screen::Inbox => div()
                .h_flex()
                .size_full()
                .child(self.sidebar(cx))
                .child(self.inbox(cx)),
            Screen::Settings => div()
                .h_flex()
                .size_full()
                .child(self.sidebar(cx))
                .child(self.settings_view(window.viewport_size().width >= px(1280.), cx)),
        };
        let mut root = div()
            .v_flex()
            .size_full()
            .bg(self.color(0xffffff))
            .text_color(self.color(INK))
            .child(div().flex_1().min_h_0().child(layout));
        if let Some(version) = &self.update {
            root=root.child(div().h_flex().gap_3().px_4().py_2().items_center()
                .child(format!("pSMS {version} is ready to install."))
                .child(button("apply-update","Restart to update").disabled(self.updating)
                .on_click(cx.listener(|s,_,_,cx|{
                    let Some(selected)=s.update.clone() else{return};
s.updating=true;cx.notify();
                    let path=s.path.clone();
                    cx.spawn(async move|this,cx|{
                        let ok=cx.background_executor().spawn(async move{
                            setup::cli(&path).and_then(|mut cmd|cmd.args(["update","--restart",&selected]).output()).is_ok_and(|o|o.status.success())
                        }).await;
                        let _=this.update(cx,|s,cx|{if ok{cx.quit()}else{s.updating=false;s.notice="Could not apply the update. Try again or run psms update.".into();cx.notify()}});
                    }).detach()
                }))));
        }
        if !self.notice.is_empty() {
            root = root.child(
                div()
                    .px_4()
                    .py_2()
                    .bg(self.color(0xe8f0ff))
                    .text_sm()
                    .child(self.notice.clone()),
            )
        }
        root
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Desktop shortcuts launch this GUI binary directly. The CLI owns archive initialization.
    if std::env::args_os().len() == 1 {
        let path = std::env::current_exe()?
            .with_file_name(format!("psms{}", std::env::consts::EXE_SUFFIX));
        let mut cli = std::process::Command::new(path);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cli.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        cli.arg("desktop").spawn()?;
        return Ok(());
    }
    let mut args = std::env::args_os().skip(1);
    let settings = args.next().is_some_and(|s| s == "settings");
    let path = PathBuf::from(
        args.next()
            .ok_or("open this application using psms desktop or psms settings")?,
    );
    let suffix = |ext: &str| {
        let mut s = path.as_os_str().to_os_string();
        s.push(ext);
        PathBuf::from(s)
    };
    let external = suffix(".storage.json").try_exists()?;
    let settings_path = if external {
        suffix(".settings.sqlite3")
    } else {
        path.clone()
    };
    let db = Connection::open_with_flags(&settings_path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS desktop_preferences(id INTEGER PRIMARY KEY,starred_only INTEGER NOT NULL)")?;
    let only_starred = db.query_row(
        "SELECT coalesce((SELECT starred_only FROM desktop_preferences WHERE id=1),0)",
        [],
        |r| r.get(0),
    )?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS desktop_setup(id INTEGER PRIMARY KEY,step INTEGER DEFAULT 0,completed INTEGER DEFAULT 0,baseline INTEGER DEFAULT 0,folder TEXT);INSERT OR IGNORE INTO desktop_setup(id,completed) SELECT 1,EXISTS(SELECT 1 FROM messages)")?;
    let (step, completed, baseline, folder) = db.query_row(
        "SELECT step,completed,baseline,folder FROM desktop_setup WHERE id=1",
        [],
        |r| {
            Ok((
                r.get::<_, u8>(0)?,
                r.get::<_, bool>(1)?,
                r.get(2)?,
                r.get::<_, Option<String>>(3)?.map(PathBuf::from),
            ))
        },
    )?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS desktop_setup_progress(id INTEGER PRIMARY KEY CHECK(id=1),reached INTEGER NOT NULL);
        INSERT OR IGNORE INTO desktop_setup_progress SELECT id,max(step,CASE WHEN folder IS NULL THEN 0 ELSE 2 END) FROM desktop_setup;
        CREATE TRIGGER IF NOT EXISTS desktop_setup_reached AFTER UPDATE OF step ON desktop_setup BEGIN
        UPDATE desktop_setup_progress SET reached=max(reached,NEW.step) WHERE id=NEW.id; END;")?;
    let reached: u8 = db.query_row(
        "SELECT reached FROM desktop_setup_progress WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS desktop_appearance(id INTEGER PRIMARY KEY CHECK(id=1),dark INTEGER NOT NULL)")?;
    let preference = db.query_row(
        "SELECT (SELECT dark FROM desktop_appearance WHERE id=1)",
        [],
        |r| r.get::<_, Option<bool>>(0),
    )?;
    // Hosted credentials last for this app session. Resume at the account choice
    // if storage selection was interrupted, including setups from older releases.
    let step = if step == 1 { 0 } else { step };
    let data_version = db.query_row("PRAGMA data_version", [], |r| r.get(0))?;
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let preferred = size(px(900.), px(620.));
            let size = cx
                .primary_display()
                .map(|display| {
                    let available = display.visible_bounds().size;
                    size(
                        preferred.width.min(available.width * 0.90),
                        preferred.height.min(available.height * 0.90),
                    )
                })
                .unwrap_or(preferred);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("pSMS · Papamo SMS".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let dark = preference
                        .unwrap_or_else(|| ThemeMode::from(window.appearance()).is_dark());
                    Theme::change(
                        if dark {
                            ThemeMode::Dark
                        } else {
                            ThemeMode::Light
                        },
                        Some(window),
                        cx,
                    );
                    let key = cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder("Actions archive key")
                            .masked(true)
                    });
                    let search =
                        cx.new(|cx| InputState::new(window, cx).placeholder("Search this page"));
                    let endpoint = cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder("archive.example.com")
                    });
                    let hook_url =
                        cx.new(|cx| InputState::new(window, cx).placeholder("Receiver URL"));
                    let delivery_url = cx.new(|cx| {
                        InputState::new(window, cx).placeholder("https://example.com/webhook")
                    });
                    let hook_token = cx.new(|cx| InputState::new(window, cx).masked(true));
                    cx.new(|cx: &mut Context<Dashboard>| {
                        cx.subscribe(
                            &search,
                            |_, _, event: &gpui_kit::component::input::InputEvent, cx| {
                                if matches!(event, gpui_kit::component::input::InputEvent::Change) {
                                    cx.notify()
                                }
                            },
                        )
                        .detach();
                        cx.spawn(async move |this,cx|{
                            loop {
                                if this.update(cx,|s,cx|s.check_updates(cx)).is_err(){break}
                                cx.background_executor().timer(Duration::from_secs(3600)).await;
                            }
                        }).detach();
                        cx.spawn(async move |this, cx| {
                            loop {
                                let Ok(params)=this.read_with(cx,|s,_|s.external.then(||(s.path.clone(),s.page,s.starred))) else {break};
                                if let Some((path,page,starred))=params {
                                    let result=cx.background_executor().spawn(async move {
                                        let mut cmd=setup::cli(&path).ok()?;
                                        cmd.args(["storage","desktop-page",&page.to_string()]);
                                        if starred {cmd.arg("--starred");}
                                        let out=cmd.output().ok()?;
                                        if !out.status.success(){return None;}
                                        serde_json::from_slice::<serde_json::Value>(&out.stdout).ok()
                                    }).await;
                                    if this.update(cx,|s,cx|{
                                        if s.page!=page || s.starred!=starred{return;}
                                        if let Some(v)=result.and_then(|v| {
                                            let mut records=Vec::new();
                                            for r in v["messages"].as_array()? {
                                                let time=s.db.query_row("SELECT strftime('%Y-%m-%d %H:%M',?1,'unixepoch','localtime')",[r["archived_at"].as_i64()?],|r|r.get(0)).ok()?;
                                                records.push((r["id"].as_str()?.into(),r["sender"].as_str()?.into(),r["body"].as_str()?.into(),time,r["starred"].as_i64()==Some(1)));
                                            }
                                            Some((records,v["newest"].as_i64()?))
                                        }) {s.external_page=Some((page,starred,v.0));s.external_newest=v.1;} else {s.external_page=None;s.notice="Could not reach the selected database.".into();}
                                        s.refresh();cx.notify();
                                    }).is_err(){break;}
                                }
                                cx.background_executor().timer(Duration::from_secs(4)).await;
                                if this
                                    .update(cx, |s, cx| {
                                        if let Ok(v) =
                                            s.db.query_row("PRAGMA data_version", [], |r| {
                                                r.get::<_, i64>(0)
                                            })
                                            && s.data_version != v
                                        {
                                            s.data_version = v;
                                            s.refresh();
                                            cx.notify();
                                        }
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                        let storage_file=cx.new(|cx|InputState::new(window,cx).placeholder("Path to private connection file"));
                        let form=connect::Form::new(&db,&endpoint,window,cx);
                        let mut s = Dashboard {form, update:None, updating:false, checking:false, update_status:"Waiting to check.".into(), checked_at:None,
                            storage_engine:None,storage_file,migrating:false,
                            db,
                            external, external_page:None, external_newest:0,
                            path,
                            screen: if settings {
                                Screen::Settings
                            } else if completed {
                                Screen::Inbox
                            } else {
                                Screen::Setup
                            },
                            dark,
                            starred: only_starred,
                            page: 0,
                            older: false,
                            threads: Vec::new(),
                            selected: None,
                            search,
                            key,
                            endpoint,
                            hook_url,
                            hook_token,
                            delivery_url,
                            expiry: Some(30),
                            session_key: None,
                            unlock_box: false,
                            notice: String::new(),
                            reached,
                            step,
                            folder,
                            baseline,
                            logo: Arc::new(Image::from_bytes(
                                ImageFormat::Png,
                                include_bytes!("../../assets/psms-mark.png").to_vec(),
                            )),
                            data_version,
                        };
                        if s.screen==Screen::Setup && s.step==1 {s.detect_domain(window,cx);}
                        s.refresh();
                        s
                    })
                },
            )
            .expect("could not create the native pSMS window");
            cx.activate(true);
        });
    Ok(())
}
