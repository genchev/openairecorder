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
    pub voice: Arc<AtomicBool>,
    sender: std::sync::mpsc::Sender<bool>,
}
impl Overlay {
    pub fn new() -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let active = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let elapsed_ms = Arc::new(AtomicU64::new(0));
        let voice = Arc::new(AtomicBool::new(false));
        #[cfg(windows)]
        {
            let flag = stop.clone();
            let p = pause.clone();
            let elapsed = elapsed_ms.clone();
            let v = voice.clone();
            std::thread::spawn(move || windows_widget(receiver, flag, p, elapsed, v));
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
            voice,
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
        self.voice.store(false, Ordering::SeqCst);
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
    voice: Arc<AtomicBool>,
) {
    use std::{
        ptr::{null, null_mut},
        sync::{
            OnceLock,
            atomic::{AtomicU32, AtomicUsize},
        },
        time::Duration,
    };
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::*,
        UI::{
            Controls::{
                DRAWITEMSTRUCT, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
                TOOLTIPS_CLASSW, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTTOOLINFOW,
            },
            HiDpi::*,
            WindowsAndMessaging::*,
        },
    };
    static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    static PAUSE: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    static VOICE: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    static SCALE: AtomicU32 = AtomicU32::new(96);
    static FONT: AtomicUsize = AtomicUsize::new(0);
    static BRUSH: AtomicUsize = AtomicUsize::new(0);
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn px(n: i32) -> i32 {
        n * SCALE.load(Ordering::Relaxed) as i32 / 96
    }
    unsafe fn layout(hwnd: HWND, dpi: u32) {
        unsafe {
            SCALE.store(dpi, Ordering::Relaxed);
            let font = CreateFontW(
                -px(16),
                0,
                0,
                0,
                500,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                wide("Segoe UI").as_ptr(),
            );
            let old = FONT.swap(font as usize, Ordering::Relaxed);
            for (id, x, y, w, h) in [
                (3, 29, 16, 72, 22),
                (2, 145, 10, 32, 32),
                (1, 187, 10, 32, 32),
            ] {
                let child = GetDlgItem(hwnd, id);
                if !child.is_null() {
                    MoveWindow(child, px(x), px(y), px(w), px(h), 1);
                    SendMessageW(child, WM_SETFONT, font as usize, 1);
                }
            }
            if old != 0 {
                DeleteObject(old as HGDIOBJ);
            }
            SetWindowRgn(
                hwnd,
                CreateRoundRectRgn(0, 0, px(234) + 1, px(52) + 1, px(18), px(18)),
                1,
            );
        }
    }
    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => unsafe {
                SetTextColor(wp as HDC, 0x00f7f0ee);
                SetBkColor(wp as HDC, 0x00261b19);
                BRUSH.load(Ordering::Relaxed) as LRESULT
            },
            WM_ERASEBKGND => 1,
            WM_PAINT => unsafe {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut rect: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rect);
                FillRect(dc, &rect, BRUSH.load(Ordering::Relaxed) as HBRUSH);
                let paused = PAUSE.get().is_some_and(|p| p.load(Ordering::SeqCst));
                let speaking = !paused && VOICE.get().is_some_and(|v| v.load(Ordering::Relaxed));
                let status = CreateSolidBrush(if paused { 0x006ec6ed } else { 0x00e85270 });
                let old = SelectObject(dc, status as HGDIOBJ);
                let pen = SelectObject(dc, GetStockObject(NULL_PEN));
                Ellipse(dc, px(12), px(22), px(20), px(30));
                SelectObject(dc, old);
                SelectObject(dc, pen);
                DeleteObject(status as HGDIOBJ);
                let bars = CreateSolidBrush(if speaking { 0x00c9ea97 } else { 0x006f554a });
                let old = SelectObject(dc, bars as HGDIOBJ);
                let pen = SelectObject(dc, GetStockObject(NULL_PEN));
                for (i, h) in [8, 17, 11].iter().enumerate() {
                    let height = if speaking { *h } else { 3 };
                    let x = 105 + i as i32 * 7;
                    RoundRect(
                        dc,
                        px(x),
                        px(26 - height / 2),
                        px(x + 3),
                        px(26 + height / 2 + 1),
                        px(3),
                        px(3),
                    );
                }
                SelectObject(dc, old);
                SelectObject(dc, pen);
                DeleteObject(bars as HGDIOBJ);
                EndPaint(hwnd, &ps);
                0
            },
            WM_DRAWITEM => unsafe {
                let item = &*(lp as *const DRAWITEMSTRUCT);
                // Owner-drawn rounded controls must paint their entire rectangle,
                // including corners that would otherwise retain the light Windows face.
                FillRect(
                    item.hDC,
                    &item.rcItem,
                    BRUSH.load(Ordering::SeqCst) as HBRUSH,
                );
                let selected = item.itemState & 1 != 0;
                let brush = CreateSolidBrush(if selected { 0x00644539 } else { 0x003b2b26 });
                let old = SelectObject(item.hDC, brush as HGDIOBJ);
                let pen = SelectObject(item.hDC, GetStockObject(NULL_PEN));
                RoundRect(item.hDC, 0, 0, px(32), px(32), px(10), px(10));
                SelectObject(item.hDC, old);
                SelectObject(item.hDC, pen);
                DeleteObject(brush as HGDIOBJ);
                let ink = CreateSolidBrush(if item.CtlID == 1 {
                    0x00a896f5
                } else {
                    0x00f7f0ee
                });
                let old = SelectObject(item.hDC, ink as HGDIOBJ);
                let pen = SelectObject(item.hDC, GetStockObject(NULL_PEN));
                if item.CtlID == 1 {
                    RoundRect(item.hDC, px(11), px(11), px(21), px(21), px(3), px(3));
                } else if PAUSE.get().is_some_and(|p| p.load(Ordering::SeqCst)) {
                    let points = [
                        POINT {
                            x: px(12),
                            y: px(10),
                        },
                        POINT {
                            x: px(23),
                            y: px(16),
                        },
                        POINT {
                            x: px(12),
                            y: px(22),
                        },
                    ];
                    Polygon(item.hDC, points.as_ptr(), 3);
                } else {
                    for x in [11, 19] {
                        RoundRect(item.hDC, px(x), px(10), px(x + 3), px(22), px(2), px(2));
                    }
                }
                SelectObject(item.hDC, old);
                SelectObject(item.hDC, pen);
                DeleteObject(ink as HGDIOBJ);
                if item.itemState & 0x10 != 0 && item.itemState & 0x200 == 0 {
                    let mut focus = item.rcItem;
                    InflateRect(&mut focus, -px(3), -px(3));
                    let outline = CreatePen(PS_SOLID, px(1).max(1), 0x00ffa8b9);
                    let old_pen = SelectObject(item.hDC, outline as HGDIOBJ);
                    let old_brush = SelectObject(item.hDC, GetStockObject(NULL_BRUSH));
                    RoundRect(
                        item.hDC,
                        focus.left,
                        focus.top,
                        focus.right,
                        focus.bottom,
                        px(6),
                        px(6),
                    );
                    SelectObject(item.hDC, old_brush);
                    SelectObject(item.hDC, old_pen);
                    DeleteObject(outline as HGDIOBJ);
                }
                1
            },
            WM_DPICHANGED => unsafe {
                layout(hwnd, (wp & 0xffff) as u32);
                let suggested = &*(lp as *const RECT);
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    suggested.left,
                    suggested.top,
                    px(234),
                    px(52),
                    SWP_NOACTIVATE,
                );
                0
            },
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
    let _ = VOICE.set(voice.clone());
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(null());
        let class = wide("OpenAIRecorderCompactOverlay");
        let brush = CreateSolidBrush(0x00261b19);
        BRUSH.store(brush as usize, Ordering::Relaxed);
        let wc = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: brush,
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            DeleteObject(brush as HGDIOBJ);
            return;
        }
        SCALE.store(GetDpiForSystem(), Ordering::Relaxed);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("OpenAIRecorder recording controls").as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN,
            80,
            80,
            px(234),
            px(52),
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            UnregisterClassW(class.as_ptr(), instance);
            DeleteObject(brush as HGDIOBJ);
            return;
        }
        let label = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide("00:00").as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            1,
            1,
            hwnd,
            3usize as HMENU,
            instance,
            null(),
        );
        let stop_button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Stop and save recording").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            0,
            0,
            1,
            1,
            hwnd,
            1usize as HMENU,
            instance,
            null(),
        );
        let pause_button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Pause recording").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            0,
            0,
            1,
            1,
            hwnd,
            2usize as HMENU,
            instance,
            null(),
        );
        layout(hwnd, GetDpiForWindow(hwnd));
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES,
        });
        let tip = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            hwnd,
            null_mut(),
            instance,
            null(),
        );
        let pause_tip = wide("Pause / resume");
        let stop_tip = wide("Stop and save");
        let voice_tip = wide("Timer · green bars indicate detected speech");
        for (child, text) in [
            (pause_button, &pause_tip),
            (stop_button, &stop_tip),
            (label, &voice_tip),
        ] {
            let info = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                hwnd,
                uId: child as usize,
                lpszText: text.as_ptr() as *mut u16,
                ..std::mem::zeroed()
            };
            SendMessageW(tip, TTM_ADDTOOLW, 0, &info as *const _ as isize);
        }
        let mut shown = false;
        let mut previous = (u64::MAX, false, false);
        loop {
            match receiver.try_recv() {
                Ok(true) => {
                    shown = true;
                    previous.0 = u64::MAX;
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
                    UnregisterClassW(class.as_ptr(), instance);
                    DeleteObject(FONT.load(Ordering::Relaxed) as HGDIOBJ);
                    DeleteObject(brush as HGDIOBJ);
                    break;
                }
                Err(_) => {}
            }
            if shown {
                let t = elapsed.load(Ordering::Relaxed) / 1000;
                let paused = pause.load(Ordering::SeqCst);
                let speaking = voice.load(Ordering::Relaxed) && !paused;
                if previous != (t, paused, speaking) {
                    SetWindowTextW(
                        label,
                        wide(&format!("{:02}:{:02}", t / 60, t % 60)).as_ptr(),
                    );
                    SetWindowTextW(
                        pause_button,
                        wide(if paused {
                            "Resume recording"
                        } else {
                            "Pause recording"
                        })
                        .as_ptr(),
                    );
                    SetWindowTextW(
                        hwnd,
                        wide(if paused {
                            "Recording paused"
                        } else if speaking {
                            "Recording · voice detected"
                        } else {
                            "Recording · listening"
                        })
                        .as_ptr(),
                    );
                    InvalidateRect(hwnd, null(), 0);
                    InvalidateRect(pause_button, null(), 0);
                    previous = (t, paused, speaking);
                }
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
