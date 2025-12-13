// WindowHub Rust 后端
// 修复：深度输入焦点, Z序切换, 安全关闭, 全局快捷键
// 新增：防止卡死的安全措施

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton},
    AppHandle, Manager, Emitter, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[cfg(windows)]
use windows::Win32::{
    Foundation::{BOOL, HWND, LPARAM, WPARAM, RECT, TRUE, POINT},
    Graphics::Gdi::{InvalidateRect, ClientToScreen, ScreenToClient},
    UI::Input::KeyboardAndMouse::{GetAsyncKeyState, SetFocus},
    UI::WindowsAndMessaging::*,
    System::Threading::{GetCurrentProcessId, GetCurrentThreadId, AttachThreadInput},
};

static ORIGINAL_STYLES: Mutex<Vec<(isize, i32, i32, RECT)>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    pub class_name: String,
    pub width: i32,
    pub height: i32,
}

#[cfg(windows)]
fn get_current_pid() -> u32 {
    unsafe { GetCurrentProcessId() }
}
#[cfg(not(windows))]
fn get_current_pid() -> u32 { 0 }

#[cfg(windows)]
fn is_self_window(hwnd: HWND) -> bool {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == get_current_pid()
    }
}

// 辅助：获取类名
#[cfg(windows)]
unsafe fn get_class_name(hwnd: HWND) -> String {
    let mut class_buf = [0u16; 256];
    let class_len = GetClassNameW(hwnd, &mut class_buf);
    String::from_utf16_lossy(&class_buf[..class_len as usize])
}

// 辅助：检查是否是危险窗口（可能导致卡死）
#[cfg(windows)]
fn is_dangerous_window(class_name: &str) -> bool {
    // 这些窗口类型嵌入后可能导致系统不稳定
    let dangerous = [
        "CabinetWClass",        // 文件资源管理器
        "ExplorerWClass",       // 文件资源管理器变体
        "Progman",              // 桌面
        "WorkerW",              // 桌面工作区
        "Shell_TrayWnd",        // 任务栏
        "Shell_SecondaryTrayWnd", // 副屏任务栏
        "TaskManagerWindow",    // 任务管理器
        "Windows.UI.Core.CoreWindow", // UWP 应用
    ];
    dangerous.iter().any(|d| class_name.contains(d))
}

#[tauri::command]
fn enumerate_windows() -> Vec<WindowInfo> {
    #[cfg(windows)]
    {
        let mut windows: Vec<WindowInfo> = Vec::new();
        unsafe {
            let _ = EnumWindows(
                Some(enum_window_callback),
                LPARAM(&mut windows as *mut Vec<WindowInfo> as isize),
            );
        }
        windows
    }
    #[cfg(not(windows))]
    Vec::new()
}

#[cfg(windows)]
unsafe extern "system" fn enum_window_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows = &mut *(lparam.0 as *mut Vec<WindowInfo>);
    if !IsWindowVisible(hwnd).as_bool() { return TRUE; }
    if is_self_window(hwnd) { return TRUE; }
    let title = get_window_title_inner(hwnd);
    if title.is_empty() || title.contains("WindowHub") { return TRUE; }
    
    let class_name = get_class_name(hwnd);
    let excluded = ["Progman", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", 
                    "Windows.UI.Core.CoreWindow", "ApplicationFrameWindow",
                    "WorkerW", "TaskManagerWindow"];
    if excluded.contains(&class_name.as_str()) { return TRUE; }
    
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_ok() {
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width > 100 && height > 100 {
            windows.push(WindowInfo { hwnd: hwnd.0 as isize, title, class_name, width, height });
        }
    }
    TRUE
}

#[cfg(windows)]
unsafe fn get_window_title_inner(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len == 0 { return String::new(); }
    let mut buf = vec![0u16; (len + 1) as usize];
    GetWindowTextW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..len as usize])
}

