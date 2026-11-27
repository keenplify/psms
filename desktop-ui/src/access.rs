use super::*;
use gpui_kit::component::checkbox::Checkbox;

fn show_token(label: &str, token: String, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(token.clone()));
    let title = format!("{label} bearer token");
    let field = cx.new(|cx| InputState::new(window, cx).default_value(token.clone()));
    window.open_dialog(cx, move |dialog, _, _| {
        let copy = token.clone();
        dialog
            .title(title.clone())
            .w(px(520.))
            .child(
                div()
                    .v_flex()
                    .gap_3()
                    .child("Copied to clipboard. Save this token before closing.")
                    .child(Input::new(&field).readonly(true)),
            )
            .footer(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        button("copy-secret", "Copy again").on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        }),
                    )
                    .child(
                        button("token-done", "Done")
                            .primary()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}
impl Dashboard {
    pub(super) fn access_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div()
            .v_flex()
            .gap_3()
            .min_w_0()
            .mb_5()
            .child(button("appearance",if self.dark{"Use light mode"}else{"Use dark mode"}).on_click(cx.listener(|s,_,w,cx|{let dark=!s.dark;if s.db.execute("INSERT INTO desktop_appearance(id,dark) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET dark=excluded.dark",[dark]).is_ok(){s.dark=dark;Theme::change(if dark{ThemeMode::Dark}else{ThemeMode::Light},Some(w),cx);}else{s.notice="Could not save appearance.".into()}cx.notify()}))).child(self.codes_view(cx)).child(self.relay_view(cx)).child(self.cloud_view(cx)).child(self.heading("Access tokens"));
        let mut choices = div().h_flex().gap_2().child(self.caption("Expiration"));
        for (index, (label, days)) in [("7 days", Some(7)), ("30 days", Some(30)), ("Never", None)]
            .into_iter()
            .enumerate()
        {
            let b = Button::new(("expiry", index))
                .label(label)
                .on_click(cx.listener(move |s, _, _, cx| {
                    s.expiry = days;
                    cx.notify()
                }));
            choices = choices.child(if self.expiry == days { b.primary() } else { b });
        }
        view = view.child(choices);
        for (index, (scope, label)) in [
            ("api", "Owner API"),
            ("browser", "Browser autofill"),
            ("send", "Send key for your devices"),
        ]
        .into_iter()
        .enumerate()
        {
            let status = app_tokens::status(&self.db, scope);
            let exists = matches!(status, Ok(Some(_)));
            let mut row =
                div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(label)
                            .child(self.caption(match status {
                                Ok(Some(text)) => text,
                                Ok(None) => "No token".into(),
                                Err(_) => "Service upgrade required".into(),
                            })),
                    );
            row = row.child(
                Button::new(("issue", index))
                    .icon(if exists {
                        IconName::RotateCw
                    } else {
                        IconName::Plus
                    })
                    .label(if exists {
                        "Rotate token"
                    } else {
                        "Create token"
                    })
                    .on_click(cx.listener(move |s, _, w, cx| {
                        match app_tokens::issue(&s.db, scope, s.expiry) {
                            Ok(token) => {
                                show_token(label, token, w, cx);
                                s.notice.clear()
                            }
                            Err(_) => {
                                s.notice =
                                    "Could not issue token. Check the service version.".into()
                            }
                        }
                        cx.notify()
                    })),
            );
            if exists {
                row = row.child(
                    Button::new(("revoke", index))
                        .icon(IconName::Ban)
                        .label("Revoke")
                        .on_click(cx.listener(move |s, _, _, cx| {
                            if app_tokens::revoke(&s.db, scope).is_ok() {
                                s.notice = format!("{label} token revoked.")
                            } else {
                                s.notice = "Could not revoke token.".into()
                            }
                            cx.notify()
                        })),
                )
            }
            view = view.child(row);
        }
        let port = device_api::port(&self.db).ok().flatten();
        view = view
            .child(self.heading("Your devices"))
            .child(self.caption(match port {
                Some(p) => format!(
                    "Devices on this network or your Tailscale can queue texts at {} with the send key above. Others are refused.",
                    device_api::url(p, "/v1")
                ),
                None => "Let your other devices queue texts on this network or Tailscale with a send key, without going through the internet.".into(),
            }))
            .child(
                div().h_flex().child(
                    Button::new("device-api")
                        .label(if port.is_some() { "Turn off device access" } else { "Turn on device access" })
                        .on_click(cx.listener(move |s, _, _, cx| {
                            let done = if port.is_some() {
                                device_api::disable(&s.db)
                            } else {
                                device_api::enable(&s.db, device_api::DEFAULT_PORT)
                            };
                            if done.is_err() {
                                s.notice = "Could not change device access.".into()
                            }
                            cx.notify()
                        })),
                ),
            );
        view = view
            .child(self.heading("Event webhooks"))
            .child(Input::new(&self.delivery_url))
            .child(
                button("add-destination", "Add destination").on_click(cx.listener(
                    |s, _, w, cx| {
                        let url = s.delivery_url.read(cx).value().trim().to_string();
                        if !hook_store::valid_url(&url) {
                            s.notice =
                                "Enter an HTTPS destination without credentials, query or fragment."
                                    .into()
                        } else {
                            match hook_store::create(&s.db, &url) {
                                Ok((_, token)) => {
                                    show_token(&url, token, w, cx);
                                    s.delivery_url.update(cx, |i, cx| i.set_value("", w, cx));
                                    s.notice.clear()
                                }
                                Err(_) => s.notice =
                                    "Could not add destination. Check whether it already exists."
                                        .into(),
                            }
                        }
                        cx.notify()
                    },
                )),
            );
        if let Ok(hooks) = hook_store::list(&self.db) {
            for (index, (id, url)) in hooks.into_iter().enumerate() {
                let remove = id.clone();
                let name = url.clone();
                view = view.child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(div().flex_1().child(self.caption(url)))
                        .child(
                            Button::new(("rotate-hook", index))
                                .icon(IconName::RotateCw)
                                .label("Rotate token")
                                .on_click(cx.listener(move |s, _, w, cx| {
                                    match hook_store::rotate(&s.db, &id) {
                                        Ok(Some(token)) => show_token(&name, token, w, cx),
                                        _ => s.notice = "Could not rotate webhook token.".into(),
                                    }
                                    cx.notify()
                                })),
                        )
                        .child(
                            Button::new(("remove-hook", index))
                                .icon(IconName::Delete)
                                .label("Remove")
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    match hook_store::remove(&s.db, &remove) {
                                        Ok(true) => s.notice = "Destination removed.".into(),
                                        _ => s.notice = "Could not remove destination.".into(),
                                    }
                                    cx.notify()
                                })),
                        ),
                );
            }
        }
        view
    }
}

