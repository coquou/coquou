// Coquou app window: pet library, hatch entry, behaviour, general settings,
// about & updates.
//
// Pet preferences live in Rust (prefs.rs): this page changes them with
// `prefs_set` and redraws from `prefs` events, which also carry the pet on
// screen (`current`) and its size. Switching pets goes through the overlay
// (`pet-command`), which loads the pet first. App settings: settings.rs.

const { invoke } = window.__TAURI__.core;
const { listen, emit } = window.__TAURI__.event;

const { ROWS, CELL, platform, DEFAULT_LINES } = window.Coquou;
window.Coquou.mirrorConsole("app");
const $ = (id) => document.getElementById(id);
const usePet = (key) => emit("pet-command", "pet:" + key);
const setPrefs = (patch) => invoke("prefs_set", { patch }).catch((e) => toast("设置失败：" + e, true));
// Run a command; show its error instead of losing it.
const act = (cmd, args, what) => invoke(cmd, args).catch((e) => toast(`${what}：${e}`, true));
// Sizes are per pet: wait until the overlay says which pet is shown.
const setSize = (v) => (ui.current ? setPrefs({ size: { pet: ui.current, value: Number(v) } }) : toast("宠物还没加载好，稍等一下"));

const SIZES = [["0.75", "小"], ["1", "标准"], ["1.3", "大"], ["1.6", "特大"], ["2", "超大"]];
const ACTIVITIES = [["quiet", "安静"], ["normal", "正常"], ["lively", "活泼"]];
const SLEEPS = [["20", "20 秒"], ["45", "45 秒"], ["120", "2 分钟"], ["300", "5 分钟"], ["0", "从不"]];
const ZONES = [
  ["corners", "两边角落", "默认，左右下角来回", [[0, 26], [94, 120]]],
  ["right", "右下角", "只在右边", [[94, 120]]],
  ["left", "左下角", "只在左边", [[0, 26]]],
  ["full", "整个底部", "可以走到中间", [[0, 120]]],
];
const SOURCE = { preset: "预设", coquou: "我的", codex: "Codex" };
const FILTERS = [["all", "全部"], ["preset", "预设"], ["coquou", "我的"], ["codex", "Codex"], ["hidden", "已隐藏"]];
// Occasions a pet speaks on (prefs.rs / install.rs LINE_KINDS).
const LINE_KINDS = [["hello", "打招呼"], ["click", "被点"], ["pet", "被摸"], ["wake", "被吵醒"], ["held", "被拎起来"], ["idle", "闲着没事"]];

// Preview chips: Codex row names (common.js ROWS) and their labels.
const CHIPS = [["idle", "待机"], ["running-right", "走路"], ["waving", "挥手"], ["jumping", "跳跃"], ["running", "干活"], ["review", "思考"], ["failed", "难过"]];

document.documentElement.classList.add(platform === "macos" ? "mac" : platform);

// Last `prefs` view: saved prefs + `current` (pet on screen) + its `size`.
let ui = { current: "", size: 1, custom: {}, zone: "corners", activity: "normal", sleepAfter: 45, chatter: true, welcomeDismissed: true };
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
function confirmDialog(title, text, ok = "删除") {
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
  const counts = Object.fromEntries(FILTERS.map(([k]) => [k, pets.filter((p) => inFilter(p, k)).length]));
  if (filter !== "all" && !counts[filter]) filter = "all";
  segmented($("filters"), FILTERS.filter(([k]) => k === "all" || counts[k]).map(([k, l]) => [k, `${l} ${counts[k]}`]), filter, (v) => {
    filter = v; renderFilters(); renderGrid();
  });
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
      el("div", { className: "tag", textContent: SOURCE[p.source] || p.source }));
    card.dataset.key = p.key;
    if (p.key === ui.current) card.append(el("span", { className: "badge", textContent: "使用中" }));
    card.classList.toggle("sel", p.key === selected);
    card.onclick = () => select(p.key);
    card.ondblclick = () => usePet(p.key);
    card.onkeydown = (e) => { if (e.key === "Enter") usePet(p.key); if (e.key === " ") { e.preventDefault(); select(p.key); } };
    card.onmouseenter = () => (hovered = p.key);
    card.onmouseleave = () => {
      if (hovered === p.key) hovered = null;
      if (ready.has(p.key)) paint(c.getContext("2d"), ready.get(p.key), "idle", 0, c.width, c.height);
    };
    loadArt(p.key).then((a) => paint(c.getContext("2d"), a, "idle", 0, c.width, c.height)).catch(() => {});
    return card;
  });
  $("grid").replaceChildren(...(cards.length ? cards : [el("div", { className: "empty", textContent: query ? "没有找到这个名字的宠物" : "这里还没有宠物" })]));
}

