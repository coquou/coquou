// Coquou app window: pet library, hatch entry, behaviour, general settings,
// about & updates.
//
// Pet preferences live in Rust (prefs.rs): this page changes them with
// `prefs_set` and redraws from `prefs` events, which also carry the pet on
// screen (`current`). Switching pets goes through the overlay
// (`pet-command`), which loads the pet first. App settings: settings.rs.

const { invoke } = window.__TAURI__.core;
const { listen, emit } = window.__TAURI__.event;

const { ROWS, CELL, platform, defaultLines, petLines, petDescription, t } = window.Coquou;
window.Coquou.mirrorConsole("app");
const $ = (id) => document.getElementById(id);
const usePet = (key) => emit("pet-command", "pet:" + key);
const setPrefs = (patch) => invoke("prefs_set", { patch }).catch((e) => toast(t("设置失败：", "Could not change the setting: ") + e, true));
// Run a command; show its error instead of losing it.
const act = (cmd, args, what) => invoke(cmd, args).catch((e) => toast(`${what}：${e}`, true));
// Sizes are per pet: wait until the overlay says which pet is shown.
const setSize = (v) => setPrefs({ size: Number(v) });

// Choices as [value, label]; functions so the labels follow the language.
const SIZES = () => [["0.5", t("超小", "XS")], ["0.75", t("小", "S")], ["1", t("标准", "M")], ["1.5", t("大", "L")], ["2", t("超大", "XL")]];
const ACTIVITIES = () => [["quiet", t("安静", "Calm")], ["normal", t("正常", "Normal")], ["lively", t("活泼", "Lively")]];
// prefs.rs SLEEP_CHOICES
const SLEEPS = () => [["30", t("30 秒", "30 s")], ["1800", t("30 分钟", "30 min")], ["0", t("从不", "Never")]];
const ZONES = () => [
  ["corners", t("两边角落", "Both corners"), t("默认，左右下角来回", "Default: back and forth between the bottom corners"), [[0, 26], [94, 120]]],
  ["right", t("右下角", "Bottom right"), t("只在右边", "Right side only"), [[94, 120]]],
  ["left", t("左下角", "Bottom left"), t("只在左边", "Left side only"), [[0, 26]]],
  ["full", t("整个底部", "Whole bottom"), t("可以走到中间", "Can walk to the middle"), [[0, 120]]],
];
// settings.rs Theme and i18n.rs Language; language names stay in their own language.
const THEMES = () => [["system", t("跟随系统", "System")], ["light", t("浅色", "Light")], ["dark", t("深色", "Dark")]];
const LANGUAGES = () => [["system", t("跟随系统", "System")], ["zh", "中文"], ["en", "English"]];
const sourceName = (s) => ({ preset: t("预设", "Preset"), coquou: t("我的", "Mine"), codex: "Codex" }[s] || s);
const FILTERS = () => [["all", t("全部", "All")], ["preset", sourceName("preset")], ["coquou", sourceName("coquou")], ["codex", "Codex"], ["hidden", t("已隐藏", "Hidden")]];
// Occasions a pet speaks on (prefs.rs / install.rs LINE_KINDS).
const LINE_KINDS = () => [["hello", t("打招呼", "Hello")], ["click", t("被点", "Clicked")], ["pet", t("被摸", "Petted")], ["wake", t("被吵醒", "Woken up")], ["held", t("被拎起来", "Picked up")], ["idle", t("闲着没事", "Idle")]];

// Preview chips: Codex row names (common.js ROWS) and their labels.
const CHIPS = () => [["idle", t("待机", "Idle")], ["running-right", t("走路", "Walk")], ["waving", t("挥手", "Wave")], ["jumping", t("跳跃", "Jump")], ["running", t("干活", "Work")], ["review", t("思考", "Think")], ["failed", t("难过", "Sad")]];

document.documentElement.classList.add(platform === "macos" ? "mac" : platform);