#[tauri::command]
fn embed_window(app: AppHandle, target_hwnd: isize) -> Result<bool, String> {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(target_hwnd as *mut _);
        if is_self_window(hwnd) { return Err("不能嵌入自身".to_string()); }

        // 检查是否是危险窗口
        let class_name = get_class_name(hwnd);
        if is_dangerous_window(&class_name) {
            return Err(format!("不支持嵌入此类型窗口: {}", class_name));
        }

        let main_window = app.get_webview_window("main").ok_or("无法获取主窗口")?;
        let parent_hwnd_raw = main_window.hwnd().map_err(|e| e.to_string())?;
        let parent = HWND(parent_hwnd_raw.0 as *mut _);
        
        let parent_style = GetWindowLongW(parent, GWL_STYLE);
        if (parent_style as u32 & WS_CLIPCHILDREN.0) == 0 {
             SetWindowLongW(parent, GWL_STYLE, parent_style | WS_CLIPCHILDREN.0 as i32);
        }

        let original_style = GetWindowLongW(hwnd, GWL_STYLE);
        let original_exstyle = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let mut original_rect = RECT::default();
        GetWindowRect(hwnd, &mut original_rect);
        
        {
            let mut styles = ORIGINAL_STYLES.lock().unwrap();
            if !styles.iter().any(|(h, _, _, _)| *h == target_hwnd) {
                styles.push((target_hwnd, original_style, original_exstyle, original_rect));
            }
        }
        
        let new_style = (original_style as u32 
            & !(WS_CAPTION.0 | WS_THICKFRAME.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0 | WS_SYSMENU.0 | WS_POPUP.0 | WS_BORDER.0 | WS_DLGFRAME.0))
            | WS_CHILD.0 | WS_VISIBLE.0;

        SetWindowLongW(hwnd, GWL_STYLE, new_style as i32);
        SetParent(hwnd, parent);
        
        SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);

        let _ = activate_window(target_hwnd);
        
        println!("嵌入窗口成功: hwnd={}, class={}", target_hwnd, class_name);
        Ok(true)
    }
    #[cfg(not(windows))]
    Err("仅支持 Windows".to_string())
}

#[tauri::command]
fn release_window(target_hwnd: isize) -> Result<bool, String> {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(target_hwnd as *mut _);
        
        // 安全地断开线程连接
        let id_current = GetCurrentThreadId();
        let id_target = GetWindowThreadProcessId(hwnd, None);
        if id_current != id_target {
            let _ = AttachThreadInput(id_current, id_target, false);
        }

        let _ = SetParent(hwnd, HWND(0 as _)); 
        
        let styles = ORIGINAL_STYLES.lock().unwrap();
        if let Some((_, original_style, original_exstyle, rect)) = styles.iter().find(|(h, _, _, _)| *h == target_hwnd) {
            SetWindowLongW(hwnd, GWL_STYLE, *original_style);
            SetWindowLongW(hwnd, GWL_EXSTYLE, *original_exstyle);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            SetWindowPos(hwnd, HWND_TOP, rect.left, rect.top, width, height, SWP_FRAMECHANGED | SWP_SHOWWINDOW);
        } else {
            let default_style = WS_OVERLAPPEDWINDOW.0 | WS_VISIBLE.0;
            SetWindowLongW(hwnd, GWL_STYLE, default_style as i32);
            SetWindowPos(hwnd, HWND_TOP, 100, 100, 800, 600, SWP_FRAMECHANGED | SWP_SHOWWINDOW);
        }
        
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
        Ok(true)
    }
    #[cfg(not(windows))]
    Err("仅支持 Windows".to_string())
}

#[tauri::command]
fn update_window_rect(target_hwnd: isize, x: i32, y: i32, width: i32, height: i32) -> Result<bool, String> {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(target_hwnd as *mut _);
        
        // 检查窗口是否还有效
        if !IsWindow(hwnd).as_bool() {
            return Ok(false);
        }
        
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok() {
             let parent = GetParent(hwnd);
             if parent.is_ok() {
                  let mut pt_tl = POINT { x: rect.left, y: rect.top };
                  ScreenToClient(parent.unwrap(), &mut pt_tl);
                  
                  let current_w = rect.right - rect.left;
                  let current_h = rect.bottom - rect.top;
                  
                  if (pt_tl.x - x).abs() <= 1 && (pt_tl.y - y).abs() <= 1 && 
                     (current_w - width).abs() <= 1 && (current_h - height).abs() <= 1 {
                      return Ok(true);
                  }
             }
        }

        SetWindowPos(hwnd, HWND::default(), x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        Ok(true)
    }
    #[cfg(not(windows))]
    Err("仅支持 Windows".to_string())
}

// 递归查找 Chrome_RenderWidgetHostHWND
#[cfg(windows)]
unsafe fn find_render_window(hwnd: HWND) -> Option<HWND> {
    let mut target = None;
    EnumChildWindows(hwnd, Some(find_render_window_callback), LPARAM(&mut target as *mut Option<HWND> as isize));
    target
}

#[cfg(windows)]
unsafe extern "system" fn find_render_window_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let target = &mut *(lparam.0 as *mut Option<HWND>);
    let class_name = get_class_name(hwnd);
    if class_name == "Chrome_RenderWidgetHostHWND" {
        *target = Some(hwnd);
        return BOOL(0);
    }
    TRUE
}