let stage = null;
async function renderDetail() {
  const p = pets.find((x) => x.key === selected);
  const box = $("detail");
  if (!p) { box.hidden = true; return; }
  box.hidden = false;
  const a = await loadArt(p.key).catch(() => null);
  if (p.key !== selected) return;
  const chips = CHIPS.filter(([k]) => ROWS[k][0] < (a?.rows || 9));
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
  const use = el("button", { className: "btn primary use" + (current ? " current" : ""), textContent: current ? "✓ 正在桌面上" : "让TA上桌面", type: "button" });
  use.onclick = () => { if (!current) usePet(p.key); };

  const ext = a?.ext || {};
  const desc = a?.manifest?.description || "";
  const mine = ui.custom?.[p.key] || {};

  // Name: edited in place; empty (or the pet's own name) goes back to its own.
  const title = el("h2", { textContent: p.displayName });
  const rename = el("button", { className: "btn ghost", type: "button", textContent: "改名" });
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
  const ownLines = (kind) => ext.lines?.[kind] ?? DEFAULT_LINES[kind];
  const areas = {};
  const linesBox = el("details", { className: "lines", open: linesOpen },
    el("summary", { textContent: mine.lines ? "台词（已改过）" : "台词" }));
  linesBox.ontoggle = () => (linesOpen = linesBox.open);
  for (const [kind, label] of LINE_KINDS) {
    const list = mine.lines?.[kind] ?? ownLines(kind);
    areas[kind] = el("textarea", { rows: Math.min(6, Math.max(2, list.length)), value: list.join("\n") });
    linesBox.append(el("label", {}, el("span", { textContent: label }), areas[kind]));
  }
  const saveLines = el("button", { className: "btn", type: "button", textContent: "保存台词" });
  saveLines.onclick = () => {
    const lines = {};
    for (const [kind] of LINE_KINDS) {
      const list = areas[kind].value.split("\n").map((t) => t.trim()).filter(Boolean);
      if (list.join("\n") !== ownLines(kind).join("\n")) lines[kind] = list;
    }
    setPrefs({ custom: { pet: p.key, lines } }).then((v) => v && toast("台词已保存"));
  };
  const resetLines = el("button", { className: "btn ghost", type: "button", textContent: "恢复原来的台词", disabled: !mine.lines });
  resetLines.onclick = () => setPrefs({ custom: { pet: p.key, lines: {} } });
  linesBox.append(el("div", { className: "row" }, saveLines, resetLines));
  const meta = el("dl", { className: "meta" });
  const add = (k, v) => { if (v) meta.append(el("dt", { textContent: k }), el("dd", { textContent: v })); };
  add("来源", { preset: "Coquou 预设", coquou: "安装在 Coquou", codex: "Codex 宠物（只读）" }[p.source]);
  if (p.displayName !== p.ownName) add("原名", p.ownName);
  add("作者", ext.author);
  add("授权", ext.license);
  add("ID", p.id);

  const more = el("div", { className: "more" });
  const exp = el("button", { className: "btn ghost", type: "button", textContent: "导出 .pet" });
  exp.onclick = () => invoke("export_pet", { key: p.key }).then((f) => toast(`已导出到「下载」：${f}`)).catch((e) => toast("导出失败：" + e, true));
  const reveal = el("button", { className: "btn ghost", type: "button", textContent: platform === "macos" ? "在访达中显示" : "打开所在文件夹" });
  reveal.onclick = () => invoke("reveal_pet", { key: p.key }).catch((e) => toast(String(e), true));
  const hide = el("button", { className: "btn ghost", type: "button", textContent: p.hidden ? "取消隐藏" : "隐藏" });
  hide.onclick = () => setPrefs({ custom: { pet: p.key, hidden: !p.hidden } })
    .then((v) => v && toast(p.hidden ? `「${p.displayName}」回到列表了` : `已隐藏「${p.displayName}」，可以在「已隐藏」里找回`));
  more.append(exp, reveal, hide);
  if (p.source === "coquou") {
    const del = el("button", { className: "btn ghost danger", type: "button", textContent: "删除" });
    del.onclick = async () => {
      if (!(await confirmDialog(`删除「${p.displayName}」？`, "宠物文件会从 Coquou 的宠物文件夹里删掉，无法恢复。可以先导出一份 .pet 备份。"))) return;
      invoke("delete_pet", { key: p.key }).then(() => toast(`已删除「${p.displayName}」`)).catch((e) => toast(String(e), true));
    };
    more.append(del);
  }

  const children = [
    el("div", { className: "stage" }, stage),
    chipBox,
    nameRow,
    el("div", { className: "src", textContent: SOURCE[p.source] || p.source }),
    desc ? el("p", { className: "desc", textContent: desc }) : null,
    use,
  ];
  if (current) {
    const sizes = el("div", { className: "seg" });
    segmented(sizes, SIZES, String(ui.size), setSize);
    children.push(el("div", { className: "sizes" }, el("span", { textContent: "大小" }), sizes));
  }
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
    } catch (err) { toast("装不上：" + err, true); }
  }
  e.target.value = "";
};