// Last `prefs` view: saved prefs + `current` (pet on screen).
let ui = { current: "", size: 1, custom: {}, zone: "corners", activity: "normal", sleepAfter: 1800, chatter: false, welcomeDismissed: true };
let pets = [];          // [{ key, id, displayName, source, ... }]
let selected = null;    // key shown in the detail panel
let filter = "all";
let query = "";
let chip = "idle";
let linesOpen = false;  // keep the lines editor open across redraws
let settings = null;

// ---------- small helpers ----------
let toastTimer = 0;
function toast(text, bad = false) {
  const t = $("toast");
  t.textContent = text;
  t.className = bad ? "on bad" : "on";
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.className = bad ? "bad" : ""), bad ? 5000 : 2600);
}
function el(tag, props = {}, ...kids) {
  const e = Object.assign(document.createElement(tag), props);
  for (const k of kids) if (k != null) e.append(k);
  return e;
}
function segmented(box, items, current, onPick) {
  box.replaceChildren(...items.map(([value, label]) => {
    const b = el("button", { textContent: label, type: "button" });
    b.classList.toggle("on", value === current);
    b.onclick = () => onPick(value);
    return b;
  }));
}
window.coquouConfirm = (...a) => confirmDialog(...a);
// The hatch page reports an unfinished run (dot on its nav item).
window.coquouHatchPending = (on) => { $("navHatchDot").hidden = !on; };
function confirmDialog(title, text, ok = t("删除", "Delete")) {
  return new Promise((resolve) => {
    $("modalTitle").textContent = title;
    $("modalText").textContent = text;
    $("modalOk").textContent = ok;
    $("modal").hidden = false;
    const done = (v) => { $("modal").hidden = true; resolve(v); };
    $("modalOk").onclick = () => done(true);
    $("modalCancel").onclick = () => done(false);
    $("modalCancel").focus();
  });
}

// ---------- navigation ----------
function showPage(name) {
  if (!$("page-" + name)) return;
  for (const b of document.querySelectorAll("nav button")) b.classList.toggle("on", b.dataset.page === name);
  for (const p of document.querySelectorAll("section.page")) p.hidden = p.id !== "page-" + name;
}
for (const b of document.querySelectorAll("nav button")) b.onclick = () => showPage(b.dataset.page);
showPage(window.__COQUOU_PAGE__ || "pets");
listen("app-page", ({ payload }) => showPage(payload));
$("now").onclick = () => { showPage("pets"); select(ui.current); };

// ---------- pet art ----------
const art = new Map();   // key → Promise<{ img, k, rows, ext, manifest }>
const ready = new Map(); // key → the resolved value, for the animation loop

// Load (once) what the previews need; the HD atlas when the pet has one.
function loadArt(key) {
  if (!art.has(key)) {
    const entry = pets.find((p) => p.key === key);
    if (!entry) return Promise.reject(new Error(`unknown pet ${key}`));
    const load = window.Coquou.loadAtlas(entry)
      .then((a) => ({ img: a.hd ?? a.atlas, k: a.hd ? a.hk : 1, rows: a.rows, ext: a.ext, manifest: a.manifest }));
    art.set(key, load.then((a) => { ready.set(key, a); return a; }, (e) => { art.delete(key); throw e; }));
  }
  return art.get(key);
}

// Draw one frame of `anim` at time t (seconds) into canvas g (w×h).
function paint(g, a, anim, t, w, h, crop = true) {
  g.clearRect(0, 0, w, h);
  if (!a) return;
  const [row, durs] = ROWS[anim] || ROWS.idle;
  const total = durs.reduce((x, y) => x + y, 0);
  let ms = (t * 1000) % total, f = 0;
  while (f < durs.length - 1 && ms >= durs[f]) { ms -= durs[f]; f++; }
  const k = a.k;
  const [cw, ch] = CELL;
  // The body sits in the lower middle of a cell; crop the margins.
  const [sx, sy, sw, sh] = crop ? [16, 26, 160, 176] : [0, 0, cw, ch];
  g.imageSmoothingQuality = "high";
  g.drawImage(a.img, (f * cw + sx) * k, (row * ch + sy) * k, sw * k, sh * k, 0, 0, w, h);
}

