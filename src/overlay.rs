use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Clone)]
pub struct Overlay {
    pub active: Arc<AtomicBool>,
    pub stop: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
    pub elapsed_ms: Arc<AtomicU64>,
    sender: std::sync::mpsc::Sender<bool>,
}
impl Overlay {
    pub fn new() -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let active = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let elapsed_ms = Arc::new(AtomicU64::new(0));
        #[cfg(windows)]
        {
            let flag = stop.clone();
            let p = pause.clone();
            let elapsed = elapsed_ms.clone();
            std::thread::spawn(move || windows_widget(receiver, flag, p, elapsed));
        }
        #[cfg(not(windows))]
        {
            drop(receiver);
        }
        Self {
            active,
            stop,
            pause,
            elapsed_ms,
            sender,
        }
    }
    pub fn start(&self) -> bool {
        if self
            .active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        self.stop.store(false, Ordering::SeqCst);
        self.pause.store(false, Ordering::SeqCst);
        self.elapsed_ms.store(0, Ordering::SeqCst);
        let _ = self.sender.send(true);
        true
    }
    pub fn hide(&self) {
        self.active.store(false, Ordering::SeqCst);
        let _ = self.sender.send(false);
    }
}

#[cfg(windows)]
fn windows_widget(
    receiver: std::sync::mpsc::Receiver<bool>,
    flag: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    elapsed: Arc<AtomicU64>,
) {
    use std::{ptr::null, ptr::null_mut, time::Duration};
    use windows_sys::Win32::{
        Foundation::*, Graphics::Gdi::*, System::LibraryLoader::*, UI::WindowsAndMessaging::*,
    };
    static STOP: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    static PAUSE: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_COMMAND if wp & 0xffff == 2 => {
                if let Some(p) = PAUSE.get() {
                    p.fetch_xor(true, Ordering::SeqCst);
                }
                0
            }
            WM_COMMAND if wp & 0xffff == 1 => {
                if let Some(s) = STOP.get() {
                    s.store(true, Ordering::SeqCst);
                }
                0
            }
            WM_CLOSE => {
                if let Some(s) = STOP.get() {
                    s.store(true, Ordering::SeqCst);
                }
                0
            }
            WM_NCHITTEST => HTCAPTION as LRESULT,
            _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
        }
    }
    let _ = STOP.set(flag);
    let _ = PAUSE.set(pause.clone());
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = wide("OpenAIRecorderOverlay");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW as usize + 1) as HBRUSH,
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            eprintln!("Could not register recording widget");
            return;
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("Recording").as_ptr(),
            WS_POPUP | WS_BORDER,
            80,
            80,
            290,
            82,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            eprintln!("Could not create recording widget");
            return;
        }
        let label = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide("● Recording  00:00").as_ptr(),
            WS_CHILD | WS_VISIBLE,
            16,
            14,
            255,
            20,
            hwnd,
            null_mut(),
            instance,
            null(),
        );
        CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Stop recording").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            16,
            40,
            123,
            27,
            hwnd,
            1usize as HMENU,
            instance,
            null(),
        );
        let pause_button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Pause").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            148,
            40,
            123,
            27,
            hwnd,
            2usize as HMENU,
            instance,
            null(),
        );
        let mut shown = false;
        loop {
            match receiver.try_recv() {
                Ok(true) => {
                    shown = true;
                    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
                Ok(false) => {
                    shown = false;
                    ShowWindow(hwnd, SW_HIDE);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    DestroyWindow(hwnd);
                    break;
                }
                Err(_) => {}
            }
            if shown {
                let t = elapsed.load(Ordering::Relaxed) / 1000;
                let paused = pause.load(Ordering::SeqCst);
                SetWindowTextW(
                    label,
                    wide(&format!(
                        "{}  {:02}:{:02}",
                        if paused {
                            "Ⅱ Paused"
                        } else {
                            "● Recording"
                        },
                        t / 60,
                        t % 60
                    ))
                    .as_ptr(),
                );
                SetWindowTextW(
                    pause_button,
                    wide(if paused { "Resume" } else { "Pause" }).as_ptr(),
                );
            }
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}