#[tauri::command]
fn activate_window(target_hwnd: isize) -> Result<bool, String> {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(target_hwnd as *mut _);
        
        // 检查窗口是否有效
        if !IsWindow(hwnd).as_bool() {
            return Ok(false);
        }
        
        let id_current = GetCurrentThreadId();
        let id_target = GetWindowThreadProcessId(hwnd, None);
        
        // 只在不同线程时才 Attach，避免死锁
        let attached = if id_current != id_target {
            AttachThreadInput(id_current, id_target, true).as_bool()
        } else {
            false
        };
        
        SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        
        if let Some(render_hwnd) = find_render_window(hwnd) {
            windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(render_hwnd);
            SetFocus(render_hwnd);
        } else {
             windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(hwnd);
             SetFocus(hwnd);
        }
        
        // 短暂延迟后断开，避免死锁
        if attached {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                unsafe { let _ = AttachThreadInput(id_current, id_target, false); }
            });
        }
        
        Ok(true)
    }
    #[cfg(not(windows))]
    Err("仅支持 Windows".to_string())
}

#[tauri::command]
fn close_target_window(target_hwnd: isize) -> Result<bool, String> {
    #[cfg(windows)]
    unsafe {
        let _ = release_window(target_hwnd);
        let hwnd = HWND(target_hwnd as *mut _);
        let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
        Ok(true)
    }
    #[cfg(not(windows))]
    Err("仅支持 Windows".to_string())
}

#[tauri::command]
fn is_window_valid(target_hwnd: isize) -> bool {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(target_hwnd as *mut _);
        IsWindow(hwnd).as_bool()
    }
    #[cfg(not(windows))]
    false
}

#[tauri::command]
fn is_cursor_in_client_area(app: AppHandle, top_offset: i32) -> bool {
    #[cfg(windows)]
    unsafe {
        let main_window = match app.get_webview_window("main") {
            Some(w) => w,
            None => return false,
        };
        let hwnd_raw = match main_window.hwnd() {
            Ok(h) => h,
            Err(_) => return false,
        };
        let hwnd = HWND(hwnd_raw.0 as *mut _);

        let mut cursor_point = POINT::default();
        GetCursorPos(&mut cursor_point);

        let mut client_point = POINT { x: 0, y: 0 };
        ClientToScreen(hwnd, &mut client_point);
        
        let mut client_rect = RECT::default();
        GetClientRect(hwnd, &mut client_rect);

        let screen_left = client_point.x;
        let screen_top = client_point.y;
        let screen_right = screen_left + client_rect.right;
        let screen_bottom = screen_top + client_rect.bottom;

        let effective_top = screen_top + top_offset;

        cursor_point.x >= screen_left && cursor_point.x <= screen_right &&
        cursor_point.y >= effective_top && cursor_point.y <= screen_bottom
    }
    #[cfg(not(windows))]
    false
}

#[tauri::command]
fn get_foreground_window() -> isize {
    #[cfg(windows)]
    unsafe { GetForegroundWindow().0 as isize }
    #[cfg(not(windows))]
    0
}

#[tauri::command]
fn get_main_window_hwnd(app: AppHandle) -> isize {
    #[cfg(windows)]
    {
        if let Some(w) = app.get_webview_window("main") {
            if let Ok(h) = w.hwnd() {
                return h.0 as isize;
            }
        }
        0
    }
    #[cfg(not(windows))]
    0
}

#[tauri::command]
fn get_window_title(target_hwnd: isize) -> String {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(target_hwnd as *mut _);
        get_window_title_inner(hwnd)
    }
    #[cfg(not(windows))]
    String::new()
}

#[tauri::command]
fn is_mouse_left_down() -> bool {
    #[cfg(windows)]
    unsafe { (GetAsyncKeyState(0x01) as u16 & 0x8000) != 0 }
    #[cfg(not(windows))]
    false
}

// 检查窗口是否可以安全嵌入
#[tauri::command]
fn can_embed_window(target_hwnd: isize) -> Result<bool, String> {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(target_hwnd as *mut _);
        if is_self_window(hwnd) { 
            return Err("不能嵌入自身".to_string()); 
        }
        let class_name = get_class_name(hwnd);
        if is_dangerous_window(&class_name) {
            return Err(format!("不支持嵌入此窗口类型: {}", class_name));
        }
        Ok(true)
    }
    #[cfg(not(windows))]
    Ok(true)
}

