//! Screen control: look at the screen and drive mouse and keyboard, for agents that need to see a real desktop app.
//! It runs here, in the daemon, behind `computer.*` calls: the switch (off until the user turns it on in the app, reset
//! on every start) is checked by the daemon before any of this code runs, and every call of an agent is audited, so
//! nothing depends on the CLI behaving. Windows uses the Win32 API directly, macOS the system tools.

use anyhow::{bail, Result};
use base64::Engine;
use serde_json::{json, Value};

pub struct Win {
    pub id: u64,
    pub title: String,
    pub process: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

fn window_arg(p: &Value) -> Option<u64> {
    p["window"].as_u64()
}

fn int(p: &Value, k: &str) -> i32 {
    p[k].as_i64().map_or(0, |v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

/// Runs one `computer.*` action. The caller has already checked that screen control is on.
pub fn call(method: &str, p: &Value) -> Result<Value> {
    match method {
        "computer.windows" => {
            let v: Vec<_> = sys::windows()?
                .iter()
                .map(|w| json!({"id": w.id, "title": w.title, "process": w.process, "x": w.x, "y": w.y, "width": w.w, "height": w.h}))
                .collect();
            Ok(json!({"windows": v}))
        }
        "computer.screenshot" => {
            let (w, h, rgba) = sys::capture(window_arg(p))?;
            let mut png_bytes = Vec::new();
            let mut enc = png::Encoder::new(&mut png_bytes, w as u32, h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()?.write_image_data(&rgba)?;
            Ok(json!({"width": w, "height": h, "png": base64::engine::general_purpose::STANDARD.encode(png_bytes)}))
        }
        "computer.click" => {
            let button = p["button"].as_str().unwrap_or("left");
            if !["left", "right", "middle"].contains(&button) {
                bail!("--button must be left, right or middle");
            }
            let (ox, oy) = origin(window_arg(p))?;
            sys::click(ox + int(p, "x"), oy + int(p, "y"), button, p["double"].as_bool() == Some(true))?;
            Ok(json!({}))
        }
        "computer.type" => {
            if let Some(w) = window_arg(p) {
                sys::focus(w)?;
            }
            sys::type_text(p["text"].as_str().unwrap_or_default())?;
            Ok(json!({}))
        }
        "computer.key" => {
            if let Some(w) = window_arg(p) {
                sys::focus(w)?;
            }
            sys::key(p["combo"].as_str().unwrap_or_default())?;
            Ok(json!({}))
        }
        "computer.scroll" => {
            let (ox, oy) = origin(window_arg(p))?;
            sys::scroll(ox + int(p, "x"), oy + int(p, "y"), int(p, "dx"), int(p, "dy"))?;
            Ok(json!({}))
        }
        "computer.focus" => {
            sys::focus(window_arg(p).ok_or_else(|| anyhow::anyhow!("window is required"))?)?;
            Ok(json!({}))
        }
        _ => bail!("unknown method {method}"),
    }
}

/// Top-left corner coordinates are relative to: the window's, or the screen's.
fn origin(window: Option<u64>) -> Result<(i32, i32)> {
    match window {
        None => Ok((0, 0)),
        Some(id) => {
            let w = sys::windows()?.into_iter().find(|w| w.id == id);
            w.map(|w| (w.x, w.y)).ok_or_else(|| anyhow::anyhow!("no window with id {id} (see `axelstra computer windows`)"))
        }
    }
}

/// Splits "ctrl+shift+s" into modifiers and the final key name.
pub fn parse_combo(combo: &str) -> Result<(Vec<String>, String)> {
    let parts: Vec<String> = combo.split('+').map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty()).collect();
    let Some((key, mods)) = parts.split_last() else { bail!("empty key combination") };
    for m in mods {
        if !["ctrl", "control", "shift", "alt", "option", "win", "cmd", "command", "meta", "super"].contains(&m.as_str()) {
            bail!("unknown modifier {m:?} (ctrl, shift, alt, win/cmd)");
        }
    }
    Ok((mods.to_vec(), key.clone()))
}

#[cfg(windows)]
mod sys {
    use super::*;
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::Storage::Xps::PrintWindow;
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    fn dpi() {
        // real pixels, so coordinates in a screenshot are the ones the mouse uses
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    }

    fn rect_of(h: HWND) -> RECT {
        let mut r: RECT = unsafe { zeroed() };
        unsafe { GetWindowRect(h, &mut r) };
        r
    }

    unsafe extern "system" fn collect(h: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp as *mut Vec<HWND>);
        if IsWindowVisible(h) != 0 {
            out.push(h);
        }
        1
    }

    pub fn windows() -> Result<Vec<Win>> {
        dpi();
        let mut hs: Vec<HWND> = vec![];
        unsafe { EnumWindows(Some(collect), &mut hs as *mut _ as LPARAM) };
        let mut out = vec![];
        for h in hs {
            let mut buf = [0u16; 512];
            let n = unsafe { GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32) };
            if n <= 0 {
                continue;
            }
            // tool windows and suspended UWP shells are not something a person sees
            let ex = unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32;
            let mut cloaked = 0u32;
            unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, &mut cloaked as *mut _ as *mut _, 4) };
            let r = rect_of(h);
            if ex & WS_EX_TOOLWINDOW != 0 || cloaked != 0 || r.right - r.left <= 1 || r.bottom - r.top <= 1 {
                continue;
            }
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(h, &mut pid) };
            out.push(Win {
                id: h as usize as u64,
                title: String::from_utf16_lossy(&buf[..n as usize]),
                process: process_name(pid),
                x: r.left,
                y: r.top,
                w: r.right - r.left,
                h: r.bottom - r.top,
            });
        }
        Ok(out)
    }

    fn process_name(pid: u32) -> String {
        unsafe {
            let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if p.is_null() {
                return String::new();
            }
            let mut buf = [0u16; 520];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(p, 0, buf.as_mut_ptr(), &mut len);
            windows_sys::Win32::Foundation::CloseHandle(p);
            if ok == 0 {
                return String::new();
            }
            let full = String::from_utf16_lossy(&buf[..len as usize]);
            full.rsplit(['\\', '/']).next().unwrap_or("").to_string()
        }
    }

    /// RGBA pixels of a window (even if partly covered) or of the whole virtual screen.
    pub fn capture(window: Option<u64>) -> Result<(i32, i32, Vec<u8>)> {
        dpi();
        unsafe {
            let (x, y, w, h, hwnd) = match window {
                Some(id) => {
                    let hw = id as usize as HWND;
                    let r = rect_of(hw);
                    (r.left, r.top, r.right - r.left, r.bottom - r.top, Some(hw))
                }
                None => (
                    GetSystemMetrics(SM_XVIRTUALSCREEN),
                    GetSystemMetrics(SM_YVIRTUALSCREEN),
                    GetSystemMetrics(SM_CXVIRTUALSCREEN),
                    GetSystemMetrics(SM_CYVIRTUALSCREEN),
                    None,
                ),
            };
            if w <= 0 || h <= 0 {
                bail!("nothing to capture (is the window minimized?)");
            }
            let screen = GetDC(std::ptr::null_mut());
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp);
            let mut done = false;
            if let Some(hw) = hwnd {
                // PW_RENDERFULLCONTENT (2) also captures modern composited windows
                done = PrintWindow(hw, mem, 2) != 0;
            }
            if !done {
                BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
            }
            let mut bi: BITMAPINFO = zeroed();
            bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h; // top-down
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            let mut px = vec![0u8; (w * h * 4) as usize];
            let lines = GetDIBits(mem, bmp, 0, h as u32, px.as_mut_ptr() as *mut _, &mut bi, DIB_RGB_COLORS);
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);
            if lines == 0 {
                bail!("could not read the screen contents");
            }
            for p in px.as_chunks_mut::<4>().0 {
                p.swap(0, 2); // BGRA -> RGBA
                p[3] = 255;
            }
            Ok((w, h, px))
        }
    }

    fn send(inputs: &[INPUT]) {
        unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    }

    fn mouse(flags: u32, data: i32) -> INPUT {
        let mut i: INPUT = unsafe { zeroed() };
        i.r#type = INPUT_MOUSE;
        i.Anonymous.mi = MOUSEINPUT { dx: 0, dy: 0, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 };
        i
    }

    fn keyboard(vk: u16, scan: u16, flags: u32) -> INPUT {
        let mut i: INPUT = unsafe { zeroed() };
        i.r#type = INPUT_KEYBOARD;
        i.Anonymous.ki = KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 };
        i
    }

    pub fn click(x: i32, y: i32, button: &str, double: bool) -> Result<()> {
        dpi();
        let (down, up) = match button {
            "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            _ => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        };
        unsafe { SetCursorPos(x, y) };
        std::thread::sleep(std::time::Duration::from_millis(40));
        for _ in 0..if double { 2 } else { 1 } {
            send(&[mouse(down, 0), mouse(up, 0)]);
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        Ok(())
    }

    pub fn scroll(x: i32, y: i32, dx: i32, dy: i32) -> Result<()> {
        dpi();
        unsafe { SetCursorPos(x, y) };
        if dy != 0 {
            send(&[mouse(MOUSEEVENTF_WHEEL, dy * 120)]);
        }
        if dx != 0 {
            send(&[mouse(MOUSEEVENTF_HWHEEL, dx * 120)]);
        }
        Ok(())
    }

    pub fn type_text(text: &str) -> Result<()> {
        for u in text.encode_utf16() {
            send(&[keyboard(0, u, KEYEVENTF_UNICODE), keyboard(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)]);
        }
        Ok(())
    }

    fn vk(name: &str) -> Result<u16> {
        Ok(match name {
            "ctrl" | "control" => VK_CONTROL,
            "shift" => VK_SHIFT,
            "alt" | "option" => VK_MENU,
            "win" | "cmd" | "command" | "meta" | "super" => VK_LWIN,
            "enter" | "return" => VK_RETURN,
            "tab" => VK_TAB,
            "esc" | "escape" => VK_ESCAPE,
            "space" => VK_SPACE,
            "backspace" => VK_BACK,
            "delete" | "del" => VK_DELETE,
            "up" => VK_UP,
            "down" => VK_DOWN,
            "left" => VK_LEFT,
            "right" => VK_RIGHT,
            "home" => VK_HOME,
            "end" => VK_END,
            "pageup" => VK_PRIOR,
            "pagedown" => VK_NEXT,
            n if n.len() >= 2 && n.starts_with('f') && n[1..].parse::<u16>().is_ok_and(|k| (1..=12).contains(&k)) => {
                VK_F1 + n[1..].parse::<u16>().unwrap() - 1
            }
            n if n.len() == 1 && n.as_bytes()[0].is_ascii_alphanumeric() => n.as_bytes()[0].to_ascii_uppercase() as u16,
            n => bail!("unknown key {n:?}"),
        })
    }

    pub fn key(combo: &str) -> Result<()> {
        let (mods, key) = parse_combo(combo)?;
        let mods: Vec<u16> = mods.iter().map(|m| vk(m)).collect::<Result<_>>()?;
        let k = vk(&key)?;
        let mut seq: Vec<INPUT> = mods.iter().map(|m| keyboard(*m, 0, 0)).collect();
        seq.push(keyboard(k, 0, 0));
        seq.push(keyboard(k, 0, KEYEVENTF_KEYUP));
        seq.extend(mods.iter().rev().map(|m| keyboard(*m, 0, KEYEVENTF_KEYUP)));
        send(&seq);
        Ok(())
    }

    pub fn focus(id: u64) -> Result<()> {
        let h = id as usize as HWND;
        unsafe {
            if IsWindow(h) == 0 {
                bail!("no window with id {id}");
            }
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }
            // a tap on Alt lets a background process take the foreground
            send(&[keyboard(VK_MENU, 0, 0), keyboard(VK_MENU, 0, KEYEVENTF_KEYUP)]);
            SetForegroundWindow(h);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod sys {
    use super::*;
    use std::process::Command;

    fn osa(script: &str) -> Result<String> {
        let o = Command::new("osascript").args(["-e", script]).output()?;
        if !o.status.success() {
            bail!(
                "{} (macOS asks for Accessibility and Screen Recording permission for the terminal app)",
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    }

    pub fn windows() -> Result<Vec<Win>> {
        let out = osa(r#"tell application "System Events"
set res to ""
repeat with p in (processes whose background only is false)
repeat with w in windows of p
set pos to position of w
set sz to size of w
set res to res & (unix id of p) & "|" & (name of p) & "|" & (item 1 of pos) & "|" & (item 2 of pos) & "|" & (item 1 of sz) & "|" & (item 2 of sz) & "|" & (name of w) & linefeed
end repeat
end repeat
return res
end tell"#)?;
        let mut list = vec![];
        for l in out.lines() {
            let f: Vec<&str> = l.splitn(7, '|').collect();
            if f.len() == 7 {
                let n = |i: usize| f[i].trim().parse::<i32>().unwrap_or(0);
                list.push(Win {
                    id: f[0].parse().unwrap_or(0),
                    title: f[6].to_string(),
                    process: f[1].to_string(),
                    x: n(2),
                    y: n(3),
                    w: n(4),
                    h: n(5),
                });
            }
        }
        Ok(list)
    }

    pub fn capture(window: Option<u64>) -> Result<(i32, i32, Vec<u8>)> {
        let tmp = std::env::temp_dir().join(format!("axelstra-cap-{}.png", std::process::id()));
        let mut cmd = Command::new("screencapture");
        cmd.arg("-x");
        if let Some(id) = window {
            let w = windows()?.into_iter().find(|w| w.id == id).ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
            cmd.arg("-R").arg(format!("{},{},{},{}", w.x, w.y, w.w, w.h));
        }
        if !cmd.arg(&tmp).status()?.success() {
            bail!("screencapture failed (Screen Recording permission?)");
        }
        let dec = png::Decoder::new(std::fs::File::open(&tmp)?);
        let mut reader = dec.read_info()?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf)?;
        let _ = std::fs::remove_file(&tmp);
        buf.truncate(info.buffer_size());
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => buf.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            _ => bail!("unexpected screenshot format"),
        };
        Ok((info.width as i32, info.height as i32, rgba))
    }

    pub fn click(x: i32, y: i32, button: &str, double: bool) -> Result<()> {
        if button != "left" {
            bail!("only left clicks are supported on macOS");
        }
        osa(&format!("tell application \"System Events\" to click at {{{x}, {y}}}"))?;
        if double {
            osa(&format!("tell application \"System Events\" to click at {{{x}, {y}}}"))?;
        }
        Ok(())
    }

    pub fn scroll(_x: i32, _y: i32, _dx: i32, dy: i32) -> Result<()> {
        // page keys are the portable way to scroll through System Events
        for _ in 0..dy.unsigned_abs().max(1) {
            osa(&format!("tell application \"System Events\" to key code {}", if dy < 0 { 121 } else { 116 }))?;
        }
        Ok(())
    }

    pub fn type_text(text: &str) -> Result<()> {
        let esc = text.replace('\\', "\\\\").replace('"', "\\\"");
        osa(&format!("tell application \"System Events\" to keystroke \"{esc}\""))?;
        Ok(())
    }

    pub fn key(combo: &str) -> Result<()> {
        let (mods, key) = parse_combo(combo)?;
        let using: Vec<&str> = mods
            .iter()
            .map(|m| match m.as_str() {
                "ctrl" | "control" => "control down",
                "shift" => "shift down",
                "alt" | "option" => "option down",
                _ => "command down",
            })
            .collect();
        let using = if using.is_empty() { String::new() } else { format!(" using {{{}}}", using.join(", ")) };
        let code = match key.as_str() {
            "enter" | "return" => Some(36),
            "tab" => Some(48),
            "esc" | "escape" => Some(53),
            "space" => Some(49),
            "backspace" => Some(51),
            "delete" | "del" => Some(117),
            "up" => Some(126),
            "down" => Some(125),
            "left" => Some(123),
            "right" => Some(124),
            "home" => Some(115),
            "end" => Some(119),
            "pageup" => Some(116),
            "pagedown" => Some(121),
            _ => None,
        };
        match code {
            Some(c) => osa(&format!("tell application \"System Events\" to key code {c}{using}"))?,
            None => osa(&format!("tell application \"System Events\" to keystroke \"{key}\"{using}"))?,
        };
        Ok(())
    }

    pub fn focus(id: u64) -> Result<()> {
        osa(&format!("tell application \"System Events\" to set frontmost of (first process whose unix id is {id}) to true"))?;
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod sys {
    use super::*;
    fn no<T>() -> Result<T> {
        bail!("screen control is only available on Windows and macOS")
    }
    pub fn windows() -> Result<Vec<Win>> {
        no()
    }
    pub fn capture(_: Option<u64>) -> Result<(i32, i32, Vec<u8>)> {
        no()
    }
    pub fn click(_: i32, _: i32, _: &str, _: bool) -> Result<()> {
        no()
    }
    pub fn scroll(_: i32, _: i32, _: i32, _: i32) -> Result<()> {
        no()
    }
    pub fn type_text(_: &str) -> Result<()> {
        no()
    }
    pub fn key(_: &str) -> Result<()> {
        no()
    }
    pub fn focus(_: u64) -> Result<()> {
        no()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_combos() {
        assert_eq!(parse_combo("Ctrl+Shift+S").unwrap(), (vec!["ctrl".to_string(), "shift".to_string()], "s".to_string()));
        assert_eq!(parse_combo("enter").unwrap().1, "enter");
        assert!(parse_combo("hyper+x").is_err());
        assert!(parse_combo("").is_err());
    }

    /// needs a real desktop session, so CI skips it: `cargo test -p axelstra-daemon -- --ignored computer`
    #[test]
    #[ignore]
    fn computer_sees_windows_and_screen() {
        let list = sys::windows().unwrap();
        assert!(!list.is_empty(), "no visible windows");
        for w in list.iter().take(5) {
            println!("{} {} {}x{} {}", w.id, w.process, w.w, w.h, w.title);
        }
        let (w, h, px) = sys::capture(None).unwrap();
        assert!(w > 100 && h > 100 && px.len() == (w * h * 4) as usize);
        assert!(px.iter().step_by(4).any(|b| *b != 0), "screenshot is all black");
        let (ww, wh, wpx) = sys::capture(Some(list[0].id)).unwrap();
        assert_eq!(wpx.len(), (ww * wh * 4) as usize);
    }
}