// ---------- library ----------
// Hidden pets only show under their own filter.
const inFilter = (p, f) => (f === "hidden" ? p.hidden : !p.hidden && (f === "all" || p.source === f));
function visible() {
  const q = query.trim().toLowerCase();
  return pets.filter((p) => inFilter(p, filter)
    && (!q || p.displayName.toLowerCase().includes(q) || p.id.toLowerCase().includes(q)));
}

function renderFilters() {
  const counts = Object.fromEntries(FILTERS().map(([k]) => [k, pets.filter((p) => inFilter(p, k)).length]));
  if (filter !== "all" && !counts[filter]) filter = "all";
  segmented($("filters"), FILTERS().filter(([k]) => k === "all" || counts[k]).map(([k, l]) => [k, `${l} ${counts[k]}`]), filter, (v) => {
    filter = v; renderFilters(); renderGrid();
  });
}

// ---------- pet actions shared by the detail panel and the card menu ----------
const toggleHidden = (p) => setPrefs({ custom: { pet: p.key, hidden: !p.hidden } })
  .then((v) => v && toast(p.hidden
    ? t(`「${p.displayName}」回到列表了`, `${p.displayName} is back in the list`)
    : t(`已隐藏「${p.displayName}」，可以在「已隐藏」里找回`, `${p.displayName} is hidden; find them under Hidden`)));
// Only pets installed in Coquou's own folder can be deleted.
async function deletePet(p) {
  if (!(await confirmDialog(t(`删除「${p.displayName}」？`, `Delete ${p.displayName}?`),
    t("宠物文件会从 Coquou 的宠物文件夹里删掉，无法恢复。可以先导出一份 .pet 备份。",
      "The pet's files are removed from Coquou's pets folder for good. You can export a .pet backup first.")))) return;
  invoke("delete_pet", { key: p.key }).then(() => toast(t(`已删除「${p.displayName}」`, `Deleted ${p.displayName}`))).catch((e) => toast(String(e), true));
}

// Right-click on a card: a native menu with the panel's quick actions. The
// last menu is kept until the next one opens, so a click that arrives after
// the popup returns still finds its handler. Items get no explicit ids: Tauri
// files handlers by item id and drops them when a menu closes, so a fixed id
// shared with the next menu would lose that menu's handler.
let cardMenu = null;
async function showCardMenu(p) {
  const { Menu } = window.__TAURI__.menu;
  const current = p.key === ui.current;
  const items = [
    { text: current ? t("正在桌面上", "On the desktop") : t("让 TA 上桌面", "Put on the desktop"), enabled: !current, action: () => usePet(p.key) },
    { item: "Separator" },
    { text: p.hidden ? t("取消隐藏", "Unhide") : t("隐藏", "Hide"), action: () => toggleHidden(p) },
  ];
  if (p.source === "coquou") items.push({ text: t("删除…", "Delete…"), action: () => deletePet(p) });
  try {
    const menu = await Menu.new({ items });
    cardMenu?.close().catch(() => {});
    cardMenu = menu;
    await menu.popup();
  } catch (e) {
    toast(t("菜单打不开：", "Cannot open the menu: ") + e, true);
  }
}

const cardCanvases = new Map(); // key → canvas
let hovered = null;

function renderGrid() {
  const list = visible();
  cardCanvases.clear();
  const cards = list.map((p) => {
    const c = el("canvas", { width: 256, height: 282 });
    cardCanvases.set(p.key, c);
    const card = el("div", { className: "pet", tabIndex: 0 },
      c,
      el("b", { textContent: p.displayName }),
      el("div", { className: "tag", textContent: sourceName(p.source) }));
    card.dataset.key = p.key;
    if (p.key === ui.current) card.append(el("span", { className: "badge", textContent: t("使用中", "In use") }));
    card.classList.toggle("sel", p.key === selected);
    card.onclick = () => select(p.key);
    card.ondblclick = () => usePet(p.key);
    card.oncontextmenu = (e) => { e.preventDefault(); select(p.key); showCardMenu(p); };
    card.onkeydown = (e) => { if (e.key === "Enter") usePet(p.key); if (e.key === " ") { e.preventDefault(); select(p.key); } };
    card.onmouseenter = () => (hovered = p.key);
    card.onmouseleave = () => {
      if (hovered === p.key) hovered = null;
      if (ready.has(p.key)) paint(c.getContext("2d"), ready.get(p.key), "idle", 0, c.width, c.height);
    };
    loadArt(p.key).then((a) => paint(c.getContext("2d"), a, "idle", 0, c.width, c.height)).catch(() => {});
    return card;
  });
  $("grid").replaceChildren(...(cards.length ? cards : [el("div", { className: "empty", textContent: query ? t("没有找到这个名字的宠物", "No pet by that name") : t("这里还没有宠物", "No pets here yet") })]));
}