// 隐藏嵌入窗口（搜索时使用）
#[tauri::command]
fn hide_window(target_hwnd: isize) -> bool {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(target_hwnd as *mut _);
        ShowWindow(hwnd, SW_HIDE);
        true
    }
    #[cfg(not(windows))]
    false
}

// 显示嵌入窗口（搜索结束时使用）
#[tauri::command]
fn show_window(target_hwnd: isize) -> bool {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(target_hwnd as *mut _);
        ShowWindow(hwnd, SW_SHOW);
        true
    }
    #[cfg(not(windows))]
    false
}


// ============================================================
// 新功能：枚举已安装应用 & 启动应用
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub path: String,  // .lnk 或 .exe 路径
}

#[tauri::command]
fn enumerate_installed_apps() -> Vec<AppInfo> {
    #[cfg(windows)]
    {
        use std::path::PathBuf;
        let mut apps: Vec<AppInfo> = Vec::new();
        
        // 扫描开始菜单目录
        let start_menu_paths = vec![
            std::env::var("APPDATA").ok().map(|p| PathBuf::from(p).join("Microsoft\\Windows\\Start Menu\\Programs")),
            std::env::var("ProgramData").ok().map(|p| PathBuf::from(p).join("Microsoft\\Windows\\Start Menu\\Programs")),
        ];
        
        for path_opt in start_menu_paths {
            if let Some(path) = path_opt {
                if path.exists() {
                    scan_shortcuts(&path, &mut apps);
                }
            }
        }
        
        // 去重
        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        apps.dedup_by(|a, b| a.name.to_lowercase() == b.name.to_lowercase());
        
        apps
    }
    #[cfg(not(windows))]
    Vec::new()
}

#[cfg(windows)]
fn scan_shortcuts(dir: &std::path::Path, apps: &mut Vec<AppInfo>) {
    use walkdir::WalkDir;
    
    for entry in WalkDir::new(dir).max_depth(3).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if let Some(ext) = path.extension() {
            if ext == "lnk" || ext == "exe" {
                if let Some(name) = path.file_stem() {
                    let name_str = name.to_string_lossy().to_string();
                    // 过滤掉一些不需要的项目
                    if !name_str.contains("Uninstall") && !name_str.contains("卸载") {
                        apps.push(AppInfo {
                            name: name_str,
                            path: path.to_string_lossy().to_string(),
                        });
                    }
                }
            }
        }
    }
}

