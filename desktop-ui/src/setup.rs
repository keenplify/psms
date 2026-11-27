use super::*;
use gpui_kit::prelude::FluentBuilder;
use std::{
    io::Write,
    process::{Command, Stdio},
};

pub fn cli(path: &std::path::Path) -> std::io::Result<Command> {
    let exe =
        std::env::current_exe()?.with_file_name(format!("psms{}", std::env::consts::EXE_SUFFIX));
    let mut cmd = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.arg("--database").arg(path);
    Ok(cmd)
}
impl Dashboard {
    pub fn storage_view(&self, cx: &mut Context<Self>) -> Div {
        if self.external {
            return div().child(self.caption("Storage - External SQL database"));
        }
        let mut choices = div().h_flex().flex_wrap().gap_1();
        for (label, engine) in [
            ("SQLite", None),
            ("PostgreSQL", Some("postgres")),
            ("MySQL", Some("mysql")),
            ("MariaDB", Some("mariadb")),
            ("SQL Server", Some("sqlserver")),
        ] {
            let b = button(label, label)
                .disabled(self.migrating)
                .on_click(cx.listener(move |s, _, _, cx| {
                    s.storage_engine = engine;
                    cx.notify();
                }));
            choices = choices.child(if self.storage_engine == engine {
                b.primary()
            } else {
                b
            });
        }
        let mut section = div()
            .v_flex()
            .gap_2()
            .child(self.caption("Archive storage"))
            .child(choices);
        if let Some(engine) = self.storage_engine {
            section=section.child(self.caption("Use a private connection file for an empty database. pSMS copies and verifies your archive and keeps the original SQLite file.")).child(Input::new(&self.storage_file)).child(button("activate","Verify and switch").disabled(self.migrating).on_click(cx.listener(move|s,_,_,cx|{
                let file=s.storage_file.read(cx).value().trim().to_string();
                if file.is_empty(){s.notice="Enter the path to your connection file.".into();cx.notify();return;}
                let path=s.path.clone();
                s.migrating=true;s.notice="Checking the server and copying the archive...".into();cx.notify();
                cx.spawn(async move |this,cx|{
                    let result=cx.background_executor().spawn(async move{
                        cli(&path).and_then(|mut cmd|cmd.args(["storage","activate",engine,"--connection-file",&file]).output())
                    }).await;
                    let _=this.update(cx,|s,cx|{
                        s.migrating=false;
                        if result.is_ok_and(|o|o.status.success()) {
                            if cli(&s.path).and_then(|mut cmd|cmd.arg("desktop").spawn()).is_ok(){cx.quit();return;}
                            s.notice="Migration verified. Reopen pSMS to load the new database.".into();
                        } else {s.notice="Could not activate the database. Check the connection file and run psms storage status. The source archive was retained.".into();}
                        cx.notify();
                    });
                }).detach();
            })));
        }
        section
    }
    pub fn open_setup(&mut self, cx: &mut Context<Self>) {
        self.step = if self.folder.is_some() { 2 } else { 0 };
        self.show(Screen::Setup, cx)
    }
    pub fn save_step(&mut self, step: u8, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .db
            .execute("UPDATE desktop_setup SET step=?1 WHERE id=1", [step])
            .is_ok()
        {
            self.step = step;
            self.reached = self.reached.max(step);
            self.notice.clear();
            if step == 1 {
                self.detect_domain(window, cx);
            }
        } else {
            self.notice = "Could not save your setup progress.".into()
        }
        cx.notify()
    }
    pub fn finish(&mut self, cx: &mut Context<Self>) {
        if self
            .db
            .execute("UPDATE desktop_setup SET completed=1 WHERE id=1", [])
            .is_err()
        {
            self.notice = "Could not save setup progress.".into();
            cx.notify();
            return;
        }
        self.show(Screen::Inbox, cx);
    }
    pub fn newest(&self) -> i64 {
        if self.external {
            return self.external_newest;
        }
        self.db.query_row("SELECT coalesce(max(rowid),0) FROM messages WHERE sender IN ('papamo:encrypted:v1','papamo:openpgp:v1')",[],|r|r.get(0)).unwrap_or(0)
    }
    pub fn setup_view(&self, cx: &mut Context<Self>) -> Div {
        let steps = [
            "Sign in",
            "Choose storage",
            "Install shortcut",
            "Verify delivery",
        ];
        let mut side = div()
            .v_flex()
            .w(px(235.))
            .flex_shrink_0()
            .h_full()
            .p_5()
            .gap_3()
            .bg(self.color(0xf0f3f8))
            .border_r_1()
            .border_color(self.color(0xd9dfe7))
            .child(img(self.logo.clone()).size(px(48.)))
            .child(self.heading("Set up pSMS"));
        for (i, label) in steps.iter().enumerate() {
            let active = i == self.step as usize;
            let reachable = i as u8 <= self.reached.max(self.step);
            side = side.child(
                div()
                    .id(("setup-step", i))
                    .opacity(if reachable { 1.0 } else { 0.5 })
                    .when(reachable && !active && !self.form.busy, |row| {
                        row.cursor_pointer()
                            .on_click(cx.listener(move |s, _, w, cx| s.save_step(i as u8, w, cx)))
                    })
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .py_2()
                    .child(
                        div()
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(self.color(if active { BLUE } else { 0xdce2ea }))
                            .text_color(self.color(if active { 0xfffffe } else { MUTED }))
                            .text_sm()
                            .child(format!("{}", i + 1)),
                    )
                    .child((*label).to_string()),
            );
        }
        let mut card = div()
            .v_flex()
            .max_w(px(520.))
            .w_full()
            .gap_4()
            .child(self.caption(format!("Step {} of 4", self.step + 1)));
        match self.step {
            0 => card = card.child(self.account_view(cx)),
            1 => {
                card = card
                    .child(self.heading("Where should messages be stored?"))
                    .child(self.database_view(cx))
            }
            2 => {
                card=card.child(self.heading("Install the shortcut")).child("Your pairing folder contains the shortcut and a private recovery key. On a trusted Mac, open Terminal in that folder and sign the shortcut.")
        .child(div().h_flex().gap_2().child(button("files","Open pairing folder").on_click(cx.listener(|s,_,_,cx|{if let Some(p)=&s.folder{cx.open_with_system(p)}})))
        .child(button("command","Copy signing command").on_click(cx.listener(|s,_,_,cx|{cx.write_to_clipboard(ClipboardItem::new_string("shortcuts sign --mode anyone --input pSMS_Encrypted.shortcut --output pSMS_Ready.shortcut".into()));s.notice="Signing command copied.".into();cx.notify()}))))
        .child("Install Actions on the iPhone, import the signed shortcut, then enable a Message automation that passes its input to that shortcut.")
        .child(div().child(button("verify","Continue").primary().on_click(cx.listener(|s,_,w,cx|s.save_step(3,w,cx)))));
            }
            3 if self
                .folder
                .as_ref()
                .is_some_and(|f| f.join("papamo-hosted").is_file()) =>
            {
                card=card.child(self.heading("Verify your hosted archive"))
            .child("Send a test message to your phone, then check that it reached your Papamo account.")
            .child(button("verify-hosted","Check hosted archive").disabled(self.form.busy).on_click(cx.listener(|s,_,w,cx|s.check_hosted(w,cx))));
            }
            _ => {
                let received = self.newest() > self.baseline;
                card=card.child(self.heading(if received{"A message arrived"}else{"Try a test message"}))
        .child(if received{"This archive received an encrypted upload since pairing. Use your recovery key to unlock its contents in the inbox."}else{"Have another phone send you a text matching your automation. This screen checks for new encrypted uploads automatically."})
        .child(div().child(button("done",if received{"Open inbox"}else{"Check for a message"}).primary().on_click(cx.listener(|s,_,_,cx|{
           if s.newest()>s.baseline{s.finish(cx)}else{s.notice="Waiting for a new encrypted upload. Check the endpoint and the phone automation.".into();cx.notify()}
        }))));
            }
        }
        let footer = div().h_flex().child(
            button("later", "Set up later")
                .disabled(self.form.busy)
                .ghost()
                .on_click(cx.listener(|s, _, _, cx| s.finish(cx))),
        );
        div().h_flex().size_full().child(side).child(
            div()
                .flex_1()
                .id("setup-body")
                .overflow_y_scroll()
                .v_flex()
                .items_center()
                .p_6()
                .gap_5()
                .child(card)
                .child(footer),
        )
    }
    fn hook_status(&self) -> Option<String> {
        let mut p = self.path.as_os_str().to_owned();
        p.push(".sync-send");
        let data = std::fs::read(p).ok()?;
        let config: serde_json::Value = serde_json::from_slice(&data).ok()?;
        Some(config.get("endpoint")?.as_str()?.into())
    }
    fn connect_hook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.hook_url.read(cx).value().trim().to_string();
        let source = url.rsplit('/').next().unwrap_or("");
        let payload = serde_json::json!({"source":source,"endpoint":url,"token":self.hook_token.read(cx).value().trim()});
        let result = (|| -> std::io::Result<bool> {
            let mut child = cli(&self.path)?
                .args(["sync", "connect", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let mut pipe = child
                .stdin
                .take()
                .ok_or_else(|| std::io::Error::other("pipe"))?;
            let sent = pipe.write_all(payload.to_string().as_bytes());
            drop(pipe);
            let status = child.wait()?;
            sent?;
            Ok(status.success())
        })();
        self.notice = if matches!(result, Ok(true)) {
            self.hook_token
                .update(cx, |i, cx| i.set_value("", window, cx));
            "Webhook paired. The service activates it within 15 seconds.".into()
        } else {
            "Check the HTTPS URL, source ID and 64-character pairing token. An already paired sender keeps its original destination.".into()
        };
        cx.notify()
    }
    pub fn settings_view(&self, wide: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let updates = div()
            .v_flex()
            .gap_3()
            .min_w_0()
            .child(self.heading("Software updates"))
            .child(self.caption(concat!("Installed version ", env!("CARGO_PKG_VERSION"))))
            .child(self.update_status.clone())
            .child(self.caption(match &self.checked_at {
                Some(time) => format!("Last checked {time} · Checks hourly"),
                None => "Checks at startup and hourly".to_owned(),
            }))
            .child(
                button("check-updates", "Check for updates")
                    .disabled(self.checking || self.updating)
                    .on_click(cx.listener(|s, _, _, cx| s.check_updates(cx))),
            )
            .child(div().mt_4().child(self.heading("Recovery kit")))
            .child(self.caption(if self.session_key.is_some() {
                "Save a fresh copy of the PDF with your 12 words. Keep it private."
            } else {
                "Unlock the archive to save your recovery kit again."
            }))
            .child(
                Button::new("save-kit")
                    .label("Save recovery kit (PDF)…")
                    .disabled(self.session_key.is_none())
                    .on_click(cx.listener(|s, _, _, cx| s.save_kit(cx))),
            );
        let mut card = div()
            .v_flex()
            .gap_3()
            .min_w_0()
            .child(self.heading("Archive"))
            .child(self.caption(self.path.display().to_string()))
            .child(self.storage_view(cx))
            .child(
                div()
                    .mt_4()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Archive replication"),
            );
        if let Ok((provider, state)) = self.db.query_row(
            "SELECT provider,state FROM desktop_tunnel WHERE id=1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        ) {
            card = card
                .child(self.caption(format!("Phone connection · {provider} · {state}")))
                .child(
                    button("disconnect", "Disconnect phone route").on_click(cx.listener(
                        |s, _, _, cx| {
                            s.notice = if s
                                .db
                                .execute("DELETE FROM desktop_tunnel WHERE id=1", [])
                                .is_ok()
                            {
                                "Cleared the managed connection.".into()
                            } else {
                                "Could not clear the connection.".into()
                            };
                            cx.notify()
                        },
                    )),
                );
        }
        let access = self.access_view(cx);
        if let Some(endpoint) = self.hook_status() {
            let pending = self
                .db
                .query_row("SELECT count(*) FROM sync_queue", [], |r| {
                    r.get::<_, i64>(0)
                })
                .ok();
            card = card
                .child("Paired destination")
                .child(self.caption(endpoint))
                .child(
                    self.caption(
                        pending
                            .map(|n| format!("{n} pending message changes"))
                            .unwrap_or_else(|| "Starting sync…".into()),
                    ),
                );
        } else {
            card=card.child("Connect a compatible receiver to replicate encrypted changes. Use the endpoint and credential from its private pairing file.")
        .child(self.caption("Receiver endpoint")).child(Input::new(&self.hook_url)).child(self.caption("Pairing token")).child(Input::new(&self.hook_token))
        .child(div().child(button("save-hook","Connect webhook").primary().on_click(cx.listener(|s,_,w,cx|s.connect_hook(w,cx)))));
        }
        let saved = self
            .db
            .query_row("SELECT seconds FROM sync_interval WHERE id=1", [], |r| {
                r.get::<_, u32>(0)
            })
            .ok();
        let mut timing = div().h_flex().flex_wrap().gap_2();
        for (i, (label, seconds)) in [
            ("Default", None),
            ("1 hour", Some(3600)),
            ("6 hours", Some(21600)),
            ("12 hours", Some(43200)),
            ("Daily", Some(86400)),
        ]
        .into_iter()
        .enumerate()
        {
            let choice=Button::new(("sync-interval",i)).label(label).on_click(cx.listener(move|s,_,_,cx|{
                let result=if let Some(seconds)=seconds{s.db.execute("INSERT INTO sync_interval(id,seconds) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET seconds=excluded.seconds",[seconds])}else{s.db.execute("DELETE FROM sync_interval WHERE id=1",[])};
                s.notice=if result.is_ok(){"Sync timing saved.".into()}else{"Could not save sync timing.".into()};cx.notify()
            }));
            timing = timing.child(if seconds == saved {
                choice.primary()
            } else {
                choice
            });
        }
        card = card.child(self.heading("Sync timing")).child(timing)
            .child(self.caption("Default is 6 hours unless configured by your server. Sync waits for spare capacity."))
            .child(
                div().child(
                    button("api-docs", "API and webhook documentation")
                        .ghost()
                        .on_click(|_, _, cx| {
                            cx.open_url(
                                "https://www.papamo.dev/docs/papamo-sms#rest-api-and-webhooks",
                            )
                        }),
                ),
            );
        let columns = if wide {
            div()
                .h_flex()
                .items_start()
                .gap_8()
                .child(div().flex_1().min_w_0().child(access))
                .child(div().flex_1().min_w_0().child(card))
        } else {
            div().v_flex().gap_5().child(access).child(card)
        };
        div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .px_6()
                    .py_3()
                    .border_b_1()
                    .border_color(self.color(0xd3d8df))
                    .child(div().flex_1().child(self.heading("Settings")))
                    .child(button("setup", "Run setup again").on_click(cx.listener(
                        |s, _, _, cx| {
                            s.step = 0;
                            s.show(Screen::Setup, cx);
                        },
                    )))
                    .child(
                        button("help", "Guide")
                            .ghost()
                            .on_click(|_, _, cx| cx.open_url(GUIDE)),
                    ),
            )
            .child(
                div()
                    .id("settings-pane")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_6()
                    .child(div().v_flex().gap_5().child(updates).child(columns)),
            )
    }
}