let stage = null;
async function renderDetail() {
  const p = pets.find((x) => x.key === selected);
  const box = $("detail");
  if (!p) { box.hidden = true; return; }
  box.hidden = false;
  const a = await loadArt(p.key).catch(() => null);
  if (p.key !== selected) return;
  const chips = CHIPS().filter(([k]) => ROWS[k][0] < (a?.rows || 9));
  if (!chips.some(([k]) => k === chip)) chip = "idle";
  stage = el("canvas", { width: 460, height: 500 });
  const chipBox = el("div", { className: "chips" });
  chips.forEach(([k, label]) => {
    const b = el("button", { textContent: label, type: "button" });
    b.classList.toggle("on", k === chip);
    b.onclick = () => { chip = k; for (const o of chipBox.children) o.classList.toggle("on", o === b); };
    chipBox.append(b);
  });
  const current = p.key === ui.current;
  const use = el("button", { className: "btn primary use" + (current ? " current" : ""), textContent: current ? t("✓ 正在桌面上", "✓ On the desktop") : t("让 TA 上桌面", "Put on the desktop"), type: "button" });
  use.onclick = () => { if (!current) usePet(p.key); };

  const ext = a?.ext || {};
  const desc = petDescription(a?.manifest, ext);
  const mine = ui.custom?.[p.key] || {};

  // Name: edited in place; empty (or the pet's own name) goes back to its own.
  const title = el("h2", { textContent: p.displayName });
  const rename = el("button", { className: "btn ghost", type: "button", textContent: t("改名", "Rename") });
  const nameRow = el("div", { className: "name-row" }, title, rename);
  rename.onclick = () => {
    const input = el("input", { type: "text", value: p.displayName, maxLength: 24, placeholder: p.ownName });
    let finished = false;
    const done = (save) => {
      if (finished) return;
      finished = true;
      const name = input.value.trim();
      if (save && name !== p.displayName) setPrefs({ custom: { pet: p.key, name: name === p.ownName ? "" : name } });
      else nameRow.replaceChildren(title, rename);
    };
    input.onkeydown = (e) => { if (e.key === "Enter") done(true); if (e.key === "Escape") done(false); };
    input.onblur = () => done(true);
    nameRow.replaceChildren(input);
    input.focus(); input.select();
  };

  // Lines: one per row; only occasions that differ from the pet's own are saved.
  const ownLines = (kind) => petLines(ext, kind) ?? defaultLines(kind);
  const areas = {};
  const linesBox = el("details", { className: "lines", open: linesOpen },
    el("summary", { textContent: mine.lines ? t("台词（已改过）", "Lines (edited)") : t("台词", "Lines") }));
  linesBox.ontoggle = () => (linesOpen = linesBox.open);
  for (const [kind, label] of LINE_KINDS()) {
    const list = mine.lines?.[kind] ?? ownLines(kind);
    areas[kind] = el("textarea", { rows: Math.min(6, Math.max(2, list.length)), value: list.join("\n") });
    linesBox.append(el("label", {}, el("span", { textContent: label }), areas[kind]));
  }
  const saveLines = el("button", { className: "btn", type: "button", textContent: t("保存台词", "Save lines") });
  saveLines.onclick = () => {
    const lines = {};
    for (const [kind] of LINE_KINDS()) {
      const list = areas[kind].value.split("\n").map((t) => t.trim()).filter(Boolean);
      if (list.join("\n") !== ownLines(kind).join("\n")) lines[kind] = list;
    }
    setPrefs({ custom: { pet: p.key, lines } }).then((v) => v && toast(t("台词已保存", "Lines saved")));
  };
  const resetLines = el("button", { className: "btn ghost", type: "button", textContent: t("恢复原来的台词", "Restore the original lines"), disabled: !mine.lines });
  resetLines.onclick = () => setPrefs({ custom: { pet: p.key, lines: {} } });
  linesBox.append(el("div", { className: "row" }, saveLines, resetLines));
  const meta = el("dl", { className: "meta" });
  const add = (k, v) => { if (v) meta.append(el("dt", { textContent: k }), el("dd", { textContent: v })); };
  add(t("来源", "Source"), {
    preset: t("Coquou 预设", "Coquou preset"),
    coquou: t("安装在 Coquou", "Installed in Coquou"),
    codex: t("Codex 宠物（只读）", "Codex pet (read-only)"),
  }[p.source]);
  if (p.displayName !== p.ownName) add(t("原名", "Original name"), p.ownName);
  add(t("作者", "Author"), ext.author);
  add(t("授权", "License"), ext.license);
  add("ID", p.id);

  const more = el("div", { className: "more" });
  const exp = el("button", { className: "btn ghost", type: "button", textContent: t("导出 .pet", "Export .pet") });
  exp.onclick = () => invoke("export_pet", { key: p.key })
    .then((f) => toast(t(`已导出到「下载」：${f}`, `Exported to Downloads: ${f}`)))
    .catch((e) => toast(t("导出失败：", "Export failed: ") + e, true));
  const reveal = el("button", { className: "btn ghost", type: "button",
    textContent: platform === "macos" ? t("在访达中显示", "Show in Finder") : t("打开所在文件夹", "Show in Explorer") });
  reveal.onclick = () => invoke("reveal_pet", { key: p.key }).catch((e) => toast(String(e), true));
  const hide = el("button", { className: "btn ghost", type: "button", textContent: p.hidden ? t("取消隐藏", "Unhide") : t("隐藏", "Hide") });
  hide.onclick = () => toggleHidden(p);
  more.append(exp, reveal, hide);
  if (p.hatched) {
    const edit = el("button", { className: "btn ghost", type: "button", textContent: t("在孵化页修改", "Change in Hatch") });
    edit.onclick = () => { showPage("hatch"); window.hatchEditPet?.(p.key); };
    more.prepend(edit);
  }
  if (p.source === "coquou") {
    const del = el("button", { className: "btn ghost danger", type: "button", textContent: t("删除", "Delete") });
    del.onclick = () => deletePet(p);
    more.append(del);
  }

  const children = [
    el("div", { className: "stage" }, stage),
    chipBox,
    nameRow,
    el("div", { className: "src", textContent: sourceName(p.source) }),
    desc ? el("p", { className: "desc", textContent: desc }) : null,
    use,
  ];
  children.push(linesBox, meta, more);
  box.replaceChildren(...children.filter(Boolean));
}

