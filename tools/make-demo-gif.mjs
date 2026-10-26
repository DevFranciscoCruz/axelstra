#!/usr/bin/env node
// Regenerates assets/demo/demo.gif from assets/demo/index.html. Headless only: a headless Chromium (Chrome, Edge or
// Chromium) is started with no window, driven over the DevTools protocol, and never shown or focused.
//
// The mock renders from one number, `window.setT(ms)` (video time), so frames are stepped deterministically: set the
// time, take a screenshot, repeat. No realtime capture and no timers, so the frames do not depend on machine speed.
//
// Prerequisites: node >= 22 (global WebSocket and fetch) and one headless-capable browser: set AXELSTRA_BROWSER to its
// path, else the usual Chrome/Edge/Chromium locations and PATH are tried. Nothing else: no ffmpeg, gifski, gifsicle or
// npm packages. PNG decoding, palette quantisation, frame deltas (transparent unchanged pixels) and the LZW encoder are
// in this file.
//
// Usage: node tools/make-demo-gif.mjs [--out assets/demo/demo.gif] [--width 820] [--fps 10] [--colors 255]
//                                     [--tol 300] [--dump DIR --at 1000,20000]
//   --tol N   treats a pixel as unchanged when its palette colour is within N (squared RGB distance) of what the
//             previous frame already shows: smaller file, slightly softer motion. 0 = exact. Default 300 (about 17 per channel), which keeps the file near 600 KB.
//   --dump    also writes the PNG at each --at time (ms of video time) into DIR, to look at a few frames.
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { inflateSync } from "node:zlib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const opt = (name, def) => {
  const i = argv.indexOf("--" + name);
  return i >= 0 && argv[i + 1] !== undefined ? argv[i + 1] : def;
};
const OUT = resolve(root, opt("out", "assets/demo/demo.gif"));
const WIDTH = Number(opt("width", 820));
const FPS = Number(opt("fps", 10));
const COLORS = Math.min(255, Number(opt("colors", 255)));
const TOL = Number(opt("tol", 300));
const DUMP = opt("dump", "");
const AT = opt("at", "").split(",").filter(Boolean).map(Number);
const PAGE = pathToFileURL(join(root, "assets/demo/index.html")).href + "?manual";

if (typeof WebSocket === "undefined") throw new Error("node >= 22 is needed (global WebSocket)");

// ---------------------------------------------------------------- browser
function findBrowser() {
  const env = process.env.AXELSTRA_BROWSER;
  if (env) {
    if (!existsSync(env)) throw new Error(`AXELSTRA_BROWSER does not exist: ${env}`);
    return env;
  }
  const pf = [process.env["ProgramFiles"], process.env["ProgramFiles(x86)"], process.env.LOCALAPPDATA].filter(Boolean);
  const known = [
    ...pf.flatMap((d) => [
      join(d, "Google/Chrome/Application/chrome.exe"),
      join(d, "Microsoft/Edge/Application/msedge.exe"),
      join(d, "Chromium/Application/chrome.exe"),
    ]),
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
  ];
  const names = [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "chrome",
    "msedge",
  ];
  const onPath = (process.env.PATH || "")
    .split(delimiter)
    .flatMap((d) => names.flatMap((n) => [join(d, n), join(d, n + ".exe")]));
  const hit = [...known, ...onPath].find((p) => existsSync(p));
  if (!hit) throw new Error("no Chrome, Edge or Chromium found: install one or set AXELSTRA_BROWSER to its executable");
  return hit;
}

function launch(exe, profile) {
  const child = spawn(
    exe,
    [
      "--headless=new",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "--disable-gpu",
      "--hide-scrollbars",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-extensions",
      "--mute-audio",
      "--force-device-scale-factor=1",
      "--window-size=1400,800",
      "about:blank",
    ],
    { stdio: ["ignore", "ignore", "pipe"], windowsHide: true },
  );
  return new Promise((ok, fail) => {
    let buf = "";
    const t = setTimeout(() => fail(new Error("the browser did not report a DevTools port in 30 s")), 30000);
    child.on("exit", (c) => fail(new Error(`the browser exited early (${c})`)));
    child.stderr.on("data", (d) => {
      buf += d;
      const m = /DevTools listening on ws:\/\/([^/]+)\//.exec(buf);
      if (m) {
        clearTimeout(t);
        ok({ child, hostPort: m[1] });
      }
    });
  });
}

