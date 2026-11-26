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
}
#[derive(Deserialize)]
struct Line {
    time: String,
    body: String,
}

impl Render for Transcript {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < px(500.);
        let dark = self.dark;
        let (bg, ink, muted, bubble) = if dark {
            (0x191b20, 0xf3f4f7, 0xb5bdca, 0x30353d)
        } else {
            (0xffffff, 0x20232b, 0x465364, 0xeef0f4)
        };
        let mut list = div()
            .id("messages")
            .size_full()
            .overflow_y_scroll()
            .bg(rgb(bg))
            .text_color(rgb(ink))
            .font_family("IBM Plex Sans")
            .p(if compact { px(12.) } else { px(24.) });
        for line in &self.messages {
            list = list.child(
                div()
                    .mb_4()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(muted))
                            .mb_1()
                            .child(line.time.clone()),
                    )
                    .child(
                        div()
                            .max_w(px(640.))
                            .p_3()
                            .bg(rgb(bubble))
                            .rounded_lg()
                            .child(line.body.clone()),
                    ),
            );
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
                        *state = messages;
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