impl Dashboard {
    /// Reply from anywhere: this Mac sends what you queue in any inbox, at home or away.
    fn relay_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div()
            .v_flex()
            .gap_2()
            .min_w_0()
            .child(self.heading("Reply from anywhere"));
        if !replies::SUPPORTED {
            return view.child(self.caption(
                "Replies are sent by pSMS for Mac through the Messages app. Install it on a Mac that stays on, then reply from any browser, the CLI or an AI client.",
            ));
        }
        if let Some(status) = relay::status(&self.db) {
            let checked = match status.checked_at {
                Some(_) => "Checking for texts to send every few seconds.",
                None => "Starting…",
            };
            view = view
                .child(format!("Sending texts queued at {}", status.base))
                .child(self.caption(format!(
                    "{checked} {} sent from this Mac. Keep pSMS open and the Mac awake; texts wait in the outbox while it is off.",
                    status.sent
                )));
            if let Some(error) = status.error {
                view = view.child(div().text_sm().text_color(rgb(0xc62828)).child(error));
            }
            let at_login = relay::opens_at_login();
            return view.child(
                div().h_flex().gap_2().child(
                    Button::new("relay-login")
                        .label(if at_login { "Opens at login ✓" } else { "Open pSMS at login" })
                        .on_click(cx.listener(move |s, _, _, cx| {
                            s.notice = match relay::set_open_at_login(!at_login) {
                                Ok(()) if at_login => "pSMS no longer opens at login.".into(),
                                Ok(()) => "pSMS now opens at login, so queued texts send as soon as this Mac is back.".into(),
                                Err(e) => e.into(),
                            };
                            cx.notify()
                        })),
                ).child(
                    Button::new("relay-stop")
                        .icon(IconName::Ban)
                        .label("Stop sending from this Mac")
                        .on_click(cx.listener(|s, _, _, cx| {
                            relay::forget(&s.db);
                            s.notice = "This Mac no longer sends queued texts.".into();
                            cx.notify()
                        })),
                ),
            );
        }
        view.child(self.caption(
            "Sign in from any computer, reply in your inbox, and this Mac sends it through Messages while it is on. The recovery words and token stay in this Mac's Keychain.",
        ))
        .child(Input::new(&self.relay_words))
        .child(
            div()
                .h_flex()
                .gap_2()
                .child(
                    Button::new("relay-kit")
                        .icon(IconName::Folder)
                        .label(if self.relay_kit.is_some() { "Recovery kit loaded" } else { "Or choose recovery kit PDF…" })
                        .on_click(cx.listener(|s, _, w, cx| s.relay_choose_kit(w, cx))),
                )
                .child(
                    Button::new("relay-papamo")
                        .primary()
                        .label(if self.relay_busy { "Waiting for sign-in…" } else { "Use my papamo.dev inbox" })
                        .disabled(self.relay_busy)
                        .on_click(cx.listener(|s, _, w, cx| s.relay_papamo(w, cx))),
                ),
        )
        .child(self.caption("Self-hosted, behind Cloudflare Tunnel, ngrok or similar:"))
        .child(Input::new(&self.relay_url))
        .child(Input::new(&self.relay_token))
        .child(
            div().h_flex().child(
                Button::new("relay-self")
                    .label("Use this server")
                    .disabled(self.relay_busy)
                    .on_click(cx.listener(|s, _, w, cx| s.relay_self(w, cx))),
            ),
        )
    }
    fn relay_key(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if let Some(key) = &self.relay_kit {
            return Some(key.clone());
        }
        let words = self.relay_words.read(cx).value().to_string();
        match recovery::key_from_backup(&words) {
            Ok(key) => Some(key),
            Err(_) => {
                self.notice =
                    "Enter your 12 recovery words or choose the recovery kit first.".into();
                cx.notify();
                None
            }
        }
    }
    fn relay_choose_kit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            let result = std::fs::read(&path)
                .map_err(|_| "Could not open that file.")
                .and_then(|b| kit::key_from_kit(&b));
            let _ = this.update(cx, |s, cx| {
                match result {
                    Ok(key) => {
                        s.relay_kit = Some(key);
                        s.notice.clear()
                    }
                    Err(e) => s.notice = e.into(),
                }
                cx.notify()
            });
        })
        .detach();
    }
    fn relay_finish(
        &mut self,
        result: Result<relay::Relay, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.relay_busy = false;
        match result.and_then(|r| relay::save(&self.db, &r).map_err(String::from)) {
            Ok(()) => {
                self.relay_kit = None;
                for field in [&self.relay_words, &self.relay_token] {
                    field.update(cx, |i, cx| i.set_value("", window, cx));
                }
                // Start with the Mac, so texts queued while it was off go out when it is back.
                let _ = relay::set_open_at_login(true);
                self.notice = "This Mac now sends the texts you queue, and pSMS opens at login. Messages may ask for permission the first time.".into();
            }
            Err(e) => self.notice = e,
        }
        cx.notify()
    }
    fn relay_papamo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.relay_key(cx) else {
            return;
        };
        let (attempt, url) = match hosted_login::Attempt::start() {
            Ok(v) => v,
            Err(e) => {
                self.notice = e.into();
                cx.notify();
                return;
            }
        };
        self.relay_busy = true;
        self.notice.clear();
        cx.open_url(&url);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let account = attempt.wait().map_err(String::from)?;
                    let token = relay::computer_token(&account.token)?;
                    relay::Relay::check(relay::HOSTED, &token, &key)
                })
                .await;
            let _ = this.update_in(cx, |s, window, cx| {
                cx.activate(true);
                window.activate_window();
                s.relay_finish(result, window, cx)
            });
        })
        .detach();
    }
    fn relay_self(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.relay_key(cx) else {
            return;
        };
        let url = self.relay_url.read(cx).value().to_string();
        let token = self.relay_token.read(cx).value().to_string();
        self.relay_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { relay::Relay::check(&url, &token, &key) })
                .await;
            let _ = this.update_in(cx, |s, window, cx| s.relay_finish(result, window, cx));
        })
        .detach();
    }
}