async function connect(hostPort) {
  const list = await (await fetch(`http://${hostPort}/json/list`)).json();
  const page = list.find((x) => x.type === "page");
  if (!page) throw new Error("no page target");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((ok, fail) => {
    ws.onopen = ok;
    ws.onerror = () => fail(new Error("DevTools socket failed"));
  });
  let id = 0;
  const wait = new Map();
  ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    const w = m.id && wait.get(m.id);
    if (!w) return;
    wait.delete(m.id);
    m.error ? w.fail(new Error(m.error.message)) : w.ok(m.result);
  };
  const send = (method, params = {}) =>
    new Promise((ok, fail) => {
      wait.set(++id, { ok, fail });
      ws.send(JSON.stringify({ id, method, params }));
    });
  return { send, close: () => ws.close() };
}

// ---------------------------------------------------------------- PNG -> RGB
function decodePng(buf) {
  let p = 8;
  let w = 0,
    h = 0,
    ct = 0;
  const idat = [];
  while (p < buf.length) {
    const len = buf.readUInt32BE(p);
    const type = buf.toString("latin1", p + 4, p + 8);
    const data = buf.subarray(p + 8, p + 8 + len);
    if (type === "IHDR") {
      w = data.readUInt32BE(0);
      h = data.readUInt32BE(4);
      ct = data[9];
      if (data[8] !== 8 || data[12] !== 0 || (ct !== 2 && ct !== 6))
        throw new Error("unsupported PNG (need 8-bit RGB/RGBA, not interlaced)");
    } else if (type === "IDAT") idat.push(data);
    p += 12 + len;
  }
  const bpp = ct === 6 ? 4 : 3;
  const raw = inflateSync(Buffer.concat(idat));
  const stride = w * bpp;
  const px = Buffer.alloc(stride * h);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (stride + 1)];
    const src = y * (stride + 1) + 1;
    const dst = y * stride;
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? px[dst + x - bpp] : 0;
      const b = y ? px[dst - stride + x] : 0;
      const c = x >= bpp && y ? px[dst - stride + x - bpp] : 0;
      let v = raw[src + x];
      if (f === 1) v += a;
      else if (f === 2) v += b;
      else if (f === 3) v += (a + b) >> 1;
      else if (f === 4) {
        const pa = Math.abs(b - c),
          pb = Math.abs(a - c),
          pc = Math.abs(a + b - 2 * c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      px[dst + x] = v & 255;
    }
  }
  const rgb = new Uint8Array(w * h * 3);
  for (let i = 0, o = 0; i < px.length; i += bpp, o += 3) {
    rgb[o] = px[i];
    rgb[o + 1] = px[i + 1];
    rgb[o + 2] = px[i + 2];
  }
  return { w, h, rgb };
}

// ---------------------------------------------------------------- palette (median cut over a colour histogram)
function buildPalette(frames, n) {
  const hist = new Map();
  for (const f of frames) {
    for (let i = 0; i < f.length; i += 3) {
      const k = (f[i] << 16) | (f[i + 1] << 8) | f[i + 2];
      hist.set(k, (hist.get(k) || 0) + 1);
    }
  }
  let boxes = [
    [...hist].map(([k, c]) => ({
      r: k >> 16,
      g: (k >> 8) & 255,
      b: k & 255,
      c,
    })),
  ];
  const range = (box, ch) => {
    let lo = 255,
      hi = 0;
    for (const e of box) {
      lo = Math.min(lo, e[ch]);
      hi = Math.max(hi, e[ch]);
    }
    return hi - lo;
  };
  while (boxes.length < n) {
    // split the box with the most pixels (weighted by its widest channel) so flat UI colours keep their own entries
    let best = -1,
      bs = 0,
      bch = "r";
    boxes.forEach((box, i) => {
      if (box.length < 2) return;
      for (const ch of ["r", "g", "b"]) {
        const score = range(box, ch) * box.reduce((a, e) => a + e.c, 0);
        if (score > bs) {
          bs = score;
          best = i;
          bch = ch;
        }
      }
    });
    if (best < 0) break;
    const box = boxes[best].sort((a, b) => a[bch] - b[bch]);
    const half = box.reduce((a, e) => a + e.c, 0) / 2;
    let acc = 0,
      cut = 1;
    for (let i = 0; i < box.length - 1; i++) {
      acc += box[i].c;
      cut = i + 1;
      if (acc >= half) break;
    }
    boxes.splice(best, 1, box.slice(0, cut), box.slice(cut));
  }
  return boxes.map((box) => {
    const tot = box.reduce((a, e) => a + e.c, 0);
    return [0, 1, 2].map((j) => Math.round(box.reduce((a, e) => a + [e.r, e.g, e.b][j] * e.c, 0) / tot));
  });
}