// ---------- welcome (first run) ----------
function renderWelcome() {
  $("welcome").hidden = ui.welcomeDismissed;
  $("welcomeText").textContent = platform === "macos"
    ? "宠物会在屏幕底部的角落里溜达。Coquou 住在屏幕右上角的菜单栏里，点那里的图标或者右键点宠物，可以叫TA回来、哄TA睡觉，或者再打开这个窗口。"
    : "宠物会在屏幕底部的角落里溜达。Coquou 住在屏幕右下角的通知区域（可能在「^」里），单击图标打开这个窗口，右键图标或者右键点宠物，可以叫TA回来、哄TA睡觉。";
}
$("welcomeOk").onclick = () => setPrefs({ welcomeDismissed: true });
$("dropHint").textContent = "把 .pet / .zip 宠物包拖进这个窗口，或拖到桌面上的宠物身上，都能安装。";

// ---------- behaviour ----------
function zoneSvg(spans) {
  const bars = spans.map(([a, b]) => `<rect x="${a}" y="58" width="${b - a}" height="12" rx="3" fill="var(--accent)"/>`).join("");
  return `<svg viewBox="0 0 120 76"><rect x="1" y="1" width="118" height="74" rx="8" fill="var(--hover)" stroke="var(--line-2)"/><rect x="30" y="14" width="60" height="30" rx="4" fill="var(--panel)" stroke="var(--line-2)"/>${bars}</svg>`;
}
function renderBehaviour() {
  $("zones").replaceChildren(...ZONES.map(([v, label, hint, spans]) => {
    const b = el("button", { className: "zone", type: "button" });
    b.innerHTML = `${zoneSvg(spans)}<b></b><small></small>`;
    b.querySelector("b").textContent = label;
    b.querySelector("small").textContent = hint;
    b.classList.toggle("on", ui.zone === v);
    b.onclick = () => setPrefs({ zone: v });
    return b;
  }));
  segmented($("activity"), ACTIVITIES, ui.activity, (v) => setPrefs({ activity: v }));
  segmented($("sleep"), SLEEPS, String(ui.sleepAfter), (v) => setPrefs({ sleepAfter: Number(v) }));
  segmented($("size"), SIZES, String(ui.size), setSize);
  $("chatter").checked = ui.chatter;
}

