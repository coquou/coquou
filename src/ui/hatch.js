// Hatch page of the app window: AI profiles, generation queue, previews,
// install. Wrapped so its names don't clash with app.js, which loads first
// (logging, `window.coquouConfirm`, and drops: a dropped image reaches
// `window.hatchSetReference`).
(() => {
const { invoke } = window.__TAURI__.core;
const { t } = window.Coquou;
const ask = (title, text, ok) => window.coquouConfirm(title, text, ok);

const $ = (id) => document.getElementById(id);
const S = HATCH.STATES;
const CONCURRENCY = 3;

const st = {
  profiles: [],
  refB64: null, refData: null,
  key: "green",
  baseB64: null,
  rows: {},        // id → { status, frames, cells, places, warnings, raw, error }
  atlasB64: null,
  hdB64: null,
};

function setStatus(el, text, kind = "") {
  el.textContent = text;
  el.className = "status" + (kind ? " " + kind : "");
}

// ---------- AI profiles ----------
const keystore = window.Coquou.keystore;
const renderKeyHint = () => {
  $("keyHint").textContent = t(`API key 只保存在本机系统的${keystore()}里。`, `The API key is only kept in this computer's ${keystore()}.`);
};
let editing = false; // the full form is shown; otherwise a one-line summary
function showForm(open) {
  const p = currentProfile();
  editing = open || !p || !p.hasKey;
  $("profileForm").hidden = !editing;
  $("profileSummary").hidden = editing;
  if (p) {
    $("sumName").textContent = `${p.name} ✓`;
    const protocol = p.protocol === "chat" ? t("Chat 协议", "Chat protocol") : t("Images 协议", "Images protocol");
    $("sumModel").textContent = `${p.model || t("（未填模型）", "(no model)")} · ${protocol} · ${t("key 已保存", "key saved")}`;
  }
}
$("editProfile").addEventListener("click", () => showForm(true));
function currentProfile() {
  return st.profiles.find((p) => p.id === $("profile").value) || null;
}
// keepOpen: leave the form showing (testing reports its result in the form's
// status line, which the summary would hide).
function fillForm(p, keepOpen = false) {
  $("pName").value = p?.name || "";
  $("pProtocol").value = p?.protocol || "images";
  $("pUrl").value = p?.baseUrl || "";
  $("pModel").value = p?.model || "";
  $("pKey").value = "";
  renderKeyPlaceholder(p);
  $("pQuality").value = p?.quality || "";
  $("pTransparent").checked = !!p?.transparent;
  $("deleteProfile").disabled = !p;
  $("imagesOpts").hidden = $("pProtocol").value !== "images";
  showForm(keepOpen);
}
function renderKeyPlaceholder(p) {
  $("pKey").placeholder = p?.hasKey ? t(`已保存在${keystore()}（留空不修改）`, `Saved in ${keystore()} (leave empty to keep it)`) : "sk-…";
}
async function loadProfiles(selectId, keepOpen = false) {
  st.profiles = await invoke("ai_profiles");
  renderProfileList(selectId ?? (st.profiles[0]?.id || ""));
  fillForm(currentProfile(), keepOpen);
}
function renderProfileList(selectId) {
  const sel = $("profile");
  sel.innerHTML = "";
  for (const p of st.profiles) sel.add(new Option(`${p.name}${p.hasKey ? "" : t("（未填 key）", " (no key)")}`, p.id));
  sel.add(new Option(t("＋ 新建配置…", "＋ New profile…"), ""));
  sel.value = selectId;
}
async function saveProfile(keepOpen = false) {
  const cur = currentProfile();
  const profile = {
    id: cur?.id || "",
    name: $("pName").value.trim() || t("未命名", "Untitled"),
    protocol: $("pProtocol").value,
    baseUrl: $("pUrl").value.trim(),
    model: $("pModel").value.trim(),
    quality: $("pQuality").value || null,
    transparent: $("pTransparent").checked,
    edits: cur?.edits ?? null,
  };
  const saved = await invoke("ai_save_profile", { profile, apiKey: $("pKey").value || null });
  await loadProfiles(saved.id, keepOpen);
  return saved;
}
$("profile").addEventListener("change", () => fillForm(currentProfile()));
$("newProfile").addEventListener("click", () => { $("profile").value = ""; fillForm(null); showForm(true); $("pName").focus(); });
$("pProtocol").addEventListener("change", () => { $("imagesOpts").hidden = $("pProtocol").value !== "images"; });
$("saveProfile").addEventListener("click", async () => {
  try { await saveProfile(); setStatus($("profileStatus"), t("已保存", "Saved"), "ok"); }
  catch (e) { setStatus($("profileStatus"), String(e), "bad"); }
});
$("testProfile").addEventListener("click", async () => {
  const el = $("profileStatus");
  try {
    const p = await saveProfile(true);
    setStatus(el, t("测试中…（生成 1 张图）", "Testing… (generating 1 image)"));
    const t0 = performance.now();
    const b64 = await invoke("ai_generate", { profileId: p.id, prompt: "A small red heart sticker on a plain white background.", images: [], size: "1024x1024" });
    const bmp = await HATCH.decode(b64);
    await loadProfiles(p.id, true); // the backend may have switched off unsupported options
    const dropped = p.transparent && !currentProfile()?.transparent
      ? t("（这个模型不支持透明背景，已自动关闭该选项）", " (this model can't do transparent backgrounds, so that option was turned off)") : "";
    const secs = ((performance.now() - t0) / 1000).toFixed(1);
    setStatus(el, t(`连接正常：${bmp.width}×${bmp.height}，用时 ${secs} 秒${dropped}`, `Works: ${bmp.width}×${bmp.height} in ${secs} s${dropped}`), "ok");
  } catch (e) { setStatus(el, String(e), "bad"); }
});
$("deleteProfile").addEventListener("click", async () => {
  const p = currentProfile();
  if (!p || !(await ask(t(`删除配置「${p.name}」？`, `Delete the profile ${p.name}?`), t("它保存的 API key 也会一起删除。", "Its saved API key is deleted too.")))) return;
  try {
    await invoke("ai_delete_profile", { id: p.id });
    await loadProfiles();
  } catch (e) { setStatus($("profileStatus"), t("删除失败：", "Could not delete: ") + e, "bad"); }
});

// ---------- reference image ----------
async function setReference(file) {
  if (!file || !file.type.startsWith("image/")) return;
  st.refB64 = await HATCH.toPngB64(file, 1024);
  st.refData = HATCH.pixels(await HATCH.decode(st.refB64));
  showReference();
}
function showReference() {
  const drop = $("drop");
  drop.classList.add("has");
  drop.innerHTML = "";
  const img = new Image(); img.src = "data:image/png;base64," + st.refB64; drop.append(img);
  const clear = document.createElement("button"); clear.className = "btn"; clear.type = "button"; clear.textContent = t("移除", "Remove");
  clear.onclick = (e) => { e.stopPropagation(); st.refB64 = st.refData = null; renderDropHint(); };
  drop.append(clear);
}
function renderDropHint() {
  const drop = $("drop");
  drop.classList.remove("has");
  drop.textContent = t("点击选择，或把图片拖进窗口", "Click to choose, or drop an image on the window");
}
$("drop").addEventListener("click", () => $("file").click());
$("file").addEventListener("change", (e) => setReference(e.target.files[0]));
// The app window's native drop handler hands image files over here.
window.hatchSetReference = setReference;

// ---------- estimate ----------
// Look rows (and their cardinal anchors) only when 16 directions are wanted.
const wanted = (s, withLook) => withLook || !(s.look || s.cardinal);
// Rows to generate; mirrored rows are derived, not generated.
function plannedStates() {
  return S.filter((s) => !s.mirrorOf && wanted(s, $("withLook").checked));
}
function updateEstimate() {
  const n = 1 + plannedStates().length;
  $("estimate").innerHTML = t(
    `预计 <b>${n}</b> 次图像生成（形象 1 次、动作 ${n - 1} 次）。失败重试另计，费用按你的接口计费。`,
    `About <b>${n}</b> image generations (1 for the look, ${n - 1} for animations), plus any retries, billed by your API provider.`);
}
$("withLook").addEventListener("change", updateEstimate);

// ---------- saved run (resume after closing the window) ----------
// Self-tests don't touch a real saved run unless asked to (`persist`).
const persist = !window.__COQUOU_SELFTEST__ || !!window.__COQUOU_SELFTEST__.persist;
function saveMeta() {
  if (!persist || !st.baseB64) return;
  const rows = {};
  for (const [id, r] of Object.entries(st.rows))
    if (r.raw || r.status === "bad") rows[id] = { status: r.status, warnings: r.warnings || [], error: r.error || "" };
  invoke("hatch_put_meta", { meta: {
    version: 1, savedAt: new Date().toISOString(),
    name: $("petName").value, desc: $("petDesc").value, style: $("style").value,
    withLook: $("withLook").checked, key: st.key, profileId: $("profile").value, petId: $("petId").value, rows,
  } }).catch((e) => console.warn("save meta failed", String(e)));
}
function saveImage(name, b64) {
  if (persist) invoke("hatch_put_image", { name, png: b64 }).catch((e) => console.warn("save image failed", name, String(e)));
}

async function offerResume() {
  if (!persist) return;
  const run = await invoke("hatch_load").catch(() => null);
  if (!run?.images?.base) return;
  const m = run.meta || {};
  const planned = S.filter((s) => !s.mirrorOf && wanted(s, m.withLook)).length;
  const done = Object.keys(run.images).filter((k) => k.startsWith("row-")).length;
  const area = $("baseArea");
  area.className = "resume";
  area.innerHTML = "";
  const p = document.createElement("div");
  const name = m.name || t("未命名", "Untitled");
  const when = new Date(m.savedAt).toLocaleString(t("zh-CN", "en"));
  p.textContent = t(
    `上次的孵化还没完成：「${name}」，动作已完成 ${done}/${planned} 行（${when}）。继续的话，已生成的图不会重新收费。`,
    `Your last hatch, ${name}, isn't finished: ${done}/${planned} animation rows done (${when}). Continuing doesn't pay again for images already made.`);
  const go = document.createElement("button"); go.className = "btn primary"; go.type = "button"; go.textContent = t("继续上次", "Continue");
  const drop = document.createElement("button"); drop.className = "btn"; drop.type = "button"; drop.textContent = t("丢弃", "Discard");
  go.onclick = () => restoreRun(run);
  drop.onclick = async () => {
    if (!(await ask(t("丢弃上次的孵化？", "Discard the last hatch?"), t("已经生成的图片都会删掉，无法恢复。", "The images made so far are deleted for good."), t("丢弃", "Discard")))) return;
    await invoke("hatch_clear");
    renderBaseHint();
  };
  const acts = document.createElement("div"); acts.className = "acts"; acts.append(go, drop);
  area.append(p, acts);
  window.coquouHatchPending?.(true);
}

async function restoreRun(run) {
  const m = run.meta || {};
  $("petName").value = m.name || ""; $("petDesc").value = m.desc || "";
  $("style").value = m.style || "auto"; $("withLook").checked = !!m.withLook; updateEstimate();
  if (m.profileId && st.profiles.some((p) => p.id === m.profileId)) { $("profile").value = m.profileId; fillForm(currentProfile()); }
  if (m.petId) $("petId").value = m.petId;
  if (run.images.ref) {
    st.refB64 = run.images.ref;
    st.refData = HATCH.pixels(await HATCH.decode(st.refB64));
    showReference();
  }
  st.key = m.key || "green";
  st.baseB64 = run.images.base;
  renderBase(await HATCH.processBase(st.baseB64));
  for (const s of S) {
    const raw = run.images["row-" + s.id];
    if (raw) await applySheet(s, raw, false);
    else if (m.rows?.[s.id]?.status === "bad") st.rows[s.id] = { status: "bad", error: m.rows[s.id].error };
  }
  relayout();
  renderCards();
  setStatus($("mainStatus"), t("已恢复上次的进度。点「生成全部动作」只会补上还没完成的行。", "Picked up where you left off. “Generate all” only fills in the rows that aren't done."), "ok");
  afterRows();
}

// ---------- generation ----------
async function generate(prompt, images, size) {
  const p = currentProfile();
  if (!p) throw new Error(t("请先保存一个 AI 接口配置", "Save an AI service profile first"));
  if (!p.hasKey) throw new Error(t("这个配置还没有填 API key", "This profile has no API key yet"));
  const out = await invoke("ai_generate", { profileId: p.id, prompt, images, size });
  return HATCH.toPngB64(out, 1536);
}

function previewCanvas(cell) {
  const c = document.createElement("canvas");
  c.width = HATCH.CELL_W; c.height = HATCH.CELL_H; c.className = "checker";
  c.getContext("2d").drawImage(cell, 0, 0);
  return c;
}

async function genBase() {
  const desc = $("petDesc").value.trim();
  const el = $("mainStatus");
  if (!st.refB64 && !desc) { setStatus(el, t("请填写描述或选择一张参考图", "Write a description or choose a reference image"), "bad"); return false; }
  $("genBase").disabled = true;
  setStatus(el, t("正在生成形象…（通常 10–60 秒）", "Generating the look… (usually 10–60 s)"));
  try {
    st.key = HATCH.chooseKey(st.refData, desc);
    const prompt = HATCH.basePrompt({ description: desc, style: $("style").value, key: st.key, hasReference: !!st.refB64 });
    st.baseB64 = await generate(prompt, st.refB64 ? [st.refB64] : [], "1024x1024");
    const base = await HATCH.processBase(st.baseB64);
    renderBase(base);
    // A new base starts a new run: old sheets no longer match it.
    if (persist) await invoke("hatch_clear").catch(() => {});
    if (st.refB64) saveImage("ref", st.refB64);
    saveImage("base", st.baseB64);
    saveMeta();
    setStatus(el, t("形象好了。满意的话点「生成全部动作」，不满意可以改描述后重新生成。", "The look is ready. Happy with it? Click “Generate all”. If not, change the description and generate again."), "ok");
    console.info("base ok", base.bgKind, base.warnings.join("; "));
    return true;
  } catch (e) {
    setStatus(el, t("生成失败：", "Generation failed: ") + e, "bad");
    console.error("base failed", String(e));
    return false;
  } finally {
    $("genBase").disabled = false;
    renderGenBase();
  }
}
function renderGenBase() {
  $("genBase").textContent = st.baseB64 ? t("重新生成形象", "Generate again") : t("生成形象", "Generate the look");
}
function renderBaseHint() {
  const area = $("baseArea");
  area.className = "empty";
  area.textContent = t("填好上面的接口和描述，点「生成形象」。", "Fill in the service and description above, then click “Generate the look”.");
}

function renderBase(base) {
  const area = $("baseArea");
  area.className = "";
  area.innerHTML = "";
  const wrap = document.createElement("div"); wrap.className = "base";
  const raw = new Image(); raw.className = "raw"; raw.src = "data:image/png;base64," + st.baseB64; raw.title = t("模型原图", "Model output");
  const cut = previewCanvas(base.cell); cut.style.width = "192px"; cut.title = t("抠图后", "Cut out");
  const info = document.createElement("div");
  info.innerHTML = `<p class="hint">${t("左：模型原图　右：抠图并缩放到 192×208 后的效果", "Left: model output. Right: cut out and scaled to 192×208.")}</p>`
    + (base.warnings.length ? `<p class="hint" style="color:var(--warn)">${base.warnings.join("；")}</p>` : "");
  wrap.append(raw, cut, info);
  area.append(wrap);
  st.rows = {};
  renderCards();
  $("rowsArea").hidden = false;
  $("finalArea").hidden = true;
}

// What the user wants changed per row, used when regenerating it.
const notes = new Map();

function renderCards() {
  const box = $("cards");
  box.innerHTML = "";
  for (const s of S) {
    if (!wanted(s, $("withLook").checked)) continue;
    const r = st.rows[s.id] || { status: "idle" };
    const card = document.createElement("div"); card.className = "hcard"; card.id = "card-" + s.id;
    const pill = {
      idle: [t("等待", "Waiting"), ""], run: [t("生成中…", "Generating…"), "run"], ok: [t("完成", "Done"), "ok"],
      warn: [t("有提醒", "Warnings"), "warn"], bad: [t("失败", "Failed"), "bad"],
    }[r.status];
    card.innerHTML = `<header><b>${s.label}</b><span class="pill ${pill[1]}">${s.mirrorOf ? t("镜像 · ", "Mirrored · ") : ""}${pill[0]}</span></header>`;
    const anim = document.createElement("canvas"); anim.className = "anim checker";
    anim.width = HATCH.CELL_W; anim.height = HATCH.CELL_H; anim.dataset.state = s.id;
    card.append(anim);
    const warn = document.createElement("div");
    warn.className = r.status === "bad" ? "bad" : "warn";
    warn.textContent = r.status === "bad" ? r.error : (r.warnings || []).join(t("；", "; "));
    card.append(warn);
    if (r.raw) {
      const d = document.createElement("details");
      d.innerHTML = `<summary>${t("模型原图", "Model output")}</summary>`;
      const img = new Image(); img.src = "data:image/png;base64," + r.raw; d.append(img);
      card.append(d);
    }
    if (!s.mirrorOf && r.status !== "idle") {
      const note = document.createElement("input");
      note.type = "text"; note.className = "note"; note.maxLength = 200;
      note.placeholder = t("想怎么改（可选），比如：腿迈得再大一点", "What to change (optional), e.g. longer steps");
      note.value = notes.get(s.id) || "";
      note.disabled = r.status === "run";
      note.oninput = () => notes.set(s.id, note.value);
      card.append(note);
    }
    if (!s.mirrorOf) {
      const btn = document.createElement("button"); btn.className = "btn"; btn.type = "button";
      btn.textContent = r.status === "idle" ? t("只生成这一行", "Generate this row") : t("重新生成", "Generate again");
      btn.disabled = r.status === "run" || !st.baseB64;
      btn.onclick = () => genRow(s).then(afterRows);
      card.append(btn);
    }
    box.append(card);
  }
}

/** Re-layout every finished row with the shared (idle-based) scale. */
function relayout() {
  const frames = {};
  for (const [id, r] of Object.entries(st.rows)) if (r.frames) frames[id] = r.frames;
  const laid = HATCH.layoutAll(frames);
  for (const [id, { cells, places }] of Object.entries(laid)) if (st.rows[id]) Object.assign(st.rows[id], { cells, places });
}

const ready = (id) => ["ok", "warn"].includes(st.rows[id]?.status);
const depsOf = (s) => (s.look ? ["look-cardinals"] : []);

/** Turn a generated sheet into frames for state `s` (and its mirror). */
async function applySheet(s, raw, save = true) {
  const out = await HATCH.processSheet(raw, s, st.key);
  st.rows[s.id] = { status: out.ok && !out.warnings.length ? "ok" : out.ok ? "warn" : "bad", frames: out.frames, cells: out.cells, warnings: out.warnings, raw,
    error: out.ok ? "" : out.errors.concat(out.warnings).join(t("；", "; ")) };
  for (const m of S.filter((x) => x.mirrorOf === s.id))
    st.rows[m.id] = { status: st.rows[s.id].status === "bad" ? "bad" : "ok", frames: out.frames, cells: HATCH.mirrorCells(out.cells), warnings: [], error: st.rows[s.id].error };
  if (save) { saveImage("row-" + s.id, raw); saveMeta(); }
  relayout();
  return st.rows[s.id].status;
}

// Errors not worth an automatic retry (bad key, bad request, wrong model…).
const permanent = (e) => /^HTTP 4(0[0-4]|1\d|2[0-8])/.test(String(e));

async function genRow(s, attempt = 1) {
  const missing = depsOf(s).filter((d) => !ready(d));
  if (missing.length) {
    st.rows[s.id] = { status: "bad", error: t("需要先生成：", "Generate these first: ") + missing.map((d) => S.find((x) => x.id === d).label).join(t("、", ", ")) };
    renderCards();
    return;
  }
  st.rows[s.id] = { status: "run" };
  renderCards();
  let retry = false;
  try {
    const prompt = HATCH.sheetPrompt(s, st.key, $("style").value, (notes.get(s.id) || "").trim());
    const refs = [st.baseB64, ...depsOf(s).map((d) => st.rows[d].raw)];
    const raw = await generate(prompt, refs, HATCH.layoutFor(s.frames).size);
    const status = await applySheet(s, raw);
    console.info("row", s.id, status, status === "bad" ? st.rows[s.id].error : (st.rows[s.id].warnings || []).join("; "));
    retry = status === "bad";
  } catch (e) {
    st.rows[s.id] = { status: "bad", error: String(e) };
    console.error("row failed", s.id, String(e));
    retry = !permanent(e);
    saveMeta();
  }
  // One automatic retry for frames that came out unusable or a flaky request.
  if (retry && attempt < 2) {
    console.info("row", s.id, "retrying once");
    await new Promise((r) => setTimeout(r, window.__COQUOU_SELFTEST__ ? 200 : 3000));
    return genRow(s, attempt + 1);
  }
  renderCards();
}

async function genAll() {
  $("genAll").disabled = $("genBase").disabled = true;
  const queue = plannedStates().filter((s) => !ready(s.id));
  const total = queue.length;
  let done = 0;
  const tick = () => setStatus($("rowsStatus"), t(`进度 ${done}/${total}`, `Progress ${done}/${total}`));
  tick();
  const running = new Set();
  // Take the next row whose dependencies are done (or can no longer finish).
  const next = () => {
    const i = queue.findIndex((s) => depsOf(s).every((d) => ready(d) || (!running.has(d) && !queue.some((q) => q.id === d))));
    return i < 0 ? null : queue.splice(i, 1)[0];
  };
  const worker = async () => {
    while (queue.length) {
      const s = next();
      if (!s) { await new Promise((r) => setTimeout(r, 300)); continue; }
      running.add(s.id);
      await genRow(s);
      running.delete(s.id); done++; tick();
    }
  };
  await Promise.all(Array.from({ length: Math.min(CONCURRENCY, total) }, worker));
  $("genAll").disabled = $("genBase").disabled = false;
  afterRows();
}
$("genBase").addEventListener("click", genBase);
$("genAll").addEventListener("click", genAll);

function afterRows() {
  const bad = S.filter((s) => wanted(s, $("withLook").checked) && !ready(s.id));
  if (bad.length) {
    setStatus($("rowsStatus"), t("还差：", "Still missing: ") + bad.map((s) => s.label).join(t("、", ", ")), bad.some((s) => st.rows[s.id]?.status === "bad") ? "bad" : "");
    $("finalArea").hidden = true;
    return false;
  }
  setStatus($("rowsStatus"), t("全部动作完成", "All animations done"), "ok");
  buildFinal();
  return true;
}

// Several callers may ask for the atlas at once; only the newest result counts.
let buildSeq = 0;
async function buildFinal() {
  const seq = ++buildSeq;
  relayout();
  const rows = {};
  for (const s of S) if (s.row != null) rows[s.row] = st.rows[s.id]?.places ?? [];
  // Without generated look rows, every direction shows the first idle frame.
  if (!$("withLook").checked) {
    const idle = st.rows["idle"].places[0];
    rows[9] = Array(8).fill(idle); rows[10] = Array(8).fill(idle);
  }
  const [{ canvas, b64 }, hd] = await Promise.all([HATCH.composeAtlas(rows), HATCH.composeAtlas(rows, 2)]);
  if (seq !== buildSeq) return; // a newer build superseded this one
  st.atlasB64 = b64;
  st.hdB64 = hd.b64;
  const view = $("atlas").getContext("2d");
  view.clearRect(0, 0, 1536, 2288); view.drawImage(canvas, 0, 0);
  if (!$("petId").value) $("petId").value = slug($("petName").value);
  $("finalArea").hidden = false;
}

function slug(name) {
  const s = name.normalize("NFKD").replace(/[^\w-]+/g, "-").replace(/^[-_]+|[-_]+$/g, "").toLowerCase().slice(0, 40);
  return s || "pet-" + Date.now().toString(36);
}

async function install() {
  const el = $("installStatus");
  try {
    const p = currentProfile();
    const done = await invoke("install_generated", {
      id: $("petId").value.trim(),
      displayName: $("petName").value.trim() || $("petId").value.trim(),
      description: $("petDesc").value.trim() || "AI hatched pet",
      spritesheetPng: st.atlasB64,
      hdPng: st.hdB64,
      coquouJson: JSON.stringify({ format: "coquou/1", hd: "spritesheet@2x.png", tags: ["ai-hatched"], generator: { tool: "coquou-hatch", protocol: p?.protocol, model: p?.model } }, null, 2),
    });
    setStatus(el, done.replaced
      ? t(`已更新「${done.displayName}」，桌宠已经换成 TA 了。`, `Updated ${done.displayName}; they're on your desktop now.`)
      : t(`已安装「${done.displayName}」，桌宠已经换成 TA 了。`, `Installed ${done.displayName}; they're on your desktop now.`), "ok");
    window.coquouHatchPending?.(false);
    if (persist) invoke("hatch_clear").catch(() => {});
    console.info("installed", done.key);
    return done;
  } catch (e) {
    setStatus(el, t("安装失败：", "Install failed: ") + e, "bad");
    console.error("install failed", String(e));
    return null;
  }
}
$("install").addEventListener("click", install);

// ---------- card animations ----------
let frameNo = 0;
setInterval(() => {
  frameNo++;
  for (const c of document.querySelectorAll("canvas.anim")) {
    const cells = st.rows[c.dataset.state]?.cells;
    const g = c.getContext("2d");
    g.clearRect(0, 0, c.width, c.height);
    if (cells?.length) g.drawImage(cells[frameNo % cells.length], 0, 0);
  }
}, 140);

// ---------- language ----------
// Redraw what this page wrote after a language change (app.js `relabel`).
// Status lines already shown keep the language they were written in.
window.hatchRelabel = () => {
  renderKeyHint();
  renderProfileList($("profile").value);
  renderKeyPlaceholder(currentProfile());
  showForm(editing);
  if (!st.refB64) renderDropHint();
  else showReference();
  if ($("baseArea").classList.contains("empty")) renderBaseHint();
  renderGenBase();
  updateEstimate();
  renderCards();
};

// ---------- start ----------
renderKeyHint();
renderDropHint();
renderBaseHint();
renderGenBase();
updateEstimate();
loadProfiles().then(() => (window.__COQUOU_SELFTEST__ ? selftest() : offerResume()));

// Dev self-test (`coquou --hatch-selftest <base url> <images|chat>`) runs the
// whole flow against a mock server (see src/tests/mock-image-api.py).
async function selftest() {
  const cfg = window.__COQUOU_SELFTEST__;
  if (!cfg) return;
  console.info("selftest start", cfg.protocol);
  let saved = { id: "" };
  try {
    saved = await invoke("ai_save_profile", {
      profile: { id: "", name: "selftest-" + cfg.protocol, protocol: cfg.protocol || "images", baseUrl: cfg.selftest, model: "mock-image", quality: null, transparent: !!cfg.transparent },
      apiKey: "test-key",
    });
    await loadProfiles(saved.id);
    $("petName").value = "自测团子"; $("petDesc").value = "a round orange blob mascot";
    if (!(await genBase())) throw new Error("base failed");
    await genAll();
    if (!afterRows()) throw new Error("rows incomplete: " + JSON.stringify(Object.fromEntries(Object.entries(st.rows).map(([k, v]) => [k, v.status + (v.error ? ":" + v.error : "")]))));
    await buildFinal();
    if (persist) {
      // Save/restore round trip: rebuild everything from the stored run and
      // expect a byte-identical atlas.
      const before = st.atlasB64;
      const beforeRaw = Object.fromEntries(Object.entries(st.rows).map(([k, v]) => [k, v.raw]));
      const run = await invoke("hatch_load");
      for (const [k, raw] of Object.entries(beforeRaw))
        if (raw && run.images["row-" + k] !== raw) console.warn("selftest stored sheet differs:", k);
      if (run.images.base !== st.baseB64) console.warn("selftest stored base differs");
      const rows = Object.keys(run?.images || {}).filter((k) => k.startsWith("row-")).length;
      st.rows = {}; st.baseB64 = null; st.atlasB64 = null; st.hdB64 = null;
      await restoreRun(run);
      await buildFinal();
      console.info("selftest resume", st.atlasB64 === before ? "OK" : "MISMATCH", `stored rows=${rows}`);
      if (st.atlasB64 !== before) throw new Error("resume produced a different atlas");
    }
    $("petId").value = "selftest-" + cfg.protocol;
    const done = await install();
    console.info("selftest", done ? "PASS " + done.key : "FAIL install");
  } catch (e) {
    console.error("selftest FAIL", String(e));
  } finally {
    await invoke("ai_delete_profile", { id: saved.id }).catch(() => {});
    if (persist) await invoke("hatch_clear").catch(() => {});
  }
}
})();