function makeMapper(palette) {
  const cache = new Int16Array(1 << 18).fill(-1); // 6 bits per channel
  const exact = new Map(palette.map((c, i) => [(c[0] << 16) | (c[1] << 8) | c[2], i]));
  return (r, g, b) => {
    const e = exact.get((r << 16) | (g << 8) | b);
    if (e !== undefined) return e;
    const k = ((r >> 2) << 12) | ((g >> 2) << 6) | (b >> 2);
    if (cache[k] >= 0) return cache[k];
    let bi = 0,
      bd = Infinity;
    for (let i = 0; i < palette.length; i++) {
      const dr = palette[i][0] - r,
        dg = palette[i][1] - g,
        db = palette[i][2] - b;
      const d = dr * dr + dg * dg + db * db;
      if (d < bd) {
        bd = d;
        bi = i;
      }
    }
    return (cache[k] = bi);
  };
}

// ---------------------------------------------------------------- GIF writer
function lzw(indices, minBits) {
  const clear = 1 << minBits,
    eoi = clear + 1;
  const out = [];
  let cur = 0,
    nbits = 0;
  const put = (code, size) => {
    cur |= code << nbits;
    nbits += size;
    while (nbits >= 8) {
      out.push(cur & 255);
      cur >>>= 8;
      nbits -= 8;
    }
  };
  let size = minBits + 1,
    next = eoi + 1;
  let dict = new Map();
  put(clear, size);
  let prefix = indices[0];
  for (let i = 1; i < indices.length; i++) {
    const c = indices[i];
    const key = (prefix << 8) | c;
    const hit = dict.get(key);
    if (hit !== undefined) {
      prefix = hit;
      continue;
    }
    put(prefix, size);
    if (next < 4096) {
      dict.set(key, next++);
      if (next > 1 << size && size < 12) size++;
    } else {
      put(clear, size);
      dict = new Map();
      size = minBits + 1;
      next = eoi + 1;
    }
    prefix = c;
  }
  put(prefix, size);
  put(eoi, size);
  if (nbits) out.push(cur & 255);
  return Buffer.from(out);
}

function writeGif(w, h, palette, frames) {
  const parts = [];
  const u16 = (n) => Buffer.from([n & 255, n >> 8]);
  const tIdx = 255; // transparent entry, never used by a colour
  const table = Buffer.alloc(256 * 3);
  palette.forEach((c, i) => table.set(c, i * 3));
  parts.push(Buffer.from("GIF89a"), u16(w), u16(h), Buffer.from([0xf7, 0, 0]), table);
  parts.push(Buffer.from([0x21, 0xff, 0x0b]), Buffer.from("NETSCAPE2.0"), Buffer.from([3, 1, 0, 0, 0]));
  for (const f of frames) {
    const disposal = 1;
    parts.push(Buffer.from([0x21, 0xf9, 4, (disposal << 2) | 1]), u16(f.delay), Buffer.from([tIdx, 0]));
    parts.push(Buffer.from([0x2c]), u16(f.x), u16(f.y), u16(f.w), u16(f.h), Buffer.from([0, 8]));
    const data = lzw(f.pixels, 8);
    for (let i = 0; i < data.length; i += 255) {
      const chunk = data.subarray(i, i + 255);
      parts.push(Buffer.from([chunk.length]), chunk);
    }
    parts.push(Buffer.from([0]));
  }
  parts.push(Buffer.from([0x3b]));
  return Buffer.concat(parts);
}

