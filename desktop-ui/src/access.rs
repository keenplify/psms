use super::*;

impl Dashboard {
    pub(super) fn access_view(&self, cx: &mut Context<Self>) -> Div {
        let mut view = div()
            .v_flex()
            .gap_3()
            .max_w(px(570.))
            .mb_5()
            .child(button("appearance",if self.dark{"Use light mode"}else{"Use dark mode"}).on_click(cx.listener(|s,_,w,cx|{let dark=!s.dark;if s.db.execute("INSERT INTO desktop_appearance(id,dark) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET dark=excluded.dark",[dark]).is_ok(){s.dark=dark;Theme::change(if dark{ThemeMode::Dark}else{ThemeMode::Light},Some(w),cx);}else{s.notice="Could not save appearance.".into()}cx.notify()}))).child(self.heading("Access tokens"));
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
        for (index, (scope, label)) in [("api", "Owner API"), ("browser", "Browser autofill")]
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
                    .label(if exists {
                        "Rotate token"
                    } else {
                        "Create token"
                    })
                    .on_click(cx.listener(move |s, _, _, cx| {
                        match app_tokens::issue(&s.db, scope, s.expiry) {
                            Ok(token) => {
                                s.issued = Some((label.into(), token));
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
                row = row.child(Button::new(("revoke", index)).label("Revoke").on_click(
                    cx.listener(move |s, _, _, cx| {
                        if app_tokens::revoke(&s.db, scope).is_ok() {
                            s.issued = None;
                            s.notice = format!("{label} token revoked.")
                        } else {
                            s.notice = "Could not revoke token.".into()
                        }
                        cx.notify()
                    }),
                ))
            }
            view = view.child(row);
        }
        if let Some((label, _)) = &self.issued {
            view =
                view.child(
                    div()
                        .h_flex()
                        .gap_2()
                        .items_center()
                        .child(self.caption(format!("{label} token is ready to copy once.")))
                        .child(button("copy-secret", "Copy token").primary().on_click(
                            cx.listener(|s, _, _, cx| {
                                if let Some((_, secret)) = s.issued.take() {
                                    cx.write_to_clipboard(ClipboardItem::new_string(secret));
                                    s.notice = "Token copied.".into()
                                }
                                cx.notify()
                            }),
                        )),
                );
        }
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
                                    s.issued = Some((url, token));
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
                                .label("Rotate token")
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    match hook_store::rotate(&s.db, &id) {
                                        Ok(Some(token)) => s.issued = Some((name.clone(), token)),
                                        _ => s.notice = "Could not rotate webhook token.".into(),
                                    }
                                    cx.notify()
                                })),
                        )
                        .child(
                            Button::new(("remove-hook", index))
                                .label("Remove")
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    match hook_store::remove(&s.db, &remove) {
                                        Ok(true) => {
                                            s.issued = None;
                                            s.notice = "Destination removed.".into()
                                        }
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
