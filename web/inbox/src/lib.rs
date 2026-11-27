use gpui_kit::{prelude::*, *};
use serde::Deserialize;
use std::{borrow::Cow, cell::RefCell, rc::Rc, sync::Arc};
use wasm_bindgen::prelude::*;

thread_local! {
    static APP: RefCell<Option<ApplicationHandle>>=const{RefCell::new(None)};
    static VIEW: RefCell<Option<Entity<Transcript>>>=const{RefCell::new(None)};
}

#[derive(Deserialize, Default)]
struct Transcript {
    messages: Vec<Line>,
    dark: bool,
    /// Keeps the newest message in view when a conversation opens or a message arrives.
    #[serde(skip)]
    scroll: ScrollHandle,
}
#[derive(Deserialize)]
struct Line {
    time: String,
    body: String,
    #[serde(default)]
    starred: bool,
    /// Sent from this archive: shown on the right in blue.
    #[serde(default)]
    outgoing: bool,
    /// For replies still in the outbox, such as "Waiting for your Mac".
    #[serde(default)]
    status: String,
}

#[wasm_bindgen]
extern "C" {
    /// Defined by canvas.js; tells the inbox page which message was clicked.
    #[wasm_bindgen(js_namespace = window, js_name = psmsStar)]
    fn psms_star(index: u32);
}

impl Render for Transcript {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < px(500.);
        let dark = self.dark;
        // Messages-style colours: grey incoming bubbles, muted centred times.
        let (bg, ink, muted, bubble) = if dark {
            (0x191b20, 0xf5f5f7, 0x98989d, 0x3a3a3c)
        } else {
            (0xffffff, 0x111111, 0x8e8e93, 0xe9e9eb)
        };
        let mut list = div()
            .id("messages")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .bg(rgb(bg))
            .text_color(rgb(ink))
            .font_family("IBM Plex Sans")
            .p(if compact { px(12.) } else { px(20.) });
        for (index, line) in self.messages.iter().enumerate() {
            // The page sends a time only where the conversation resumes after a gap.
            if !line.time.is_empty() {
                list = list.child(
                    div()
                        .flex()
                        .justify_center()
                        .mt_4()
                        .mb_2()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(muted))
                        .child(line.time.clone()),
                );
            }
            // Clicking a bubble stars or unstars it; starred ones show a gold star.
            let pending = !line.status.is_empty();
            let row = div()
                .flex()
                .items_center()
                .gap_1()
                .mb_1()
                .when(line.outgoing, |d| d.flex_row_reverse())
                .child(
                    div()
                        .id(("bubble", index))
                        .cursor_pointer()
                        .max_w(if compact { px(280.) } else { px(560.) })
                        .px_3()
                        .py_2()
                        .when(line.outgoing, |d| d.bg(rgb(0x0a84ff)).text_color(rgb(0xffffff)))
                        .when(!line.outgoing, |d| d.bg(rgb(bubble)))
                        .when(pending, |d| d.opacity(0.6))
                        .rounded(px(18.))
                        .when(!pending, |d| d.on_click(move |_, _, _| psms_star(index as u32)))
                        .child(line.body.clone()),
                )
                .when(line.starred, |d| d.child(div().text_sm().child("⭐")));
            list = list.child(row);
            if pending {
                list = list.child(
                    div()
                        .flex()
                        .justify_end()
                        .mb_2()
                        .text_xs()
                        .text_color(rgb(muted))
                        .child(line.status.clone()),
                );
            }
        }
        list
    }
}

#[wasm_bindgen]
pub fn run() -> Result<(), JsValue> {
    gpui_kit::platform::web_init();
    let platform = Rc::new(
        gpui_kit::web::WebPlatform::new_with_backend_and_font_fallback(
            false,
            gpui_kit::web::WebBackendPreference::Auto,
            gpui_kit::web::CanvasFontFallback::EmojiAndCjk,
        ),
    );
    let http = Arc::new(platform.fetch_http_client());
    let app = Application::with_platform(platform).with_http_client(http);
    let handle = app.run_embedded(|cx| {
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(include_bytes!(
                "../fonts/IBMPlexSans-Regular.ttf"
            ))])
            .expect("Could not load inbox font");
        cx.open_window(WindowOptions::default(), |_, cx| {
            let view = cx.new(|_| Transcript::default());
            VIEW.with(|slot| slot.replace(Some(view.clone())));
            view
        })
        .expect("Could not open browser canvas");
        cx.activate(true);
    });
    APP.with(|slot| slot.replace(Some(handle)));
    Ok(())
}

#[wasm_bindgen]
pub fn set_transcript(data: &str) -> Result<bool, JsValue> {
    let messages = serde_json::from_str::<Transcript>(data)
        .map_err(|_| JsValue::from_str("Invalid transcript"))?;
    Ok(APP.with(|app| {
        VIEW.with(|view| {
            let app = app.borrow();
            let view = view.borrow();
            if let (Some(app), Some(view)) = (app.as_ref(), view.as_ref()) {
                app.update(|cx| {
                    view.update(cx, |state, cx| {
                        let last = |t: &Transcript| t.messages.last().map(|l| (l.body.clone(), l.status.clone()));
                        let moved = state.messages.len() != messages.messages.len() || last(state) != last(&messages);
                        let scroll = state.scroll.clone();
                        *state = messages;
                        state.scroll = scroll;
                        if moved {
                            state.scroll.scroll_to_bottom();
                        }
                        cx.notify()
                    })
                });
                true
            } else {
                false
            }
        })
    }))
}