// ---------------------------------------------------------------- main
async function main() {
  const profile = mkdtempSync(join(tmpdir(), "axelstra-demo-"));
  const exe = findBrowser();
  console.log(`browser: ${exe} (headless)`);
  const { child, hostPort } = await launch(exe, profile);
  const shots = [];
  try {
    const cdp = await connect(hostPort);
    await cdp.send("Page.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 1400,
      height: 800,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await cdp.send("Page.navigate", { url: PAGE });
    const ev = async (expression) =>
      (
        await cdp.send("Runtime.evaluate", {
          expression,
          awaitPromise: true,
          returnByValue: true,
        })
      ).result.value;
    for (
      let i = 0;
      i < 100 && !(await ev("typeof window.setT === 'function' && typeof window.TOTAL === 'number'"));
      i++
    )
      await new Promise((r) => setTimeout(r, 100));
    await ev("document.fonts.ready.then(() => true)");
    const total = await ev("window.TOTAL");
    const box = await ev(
      "(() => { const r = document.getElementById('win').getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; })()",
    );
    const scale = WIDTH / box.w;
    const step = 1000 / FPS;
    const n = Math.floor(total / step);
    console.log(
      `${(total / 1000).toFixed(1)} s of video, ${n} steps of ${step} ms, ${WIDTH}x${Math.round(box.h * scale)}`,
    );
    if (DUMP) mkdirSync(DUMP, { recursive: true });
    const wanted = new Set(AT.map((ms) => Math.round(ms / step)));
    for (let i = 0; i < n; i++) {
      await ev(`window.setT(${i * step})`);
      const { data } = await cdp.send("Page.captureScreenshot", {
        format: "png",
        clip: { x: box.x, y: box.y, width: box.w, height: box.h, scale },
        captureBeyondViewport: false,
      });
      const png = Buffer.from(data, "base64");
      if (wanted.has(i)) writeFileSync(join(DUMP, `frame-${String(Math.round(i * step)).padStart(6, "0")}.png`), png);
      shots.push(decodePng(png));
      if (i % 100 === 0) console.log(`  captured ${i}/${n}`);
    }
    cdp.close();
  } finally {
    child.kill();
    await new Promise((r) => setTimeout(r, 300));
    try {
      rmSync(profile, { recursive: true, force: true });
    } catch {}
  }

  const { w, h } = shots[0];
  const sample = shots.filter((_, i) => i % 6 === 0).map((s) => s.rgb);
  const palette = buildPalette(sample, COLORS);
  while (palette.length < 256) palette.push([0, 0, 0]);
  const map = makeMapper(palette.slice(0, COLORS));
  const delayCs = Math.max(2, Math.round(100 / FPS));

  const shown = new Uint8Array(w * h).fill(255); // what a decoder shows after the frames written so far
  const frames = [];
  for (const s of shots) {
    const idx = new Uint8Array(w * h);
    for (let i = 0, o = 0; i < idx.length; i++, o += 3) idx[i] = map(s.rgb[o], s.rgb[o + 1], s.rgb[o + 2]);
    let x0 = w,
      y0 = h,
      x1 = -1,
      y1 = -1;
    const keep = new Uint8Array(w * h); // 1 = pixel must be redrawn
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const i = y * w + x;
        const a = idx[i],
          b = shown[i];
        if (a === b) continue;
        if (TOL > 0 && b !== 255) {
          const dr = palette[a][0] - palette[b][0],
            dg = palette[a][1] - palette[b][1],
            db = palette[a][2] - palette[b][2];
          if (dr * dr + dg * dg + db * db <= TOL) continue;
        }
        keep[i] = 1;
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
    }
    if (x1 < 0) {
      // nothing changed: this step only lengthens the last frame
      if (frames.length) frames[frames.length - 1].delay += delayCs;
      continue;
    }
    const fw = x1 - x0 + 1,
      fh = y1 - y0 + 1;
    const pixels = new Uint8Array(fw * fh);
    for (let y = 0; y < fh; y++) {
      for (let x = 0; x < fw; x++) {
        const i = (y0 + y) * w + x0 + x;
        if (keep[i]) {
          pixels[y * fw + x] = idx[i];
          shown[i] = idx[i];
        } else pixels[y * fw + x] = 255;
      }
    }
    // the first frame has nothing under it: it must be opaque and full size
    frames.push({ x: x0, y: y0, w: fw, h: fh, pixels, delay: delayCs });
  }
  const gif = writeGif(w, h, palette, frames);
  mkdirSync(dirname(OUT), { recursive: true });
  writeFileSync(OUT, gif);
  console.log(`${OUT}: ${gif.length} bytes, ${frames.length} frames, ${w}x${h}`);
}

main().catch((e) => {
  console.error(`make-demo-gif: ${e.message}`);
  process.exit(1);
});
