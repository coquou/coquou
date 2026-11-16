// Shared by both windows (loaded first): logging, platform, constants and
// reading pets (Codex atlas layout + the coquou.json extension).
// Exposed as `window.Coquou`.
(() => {
  const { invoke } = window.__TAURI__.core;

  // ---------- logging ----------
  // Mirror info/warn/error and uncaught errors to the app log file (there are
  // no devtools in release builds). `tag` names the window in the log.
  function mirrorConsole(tag) {
    const send = (level, text) => invoke("log", { level, msg: `[${tag}] ${text}` }).catch(() => {});
    const fmt = (a) => (a instanceof Error ? a.message : typeof a === "object" ? JSON.stringify(a) : String(a));
    for (const level of ["info", "warn", "error"]) {
      const orig = console[level].bind(console);
      console[level] = (...args) => { orig(...args); send(level, args.map(fmt).join(" ")); };
    }
    window.addEventListener("error", (e) => send("error", `uncaught: ${e.message} @ ${e.filename}:${e.lineno}`));
    window.addEventListener("unhandledrejection", (e) => send("error", `unhandled: ${fmt(e.reason)}`));
  }

  // ---------- platform & names ----------
  const ua = navigator.userAgent;
  const platform = /Mac/.test(ua) ? "macos" : /Windows/.test(ua) ? "windows" : "linux";
  const keystore = platform === "windows" ? "凭据管理器" : "钥匙串";
  // Speech lines for pets without their own (doc/pet-format.md `lines`).
  const DEFAULT_LINES = {
    hello: ["来啦～", "新的一天，一起加油！"],
    click: ["干嘛呀～", "嘿嘿", "被发现了！"],
    pet: ["好舒服～", "再摸摸嘛", "嘿嘿嘿"],
    wake: ["唔……谁吵我", "我没睡着！"],
    held: ["哇！放我下来！", "好高呀"],
    idle: ["休息一下眼睛吧", "记得喝水哦", "我就在这儿陪你"],
  };

  // ---------- Codex atlas ----------
  // doc/pet-format.md: 8 columns of 192×208 cells; v1 has 9 rows, v2 11
  // (rows 9–10: 16 look directions, 0° = up, clockwise).
  const CELL = [192, 208];
  const COLS = 8;
  const tail = (n, each, last) => [...Array(n - 1).fill(each), last];
  // Standard rows: [row, frame durations in ms].
  const ROWS = {
    "idle": [0, [280, 110, 110, 140, 140, 320]],
    "running-right": [1, tail(8, 120, 220)],
    "running-left": [2, tail(8, 120, 220)],
    "waving": [3, tail(4, 140, 280)],
    "jumping": [4, tail(5, 140, 280)],
    "failed": [5, tail(8, 140, 240)],
    "waiting": [6, tail(6, 150, 260)],
    "running": [7, tail(6, 120, 220)],
    "review": [8, tail(6, 150, 280)],
  };
  const LOOK_ROWS = [9, 10];

  const petBytes = (key, path) => invoke("read_pet_file", { key, path });
  const petImage = async (key, path) => createImageBitmap(new Blob([await petBytes(key, path)]));
  async function petJson(key, path) {
    try {
      return JSON.parse(new TextDecoder().decode(await petBytes(key, path)));
    } catch (e) {
      throw new Error(`${path}: ${e}`);
    }
  }

  // Load a pet's manifest, extension file and atlas (+ the optional HD copy,
  // used only when it is exactly 2–4× the atlas). `entry` comes from
  // `list_pets` (`extFile` names the extension file, if any).
  async function loadAtlas(entry) {
    const key = entry.key;
    const manifest = await petJson(key, "pet.json");
    const ext = entry.extFile ? await petJson(key, entry.extFile) : null;
    const version = manifest.spriteVersionNumber ?? 1;
    const rows = version >= 2 ? 11 : 9;
    const atlas = await petImage(key, manifest.spritesheetPath || "spritesheet.webp");
    const [cw, ch] = CELL;
    if (atlas.width !== cw * COLS || atlas.height !== ch * rows)
      throw new Error(`spritesheet is ${atlas.width}×${atlas.height}, Codex v${version} needs ${cw * COLS}×${ch * rows}`);
    let hd = null, hk = 1;
    if (ext?.hd) {
      try {
        const img = await petImage(key, ext.hd);
        const k = img.width / atlas.width;
        if (Number.isInteger(k) && k >= 2 && k <= 4 && img.height === atlas.height * k) { hd = img; hk = k; }
        else console.warn(`${ext.hd} is ${img.width}×${img.height}, not 2–4× the atlas; ignored`);
      } catch (e) { console.warn("hd atlas:", e); }
    }
    return { manifest, ext, version, rows, atlas, hd, hk };
  }

  window.Coquou = {
    mirrorConsole, platform, keystore, DEFAULT_LINES,
    CELL, COLS, ROWS, LOOK_ROWS, petBytes, petImage, petJson, loadAtlas,
  };
})();
