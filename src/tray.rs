use std::{
    error::Error,
    fs::File,
    path::{Path, PathBuf},
    process::Child,
    sync::{Arc, Mutex, mpsc},
    thread,
};
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_QUIT,
};

pub struct Tray {
    tid: u32,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Tray {
    fn drop(&mut self) {
        // The message queue belongs to our tray thread and has been created by the tray window.
        unsafe {
            PostThreadMessageW(self.tid, WM_QUIT, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub fn start(db: &Path, stop: PathBuf) -> Result<Tray, Box<dyn Error>> {
    let db = db.to_owned();
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let menu = Menu::new();
        let open = MenuItem::with_id("open", "Open", true, None);
        let settings = MenuItem::with_id("settings", "Settings", true, None);
        let exit = MenuItem::with_id("exit", "Exit", true, None);
        let make = || -> Result<_, Box<dyn Error>> {
            menu.append_items(&[&open, &settings, &exit])?;
            Ok(TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("pSMS · Papamo SMS")
                .with_icon(Icon::from_resource(1, Some((32, 32)))?)
                .build()?)
        };
        let _icon = match make() {
            Ok(icon) => icon,
            Err(e) => {
                let _ = tx.send(Err(e.to_string()));
                return;
            }
        };
        let children = Arc::new(Mutex::new(Vec::<Child>::new()));
        let windows = children.clone();
        MenuEvent::set_event_handler(Some(Arc::new(move |event| {
            if event.id == "exit" {
                if let Err(e) = File::create(&stop) {
                    eprintln!("Could not stop pSMS: {e}");
                }
                return;
            }
            match crate::native::launch(&db, event.id == "settings") {
                Ok(child) => {
                    if let Ok(mut list) = windows.lock() {
                        list.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
                        list.push(child);
                    }
                }
                Err(e) => eprintln!("{e}"),
            }
        })));
        // The Windows notification area dispatches menu input through this thread's message loop.
        unsafe {
            let _ = tx.send(Ok(GetCurrentThreadId()));
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        MenuEvent::set_event_handler(None);
        if let Ok(mut list) = children.lock() {
            for child in list.iter_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    });
    Ok(Tray {
        tid: rx.recv()??,
        thread: Some(handle),
    })
}
