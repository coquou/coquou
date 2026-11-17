// Shared by both windows (loaded first): logging, interface language,
// platform, constants and reading pets (Codex atlas layout + the coquou.json
// extension). Exposed as `window.Coquou`.
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

  // ---------- interface language ----------
  // Chinese is the source text and English sits next to it: `t("中文",
  // "English")` in scripts; in HTML, `data-en` replaces an element's own
  // text (its first direct text node, so icons and child elements stay) and
  // `data-en-placeholder` / `-title` / `-aria-label` replace attributes.
  // Text a script sets is the script's to redraw after `setLang`.
  // The language is resolved in Rust (i18n.rs, `get_settings().lang`); the
  // app window gets it injected as `__COQUOU_LANG__` before the first paint.
  let lang = window.__COQUOU_LANG__ === "en" ? "en" : "zh";
  const t = (zh, en) => (lang === "en" ? en : zh);
  const ATTRS = ["placeholder", "title", "aria-label"];
  const ownText = (e) => [...e.childNodes].find((n) => n.nodeType === Node.TEXT_NODE && n.data.trim());
  function translatePage() {
    document.documentElement.lang = t("zh-CN", "en");
    for (const e of document.querySelectorAll("[data-en]")) {
      const node = ownText(e);
      if (!node) continue;
      e.dataset.zh ??= node.data.trim();
      node.data = t(e.dataset.zh, e.dataset.en);
    }
    for (const a of ATTRS) {
      for (const e of document.querySelectorAll(`[data-en-${a}]`)) {
        e.setAttribute(`data-zh-${a}`, e.getAttribute(`data-zh-${a}`) ?? e.getAttribute(a) ?? "");
        e.setAttribute(a, t(e.getAttribute(`data-zh-${a}`), e.getAttribute(`data-en-${a}`)));
      }
    }
  }
  // Switch to "zh" / "en"; true when it changed (the caller redraws).
  function setLang(next) {
    next = next === "en" ? "en" : "zh";
    if (next === lang) return false;
    lang = next;
    translatePage();
    return true;
  }
  translatePage();

  // ---------- platform & names ----------
  const ua = navigator.userAgent;
  const platform = /Mac/.test(ua) ? "macos" : /Windows/.test(ua) ? "windows" : "linux";
  const keystore = () => (platform === "windows" ? t("凭据管理器", "Credential Manager") : t("钥匙串", "Keychain"));
  // Speech lines for pets without their own (doc/pet-format.md `lines`).
  const DEFAULT_LINES = {
    zh: {
      hello: ["来啦～", "新的一天，一起加油！"],
      click: ["干嘛呀～", "嘿嘿", "被发现了！"],
      pet: ["好舒服～", "再摸摸嘛", "嘿嘿嘿"],
      wake: ["唔……谁吵我", "我没睡着！"],
      held: ["哇！放我下来！", "好高呀"],
      idle: ["休息一下眼睛吧", "记得喝水哦", "我就在这儿陪你"],
    },
    en: {
      hello: ["I'm here!", "New day, let's do this!"],
      click: ["What's up?", "Hehe", "You found me!"],
      pet: ["That's nice~", "More, please", "Hehehe"],
      wake: ["Mmh… who's there?", "I wasn't sleeping!"],
      held: ["Whoa! Put me down!", "So high up!"],
      idle: ["Rest your eyes a little", "Remember to drink water", "I'm right here with you"],
    },
  };
  const defaultLines = (kind) => DEFAULT_LINES[lang][kind];

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
    mirrorConsole, t, setLang, platform, keystore, defaultLines,
    CELL, COLS, ROWS, LOOK_ROWS, petImage, loadAtlas,
  };
})();
