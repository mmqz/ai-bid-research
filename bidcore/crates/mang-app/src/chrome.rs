//! 自绘标题栏（Windows）：去掉系统标题栏，菜单栏当标题栏。
//!
//! 五个踩出来的 Windows 细节（第 1、2 条是把标题栏彻底搞坏的元凶）：
//!
//!   1. tao 的 WM_NCCALCSIZE 会替我们做「无边框」，但它对**未装饰且有阴影**的窗口
//!      走的是 `util::calculate_window_insets()` 分支：客户区 = 窗口矩形按阴影内缩。
//!      tao 的 WindowBuilder 默认 `decoration_shadow = true`，所以我们以前在子类里
//!      返回 0 也没用 —— 等我们收到这条消息时客户区已经算错了，WebView 于是顶在
//!      窗口第 1 像素，把整条标题带盖住（真实鼠标全被 WebView 吃掉）。
//!      现在显式 `with_undecorated_shadow(false)`：tao 原样保留 rgrc，整窗 = 客户区，
//!      WebView 从 (0,0) 铺满，标题带按「客户区顶端 32 个逻辑像素」算。
//!   2. 鼠标其实永远落在 WebView 子窗口上（它铺满整个客户区），系统先问 WebView、
//!      再问它的父链。如果 WebView 返回 HTCAPTION，系统会把它当普通客户区（拖动/双击
//!      全失效）。所以本窗口的命中测试必须回答「标题带和四条边是我的非客户区」，
//!      由父链的最后一份答案决定 —— 见 hit_test。
//!   3. 命中测试的坐标系：系统派发 WM_NCHITTEST 用的是和 GetWindowRect 一致的坐标，
//!      不是 DWMWA_EXTENDED_FRAME_BOUNDS（本机两者差 45px，用后者整条标题带都会落空）。
//!   4. 顶边缩放和标题栏叠在一起：Win11 把可缩放边框叠在标题栏上。这里让标题带优先
//!      HTCAPTION（双击最大化要它），拖高走标题带以外的顶边 + 左右上角。
//!   5. 窗口按钮（最小化/最大化/关闭）由 UI 走 IPC window.action -> ShowWindow/SetWindowPos；
//!      按钮占的宽度由 UI 通过 window.buttons 上报，标题带右端要给它让位。

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::WindowsAndMessaging::{
        ChildWindowFromPointEx, GetClassNameW, GetWindowLongW, GetWindowRect,
        IsWindowVisible, PostMessageW, SendMessageW, SetWindowLongW,
        SetWindowPos, ShowWindow, CWP_SKIPDISABLED, CWP_SKIPINVISIBLE, CWP_SKIPTRANSPARENT,
        GWL_STYLE, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTRIGHT,
        HTTOP, HTTOPLEFT, HTTOPRIGHT, SC_CLOSE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SWP_NOZORDER, SW_MINIMIZE, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
        WM_MBUTTONDOWN, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONDBLCLK,
        WM_RBUTTONDOWN, WM_SYSCOMMAND, WS_THICKFRAME,
    };

    const SUBCLASS_ID: usize = 0x4D41_4E48;
    /// 标题带高度（逻辑像素，按 DPI 缩放）
    static CAPTION_H: AtomicI32 = AtomicI32::new(32);
    /// 右侧窗口按钮占的宽度（逻辑像素）
    static BUTTONS_W: AtomicI32 = AtomicI32::new(138);
    /// 我们自己的「最大化」状态位（不用 IsZoomed：去边框窗口不走 SW_MAXIMIZE）
    static IS_MAXED: AtomicBool = AtomicBool::new(false);
    /// 最大化状态变化时的回调（主线程设置；双击标题栏后要把状态推回 UI）
    static STATE_EMIT: std::sync::Mutex<Option<Box<dyn Fn(bool) + Send>>> = std::sync::Mutex::new(None);

    pub fn install(hwnd_raw: isize) -> bool {
        let hwnd = HWND(hwnd_raw as _);
        unsafe {
            let ok = SetWindowSubclass(hwnd, Some(proc_), SUBCLASS_ID, 0).as_bool();
            if ok {
                // 子类挂上之前窗口已经建好了，这里按当前状态补一次
                apply_workarea(hwnd);
            }
            ok
        }
    }

    /// 注册「最大化状态变化」回调（双击标题栏时 UI 的按钮图标要跟着换）
    pub fn set_state_emit(cb: Box<dyn Fn(bool) + Send>) {
        if let Ok(mut slot) = STATE_EMIT.lock() {
            *slot = Some(cb);
        }
    }

    fn emit_state() {
        let maxed = IS_MAXED.load(Ordering::Relaxed);
        if let Ok(slot) = STATE_EMIT.lock() {
            if let Some(cb) = slot.as_ref() {
                cb(maxed);
            }
        }
    }

    /// 窗口按钮占位宽度：标题带右端这一段留给 UI 的按钮（不返回 HTCAPTION）。
    pub fn set_buttons_width(px: i32) {
        if px > 0 {
            BUTTONS_W.store(px.clamp(64, 400), Ordering::Relaxed);
        }
    }

    fn scale(hwnd: HWND) -> f64 {
        let dpi = unsafe { GetDpiForWindow(hwnd) } as f64;
        if dpi > 0.0 { dpi / 96.0 } else { 1.0 }
    }

    fn caption_px(hwnd: HWND) -> i32 {
        (CAPTION_H.load(Ordering::Relaxed) as f64 * scale(hwnd)).round() as i32
    }

    fn border_px(hwnd: HWND) -> i32 {
        (5.0 * scale(hwnd)).round() as i32
    }

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(hwnd: HWND, attr: u32, pv: *const core::ffi::c_void, size: u32) -> i32;
    }

    /// 命中测试用的窗口矩形。
    ///
    /// 用 GetWindowRect，是因为它和系统派发 WM_NCHITTEST 时用的坐标系一致 ——
    /// 实测验证：以 DWM 的 DWMWA_EXTENDED_FRAME_BOUNDS 为基准去探测，整条标题带
    /// 都落在窗口外面（返回 HTCLIENT）；以 GetWindowRect 为基准，标题带 / 四边
    /// 都恰好对上。所以这里**不要**换成 DWM 矩形，两者在本机相差 45px。
    fn hit_rect(hwnd: HWND) -> RECT {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd, &mut r);
        }
        r
    }

    /// 无边框窗口的外观参数：可拖拽边框厚度 + 小圆角。
    fn sync_frame(hwnd: HWND) {
        const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
        const DWMWA_VISIBLE_FRAME_BORDER_THICKNESS: u32 = 37;
        let border: i32 = border_px(hwnd).max(1);
        let corner: i32 = 3; // DWMWCP_ROUNDSMALL
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_VISIBLE_FRAME_BORDER_THICKNESS,
                &border as *const i32 as *const core::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const i32 as *const core::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }
    }

    /// 标题带判定：y 在客户区顶部 caption 高度内，且 x 不压到右侧窗口按钮。
    unsafe fn in_caption_band(hwnd: HWND, sx: i32, sy: i32) -> bool {
        // 客户区原点用「真实窗口矩形」的左上角：无边框窗口上整窗 = 客户区，
        // 而 ClientToScreen/GetWindowRect 在本机会给出与真实位置不一致的值。
        let wr = hit_rect(hwnd);
        let origin = POINT { x: wr.left, y: wr.top };
        let cw = wr.right - wr.left;
        let cap = caption_px(hwnd);
        let buttons = (BUTTONS_W.load(Ordering::Relaxed) as f64 * scale(hwnd)).round() as i32;
        let in_band = sy >= origin.y && sy < origin.y + cap;
        let in_x = sx >= origin.x && sx < origin.x + (cw - buttons).max(120);
        in_band && in_x
    }

    unsafe fn hit_test(hwnd: HWND, lparam: LPARAM) -> LRESULT {
        let sx = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
        let sy = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        let wr = hit_rect(hwnd);
        // 先算左右下三条边（顶边里的「上边缘缩放」最后再补，见下）
        let maximized = IS_MAXED.load(Ordering::Relaxed);
        if !maximized {
            let b = border_px(hwnd);
            let left = sx - wr.left < b;
            let right = wr.right - sx <= b;
            let bottom = wr.bottom - sy <= b;
            if bottom && left {
                return LRESULT(HTBOTTOMLEFT as isize);
            }
            if bottom && right {
                return LRESULT(HTBOTTOMRIGHT as isize);
            }
            if left {
                return LRESULT(HTLEFT as isize);
            }
            if right {
                return LRESULT(HTRIGHT as isize);
            }
            if bottom {
                return LRESULT(HTBOTTOM as isize);
            }
        }
        // 标题带优先于「顶边缩放」：Win11 把可缩放边框叠在标题栏上，
        // 顶边 5px 既要能拖高，也要能当标题栏双击 —— 双击需要 HTCAPTION，
        // 所以整条标题带（含最上面几像素）都按 HTCAPTION 处理。
        // 拖高改走左右两侧的上角（HTTOPLEFT / HTTOPRIGHT）和两侧边。
        if unsafe { in_caption_band(hwnd, sx, sy) } {
            return LRESULT(HTCAPTION as isize);
        }
        if !maximized {
            let b = border_px(hwnd);
            let left = sx - wr.left < b;
            let right = wr.right - sx <= b;
            let top = sy - wr.top < b;
            if top && left {
                return LRESULT(HTTOPLEFT as isize);
            }
            if top && right {
                return LRESULT(HTTOPRIGHT as isize);
            }
            if top {
                return LRESULT(HTTOP as isize);
            }
        }
        LRESULT(HTCLIENT as isize)
    }

    /// 客户区对齐：把「有没有 WS_THICKFRAME」对齐到窗口可见性。
    ///
    /// 去边框窗口必须在 NC 计算期间带 WS_THICKFRAME，DWM 才会给阴影和边缘；
    /// 但可见状态下带着它，WebView 就会比客户区小一格、边缘还会被画出 1px 边。
    /// 所以：**不可见时挂上（让 DWM 把尺寸/边缘算完整），可见时摘掉（整窗 = 客户区）**。
    unsafe fn apply_workarea(hwnd: HWND) {
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
        let has_thick = style & WS_THICKFRAME.0 != 0;
        let want_thick = !visible;
        if has_thick != want_thick {
            let next = if want_thick {
                style | WS_THICKFRAME.0
            } else {
                style & !WS_THICKFRAME.0
            };
            unsafe {
                let _ = SetWindowLongW(hwnd, GWL_STYLE, next as i32);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
        }
        sync_frame(hwnd);
    }

    /// 子窗口里，除了 WebView 自己的宿主链，别再去问。
    fn is_hit_passthrough(hwnd: HWND, hit: HWND) -> bool {
        if hit.is_invalid() || hit.0.is_null() {
            return false;
        }
        let mut cls = [0u16; 64];
        unsafe {
            let n = GetClassNameW(hit, &mut cls);
            let name = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
            // WebView 内部的窗口：它的命中测试一律 HTCLIENT，问它等于没问
            if name.contains("Chrome_") || name == "Intermediate D3D Window" {
                return false;
            }
            hit.0 != hwnd.0
        }
    }

    /// 把「本来该发给鼠标下子窗口」的消息转给它（见模块注释第 3 条）。
    ///
    /// 返回 true 表示**这条消息已经由子窗口处理**，调用方应当直接返回，不要再往下走。
    unsafe fn try_dispatch(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
        let pt = get_message_pt(lparam);
        let mut target = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(hwnd, &mut target);
        }
        if target.x < 0 || target.y < 0 {
            return false;
        }
        if unsafe { in_caption_band(hwnd, pt.x, pt.y) } {
            // 标题带：先让菜单栏认出自己是不是可点的控件
            if unsafe { child_wants_mouse(hwnd, target) } {
                return true;
            }
            // 空白处：交还系统做拖动 / 双击最大化
            unsafe {
                let _ = ReleaseCapture();
                let _ = SendMessageW(
                    hwnd,
                    WM_NCLBUTTONDOWN,
                    Some(WPARAM(HTCAPTION as usize)),
                    Some(LPARAM(((pt.y as isize) << 16) | ((pt.x as isize) & 0xFFFF))),
                );
            }
            let _ = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            return true;
        }
        if unsafe { child_wants_mouse(hwnd, target) } {
            true
        } else {
            let _ = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            false
        }
    }

    /// 鼠标下面的子窗口是否「要」这次点击（HTCLIENT 才算要）。
    /// 菜单按钮 / 侧栏 / 正文都返回 HTCLIENT；标题带空白处（菜单栏的 gap、spacer）不算。
    unsafe fn child_wants_mouse(hwnd: HWND, client_pt: POINT) -> bool {
        unsafe {
            let mut screen = client_pt;
            let _ = ClientToScreen(hwnd, &mut screen);
            let hit = ChildWindowFromPointEx(
                hwnd,
                client_pt,
                CWP_SKIPINVISIBLE | CWP_SKIPDISABLED | CWP_SKIPTRANSPARENT,
            );
            if is_hit_passthrough(hwnd, hit) {
                return true;
            }
            let r = SendMessageW(
                hit,
                WM_NCHITTEST,
                Some(WPARAM(0)),
                Some(LPARAM(((screen.y as isize) << 16) | ((screen.x as isize) & 0xFFFF))),
            );
            r.0 == HTCLIENT as isize
        }
    }

    fn get_message_pt(lparam: LPARAM) -> POINT {
        POINT {
            x: (lparam.0 & 0xFFFF) as u16 as i16 as i32,
            y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
        }
    }

    fn pre_filter(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_NCCALCSIZE => {
                // 客户区 = 整窗：tao 在「无装饰无阴影」时已经原样保留 rgrc，
                // 这里只把结果钉死，免得将来 tao 改行为又悄悄缩一条。
                if wparam.0 != 0 {
                    unsafe {
                        DefSubclassProc(hwnd, msg, wparam, lparam);
                    }
                    Some(LRESULT(0))
                } else {
                    None
                }
            }
            WM_NCHITTEST => {
                // 我们不是鼠标下的窗口（WebView 子窗口才是），但仍然要给出答案，
                // 好让 WebView 知道「这一块归外层管」。
                let r = unsafe { hit_test(hwnd, lparam) };
                Some(r)
            }
            _ => None,
        }
    }

    /// 主窗口命中测试：边和标题带归外层，其余透传。
    ///
    /// 关键：鼠标其实在 WebView 子窗口上（WebView 铺满整个客户区）。
    /// 按 Win32 规则，WebView 的 WM_NCHITTEST 返回 HTCAPTION 时系统会把拖动/双击
    /// 交给 WebView 的 HTCLIENT（Chromium 的默认实现），窗口永远动不了；
    /// 所以外层窗口必须回答「这一块是我的非客户区」—— 也就是 WebView 的命中测试里
    /// 透传祖先窗口的结论。见 wry 侧 WebView 的实现说明。
    unsafe fn hit_test_passthrough(hwnd: HWND, lparam: LPARAM) -> LRESULT {
        let r = unsafe { hit_test(hwnd, lparam) };
        if r.0 == HTCLIENT as isize {
            unsafe { DefSubclassProc(hwnd, WM_NCHITTEST, WPARAM(0), lparam) }
        } else {
            r
        }
    }

    unsafe extern "system" fn proc_(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        use windows::Win32::UI::WindowsAndMessaging::{
            WM_CREATE, WM_DPICHANGED, WM_NCACTIVATE, WM_SHOWWINDOW, WM_WINDOWPOSCHANGED,
        };
        match msg {
            // 建好 / 显隐切换时对齐一次客户区（见 apply_workarea）
            WM_CREATE | WM_SHOWWINDOW => {
                unsafe { apply_workarea(hwnd) };
                unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
            }
            WM_WINDOWPOSCHANGED | WM_DPICHANGED | WM_NCACTIVATE => {
                sync_frame(hwnd);
                unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
            }
            WM_NCCALCSIZE => {
                if let Some(r) = pre_filter(hwnd, msg, wparam, lparam) {
                    return r;
                }
                unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
            }
            WM_NCHITTEST => unsafe { hit_test_passthrough(hwnd, lparam) },
            // 鼠标按下：标题带空白处 → 交还系统拖动（细节见 try_dispatch）
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_NCLBUTTONDBLCLK | WM_RBUTTONDOWN
            | WM_MBUTTONDOWN => {
                if unsafe { try_dispatch(hwnd, msg, wparam, lparam) } {
                    return LRESULT(0);
                }
                unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
            }
            _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
        }
    }

    /// 窗口按钮动作。去边框窗口上 ShowWindow(SW_MAXIMIZE) 会被 tao 的尺寸缓存写回，
    /// 表现为「点一次只闪一下」。这里自己算目标矩形，用 SetWindowPos 一次到位。
    pub fn window_action(hwnd_raw: isize, action: &str) -> bool {
        let hwnd = HWND(hwnd_raw as _);
        unsafe {
            match action {
                "minimize" => {
                    let _ = ShowWindow(hwnd, SW_MINIMIZE);
                    true
                }
                "maximize" => {
                    toggle_maximize(hwnd, true);
                    true
                }
                "restore" => {
                    toggle_maximize(hwnd, false);
                    true
                }
                "toggle" => {
                    toggle_maximize(hwnd, !IS_MAXED.load(Ordering::Relaxed));
                    true
                }
                "close" => {
                    let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(SC_CLOSE as usize), LPARAM(0));
                    true
                }
                _ => false,
                // ── 标题栏拖动（JS 侧 mousedown 触发）──────────────────
                // 为什么走 IPC 而不是只靠 WM_NCHITTEST：鼠标永远落在 WebView
                // 子窗口上，父窗口的 hit_test 根本没机会回答；UI 侧知道哪里
                // 是「标题栏空白处」（菜单栏的 gap/spacer），由它发起最可靠。
                "drag" => {
                    // ReleaseCapture 必须在发起拖动前调，否则 SendMessage 会被忽略
                    let _ = ReleaseCapture();
                    // WM_NCLBUTTONDOWN + HTCAPTION = 系统进入「拖动窗口」模态循环
                    let _ = SendMessageW(
                        hwnd,
                        WM_NCLBUTTONDOWN,
                        Some(WPARAM(HTCAPTION as usize)),
                        Some(LPARAM(0)),
                    );
                    true
                }
                // 双击标题栏 -> 最大化/还原（系统双击在 WebView 上也被吃掉，走 IPC）
                "toggleMaximize" => {
                    toggle_maximize(hwnd, !IS_MAXED.load(Ordering::Relaxed));
                    true
                }
            }
        }
    }

    /// 最大化 = 铺满主屏；还原 = 回到记住的矩形（第一次还原用 80% 屏幕）。
    ///
    /// 只做一件事：把窗口 SetWindowPos 到目标矩形。不调 ShowWindow(SW_MAXIMIZE)，
    /// 因为去边框窗口上系统最大化会和我们的尺寸互相覆盖，表现为闪两下。
    fn toggle_maximize(hwnd: HWND, maximize: bool) {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        static SAVED: AtomicI32 = AtomicI32::new(0);
        static SX: AtomicI32 = AtomicI32::new(0);
        static SY: AtomicI32 = AtomicI32::new(0);
        static SW: AtomicI32 = AtomicI32::new(0);
        static SH: AtomicI32 = AtomicI32::new(0);
        unsafe {
            let scr_w = GetSystemMetrics(SM_CXSCREEN).max(640);
            let scr_h = GetSystemMetrics(SM_CYSCREEN).max(480);
            if maximize {
                let mut r = RECT::default();
                if GetWindowRect(hwnd, &mut r).is_ok() {
                    let cur_w = r.right - r.left;
                    let cur_h = r.bottom - r.top;
                    if cur_w < scr_w - 8 || cur_h < scr_h - 8 {
                        SX.store(r.left, Ordering::Relaxed);
                        SY.store(r.top, Ordering::Relaxed);
                        SW.store(cur_w, Ordering::Relaxed);
                        SH.store(cur_h, Ordering::Relaxed);
                        SAVED.store(1, Ordering::Relaxed);
                    }
                }
                let _ = SetWindowPos(hwnd, None, 0, 0, scr_w, scr_h, SWP_NOZORDER | SWP_NOACTIVATE);
                IS_MAXED.store(true, Ordering::Relaxed);
            } else {
                let saved = SAVED.load(Ordering::Relaxed) == 1;
                let (w, h) = if saved {
                    (SW.load(Ordering::Relaxed), SH.load(Ordering::Relaxed))
                } else {
                    ((scr_w as f64 * 0.8) as i32, (scr_h as f64 * 0.78) as i32)
                };
                let (x, y) = if saved {
                    (SX.load(Ordering::Relaxed), SY.load(Ordering::Relaxed))
                } else {
                    (40, 40)
                };
                let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
                IS_MAXED.store(false, Ordering::Relaxed);
            }
            sync_frame(hwnd);
        }
        emit_state();
    }

    pub fn is_maximized(_hwnd_raw: isize) -> bool {
        IS_MAXED.load(Ordering::Relaxed)
    }

}

#[cfg(windows)]
pub use imp::*;

#[cfg(not(windows))]
pub fn install(_h: isize) -> bool {
    false
}
#[cfg(not(windows))]
pub fn set_buttons_width(_px: i32) {}
#[cfg(not(windows))]
pub fn window_action(_h: isize, _a: &str) -> bool {
    false
}
#[cfg(not(windows))]
pub fn is_maximized(_h: isize) -> bool {
    false
}
#[cfg(not(windows))]
pub fn maximized_script(_m: bool) -> String {
    String::new()
}