function select(key) {
  if (!pets.some((p) => p.key === key)) key = pets[0]?.key ?? null;
  selected = key;
  for (const c of document.querySelectorAll(".pet")) c.classList.toggle("sel", c.dataset.key === key);
  chip = "idle";
  renderDetail();
}

async function loadPets() {
  const list = await invoke("list_pets").catch(() => []);
  const order = { preset: 0, coquou: 1, codex: 2 };
  pets = list.sort((a, b) => order[a.source] - order[b.source]);
  if (!pets.some((p) => p.key === selected)) selected = pets.some((p) => p.key === ui.current) ? ui.current : pets[0]?.key;
  renderFilters();
  renderGrid();
  renderDetail();
  renderNow();
}

function renderNow() {
  const p = pets.find((x) => x.key === ui.current);
  $("nowName").textContent = p ? p.displayName : "—";
}

// Animation: the detail stage, the hovered card and the sidebar thumbnail.
function tick(now) {
  const t = now / 1000;
  if (!document.hidden) {
    if (stage && ready.has(selected)) paint(stage.getContext("2d"), ready.get(selected), chip, t, stage.width, stage.height);
    if (hovered && cardCanvases.has(hovered) && ready.has(hovered)) {
      const c = cardCanvases.get(hovered);
      paint(c.getContext("2d"), ready.get(hovered), "waving", t, c.width, c.height);
    }
    const nc = $("nowThumb");
    if (ready.has(ui.current)) paint(nc.getContext("2d"), ready.get(ui.current), "idle", t, nc.width, nc.height);
    else if (ui.current) loadArt(ui.current).catch(() => {});
  }
  requestAnimationFrame(tick);
}
requestAnimationFrame(tick);

