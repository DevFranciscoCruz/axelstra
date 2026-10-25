// Win32 helpers for the test drivers (Windows only): geometry of the app's own window and a real resize of it.
//
// Why not CDP `Browser.setWindowBounds`: in WebView2 it does not resize the host HWND, it moves WebView2's inner
// `Chrome_WidgetWin_1` child by the browser-window position on every call (rel 8,31 -> 120,166 -> 232,301 ...), so the
// page paints displaced inside the window (empty strip at top and left, right/bottom cut off). The host HWND and the
// WRY_WEBVIEW child stay put. A resize must go through the host window (SetWindowPos), which wry turns into new
// controller bounds. Only windows of the given pid are touched, never activated (SWP_NOACTIVATE).
import { spawnSync } from "node:child_process";

const PS = `
Add-Type @'
using System; using System.Text; using System.Runtime.InteropServices;
public class W {
 public delegate bool EP(IntPtr h, IntPtr l);
 [StructLayout(LayoutKind.Sequential)] public struct R { public int L,T,Rr,B; }
 [DllImport("user32.dll")] public static extern bool EnumWindows(EP p, IntPtr l);
 [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, EP p, IntPtr l);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out R r);
 [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
 static string Cls(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
 public static IntPtr FindMain(uint pid) {
  IntPtr found = IntPtr.Zero;
  EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid && Cls(h) == "Tauri Window") found = h; return true; }, IntPtr.Zero);
  return found;
 }
 public static string Info(uint pid) {
  IntPtr h = FindMain(pid); if (h == IntPtr.Zero) return "null";
  R w, c; GetWindowRect(h, out w); GetClientRect(h, out c);
  var kids = new StringBuilder();
  EnumChildWindows(h, (ch, l) => { R cw, cc; GetWindowRect(ch, out cw); GetClientRect(ch, out cc);
   if (kids.Length > 0) kids.Append(',');
   kids.Append("{\\"cls\\":\\"" + Cls(ch) + "\\",\\"x\\":" + (cw.L - w.L) + ",\\"y\\":" + (cw.T - w.T) + ",\\"w\\":" + cc.Rr + ",\\"h\\":" + cc.B + "}"); return true; }, IntPtr.Zero);
  return "{\\"dpi\\":" + GetDpiForWindow(h) + ",\\"winW\\":" + (w.Rr - w.L) + ",\\"winH\\":" + (w.B - w.T) + ",\\"clientW\\":" + c.Rr + ",\\"clientH\\":" + c.B + ",\\"children\\":[" + kids + "]}";
 }
 public static string Resize(uint pid, int cw, int ch) {
  IntPtr h = FindMain(pid); if (h == IntPtr.Zero) return "null";
  R w, c; GetWindowRect(h, out w); GetClientRect(h, out c);
  int fw = (w.Rr - w.L) - c.Rr, fh = (w.B - w.T) - c.B;
  // SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE
  SetWindowPos(h, IntPtr.Zero, 0, 0, cw + fw, ch + fh, 0x0002 | 0x0004 | 0x0010);
  return Info(pid);
 }
}
'@
`;

const run = (body) => {
  const r = spawnSync("powershell", ["-NoProfile", "-Command", PS + body], {
    encoding: "utf8",
    timeout: 30000,
  });
  const out = r.stdout.trim();
  if (r.status !== 0 || !out || out === "null") throw new Error(`winrect: no app window (${r.stderr || out})`);
  return JSON.parse(out);
};

/** `{dpi, winW, winH, clientW, clientH, children: [{cls, x, y, w, h}]}` of the app's main window (child x/y relative to the window's top-left). */
export const windowInfo = (pid) => run(`[W]::Info(${pid})`);

/** Resizes the host window so its client area is `cw` x `ch` physical pixels; returns the new `windowInfo`. */
export const resizeClient = (pid, cw, ch) => run(`[W]::Resize(${pid}, ${cw}, ${ch})`);

/**
 * The webview's own child windows must keep the offset of the WRY_WEBVIEW child (the controller bounds): a browser
 * window that wandered inside it is the "content displaced" bug. Returns a list of problems (empty when fine).
 */
export const layoutProblems = (info) => {
  const out = [];
  const host = info.children.find((c) => c.cls === "WRY_WEBVIEW");
  if (!host) return ["no WRY_WEBVIEW child window"];
  if (host.w !== info.clientW || host.h !== info.clientH)
    out.push(`WRY_WEBVIEW is ${host.w}x${host.h}, client area is ${info.clientW}x${info.clientH}`);
  for (const c of info.children) {
    if (!/^Chrome_(WidgetWin_1|RenderWidgetHostHWND)$/.test(c.cls)) continue;
    if (c.x !== host.x || c.y !== host.y)
      out.push(`${c.cls} sits at ${c.x},${c.y}, the webview at ${host.x},${host.y}`);
    if (Math.abs(c.w - host.w) > 1 || Math.abs(c.h - host.h) > 1)
      out.push(`${c.cls} is ${c.w}x${c.h}, the webview ${host.w}x${host.h}`);
  }
  return out;
};