impl Dashboard {
    /// papamo.dev Plus for this self-hosted archive: phones without a tunnel, an off-site backup,
    /// the web inbox, teams and managed webhooks. Status is read here; changes go through the API.
    fn cloud_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div()
            .v_flex()
            .gap_2()
            .min_w_0()
            .child(self.heading("papamo.dev Plus"));
        if self.external {
            return view.child(
                self.caption("Connecting to papamo.dev works with the built-in SQLite archive."),
            );
        }
        let link = self
            .db
            .query_row(
                "SELECT last_sync, error, pushed, pulled FROM cloud_link WHERE id=1",
                [],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )
            .ok();
        let Some((synced, error, pushed, pulled)) = link else {
            return view
                .child(self.caption(
                    "With a Plus plan, connect this archive to papamo.dev: your phone sends there with no tunnel, this archive keeps a sealed off-site backup, and you can read, reply and share it from any browser. Everything stays sealed with your archive key.",
                ))
                .child(
                    div().h_flex().child(
                        Button::new("cloud-connect")
                            .primary()
                            .label(if self.relay_busy { "Waiting for sign-in…" } else { "Connect to papamo.dev" })
                            .disabled(self.relay_busy)
                            .on_click(cx.listener(|s, _, w, cx| s.cloud_connect(w, cx))),
                    ),
                );
        };
        view = view
            .child(format!(
                "Connected · {pushed} backed up · {pulled} received from papamo.dev"
            ))
            .child(self.caption(match synced {
                Some(_) => {
                    "Syncing every few seconds. Phones can send to papamo.dev without a tunnel."
                }
                None => "Starting…",
            }));
        if let Some(error) = error {
            view = view.child(div().text_sm().text_color(rgb(0xc62828)).child(error));
        }
        view.child(
            div()
                .h_flex()
                .gap_2()
                .child(Button::new("cloud-restore").label("Restore from papamo.dev").on_click(cx.listener(|s, _, _, cx| {
                    s.notice = match s.api().map(|a| a.restore_cloud()) {
                        Some(Ok(())) => "Restoring: messages from papamo.dev are coming down now.".into(),
                        _ => "Start the archive service, then try again.".into(),
                    };
                    cx.notify()
                })))
                .child(Button::new("cloud-disconnect").icon(IconName::Ban).label("Disconnect").on_click(cx.listener(|s, _, _, cx| {
                    s.notice = match s.api().map(|a| a.disconnect_cloud()) {
                        Some(Ok(())) => "Disconnected. Your papamo.dev copy stays until you delete it there.".into(),
                        _ => "Start the archive service, then try again.".into(),
                    };
                    cx.notify()
                }))),
        )
    }
    fn cloud_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = self.api() else {
            self.notice = "Start the archive service, then try again.".into();
            cx.notify();
            return;
        };
        let (attempt, url) = match hosted_login::Attempt::start() {
            Ok(v) => v,
            Err(e) => {
                self.notice = e.into();
                cx.notify();
                return;
            }
        };
        self.relay_busy = true;
        cx.open_url(&url);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let account = attempt.wait().map_err(String::from)?;
                    let token = relay::computer_token(&account.token)?;
                    api.connect_cloud(&token)
                        .map_err(|_| "papamo.dev did not accept this archive. Connecting needs a Plus plan.".to_string())
                })
                .await;
            let _ = this.update_in(cx, |s, window, cx| {
                s.relay_busy = false;
                cx.activate(true);
                window.activate_window();
                s.notice = match result {
                    Ok(()) => "Connected. This archive now backs up to papamo.dev and receives what your phone sends there.".into(),
                    Err(e) => e,
                };
                cx.notify()
            });
        })
        .detach();
    }
}

