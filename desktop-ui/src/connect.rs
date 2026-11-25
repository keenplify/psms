use super::*;
use gpui_kit::component::checkbox::Checkbox;
use std::{io::Write, process::Stdio};

#[path = "../../src/hostname_label.rs"]
mod hostname_label;
use hostname_label::suggested_name;
pub struct Form {
    pub name: Entity<InputState>,
    pub zone: Entity<InputState>,
    pub credential: Entity<InputState>,
    pub provider: &'static str,
    pub custom: bool,
    pub busy: bool,
    pub generation: u64,
    pub hint: String,
    pub suggestion: String,
    pub suffix: String,
}
impl Form {
    pub fn new(
        db: &Connection,
        endpoint: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> Self {
        let suggestion = suggested_name(&hostname::get().unwrap_or_default().to_string_lossy());
        let mut name = suggestion.clone();
        let mut suffix = "ngrok.app".to_string();
        let mut zone = String::new();
        let mut provider = "ngrok";
        let mut custom = false;
        let saved = db.query_row(
            "SELECT provider,host FROM desktop_tunnel WHERE id=1 AND state='ready'",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        );
        if let Ok((saved_provider, host)) = saved {
            provider = match saved_provider.as_str() {
                "cloudflare" => "cloudflare",
                "direct" => "direct",
                _ => "ngrok",
            };
            match host.split_once('.') {
                Some((label, domain))
                    if provider == "cloudflare"
                        || (provider == "ngrok"
                            && matches!(
                                domain,
                                "ngrok.app" | "ngrok-free.dev" | "ngrok-free.app"
                            )) =>
                {
                    name = label.into();
                    if provider == "ngrok" {
                        suffix = domain.into();
                    } else {
                        zone = domain.into();
                    }
                }
                _ => {
                    custom = true;
                    endpoint.update(cx, |i, cx| i.set_value(host, window, cx));
                }
            }
        }
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let zone = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("example.com")
                .default_value(zone)
        });
        let credential = cx.new(|cx| InputState::new(window, cx).masked(true));
        for input in [&name, &zone, &credential, endpoint] {
            cx.subscribe(
                input,
                |s, _, event: &gpui_kit::component::input::InputEvent, cx| {
                    if matches!(event, gpui_kit::component::input::InputEvent::Change) {
                        s.form.generation += 1;
                        s.form.hint.clear();
                        cx.notify();
                    }
                },
            )
            .detach();
        }
        Self {
            name,
            zone,
            credential,
            suggestion,
            suffix,
            provider,
            custom,
            busy: false,
            generation: 0,
            hint: String::new(),
        }
    }
}
impl Dashboard {
    pub fn detect_domain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form.generation += 1;
        let ticket = self.form.generation;
        if self.form.custom
            || self.form.provider != "ngrok"
            || self.form.name.read(cx).value().as_ref() != self.form.suggestion
        {
            return;
        }
        self.form.hint = "Checking for an existing public hostname…".into();
        let path = self.path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { setup::cli(&path)?.args(["tunnel", "detect"]).output() })
                .await;
            let _ = this.update_in(cx, |s, w, cx| {
                if s.step != 1
                    || s.screen != Screen::Setup
                    || s.form.generation != ticket
                    || s.form.busy
                {
                    return;
                }
                s.form.hint.clear();
                if s.form.name.read(cx).value().as_ref() == s.form.suggestion
                    && s.form.credential.read(cx).value().is_empty()
                    && !s.form.custom
                    && let Ok(out) = result
                    && out.status.success()
                    && let Ok(host) = String::from_utf8(out.stdout)
                    && !host.trim().is_empty()
                {
                    s.endpoint
                        .update(cx, |i, cx| i.set_value(host.trim().to_owned(), w, cx));
                    s.form.custom = true;
                    s.form.provider = "direct";
                    s.form.hint = "Verified this computer’s public hostname.".into();
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn connection_view(&self, cx: &mut Context<Self>) -> Div {
        let selector = div().h_flex().gap_1().children(
            [("ngrok", "ngrok"), ("cloudflare", "Cloudflare")]
                .into_iter()
                .map(|(id, label)| {
                    let b = button(id, label)
                        .disabled(self.form.busy)
                        .on_click(cx.listener(move |s, _, w, cx| {
                            s.form.generation += 1;
                            s.form.hint.clear();
                            if s.form.provider != id {
                                s.form.credential.update(cx, |i, cx| i.set_value("", w, cx));
                            }
                            s.form.provider = id;
                            cx.notify();
                        }));
                    if self.form.provider == id {
                        b.primary()
                    } else {
                        b
                    }
                }),
        );
        let checkbox = Checkbox::new("custom-domain")
            .checked(self.form.custom)
            .label("Use custom domain")
            .disabled(self.form.busy)
            .on_click(cx.listener(|s, on, _, cx| {
                s.form.custom = *on;
                s.form.generation += 1;
                s.form.hint.clear();
                if !*on && s.form.provider == "direct" {
                    s.form.provider = "ngrok";
                }
                cx.notify();
            }));
        let mut form = div()
            .v_flex()
            .gap_2()
            .child(self.caption(if self.form.provider == "direct" {
                "Existing HTTPS route"
            } else {
                "Named tunnel"
            }))
            .child(checkbox);
        let mut row = div().h_flex().gap_1().items_center();
        if self.form.custom {
            row = row.child(div().flex_1().min_w_0().child(Input::new(&self.endpoint)));
        } else {
            let suffix = if self.form.provider == "cloudflare" {
                let z = self.form.zone.read(cx).value();
                if z.is_empty() {
                    ".your-zone".into()
                } else {
                    format!(".{z}")
                }
            } else {
                format!(".{}", self.form.suffix)
            };
            row = row
                .child(div().flex_1().min_w_0().child(Input::new(&self.form.name)))
                .child(self.caption(suffix));
        }
        form = form.child(row.child(selector));
        if self.form.custom {
            let direct = button("direct", "Use existing HTTPS route")
                .ghost()
                .disabled(self.form.busy)
                .on_click(cx.listener(|s, _, _, cx| {
                    s.form.provider = "direct";
                    s.form.generation += 1;
                    s.form.hint.clear();
                    cx.notify()
                }));
            form = form.child(direct);
        }
        match self.form.provider {
            "cloudflare" => {
                if !self.form.custom {
                    form = form
                        .child(self.caption("Cloudflare DNS zone"))
                        .child(Input::new(&self.form.zone));
                }
                form=form.child(self.caption("Sign in once with cloudflared installed. pSMS creates the named tunnel and registers the hostname in your zone.")).child(button("cf-login","Sign in to Cloudflare").on_click(cx.listener(|s,_,_,cx|{
                    let mut cmd=std::process::Command::new("cloudflared");
                    #[cfg(windows)]{use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
                    s.notice=if cmd.args(["tunnel","login"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok(){"Complete Cloudflare authorization in your browser, then connect.".into()}else{"Install cloudflared and ensure it is on PATH.".into()};cx.notify()
                })));
            }
            "ngrok" => {
                form=form.child(self.caption("ngrok authtoken")).child(Input::new(&self.form.credential)).child(self.caption("A chosen .ngrok.app name requires a suitable plan. With the suggested computer name, pSMS can use your account’s assigned domain instead.")).child(button("ngrok-account","Get authtoken").ghost().on_click(|_,_,cx|cx.open_url("https://dashboard.ngrok.com/get-started/your-authtoken")));
            }
            _ => {
                form =
                    form.child(self.caption("The HTTPS route must already point to this archive."));
            }
        }
        if !self.form.hint.is_empty() {
            form = form.child(self.caption(self.form.hint.clone()));
        }
        form.child(
            button(
                "prepare",
                if self.form.busy {
                    "Connecting…"
                } else {
                    "Connect and create shortcut"
                },
            )
            .primary()
            .disabled(self.form.busy)
            .on_click(cx.listener(|s, _, _, cx| s.connect_phone(cx))),
        )
    }
    fn connect_phone(&mut self, cx: &mut Context<Self>) {
        if self.form.busy {
            return;
        }
        let name = self.form.name.read(cx).value().trim().to_lowercase();
        let provider = self.form.provider;
        let host = if self.form.custom {
            self.endpoint
                .read(cx)
                .value()
                .trim()
                .trim_end_matches('/')
                .trim_start_matches("https://")
                .to_ascii_lowercase()
        } else if provider == "cloudflare" {
            format!("{name}.{}", self.form.zone.read(cx).value().trim())
        } else {
            format!("{name}.{}", self.form.suffix)
        };
        if !self.form.custom
            && (name.is_empty()
                || name.len() > 63
                || name.starts_with('-')
                || name.ends_with('-')
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        {
            self.notice = "Use letters, numbers and hyphens for the tunnel name.".into();
            cx.notify();
            return;
        }
        let fallback = provider == "ngrok"
            && !self.form.custom
            && name == self.form.suggestion
            && self.form.suffix == "ngrok.app";
        let credential = self.form.credential.read(cx).value().trim().to_owned();
        let path = self.path.clone();
        let baseline = self.newest();
        self.form.generation += 1;
        self.form.hint.clear();
        self.form.busy = true;
        self.notice.clear();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let io = |_: std::io::Error| "Could not start the pSMS CLI.".to_string();
                    let mut cmd = setup::cli(&path).map_err(io)?;
                    cmd.args([
                        "tunnel",
                        "connect",
                        if fallback { "ngrok-default" } else { provider },
                        &host,
                    ]);
                    let mut proc = cmd
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .map_err(io)?;
                    proc.stdin
                        .take()
                        .ok_or("Could not supply tunnel credentials.")?
                        .write_all(credential.as_bytes())
                        .map_err(io)?;
                    let out = proc.wait_with_output().map_err(io)?;
                    if !out.status.success() {
                        return Err(String::from_utf8_lossy(&out.stderr)
                            .trim()
                            .trim_start_matches("Error: ")
                            .trim_matches('"')
                            .to_owned());
                    }
                    let endpoint = String::from_utf8(out.stdout)
                        .map_err(|_| "Invalid endpoint.")?
                        .trim()
                        .to_owned();
                    let folder =
                        path.with_file_name(format!("pSMS-pairing-{}", uuid::Uuid::new_v4()));
                    let out = setup::cli(&path)
                        .map_err(io)?
                        .args(["onboard", "--endpoint", &endpoint, "--output"])
                        .arg(&folder)
                        .output()
                        .map_err(io)?;
                    if !out.status.success() {
                        return Err("Connected, but could not write the private shortcut.".into());
                    }
                    Ok((folder, endpoint))
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.form.busy = false;
                match result {
                    Ok((folder, url)) => {
                        if s.db
                            .execute(
                                "UPDATE desktop_setup SET folder=?1,baseline=?2,step=2 WHERE id=1",
                                (folder.to_string_lossy().as_ref(), baseline),
                            )
                            .is_ok()
                        {
                            s.folder = Some(folder);
                            s.baseline = baseline;
                            s.step = 2;
                            s.notice = format!("Connected to {url}");
                        } else {
                            s.notice = format!("Shortcut saved in {}", folder.display());
                        }
                    }
                    Err(reason) => s.notice = reason,
                }
                cx.notify();
            });
        })
        .detach();
    }
}