// ---------- general ----------
async function loadSettings() {
  settings = await invoke("get_settings");
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
  $(id).onchange = (e) => invoke("set_setting", { name: id, value: e.target.checked }).catch((err) => { toast("设置失败：" + err, true); loadSettings(); });
}
$("display").onchange = (e) => act("set_setting", { name: "display", value: e.target.value }, "切换显示器失败").then(loadSettings);
$("openDir").onclick = () => act("open_window", { name: "pets-dir" }, "打不开宠物文件夹");
for (const b of document.querySelectorAll("[data-link]")) b.onclick = () => act("open_link", { which: b.dataset.link }, "打不开链接");

// ---------- updates ----------
function ago(sec) {
  const d = Math.max(0, Date.now() / 1000 - sec);
  if (d < 60) return "刚刚";
  if (d < 3600) return `${Math.floor(d / 60)} 分钟前`;
  if (d < 86400) return `${Math.floor(d / 3600)} 小时前`;
  return `${Math.floor(d / 86400)} 天前`;
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
function renderUpdate(s) {
  const a = s.available;
  $("navUpdateDot").hidden = !a;
  $("updState").textContent = s.installing ? "正在下载更新…" : s.checking ? "正在检查更新…"
    : a ? `发现新版本 v${a.version}` : s.lastChecked ? `已是最新版本 v${s.current}` : `当前版本 v${s.current}`;
  $("updDetail").textContent = s.lastError || (s.lastChecked ? `上次检查：${ago(s.lastChecked)}` : "");
  $("updDetail").style.color = s.lastError ? "var(--bad)" : "";
  $("updCheck").disabled = s.checking || s.installing;
  $("updAvail").hidden = !a;
  if (a) {
    $("updVersion").textContent = `v${s.current} → v${a.version}`;
    $("updDate").textContent = a.date ? `发布于 ${a.date}` : "";
    $("updNotes").innerHTML = notesHtml(a.notes || "");
    $("updInstall").disabled = s.installing;
    $("updInstall").textContent = s.installing ? "正在更新…" : "更新并重启";
  }
  $("updProgress").hidden = !s.installing;
}
$("updCheck").onclick = () => invoke("update_check").then(renderUpdate).catch((e) => toast(String(e), true));
$("updInstall").onclick = () => invoke("update_install").catch((e) => toast("更新失败：" + e, true));
listen("update-status", ({ payload }) => renderUpdate(payload));
listen("update-progress", ({ payload }) => {
  const { downloaded, total } = payload;
  $("updProgress").hidden = false;
  $("updProgress").firstElementChild.style.width = total ? `${Math.min(100, (downloaded / total) * 100)}%` : "50%";
});

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
  toast(`${payload.replaced ? "更新" : "安装"}好了：${payload.displayName}`);
  selected = payload.key;
  showPage("pets");
  loadPets();
});
listen("pet-install-failed", ({ payload }) => toast(`装不上${payload.file ? " " + payload.file : ""}：${payload.error}`, true));
listen("pets-changed", ({ payload }) => { if (payload?.removed) { art.delete(payload.removed); ready.delete(payload.removed); } loadPets(); });

// Drag & drop: on the hatch page a single image becomes the reference image;
// anything else (.pet / .zip / folders) is installed.
const IMAGE = /\.(png|jpe?g|webp|gif|bmp)$/i;
const onHatch = () => !$("page-hatch").hidden;
const asReference = (paths) => onHatch() && paths?.length === 1 && IMAGE.test(paths[0]);
window.__TAURI__.webviewWindow.getCurrentWebviewWindow().onDragDropEvent(({ payload }) => {
  if (payload.type === "enter") $("dropText").textContent = asReference(payload.paths) ? "松手设为参考图" : "松手安装宠物";
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
    } catch (e) { toast("读不了这张图：" + e, true); }
  } else invoke("install_dropped");
});

(async () => {
  ui = await invoke("prefs_get");
  renderWelcome();
  renderBehaviour();
  await Promise.all([loadPets(), loadSettings()]);
  invoke("update_status").then(renderUpdate).catch(() => {});
})();