impl Dashboard {
    /// One-time codes: a notification when a text with a code arrives, and Tab to type it.
    fn codes_view(&self, cx: &mut Context<Self>) -> Div {
        let on = otp::enabled(&self.db);
        let mut view = div()
            .v_flex()
            .gap_2()
            .min_w_0()
            .child(self.heading("One-time codes"))
            .child(self.caption(
                "When a text with a verification code arrives, pSMS shows a notification. Press Tab within a minute to type the code into any app, or choose Copy code. Codes stay in memory only.",
            ));
        let mut row = div().h_flex().gap_2().child(
            Button::new("codes-toggle")
                .label(if on {
                    "Turn off code notifications"
                } else {
                    "Turn on code notifications"
                })
                .on_click(cx.listener(move |s, _, _, cx| {
                    if otp::set_enabled(&s.db, !on).is_err() {
                        s.notice = "Could not save this setting.".into()
                    }
                    cx.notify()
                })),
        );
        if on && replies::SUPPORTED && !otp::can_type() {
            row = row.child(
                Button::new("codes-allow")
                    .primary()
                    .label("Allow typing with Tab")
                    .on_click(cx.listener(|s, _, _, cx| {
                        otp::request_typing_permission();
                        s.notice = "Turn on pSMS in Privacy & Security → Accessibility, then come back. Tab starts working right away.".into();
                        cx.notify()
                    })),
            );
            view = view.child(self.caption("Typing with Tab needs Accessibility permission. The Copy and Type buttons on the notification work without it."));
        }
        view.child(row)
    }
}