$("search").oninput = (e) => { query = e.target.value; renderGrid(); };
$("hatchBtn").onclick = () => showPage("hatch");
$("importBtn").onclick = () => $("importFile").click();
$("importFile").onchange = async (e) => {
  for (const file of e.target.files) {
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const done = await invoke("install_bytes", bytes, { headers: { "x-name": encodeURIComponent(file.name) } });
      selected = done.key;
    } catch (err) { toast(t("装不上：", "Cannot install: ") + err, true); }
  }
  e.target.value = "";
};

// ---------- welcome (first run) ----------
function renderWelcome() {
  $("welcome").hidden = ui.welcomeDismissed;
  $("welcomeText").textContent = platform === "macos"
    ? t("宠物会在屏幕底部的角落里溜达。Coquou 住在屏幕右上角的菜单栏里，点那里的图标或者右键点宠物，可以叫 TA 回来、哄 TA 睡觉，或者再打开这个窗口。",
      "Your pet strolls around the bottom corners of the screen. Coquou lives in the menu bar at the top right: click its icon there, or right-click the pet, to call them back, put them to sleep, or open this window again.")
    : t("宠物会在屏幕底部的角落里溜达。Coquou 住在屏幕右下角的通知区域（可能在「^」里），单击图标打开这个窗口，右键图标或者右键点宠物，可以叫 TA 回来、哄 TA 睡觉。",
      "Your pet strolls around the bottom corners of the screen. Coquou lives in the notification area at the bottom right (maybe under “^”): click its icon to open this window; right-click the icon or the pet to call them back or put them to sleep.");
}
$("welcomeOk").onclick = () => setPrefs({ welcomeDismissed: true });

// ---------- behaviour ----------
function zoneSvg(spans) {
  const bars = spans.map(([a, b]) => `<rect x="${a}" y="58" width="${b - a}" height="12" rx="3" fill="var(--accent)"/>`).join("");
  return `<svg viewBox="0 0 120 76"><rect x="1" y="1" width="118" height="74" rx="8" fill="var(--hover)" stroke="var(--line-2)"/><rect x="30" y="14" width="60" height="30" rx="4" fill="var(--panel)" stroke="var(--line-2)"/>${bars}</svg>`;
}
function renderBehaviour() {
  $("zones").replaceChildren(...ZONES().map(([v, label, hint, spans]) => {
    const b = el("button", { className: "zone", type: "button" });
    b.innerHTML = `${zoneSvg(spans)}<b></b><small></small>`;
    b.querySelector("b").textContent = label;
    b.querySelector("small").textContent = hint;
    b.classList.toggle("on", ui.zone === v);
    b.onclick = () => setPrefs({ zone: v });
    return b;
  }));
  segmented($("activity"), ACTIVITIES(), ui.activity, (v) => setPrefs({ activity: v }));
  segmented($("sleep"), SLEEPS(), String(ui.sleepAfter), (v) => setPrefs({ sleepAfter: Number(v) }));
  segmented($("size"), SIZES(), String(ui.size), setSize);
  $("chatter").checked = ui.chatter;
}

// ---------- general ----------
const setSetting = (name, value) => invoke("set_setting", { name, value })
  .catch((e) => { toast(t("设置失败：", "Could not change the setting: ") + e, true); loadSettings(); });
