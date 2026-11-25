#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine, engine::general_purpose::STANDARD};
use gpui_kit::{
    component::{
        button::Button,
        input::{Input, InputState},
        *,
    },
    *,
};
use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;

struct Message {
    id: String,
    provider: String,
    body: String,
    time: String,
    starred: bool,
}
struct Dashboard {
    db: Connection,
    path: PathBuf,
    settings: bool,
    only_starred: bool,
    page: i64,
    rows: Vec<Message>,
    key: Entity<InputState>,
    notice: String,
    revealed: Option<(String, String)>,
}
impl Dashboard {
    fn refresh(&mut self) {
        self.revealed = None;
        let result = (|| -> rusqlite::Result<Vec<Message>> {
            let mut q=self.db.prepare("SELECT m.id,m.sender,m.body,datetime(m.archived_at,'unixepoch','localtime'),coalesce(f.starred,0) FROM messages m LEFT JOIN message_flags f ON m.id=f.message_id WHERE ?1=0 OR f.starred=1 ORDER BY m.rowid DESC LIMIT 21 OFFSET ?2")?;
            q.query_map((self.only_starred, self.page * 20), |r| {
                Ok(Message {
                    id: r.get(0)?,
                    provider: r.get(1)?,
                    body: r.get(2)?,
                    time: r.get(3)?,
                    starred: r.get(4)?,
                })
            })?
            .collect()
        })();
        match result {
            Ok(rows) => {
                self.rows = rows;
                self.notice.clear();
            }
            Err(e) => self.notice = format!("Could not read archive: {e}"),
        }
    }
    fn decrypt(&mut self, index: usize, cx: &App) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let key = self.key.read(cx).value();
        let decoded = (|| {
            let bytes = STANDARD.decode(&row.body).ok()?;
            if bytes.len() <= 28 {
                return None;
            }
            let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).ok()?;
            let clear = cipher
                .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
                .ok()?;
            let json: serde_json::Value = serde_json::from_slice(&clear).ok()?;
            Some(format!(
                "{}\n{}",
                json.get("sender")?.as_str()?,
                json.get("body")?.as_str()?
            ))
        })();
        self.revealed = decoded.map(|s| (row.id.clone(), s));
        self.notice = if self.revealed.is_some() {
            String::new()
        } else {
            "Unable to decrypt. Check the archive key.".into()
        };
    }
}
impl Render for Dashboard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut panel = div()
            .id("archive")
            .v_flex()
            .size_full()
            .overflow_y_scroll()
            .p_6()
            .gap_4()
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .items_center()
                    .child(div().text_xl().child("pSMS"))
                    .child(
                        Button::new("inbox")
                            .label("Dashboard")
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.settings = false;
                                s.refresh();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("settings")
                            .label("Settings")
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.settings = true;
                                s.revealed = None;
                                cx.notify();
                            })),
                    ),
            );
        if self.settings {
            panel=panel.child(div().text_xl().child("Archive settings"))
                .child(format!("Archive: {}",self.path.display()))
                .child(Button::new("setup-guide").label("Setup guide").on_click(|_, _, cx| cx.open_url("https://papamo.dev/docs/papamo-sms")))
                .child(Button::new("filter").label(if self.only_starred{"Show all messages"}else{"Show starred messages only"}).on_click(cx.listener(|s,_,_,cx|{
                    let next=!s.only_starred;
                    match s.db.execute("INSERT INTO desktop_preferences(id,starred_only) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET starred_only=excluded.starred_only",[next]){
                        Ok(_)=>{s.only_starred=next;s.page=0;s.refresh();s.notice="Preference saved.".into();},Err(e)=>s.notice=e.to_string()
                    }cx.notify();
                }))).child("New messages require an encrypted envelope. Configure the phone and remote connection from the pSMS CLI.");
        } else {
            panel = panel
                .child(div().text_xl().child("Message archive"))
                .child(Input::new(&self.key))
                .child(
                    Button::new("refresh")
                        .label("Refresh")
                        .on_click(cx.listener(|s, _, _, cx| {
                            s.refresh();
                            cx.notify();
                        })),
                );
            if self.rows.is_empty() {
                panel = panel.child("No messages on this page.");
            }
            for (index, row) in self.rows.iter().take(20).enumerate() {
                let mut card = div()
                    .v_flex()
                    .border_1()
                    .p_3()
                    .gap_2()
                    .child(row.time.clone());
                let action = row.provider == "papamo:encrypted:v1";
                if action {
                    card = card.child(
                        Button::new(("decrypt", index))
                            .label("Decrypt Actions message")
                            .on_click(cx.listener(move |s, _, _, cx| {
                                s.decrypt(index, cx);
                                cx.notify();
                            })),
                    );
                } else if row.provider == "papamo:openpgp:v1" {
                    card = card.child(
                        Button::new(("copy", index))
                            .label("Copy encrypted PGP message")
                            .on_click(cx.listener(move |s, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    s.rows[index].body.clone(),
                                ));
                                s.notice = "Encrypted message copied.".into();
                                cx.notify();
                            })),
                    );
                } else {
                    card = card.child(row.provider.clone()).child(row.body.clone());
                }
                if let Some((id, text)) = &self.revealed
                    && id == &row.id
                {
                    card = card.child(text.clone());
                }
                let id = row.id.clone();
                let starred = row.starred;
                card=card.child(Button::new(("star",index)).label(if starred{"Unstar"}else{"Star"}).on_click(cx.listener(move |s,_,_,cx|{
                    match s.db.execute("INSERT INTO message_flags(message_id,starred) VALUES (?1,?2) ON CONFLICT(message_id) DO UPDATE SET starred=excluded.starred",(&id,!starred)){
                        Ok(_)=>s.refresh(),Err(e)=>s.notice=e.to_string()
                    }cx.notify();
                })));
                panel = panel.child(card);
            }
            panel = panel.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("prev")
                            .label("Newer")
                            .disabled(self.page == 0)
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.page = (s.page - 1).max(0);
                                s.refresh();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("next")
                            .label("Older")
                            .disabled(self.rows.len() <= 20)
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.page += 1;
                                s.refresh();
                                cx.notify();
                            })),
                    ),
            );
        }
        panel.child(self.notice.clone())
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
    let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS desktop_preferences(id INTEGER PRIMARY KEY,starred_only INTEGER NOT NULL)")?;
    let only_starred = db.query_row(
        "SELECT coalesce((SELECT starred_only FROM desktop_preferences WHERE id=1),0)",
        [],
        |r| r.get(0),
    )?;
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
            gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("pSMS · Papamo SMS".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let key = cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder("Actions archive key")
                            .masked(true)
                    });
                    cx.new(|_| {
                        let mut s = Dashboard {
                            db,
                            path,
                            settings,
                            only_starred,
                            page: 0,
                            rows: Vec::new(),
                            key,
                            notice: String::new(),
                            revealed: None,
                        };
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