impl Dashboard {
    /// Self-hosting without papamo.dev: offer Plus until it is closed or turned off for good.
    pub(super) fn show_upsell(&self) -> bool {
        !self.upsell_closed
            && !self.external
            && self.hosted.is_none()
            && self
                .db
                .query_row("SELECT 1 FROM cloud_link WHERE id=1", [], |_| Ok(()))
                .is_err()
            && self
                .db
                .query_row("SELECT dismissed FROM desktop_upsell WHERE id=1", [], |r| {
                    r.get::<_, bool>(0)
                })
                .map(|d| !d)
                .unwrap_or(true)
    }
    pub(super) fn upsell_banner(&self, cx: &mut Context<Self>) -> Div {
        // Readable in both themes: dark green with light text, or pale green with dark text.
        let (bg, border, ink, muted) = if self.dark {
            (0x1d2b1c, 0x35502f, 0xf2f6ee, 0xb9c8b3)
        } else {
            (0xeef6e4, 0xd5e6c3, 0x1b2a17, 0x44573d)
        };
        div()
            .h_flex()
            .gap_3()
            .items_center()
            .px_4()
            .py_3()
            .bg(rgb(bg))
            .border_b_1()
            .border_color(rgb(border))
            .text_color(rgb(ink))
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Keep self-hosting, add papamo.dev with Plus"))
                    .child(div().text_sm().text_color(rgb(muted)).child(
                        "Read and reply from any browser, keep a sealed off-site backup, and let your phone send with no tunnel. Your texts stay on this Mac.",
                    ))
                    .child(
                        Checkbox::new("upsell-never")
                            .checked(self.upsell_never)
                            .label("Don't show again")
                            .on_click(cx.listener(|s, on, _, cx| {
                                s.upsell_never = *on;
                                cx.notify()
                            })),
                    ),
            )
            .child(
                Button::new("upsell-see")
                    .primary()
                    .label("See Plus")
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url("https://www.papamo.dev/sms#plans"))),
            )
            .child(
                Button::new("upsell-close")
                    .ghost()
                    .icon(IconName::Close)
                    .tooltip("Close")
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.upsell_closed = true;
                        if s.upsell_never {
                            let _ = s.db.execute_batch(
                                "CREATE TABLE IF NOT EXISTS desktop_upsell(id INTEGER PRIMARY KEY CHECK(id=1), dismissed INTEGER NOT NULL);
                                 INSERT INTO desktop_upsell(id,dismissed) VALUES(1,1) ON CONFLICT(id) DO UPDATE SET dismissed=1;",
                            );
                        }
                        cx.notify()
                    })),
            )
    }
}