function renderAppearance() {
  segmented($("theme"), THEMES(), settings.theme, (v) => setSetting("theme", v));
  segmented($("language"), LANGUAGES(), settings.language, (v) => setSetting("language", v));
}
async function loadSettings() {
  settings = await invoke("get_settings");
  if (window.Coquou.setLang(settings.lang)) relabel();
  renderAppearance();
  $("version").textContent = "v" + settings.version;
  for (const id of ["hideFullscreen", "batterySaver", "autostart", "autoUpdate"]) $(id).checked = settings[id];
  const sel = $("display");
  sel.replaceChildren(...settings.displays.map((d) => el("option", { value: d.key, textContent: d.label })));
  sel.value = settings.currentDisplay ?? "";
  sel.disabled = settings.displays.length < 2;
  $("petsDir").textContent = settings.petsDir || "";
  $("codexDir").textContent = settings.codexDir || "";
}
$("chatter").onchange = (e) => setPrefs({ chatter: e.target.checked });
for (const id of ["hideFullscreen", "batterySaver", "autostart", "autoUpdate"]) {
  $(id).onchange = (e) => setSetting(id, e.target.checked);
}
$("display").onchange = (e) => act("set_setting", { name: "display", value: e.target.value }, t("切换显示器失败", "Could not switch screens")).then(loadSettings);
$("openDir").onclick = () => act("open_window", { name: "pets-dir" }, t("打不开宠物文件夹", "Cannot open the pets folder"));
for (const b of document.querySelectorAll("[data-link]")) b.onclick = () => act("open_link", { which: b.dataset.link }, t("打不开链接", "Cannot open the link"));

