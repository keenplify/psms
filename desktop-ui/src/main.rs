#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod access;
mod api;
#[path = "../../src/app_tokens.rs"]
#[allow(dead_code)]
mod app_tokens;
mod archive;
mod cards;
mod connect;
#[path = "../../src/device_api.rs"]
mod device_api;
#[path = "../../src/hook_store.rs"]
mod hook_store;
mod hosted_login;
#[path = "../../src/kit.rs"]
mod kit;
#[path = "../../src/onboard.rs"]
mod onboard;
mod otp;
#[path = "../../src/recovery.rs"]
mod recovery;
mod relay;
mod replies;
mod setup;
use archive::Thread;
use gpui_kit::prelude::FluentBuilder as _;
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
    /// Reply from anywhere: recovery words, a self-hosted URL and its token.
    relay_words: Entity<InputState>,
    relay_url: Entity<InputState>,
    relay_token: Entity<InputState>,
    relay_kit: Option<String>,
    relay_busy: bool,
    /// The newest message already checked for one-time codes.
    code_rowid: i64,
    /// A papamo.dev inbox shown instead of the local archive, through the relay's connection.
    hosted: Option<Arc<relay::Relay>>,
    hosted_cards: std::collections::HashMap<String, cards::Entry>,
    /// What the shown hosted page reflects: newest message time, page and star filter.
    hosted_seen: Option<(i64, i64, bool)>,
    /// A computer token from setup, waiting for the recovery words to open the hosted inbox.
    pending_hosted: Option<String>,
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
    /// Decrypted contact cards for the unlocked archive, keyed by contact ID.
    cards: std::collections::HashMap<String, cards::Entry>,
    card_open: bool,
    card_editing: bool,
    /// Name, company, job title and notes in the contact editor.
    card_fields: [Entity<InputState>; 4],
    /// Label and value inputs for each phone number and email being edited.
    phone_rows: Vec<(Entity<InputState>, Entity<InputState>)>,
    email_rows: Vec<(Entity<InputState>, Entity<InputState>)>,
    /// The photo being edited: None keeps the card's photo, Some(None) removes it.
    card_photo: Option<Option<(String, String)>>,
    reply: Entity<InputState>,
    sending: bool,
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

    /// Announces one-time codes in texts that arrived since the last check.
    fn check_codes(&mut self) {
        if self.external || self.hosted.is_some() {
            return;
        }
        let Some(key) = self.session_key.clone() else {
            return;
        };
        let since = self.code_rowid;
        let rows: Vec<(i64, String, String)> = self
            .db
            .prepare("SELECT rowid,id,body FROM messages WHERE rowid>?1 AND sender='papamo:encrypted:v1' AND archived_at>unixepoch()-?2 ORDER BY rowid")
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![since, otp::RECENT_SECS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect()
            })
            .unwrap_or_default();
        if let Ok(newest) =
            self.db
                .query_row("SELECT coalesce(max(rowid),0) FROM messages", [], |r| {
                    r.get(0)
                })
        {
            self.code_rowid = newest;
        }
        if !otp::enabled(&self.db) {
            return;
        }
        for (_, id, body) in rows {
            if let Some((sender, text, false)) = archive::open_message(&key, &body)
                && let Some(code) = otp::code(&text)
            {
                otp::announce(&id, &sender, &code);
            }
        }
    }
    fn refresh(&mut self) {
        self.check_codes();
        let page = if self.external || self.hosted.is_some() {
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
                self.cards = match (&self.session_key, self.external) {
                    _ if self.hosted.is_some() => self.hosted_cards.clone(),
                    (Some(key), false) => cards::load(&self.db, key),
                    _ => Default::default(),
                };
                self.mark_selected_read();
            }
            Err(_) => self.notice = "Could not read this archive. Refresh to try again.".into(),
        }
    }
    /// The open conversation counts as read once its messages can be read.
    fn mark_selected_read(&mut self) {
        if (self.external && self.hosted.is_none())
            || self.session_key.is_none()
            || self.screen != Screen::Inbox
        {
            return;
        }
        let Some(thread) = self
            .threads
            .iter_mut()
            .find(|t| Some(&t.key) == self.selected.as_ref())
        else {
            return;
        };
        let unread: Vec<String> = thread
            .items
            .iter()
            .filter(|m| !m.read && !m.locked)
            .map(|m| m.id.clone())
            .collect();
        if unread.is_empty() {
            return;
        }
        let marked = match &self.hosted {
            Some(hosted) => hosted.mark_read(&unread).is_ok(),
            None => self.api().is_some_and(|api| api.mark_read(&unread).is_ok()),
        };
        if marked
            && let Some(thread) = self
                .threads
                .iter_mut()
                .find(|t| Some(&t.key) == self.selected.as_ref())
        {
            for mail in thread.items.iter_mut().filter(|m| unread.contains(&m.id)) {
                mail.read = true;
            }
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
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(120)
                    .collect()
            };
            let unread = t.items.iter().any(|m| !m.read && !m.locked);
            let when = m.when.clone();
            let key = t.key.clone();
            let entry = t.contact.as_ref().and_then(|c| self.cards.get(c));
            let name = entry
                .map(|e| e.card.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| t.title.clone());
            list = list.child(
                div()
                    .id(("conversation", i))
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .pl_1()
                    .pr_3()
                    .py_2()
                    .rounded_lg()
                    .cursor_pointer()
                    .bg(if selected {
                        rgb(0x0a7cff)
                    } else {
                        self.color(0xf1f3f6)
                    })
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.selected = Some(key.clone());
                        s.show(Screen::Inbox, cx)
                    }))
                    .child(
                        // Unread conversations get a blue dot, as in Messages.
                        div()
                            .flex_shrink_0()
                            .size(px(8.))
                            .rounded_full()
                            .when(unread && !selected, |d| d.bg(rgb(0x0a7cff))),
                    )
                    .child(self.avatar(entry, initials, 40., selected))
                    .child(
                        div()
                            .v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(self.color(if selected {
                                                0xfffffe
                                            } else {
                                                INK
                                            }))
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(self.color(if selected {
                                                0xe6efff
                                            } else {
                                                MUTED
                                            }))
                                            .child(when),
                                    ),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .line_clamp(2)
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
    fn avatar(
        &self,
        entry: Option<&cards::Entry>,
        initials: String,
        size: f32,
        selected: bool,
    ) -> Div {
        let base = div()
            .flex_shrink_0()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .overflow_hidden();
        if let Some(image) = entry.and_then(|e| e.image.clone()) {
            return base.child(img(image).size(px(size)).rounded_full());
        }
        let initials = entry
            .map(|e| {
                e.card
                    .name
                    .split_whitespace()
                    .take(2)
                    .filter_map(|w| w.chars().next())
                    .collect::<String>()
                    .to_uppercase()
            })
            .filter(|i| !i.is_empty())
            .or_else(|| {
                Some(initials).filter(|i| i.chars().all(char::is_alphabetic) && !i.is_empty())
            });
        let base = base
            .bg(self.color(if selected { 0x467ed3 } else { 0xa2a2a7 }))
            .text_color(rgb(0xffffff))
            .font_weight(FontWeight::SEMIBOLD);
        // No name to abbreviate: show a person, as Messages does for unknown numbers.
        match initials {
            Some(letters) => base.child(letters),
            None => base.child(Icon::new(IconName::User).size(px(size * 0.55))),
        }
    }
    fn edit_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self
            .threads
            .iter()
            .find(|t| Some(&t.key) == self.selected.as_ref())
        else {
            return;
        };
        let card = t
            .contact
            .as_ref()
            .and_then(|c| self.cards.get(c))
            .map(|e| e.card.clone())
            .unwrap_or_else(|| cards::Card {
                phones: vec![cards::Labeled {
                    label: "mobile".into(),
                    value: t.title.clone(),
                }],
                ..Default::default()
            });
        let values = [card.name, card.org, card.title, card.note];
        for (field, value) in self.card_fields.clone().iter().zip(values) {
            field.update(cx, |i, cx| i.set_value(value, window, cx));
        }
        self.phone_rows.clear();
        self.email_rows.clear();
        self.card_photo = None;
        for item in &card.phones {
            self.add_row(true, item, window, cx);
        }
        for item in &card.emails {
            self.add_row(false, item, window, cx);
        }
        self.card_editing = true;
        cx.notify();
    }
    /// Adds a label/value pair to the phone or email list in the contact editor.
    fn add_row(
        &mut self,
        phone: bool,
        item: &cards::Labeled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if (if phone {
            &self.phone_rows
        } else {
            &self.email_rows
        })
        .len()
            >= 10
        {
            return;
        }
        let (label_hint, value_hint) = if phone {
            ("mobile", "+1 555 0100")
        } else {
            ("home", "name@example.com")
        };
        let label = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(label_hint)
                .default_value(item.label.clone())
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(value_hint)
                .default_value(item.value.clone())
        });
        for field in [&label, &value] {
            cx.subscribe(
                field,
                |_, _, event: &gpui_kit::component::input::InputEvent, cx| {
                    if matches!(event, gpui_kit::component::input::InputEvent::Change) {
                        cx.notify()
                    }
                },
            )
            .detach();
        }
        if phone {
            self.phone_rows.push((label, value));
        } else {
            self.email_rows.push((label, value));
        }
        cx.notify();
    }
    fn save_card(&mut self, reset: bool, cx: &mut Context<Self>) {
        let (Some(key), Some(contact)) = (
            self.session_key.clone(),
            self.threads
                .iter()
                .find(|t| Some(&t.key) == self.selected.as_ref())
                .and_then(|t| t.contact.clone()),
        ) else {
            self.notice = "This conversation can't store contact details.".into();
            cx.notify();
            return;
        };
        let card = (!reset).then(|| {
            let read = |i: usize| self.card_fields[i].read(cx).value().trim().to_string();
            let rows = |rows: &[(Entity<InputState>, Entity<InputState>)], fallback: &str| {
                rows.iter()
                    .map(|(label, value)| cards::Labeled {
                        label: Some(label.read(cx).value().trim().to_string())
                            .filter(|l| !l.is_empty())
                            .unwrap_or_else(|| fallback.into()),
                        value: value.read(cx).value().trim().to_string(),
                    })
                    .filter(|l| !l.value.is_empty())
                    .collect::<Vec<_>>()
            };
            cards::Card {
                name: read(0),
                org: read(1),
                title: read(2),
                phones: rows(&self.phone_rows, "mobile"),
                emails: rows(&self.email_rows, "email"),
                note: read(3),
                photo: match &self.card_photo {
                    Some(choice) => choice.clone(),
                    None => self.cards.get(&contact).and_then(|e| e.card.photo.clone()),
                },
            }
        });
        let saved = match (cards::envelope(&key, card.as_ref()), self.api()) {
            (Err(e), _) => Err(e),
            (Ok(envelope), _) if self.hosted.is_some() => {
                self.hosted_seen = None;
                self.hosted
                    .as_ref()
                    .map(|h| h.save_card(&contact, envelope))
                    .unwrap_or(Ok(()))
                    .map_err(|_| "Could not save this contact.")
            }
            (Ok(_), None) => Err("Start the archive service to save contacts."),
            (Ok(envelope), Some(api)) => api
                .save_card(&contact, envelope)
                .map_err(|_| "Could not save this contact."),
        };
        match saved {
            Ok(()) => {
                self.card_editing = false;
                self.notice = if reset {
                    "Showing your phone's details."
                } else {
                    "Contact saved."
                }
                .into();
                self.refresh();
            }
            Err(e) => self.notice = e.into(),
        }
        cx.notify();
    }
    fn choose_photo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose photo".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let photo = cx
                .background_executor()
                .spawn(async move { cards::photo_from_file(&path) })
                .await;
            let _ = this.update(cx, |s, cx| {
                match photo {
                    Ok(photo) => s.card_photo = Some(Some(photo)),
                    Err(e) => s.notice = e.into(),
                }
                cx.notify()
            });
        })
        .detach();
    }
    /// Saves a fresh copy of the recovery kit while the archive is unlocked.
    fn save_kit(&mut self, cx: &mut Context<Self>) {
        let Some(entropy) = self.session_key.as_ref().and_then(|k| {
            (k.len() == 32 && k.bytes().all(|b| b.is_ascii_hexdigit())).then(|| {
                let mut out = [0u8; 16];
                for (i, b) in out.iter_mut().enumerate() {
                    *b = u8::from_str_radix(&k[i * 2..i * 2 + 2], 16).unwrap_or_default();
                }
                out
            })
        }) else {
            self.notice = "Unlock with your 12 words to save a recovery kit.".into();
            cx.notify();
            return;
        };
        let folder = dirs_downloads();
        let target = cx.prompt_for_new_path(&folder, Some("pSMS-Recovery-Kit.pdf"));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = target.await else {
                return;
            };
            let pdf = recovery::printable(&entropy, "this computer", None);
            let saved = std::fs::write(&path, pdf).is_ok();
            let _ = this.update(cx, |s, cx| {
                s.notice = if saved {
                    "Recovery kit saved. Keep it private."
                } else {
                    "Could not save the recovery kit."
                }
                .into();
                cx.notify()
            });
        })
        .detach();
    }
    fn contact_panel(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let thread = self
            .threads
            .iter()
            .find(|t| Some(&t.key) == self.selected.as_ref());
        let entry = thread
            .and_then(|t| t.contact.as_ref())
            .and_then(|c| self.cards.get(c));
        let card = entry.map(|e| e.card.clone()).unwrap_or_default();
        let fallback = thread.map(|t| t.title.clone()).unwrap_or_default();
        let name = if card.name.is_empty() {
            fallback.clone()
        } else {
            card.name.clone()
        };
        let editable = thread.is_some_and(|t| t.contact.is_some())
            && self.session_key.is_some()
            && !self.external;
        let mut panel = div()
            .id("contact-panel")
            .v_flex()
            .w(px(300.))
            .h_full()
            .flex_shrink_0()
            .overflow_y_scroll()
            .p_4()
            .gap_3()
            .bg(self.color(0xffffff))
            .border_l_1()
            .border_color(self.color(0xdce0e5))
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .child(
                        Button::new("card-close")
                            .icon(IconName::Close)
                            .ghost()
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.card_open = false;
                                s.card_editing = false;
                                cx.notify()
                            })),
                    )
                    .when(editable && !self.card_editing, |d| {
                        d.child(
                            Button::new("card-edit")
                                .label("Edit")
                                .on_click(cx.listener(|s, _, w, cx| s.edit_card(w, cx))),
                        )
                    }),
            );
        if self.card_editing {
            let draft = cards::Card {
                name: self.card_fields[0].read(cx).value().to_string(),
                photo: match &self.card_photo {
                    Some(choice) => choice.clone(),
                    None => card.photo.clone(),
                },
                ..Default::default()
            };
            let preview = cards::Entry {
                image: cards::preview(&draft),
                card: draft,
                edited: false,
                from_phone: false,
            };
            panel = panel.child(
                div()
                    .v_flex()
                    .items_center()
                    .gap_2()
                    .child(self.avatar(Some(&preview), name.chars().take(1).collect(), 72., false))
                    .child(
                        div()
                            .h_flex()
                            .gap_1()
                            .child(
                                Button::new("photo-choose")
                                    .ghost()
                                    .label("Choose photo…")
                                    .on_click(cx.listener(|s, _, w, cx| s.choose_photo(w, cx))),
                            )
                            .when(preview.card.photo.is_some(), |d| {
                                d.child(
                                    Button::new("photo-remove")
                                        .ghost()
                                        .label("Remove photo")
                                        .on_click(cx.listener(|s, _, _, cx| {
                                            s.card_photo = Some(None);
                                            cx.notify()
                                        })),
                                )
                            }),
                    ),
            );
            for (label, field) in ["Name", "Company", "Job title"]
                .iter()
                .zip(self.card_fields.iter())
            {
                panel = panel.child(
                    div()
                        .v_flex()
                        .gap_1()
                        .child(self.caption(*label))
                        .child(Input::new(field)),
                );
            }
            for (phone, title) in [(true, "Phone numbers"), (false, "Emails")] {
                let rows = if phone {
                    &self.phone_rows
                } else {
                    &self.email_rows
                };
                let mut group = div().v_flex().gap_1().child(self.caption(title));
                for (i, (label, value)) in rows.iter().enumerate() {
                    group = group.child(
                        div()
                            .h_flex()
                            .gap_1()
                            .child(div().w(px(84.)).child(Input::new(label)))
                            .child(div().flex_1().min_w_0().child(Input::new(value)))
                            .child(
                                Button::new((
                                    if phone {
                                        "phone-remove"
                                    } else {
                                        "email-remove"
                                    },
                                    i,
                                ))
                                .icon(IconName::Close)
                                .ghost()
                                .on_click(cx.listener(
                                    move |s, _, _, cx| {
                                        let rows = if phone {
                                            &mut s.phone_rows
                                        } else {
                                            &mut s.email_rows
                                        };
                                        if i < rows.len() {
                                            rows.remove(i);
                                        }
                                        cx.notify()
                                    },
                                )),
                            ),
                    );
                }
                group = group.child(
                    Button::new(if phone { "phone-add" } else { "email-add" })
                        .icon(IconName::Plus)
                        .ghost()
                        .label(if phone {
                            "Add phone number"
                        } else {
                            "Add email"
                        })
                        .on_click(cx.listener(move |s, _, w, cx| {
                            let label = if phone { "mobile" } else { "home" };
                            s.add_row(
                                phone,
                                &cards::Labeled {
                                    label: label.into(),
                                    value: String::new(),
                                },
                                w,
                                cx,
                            )
                        })),
                );
                panel = panel.child(group);
            }
            panel = panel
                .child(div().v_flex().gap_1().child(self.caption("Notes")).child(Input::new(&self.card_fields[3])))
                .child(self.caption("Labels can be anything, like mobile, work or a custom tag. Saved encrypted with your archive key."))
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(Button::new("card-cancel").label("Cancel").ghost().on_click(cx.listener(|s, _, _, cx| {
                            s.card_editing = false;
                            cx.notify()
                        })))
                        .child(Button::new("card-save").label("Save").primary().on_click(cx.listener(|s, _, _, cx| s.save_card(false, cx)))),
                )
                .when(entry.is_some_and(|e| e.edited && e.from_phone), |d| {
                    d.child(Button::new("card-reset").label("Use phone details").ghost().on_click(cx.listener(|s, _, _, cx| s.save_card(true, cx))))
                });
            return panel;
        }
        panel = panel.child(
            div()
                .v_flex()
                .items_center()
                .gap_1()
                .child(self.avatar(entry, name.chars().take(1).collect(), 88., false))
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(self.color(INK))
                        .child(name),
                )
                .when(!card.org.is_empty() || !card.title.is_empty(), |d| {
                    d.child(
                        self.caption(
                            [card.title.clone(), card.org.clone()]
                                .into_iter()
                                .filter(|v| !v.is_empty())
                                .collect::<Vec<_>>()
                                .join(" · "),
                        ),
                    )
                }),
        );
        let phones = if card.phones.is_empty() && fallback.chars().any(|c| c.is_ascii_digit()) {
            vec![cards::Labeled {
                label: "phone".into(),
                value: fallback,
            }]
        } else {
            card.phones.clone()
        };
        let mut fields = div()
            .v_flex()
            .rounded_xl()
            .bg(self.color(0xf3f4f6))
            .overflow_hidden();
        let mut any = false;
        for cards::Labeled { label, value } in phones.iter().chain(card.emails.iter()) {
            fields = fields.child(
                div()
                    .v_flex()
                    .px_3()
                    .py_2()
                    .when(any, |d| d.border_t_1().border_color(self.color(0xdce0e5)))
                    .child(self.caption(label.clone()))
                    .child(
                        div()
                            .text_color(rgb(if self.dark { 0x4ea1ff } else { 0x0a7cff }))
                            .child(value.clone()),
                    ),
            );
            any = true;
        }
        if any {
            panel = panel.child(fields);
        }
        if !card.note.is_empty() {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(self.color(INK))
                    .child(card.note.clone()),
            );
        }
        panel.child(self.caption(match entry {
            Some(e) if e.edited => "Edited by you.",
            Some(_) => "From your iPhone's contacts.",
            None if self.session_key.is_none() => "Unlock the archive to see contact details.",
            None => "No contact details yet.",
        }))
    }
    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.key.read(cx).value().to_string();
        let Ok(key) = recovery::key_from_backup(&key) else {
            self.notice =
                "Enter the 12 words from your recovery kit or your original archive key.".into();
            cx.notify();
            return;
        };
        self.use_key(key, window, cx);
    }
    /// Unlocks with the recovery kit PDF instead of typing the words.
    fn unlock_with_kit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open recovery kit".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    let bytes = std::fs::read(&path).map_err(|_| "Could not open that file.")?;
                    kit::key_from_kit(&bytes)
                })
                .await;
            let _ = this.update_in(cx, |s, window, cx| match result {
                Ok(key) => s.use_key(key, window, cx),
                Err(e) => {
                    s.notice = e.into();
                    cx.notify()
                }
            });
        })
        .detach();
    }
    fn use_key(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        // Finishing "Use my papamo.dev inbox": check the words against that inbox, then keep the
        // connection in the Keychain like Reply from anywhere does.
        if let Some(token) = self.pending_hosted.clone() {
            self.notice = "Opening your papamo.dev inbox…".into();
            cx.notify();
            cx.spawn_in(window, async move |this, cx| {
                let checked = cx
                    .background_executor()
                    .spawn(async move { relay::Relay::check(relay::HOSTED, &token, &key) })
                    .await;
                let _ = this.update_in(cx, |s, window, cx| {
                    match checked.and_then(|r| relay::save(&s.db, &r).map(|_| r).map_err(String::from)) {
                        Ok(r) => {
                            s.pending_hosted = None;
                            s.session_key = Some(r.key().to_string());
                            s.hosted = Some(Arc::new(r));
                            s.hosted_seen = None;
                            s.key.update(cx, |i, cx| i.set_value("", window, cx));
                            s.unlock_box = false;
                            s.notice = "Your papamo.dev inbox is open here. Replies from this Mac go through Messages.".into();
                        }
                        Err(e) => s.notice = e,
                    }
                    cx.notify()
                });
            })
            .detach();
            return;
        }
        if self.hosted.is_some() {
            self.session_key = Some(key);
            self.key.update(cx, |i, cx| i.set_value("", window, cx));
            self.unlock_box = false;
            self.refresh();
            cx.notify();
            return;
        }
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
        let entry = thread
            .and_then(|t| t.contact.as_ref())
            .and_then(|c| self.cards.get(c));
        let title = entry
            .map(|e| e.card.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| thread.map(|t| t.title.clone()))
            .unwrap_or_else(|| "Inbox".into());
        let can_open = thread.is_some_and(|t| !t.items[0].locked);
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
                        .id("contact-open")
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_lg()
                        .when(can_open, |d| {
                            d.cursor_pointer()
                                .hover(|d| d.bg(rgb(if self.dark { 0x2c2f36 } else { 0xeceff3 })))
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.card_open = !s.card_open;
                                    s.card_editing = false;
                                    cx.notify()
                                }))
                        })
                        .child(self.avatar(entry, title.chars().take(1).collect(), 30., false))
                        .child(
                            div()
                                .text_lg()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .when(can_open, |d| d.child(self.caption("›"))),
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
                    )
                    .child(
                        Button::new("unlock-kit")
                            .icon(IconName::FolderOpen)
                            .ghost()
                            .label("Open recovery kit PDF…")
                            .on_click(cx.listener(|s, _, w, cx| s.unlock_with_kit(w, cx))),
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
                let mut actions = div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(self.caption(m.time.clone()))
                    .child(
                        Button::new(("star", i))
                            .icon(if starred {
                                IconName::StarOff
                            } else {
                                IconName::Star
                            })
                            .ghost()
                            .label(if starred { "Unstar" } else { "Star" })
                            .on_click(cx.listener(move |s, _, _, cx| {
                                if !(if s.external {
                                    setup::cli(&s.path)
                                        .and_then(|mut cmd| {
                                            cmd.args([
                                                "storage",
                                                "desktop-star",
                                                &id,
                                                &(!starred).to_string(),
                                            ])
                                            .output()
                                        })
                                        .is_ok_and(|out| out.status.success())
                                } else if let Some(hosted) = &s.hosted {
                                    let done = hosted.star(&id, !starred).is_ok();
                                    s.hosted_seen = None;
                                    done
                                } else {
                                    s.api().is_some_and(|api| api.star(&id, !starred).is_ok())
                                }) {
                                    s.notice = "Could not change this star.".into()
                                }
                                s.refresh();
                                cx.notify()
                            })),
                    );
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
                // Replies from this computer sit on the right in blue, as in Messages.
                let outgoing = m.outgoing;
                feed = feed.child(
                    div()
                        .v_flex()
                        .when(outgoing, |d| d.items_end())
                        .when(!outgoing, |d| d.items_start())
                        .gap_1()
                        .child(actions)
                        .child(
                            div()
                                .max_w(px(520.))
                                .px_4()
                                .py_2()
                                .rounded_xl()
                                .when(outgoing, |d| d.bg(rgb(0x0a7cff)).text_color(rgb(0xffffff)))
                                .when(!outgoing, |d| d.bg(self.color(0xe9eaee)))
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
        let can_reply = replies::SUPPORTED
            && self.session_key.is_some()
            && !self.external
            && thread.is_some_and(|t| {
                !t.items[0].locked && t.title.chars().any(|c| c.is_ascii_digit() || c == '@')
            });
        let hint = if !replies::SUPPORTED {
            "Replies need pSMS for Mac, which sends through the Messages app."
        } else if self.session_key.is_none() {
            "Unlock the archive to reply."
        } else {
            "Choose a conversation with a phone number to reply."
        };
        pane.child(
            div()
                .border_t_1()
                .border_color(self.color(0xdce0e5))
                .p_3()
                .child(if can_reply {
                    div()
                        .h_flex()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.reply)))
                        .child(
                            Button::new("reply-send")
                                .primary()
                                .label(if self.sending { "Sending…" } else { "Send" })
                                .disabled(self.sending)
                                .on_click(cx.listener(|s, _, w, cx| s.send_reply(w, cx))),
                        )
                } else {
                    div()
                        .rounded_full()
                        .bg(self.color(0xf3f4f6))
                        .px_4()
                        .py_2()
                        .text_color(self.color(MUTED))
                        .child(hint)
                }),
        )
    }
    /// Sends the typed reply through Messages, then keeps a sealed copy in this archive.
    fn send_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.reply.read(cx).value().trim().to_string();
        let (Some(key), Some(to)) = (
            self.session_key.clone(),
            self.threads
                .iter()
                .find(|t| Some(&t.key) == self.selected.as_ref())
                .map(|t| t.title.clone()),
        ) else {
            return;
        };
        if text.is_empty() || self.sending {
            return;
        }
        // A papamo.dev inbox keeps the sent copy there; the local archive keeps it here.
        let hosted = self.hosted.clone();
        let api = self.api();
        if hosted.is_none() && api.is_none() {
            self.notice = "Start the archive service to send replies.".into();
            cx.notify();
            return;
        }
        self.sending = true;
        self.notice.clear();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let sent = cx
                .background_executor()
                .spawn(async move {
                    replies::send(&to, &text)?;
                    match (hosted, api) {
                        (Some(h), _) => h.store_sent(&to, &text).map_err(|_| {
                            "Sent, but could not save the reply in your inbox.".to_string()
                        }),
                        (None, Some(api)) => {
                            replies::store(&api, &key, &to, &text).map_err(String::from)
                        }
                        (None, None) => Ok(()),
                    }
                })
                .await;
            let _ = this.update_in(cx, |s, window, cx| {
                s.sending = false;
                match sent {
                    Ok(()) => {
                        s.notice = "Sent.".into();
                        s.hosted_seen = None;
                        s.reply.update(cx, |i, cx| i.set_value("", window, cx));
                        s.refresh();
                    }
                    Err(e) => s.notice = e,
                }
                cx.notify()
            });
        })
        .detach();
    }
    /// The local archive's API, which every change goes through.
    fn api(&self) -> Option<api::Api> {
        if self.external {
            return None;
        }
        api::Api::local(&self.path, &self.path)
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
                .child(self.inbox(cx))
                .when(self.card_open && self.selected.is_some(), |d| {
                    d.child(self.contact_panel(cx))
                }),
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
    // The service creates this too; an older service may not have yet.
    if !external {
        db.execute_batch("CREATE TABLE IF NOT EXISTS message_reads(message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE, read_at INTEGER NOT NULL DEFAULT (unixepoch()))")?;
    }
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
    // A papamo.dev inbox set up in Settings → Reply from anywhere is shown, already unlocked.
    let hosted = (!external && !settings)
        .then(|| relay::hosted(&db))
        .flatten()
        .map(Arc::new);
    // Only texts that arrive from now on are checked for one-time codes.
    let code_rowid: i64 = if external {
        0
    } else {
        db.query_row("SELECT coalesce(max(rowid),0) FROM messages", [], |r| {
            r.get(0)
        })?
    };
    if !settings {
        relay::start(settings_path.clone());
    }
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            if !settings {
                otp::start();
            }
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
                    let reply = cx.new(|cx| InputState::new(window, cx).placeholder("Message"));
                    let card_fields = ["Name", "Company", "Job title", "Notes"]
                        .map(|hint| cx.new(|cx| InputState::new(window, cx).placeholder(hint)));
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
                    let relay_words = cx.new(|cx| {
                        InputState::new(window, cx).masked(true).placeholder("Your 12 recovery words")
                    });
                    let relay_url = cx.new(|cx| {
                        InputState::new(window, cx).placeholder("https://your-tunnel.example.com/v1")
                    });
                    let relay_token = cx.new(|cx| {
                        InputState::new(window, cx).masked(true).placeholder("Owner or API token")
                    });
                    cx.new(|cx: &mut Context<Dashboard>| {
                        // Inputs only redraw when their owner does, so re-render on every edit.
                        for field in [&search, &reply].into_iter().chain(card_fields.iter()) {
                            cx.subscribe(
                                field,
                                |_, _, event: &gpui_kit::component::input::InputEvent, cx| {
                                    if matches!(event, gpui_kit::component::input::InputEvent::Change) {
                                        cx.notify()
                                    }
                                },
                            )
                            .detach();
                        }
                        cx.spawn(async move |this,cx|{
                            loop {
                                if this.update(cx,|s,cx|s.check_updates(cx)).is_err(){break}
                                cx.background_executor().timer(Duration::from_secs(3600)).await;
                            }
                        }).detach();
                        cx.spawn(async move |this, cx| {
                            loop {
                                // A papamo.dev inbox: fetch only when something arrived or the view changed.
                                let hosted = this.read_with(cx, |s, _| {
                                    s.hosted.clone().map(|h| (h, s.page, s.starred, s.hosted_seen))
                                });
                                let Ok(hosted) = hosted else { break };
                                if let Some((h, page, starred, seen)) = hosted {
                                    let newest = relay::NEWEST.load(std::sync::atomic::Ordering::Relaxed);
                                    if seen != Some((newest, page, starred)) {
                                        let fetch = h.clone();
                                        let result = cx
                                            .background_executor()
                                            .spawn(async move { fetch.page(page, archive::LIMIT as i64) })
                                            .await;
                                        if this
                                            .update(cx, |s, cx| {
                                                match result {
                                                    Ok((messages, cards)) => {
                                                        let mut records = Vec::new();
                                                        for r in messages["messages"].as_array().into_iter().flatten() {
                                                            if starred && r["starred"] != true {
                                                                continue;
                                                            }
                                                            let (Some(id), Some(sender), Some(body), Some(at)) = (
                                                                r["id"].as_str(),
                                                                r["sender"].as_str(),
                                                                r["body"].as_str(),
                                                                r["received_at"].as_i64(),
                                                            ) else {
                                                                continue;
                                                            };
                                                            let Some((time, when)) = archive::times(&s.db, at) else { continue };
                                                            records.push((
                                                                id.to_string(),
                                                                sender.to_string(),
                                                                body.to_string(),
                                                                time,
                                                                r["starred"] == true,
                                                                r["contact_id"].as_str().map(String::from),
                                                                r["read"] == true,
                                                                when,
                                                            ));
                                                        }
                                                        s.external_page = Some((page, starred, records));
                                                        s.hosted_cards = cards::from_hosted(&cards, h.key());
                                                        s.hosted_seen = Some((newest, page, starred));
                                                    }
                                                    Err(e) => {
                                                        s.notice = e;
                                                        s.hosted_seen = Some((newest, page, starred));
                                                    }
                                                }
                                                s.refresh();
                                                cx.notify();
                                            })
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                }
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
                                                let time=s.db.query_row("SELECT strftime('%Y-%m-%d %H:%M',?1,'unixepoch','localtime')",[r["archived_at"].as_i64()?],|r|r.get::<_,String>(0)).ok()?;
                                                records.push((r["id"].as_str()?.into(),r["sender"].as_str()?.into(),r["body"].as_str()?.into(),time.clone(),r["starred"].as_i64()==Some(1),None,true,time));
                                            }
                                            Some((records,v["newest"].as_i64()?))
                                        }) {s.external_page=Some((page,starred,v.0));s.external_newest=v.1;} else {s.external_page=None;s.notice="Could not reach the selected database.".into();}
                                        s.refresh();cx.notify();
                                    }).is_err(){break;}
                                }
                                cx.background_executor().timer(Duration::from_secs(4)).await;
                                // Deliver texts other apps queued, while this Mac is open.
                                if replies::SUPPORTED
                                    && let Ok(Some((api, key))) = this.update(cx, |s, _| s.api().map(|a| (a, s.session_key.clone())))
                                {
                                    cx.background_executor()
                                        .spawn(async move { replies::deliver(&api, key.as_deref()) })
                                        .await;
                                }
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
                            relay_words,
                            relay_url,
                            relay_token,
                            relay_kit: None,
                            relay_busy: false,
                            code_rowid,
                            session_key: hosted.as_ref().map(|h| h.key().to_string()),
                            hosted,
                            hosted_cards: Default::default(),
                            hosted_seen: None,
                            pending_hosted: None,
                            expiry: Some(30),
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
                            cards: Default::default(),
                            card_open: false,
                            card_editing: false,
                            card_fields,
                            phone_rows: Vec::new(),
                            email_rows: Vec::new(),
                            card_photo: None,
                            reply,
                            sending: false,
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

/// The user's Downloads folder, or their home folder when it can't be found.
fn dirs_downloads() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let downloads = home.join("Downloads");
    if downloads.is_dir() { downloads } else { home }
}