#[tauri::command]
async fn launch_app(path: String) -> Result<isize, String> {
    #[cfg(windows)]
    {
        use std::process::Command;
        use std::time::Duration;
        
        // 获取启动前的窗口列表
        let before_windows: std::collections::HashSet<isize> = enumerate_windows()
            .iter()
            .map(|w| w.hwnd)
            .collect();
        
        // 启动应用
        let result = if path.ends_with(".lnk") {
            // 使用 explorer 打开快捷方式
            Command::new("cmd")
                .args(["/C", "start", "", &path])
                .spawn()
        } else {
            Command::new(&path).spawn()
        };
        
        if let Err(e) = result {
            return Err(format!("启动失败: {}", e));
        }
        
        // 等待新窗口出现（最多等待 10 秒）
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            
            let current_windows = enumerate_windows();
            for win in &current_windows {
                if !before_windows.contains(&win.hwnd) {
                    // 找到新窗口！
                    return Ok(win.hwnd);
                }
            }
        }
        
        Err("应用已启动，但未检测到新窗口".to_string())
    }
    #[cfg(not(windows))]
    Err("仅支持 Windows".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(|app, shortcut, event| {
            if event.state == ShortcutState::Pressed {
                 let s = shortcut.to_string();
                 println!("[HANDLER] 处理快捷键: {}", s);
                 
                 // Tauri v2 格式: alt+Digit1, control+KeyK, shift+control+Tab
                 // 转换为小写进行匹配
                 let s_lower = s.to_lowercase();
                 
                 // Alt+1~9: 切换到指定标签
                 if s_lower.starts_with("alt+digit") {
                     if let Some(c) = s_lower.chars().last() {
                         if let Some(digit) = c.to_digit(10) {
                             println!("[HANDLER] 发送事件: switch-tab({})", digit);
                             let _ = app.emit("switch-tab", digit);
                             return;
                         }
                     }
                 }
                 
                 // Ctrl+W: 关闭当前标签
                 if s_lower == "control+keyw" {
                     println!("[HANDLER] 发送事件: close-current-tab");
                     let _ = app.emit("close-current-tab", ());
                     return;
                 }
                 
                 // Ctrl+Tab: 下一个标签
                 if s_lower == "control+tab" {
                     println!("[HANDLER] 发送事件: next-tab");
                     let _ = app.emit("next-tab", ());
                     return;
                 }
                 
                 // Ctrl+Shift+Tab: 上一个标签
                 if s_lower == "shift+control+tab" || s_lower == "control+shift+tab" {
                     println!("[HANDLER] 发送事件: prev-tab");
                     let _ = app.emit("prev-tab", ());
                     return;
                 }
                 
                 // Ctrl+K: 打开搜索
                 if s_lower == "control+keyk" {
                     println!("[HANDLER] 发送事件: open-search");
                     let _ = app.emit("open-search", ());
                     return;
                 }

                 // Alt+Space: Toggle Window
                 if s_lower == "alt+space" {
                     if let Some(window) = app.get_webview_window("main") {
                        if window.is_visible().unwrap_or(false) {
                            let _ = window.hide();
                        } else {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                     }
                     return;
                 }
                 
                 println!("[HANDLER] 未匹配的快捷键: {}", s);
            }
        })
        .build())
        .invoke_handler(tauri::generate_handler![
            enumerate_windows,
            embed_window,
            release_window,
            update_window_rect,
            activate_window,
            get_foreground_window,
            get_window_title,
            is_mouse_left_down,
            is_cursor_in_client_area,
            get_main_window_hwnd,
            close_target_window,
            is_window_valid,
            can_embed_window,
            hide_window,
            show_window,
            enumerate_installed_apps,
            launch_app
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                window.hide().unwrap();
                api.prevent_close();
            }
        })
        .setup(|app| {
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                
                println!("[SETUP] 开始注册全局快捷键...");
                
                // Alt+1~9: 切换到指定标签
                for i in 1..=9 {
                    let shortcut = format!("Alt+{}", i);
                    match app.global_shortcut().register(shortcut.as_str()) {
                        Ok(_) => println!("[SETUP] ✅ 注册成功: {}", shortcut),
                        Err(e) => println!("[SETUP] ❌ 注册失败: {} - {:?}", shortcut, e),
                    }
                }
                
                // Ctrl+W: 关闭当前标签
                match app.global_shortcut().register("Ctrl+W") {
                    Ok(_) => println!("[SETUP] ✅ 注册成功: Ctrl+W"),
                    Err(e) => println!("[SETUP] ❌ 注册失败: Ctrl+W - {:?}", e),
                }
                
                // Ctrl+Tab: 下一个标签
                match app.global_shortcut().register("Ctrl+Tab") {
                    Ok(_) => println!("[SETUP] ✅ 注册成功: Ctrl+Tab"),
                    Err(e) => println!("[SETUP] ❌ 注册失败: Ctrl+Tab - {:?}", e),
                }
                
                // Ctrl+Shift+Tab: 上一个标签
                match app.global_shortcut().register("Ctrl+Shift+Tab") {
                    Ok(_) => println!("[SETUP] ✅ 注册成功: Ctrl+Shift+Tab"),
                    Err(e) => println!("[SETUP] ❌ 注册失败: Ctrl+Shift+Tab - {:?}", e),
                }
                
                // Ctrl+K: 打开搜索
                match app.global_shortcut().register("Ctrl+K") {
                    Ok(_) => println!("[SETUP] ✅ 注册成功: Ctrl+K"),
                    Err(e) => println!("[SETUP] ❌ 注册失败: Ctrl+K - {:?}", e),
                }

                // Alt+Space: Toggle
                match app.global_shortcut().register("Alt+Space") {
                    Ok(_) => println!("[SETUP] ✅ 注册成功: Alt+Space"),
                    Err(e) => println!("[SETUP] ❌ 注册失败: Alt+Space - {:?}", e),
                }
                
                
                println!("[SETUP] 快捷键注册完成！");

                // --- 托盘图标设置 ---
                let quit_i = MenuItem::with_id(app, "quit", "退出 WindowHub", true, None::<&str>)?;
                let show_i = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

                let _ = TrayIconBuilder::new()
                    .icon(app.default_window_icon().unwrap().clone())
                    .menu(&menu)
                    .on_menu_event(|app, event| {
                        match event.id.as_ref() {
                            "quit" => {
                                app.exit(0);
                            }
                            "show" => {
                                if let Some(window) = app.get_webview_window("main") {
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                            _ => {}
                        }
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                if window.is_visible().unwrap_or(false) {
                                    let _ = window.hide();
                                } else {
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                    })
                    .build(app);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}