// ---------- updates ----------
function ago(sec) {
  const d = Math.max(0, Date.now() / 1000 - sec);
  const n = (unit) => Math.floor(d / unit);
  if (d < 60) return t("刚刚", "just now");
  if (d < 3600) return t(`${n(60)} 分钟前`, `${n(60)} min ago`);
  if (d < 86400) return t(`${n(3600)} 小时前`, `${n(3600)} h ago`);
  return t(`${n(86400)} 天前`, n(86400) === 1 ? "1 day ago" : `${n(86400)} days ago`);
}
// Release notes are Markdown on GitHub: render headings, lists, bold, code.
function notesHtml(md) {
  const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
  const inline = (s) => esc(s).replace(/\*\*(.+?)\*\*/g, "<b>$1</b>").replace(/`([^`]+)`/g, "<code>$1</code>").replace(/\[([^\]]+)\]\([^)]+\)/g, "$1");
  let html = "", list = false;
  for (const raw of md.split(/\r?\n/)) {
    const line = raw.trim();
    const item = line.match(/^[-*+]\s+(.*)/);
    if (item) { if (!list) { html += "<ul>"; list = true; } html += `<li>${inline(item[1])}</li>`; continue; }
    if (list) { html += "</ul>"; list = false; }
    const h = line.match(/^#{1,6}\s+(.*)/);
    if (h) html += `<h4>${inline(h[1])}</h4>`;
    else if (line) html += `<p>${inline(line)}</p>`;
  }
  return html + (list ? "</ul>" : "");
}
let updateStatus = null; // last `update-status`, redrawn when the language changes
function renderUpdate(s) {
  updateStatus = s;
  const a = s.available;
  $("navUpdateDot").hidden = !a;
  $("updState").textContent = s.installing ? t("正在下载更新…", "Downloading the update…")
    : s.checking ? t("正在检查更新…", "Checking for updates…")
    : a ? t(`发现新版本 v${a.version}`, `Version v${a.version} is available`)
    : s.lastChecked ? t(`已是最新版本 v${s.current}`, `Up to date: v${s.current}`)
    : t(`当前版本 v${s.current}`, `Current version v${s.current}`);
  $("updDetail").textContent = s.lastError || (s.lastChecked ? t(`上次检查：${ago(s.lastChecked)}`, `Last checked ${ago(s.lastChecked)}`) : "");
  $("updDetail").style.color = s.lastError ? "var(--bad)" : "";
  $("updCheck").disabled = s.checking || s.installing;
  $("updAvail").hidden = !a;
  if (a) {
    $("updVersion").textContent = `v${s.current} → v${a.version}`;
    $("updDate").textContent = a.date ? t(`发布于 ${a.date}`, `Released ${a.date}`) : "";
    $("updNotes").innerHTML = notesHtml(a.notes || "");
    $("updInstall").disabled = s.installing;
    $("updInstall").textContent = s.installing ? t("正在更新…", "Updating…") : t("更新并重启", "Update and restart");
  }
  $("updProgress").hidden = !s.installing;
}
$("updCheck").onclick = () => invoke("update_check").then(renderUpdate).catch((e) => toast(String(e), true));
$("updInstall").onclick = () => invoke("update_install").catch((e) => toast(t("更新失败：", "Update failed: ") + e, true));
listen("update-status", ({ payload }) => renderUpdate(payload));
listen("update-progress", ({ payload }) => {
  const { downloaded, total } = payload;
  $("updProgress").hidden = false;
  $("updProgress").firstElementChild.style.width = total ? `${Math.min(100, (downloaded / total) * 100)}%` : "50%";
});

// ---------- language ----------
// After a language change: redraw everything scripts wrote (common.js has
// already swapped the page's static text).
function relabel() {
  loadPets(); // names can be translated (pets.rs own_name); redraws filters, grid, detail
  renderWelcome();
  renderBehaviour();
  if (updateStatus) renderUpdate(updateStatus);
  window.hatchRelabel?.();
}

// ---------- live updates ----------
function applyPrefs(next) {
  const petChanged = next.current !== ui.current;
  const customChanged = JSON.stringify(next.custom) !== JSON.stringify(ui.custom);
  ui = next;
  if (customChanged) loadPets(); // names and hidden flags come with the list
  renderWelcome();
  renderBehaviour();
  renderNow();
  if (petChanged) renderGrid();
  renderDetail();
}
listen("prefs", ({ payload }) => applyPrefs(payload));
listen("settings-changed", loadSettings);
listen("pet-installed", ({ payload }) => {
  art.delete(payload.key); ready.delete(payload.key);
  toast(payload.replaced ? t(`更新好了：${payload.displayName}`, `Updated: ${payload.displayName}`) : t(`安装好了：${payload.displayName}`, `Installed: ${payload.displayName}`));
  selected = payload.key;
  showPage("pets");
  loadPets();
});
listen("pet-install-failed", ({ payload }) => toast(t(`装不上${payload.file ? " " + payload.file : ""}：`, `Cannot install${payload.file ? " " + payload.file : ""}: `) + payload.error, true));
listen("pets-changed", ({ payload }) => { if (payload?.removed) { art.delete(payload.removed); ready.delete(payload.removed); } loadPets(); });

// Drag & drop: on the hatch page a single image becomes the reference image;
// anything else (.pet / .zip / folders) is installed.
const IMAGE = /\.(png|jpe?g|webp|gif|bmp)$/i;
const onHatch = () => !$("page-hatch").hidden;
const asReference = (paths) => onHatch() && paths?.length === 1 && IMAGE.test(paths[0]);
window.__TAURI__.webviewWindow.getCurrentWebviewWindow().onDragDropEvent(({ payload }) => {
  if (payload.type === "enter") $("dropText").textContent = asReference(payload.paths) ? t("松手设为参考图", "Drop to use as the reference image") : t("松手安装宠物", "Drop to install the pet");
  $("dropOverlay").classList.toggle("on", payload.type === "enter" || payload.type === "over");
});
listen("app-drop", async ({ payload: paths }) => {
  $("dropOverlay").classList.remove("on");
  if (asReference(paths)) {
    try {
      const bytes = await invoke("read_dropped", { path: paths[0] });
      const name = paths[0].split(/[\\/]/).pop();
      const ext = name.split(".").pop().toLowerCase();
      window.hatchSetReference?.(new File([bytes], name, { type: "image/" + (ext === "jpg" ? "jpeg" : ext) }));
    } catch (e) { toast(t("读不了这张图：", "Cannot read this image: ") + e, true); }
  } else invoke("install_dropped");
});

(async () => {
  ui = await invoke("prefs_get");
  renderWelcome();
  renderBehaviour();
  await Promise.all([loadPets(), loadSettings()]);
  invoke("update_status").then(renderUpdate).catch(() => {});
})();
