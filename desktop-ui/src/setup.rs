use super::*;
use std::{
    io::Write,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
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
    pub fn open_setup(&mut self, cx: &mut Context<Self>) {
        self.step = if self.folder.is_some() { 2 } else { 0 };
        self.show(Screen::Setup, cx)
    }
    fn save_step(&mut self, step: u8, cx: &mut Context<Self>) {
        if self
            .db
            .execute("UPDATE desktop_setup SET step=?1 WHERE id=1", [step])
            .is_ok()
        {
            self.step = step;
            self.notice.clear()
        } else {
            self.notice = "Could not save your setup progress.".into()
        }
        cx.notify()
    }
    fn finish(&mut self, cx: &mut Context<Self>) {
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
    fn prepare(&mut self, cx: &mut Context<Self>) {
        let endpoint = self.endpoint.read(cx).value().trim().to_owned();
        let folder = self.path.with_file_name(format!(
            "pSMS-pairing-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let result = cli(&self.path).and_then(|mut cmd| {
            cmd.arg("onboard")
                .arg("--endpoint")
                .arg(&endpoint)
                .arg("--output")
                .arg(&folder)
                .output()
        });
        let baseline = self.newest();
        match result{
       Ok(out) if out.status.success()=>{
         if self.db.execute("UPDATE desktop_setup SET folder=?1,baseline=?2,step=2 WHERE id=1",(folder.to_string_lossy().as_ref(),baseline)).is_ok(){self.folder=Some(folder);self.baseline=baseline;self.step=2;self.notice.clear()}
         else{self.notice=format!("Files saved in {}. Could not save wizard progress.",folder.display())}
       }
       _=>self.notice="Could not prepare the shortcut. Enter this archive’s full HTTPS address ending in /messages and check the archive is writable.".into()
    }
        cx.notify()
    }
    pub fn newest(&self) -> i64 {
        if self.external {
            return self.external_newest;
        }
        self.db.query_row("SELECT coalesce(max(rowid),0) FROM messages WHERE sender IN ('papamo:encrypted:v1','papamo:openpgp:v1')",[],|r|r.get(0)).unwrap_or(0)
    }
    pub fn setup_view(&self, cx: &mut Context<Self>) -> Div {
        let steps = [
            "Your archive",
            "Connect a phone",
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
            side = side.child(
                div()
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
        match self.step{
    0=>card=card.child(self.heading("Your private message archive")).child("Incoming messages will appear in this inbox. Your phone encrypts them before sending.").child(self.caption(format!("Archive on this computer: {}",self.path.display())))
        .child(div().child(button("start","Connect a phone").primary().on_click(cx.listener(|s,_,_,cx|s.save_step(1,cx))))),
    1=>card=card.child(self.heading("Connect your iPhone")).child("Enter the public HTTPS endpoint for this archive. If it’s on your computer, point a tunnel or HTTPS proxy at the local service first.")
        .child(Input::new(&self.endpoint)).child(self.caption("Use your own endpoint and upload credentials together. Android capture is coming next."))
        .child(div().h_flex().gap_2().child(button("prepare","Create private shortcut").primary().on_click(cx.listener(|s,_,_,cx|s.prepare(cx))))
            .child(button("network-guide","Network setup").ghost().on_click(|_,_,cx|cx.open_url(GUIDE)))),
    2=>{card=card.child(self.heading("Install the shortcut")).child("Your pairing folder contains the shortcut and a private recovery key. On a trusted Mac, open Terminal in that folder and sign the shortcut.")
        .child(div().h_flex().gap_2().child(button("files","Open pairing folder").on_click(cx.listener(|s,_,_,cx|{if let Some(p)=&s.folder{cx.open_with_system(p)}})))
        .child(button("command","Copy signing command").on_click(cx.listener(|s,_,_,cx|{cx.write_to_clipboard(ClipboardItem::new_string("shortcuts sign --mode anyone --input pSMS_Encrypted.shortcut --output pSMS_Ready.shortcut".into()));s.notice="Signing command copied.".into();cx.notify()}))))
        .child("Install Actions on the iPhone, import the signed shortcut, then enable a Message automation that passes its input to that shortcut.")
        .child(div().child(button("verify","Continue").primary().on_click(cx.listener(|s,_,_,cx|s.save_step(3,cx)))));},
    _=>{let received=self.newest()>self.baseline;
       card=card.child(self.heading(if received{"A message arrived"}else{"Try a test message"}))
        .child(if received{"This archive received an encrypted upload since pairing. Use your recovery key to unlock its contents in the inbox."}else{"Have another phone send you a text matching your automation. This screen checks for new encrypted uploads automatically."})
        .child(div().child(button("done",if received{"Open inbox"}else{"Check for a message"}).primary().on_click(cx.listener(|s,_,_,cx|{
           if s.newest()>s.baseline{s.finish(cx)}else{s.notice="Waiting for a new encrypted upload. Check the endpoint and the phone automation.".into();cx.notify()}
        }))));}
    }
        let mut footer = div().h_flex().gap_3().items_center();
        if self.step > 0 {
            footer = footer.child(
                button("back", "Back")
                    .ghost()
                    .on_click(cx.listener(|s, _, _, cx| s.save_step(s.step.saturating_sub(1), cx))),
            )
        }
        footer = footer.child(
            button("later", "Set up later")
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
    pub fn settings_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut card = div()
            .v_flex()
            .gap_3()
            .max_w(px(570.))
            .child(self.heading("Settings"))
            .child(self.caption(self.path.display().to_string()))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        button("return", "Open inbox")
                            .on_click(cx.listener(|s, _, _, cx| s.show(Screen::Inbox, cx))),
                    )
                    .child(
                        button("setup", "Phone setup")
                            .on_click(cx.listener(|s, _, _, cx| s.open_setup(cx))),
                    )
                    .child(
                        button("help", "Guide")
                            .ghost()
                            .on_click(|_, _, cx| cx.open_url(GUIDE)),
                    ),
            )
            .child(
                div()
                    .mt_4()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Archive replication"),
            );
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
        let mut timing = div().h_flex().gap_2();
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
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .id("settings-pane")
            .overflow_y_scroll()
            .p_6()
            .child(access)
            .child(card)
    }
}
