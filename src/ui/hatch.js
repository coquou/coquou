// Hatch page of the app window: AI profiles, generation queue, previews,
// install. Wrapped so its names don't clash with app.js, which loads first
// (logging, `window.coquouConfirm`, and drops: a dropped image reaches
// `window.hatchSetReference`).
(() => {
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
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
  rows: {},        // id → { status, frames, cells, places, warnings, raw, error, atlas }
  atlasB64: null,
  hdB64: null,
  // Editing an installed pet: { key, id, name, description, legacy } (see
  // editPet), and its atlas ({ img, k }) for the rows taken from it.
  source: null,
  petAtlas: null,
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
  clear.onclick = (e) => { e.stopPropagation(); st.refB64 = st.refData = null; renderDropHint(); renderReset(); };
  drop.append(clear);
  renderReset();
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
    if (r.raw || r.atlas || r.status === "bad") rows[id] = { status: r.status, warnings: r.warnings || [], error: r.error || "", atlas: !!r.atlas };
  invoke("hatch_put_meta", { meta: {
    version: 1, savedAt: new Date().toISOString(),
    name: $("petName").value, desc: $("petDesc").value, style: $("style").value,
    withLook: $("withLook").checked, key: st.key, profileId: $("profile").value, petId: $("petId").value, rows,
    source: st.source,
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
  p.textContent = m.source
    ? t(`上次对「${name}」的修改还没完成（${when}）。继续的话，已生成的图不会重新收费。`,
      `Your changes to ${name} aren't finished (${when}). Continuing doesn't pay again for images already made.`)
    : t(`上次的孵化还没完成：「${name}」，动作已完成 ${done}/${planned} 行（${when}）。继续的话，已生成的图不会重新收费。`,
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
  renderReset();
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
  st.source = m.source || null;
  st.petAtlas = null;
  if (run.images.atlas) {
    const bmp = await HATCH.decode(run.images.atlas);
    st.petAtlas = { img: HATCH.pixels(bmp), k: bmp.width / (HATCH.CELL_W * window.Coquou.COLS) };
  }
  renderBase(await HATCH.processBase(st.baseB64));
  for (const s of S) {
    const raw = run.images["row-" + s.id];
    if (raw) await applySheet(s, raw, false);
    else if (m.rows?.[s.id]?.atlas && st.petAtlas) applyAtlas(s);
    else if (m.rows?.[s.id]?.status === "bad") st.rows[s.id] = { status: "bad", error: m.rows[s.id].error };
  }
  relayout();
  renderCards();
  renderInstallMode();
  renderSource();
  setStatus($("mainStatus"), st.source ? "" : t("已恢复上次的进度。点「生成全部动作」只会补上还没完成的行。", "Picked up where you left off. “Generate all” only fills in the rows that aren't done."), "ok");
  afterRows();
}

// ---------- continue from an installed pet ----------
// Pets hatched by Coquou (coquou.json `tags` has "ai-hatched"). Since this
// version their hatch material is saved with them (coquou.json `hatch`,
// doc/pet-format.md); older ones only have the atlas.
let hatched = []; // list_pets entries
const petFile = (key, path) => invoke("read_pet_file", { key, path });
const petJson = async (key, path) => JSON.parse(new TextDecoder().decode(await petFile(key, path)));

// Installs and deletions each trigger a scan; run them one after another so
// an older scan can't finish last and bring back a pet that is gone.
let scanning = Promise.resolve();
function loadHatched() {
  scanning = scanning.then(scanHatched);
  return scanning;
}
async function scanHatched() {
  const list = await invoke("list_pets").catch((e) => { console.warn("list pets failed", String(e)); return []; });
  const found = [];
  for (const p of list) {
    if (!p.extFile) continue;
    try {
      const ext = await petJson(p.key, p.extFile);
      if (Array.isArray(ext?.tags) && ext.tags.includes("ai-hatched")) found.push(p);
    } catch (e) { console.warn("cannot read", p.key, p.extFile, String(e)); }
  }
  hatched = found;
}

/**
 * Turn an installed hatched pet into a hatch run, as if it had been resumed:
 * stored sheets are processed again; rows without one are cut from the atlas
 * (the HD copy when there is one). Without stored material, the first idle
 * frame on the key colour stands in for the base image.
 */
async function editPet(key) {
  const entry = hatched.find((p) => p.key === key);
  if (!entry) throw new Error(t("找不到这只宠物", "Cannot find this pet"));
  const manifest = await petJson(key, "pet.json");
  const ext = await petJson(key, entry.extFile);
  const h = ext.hatch && typeof ext.hatch === "object" ? ext.hatch : null;
  const atlasB64 = HATCH.bytesToB64(await petFile(key, ext.hd || manifest.spritesheetPath || "spritesheet.webp"));
  const bmp = await HATCH.decode(atlasB64);
  const k = bmp.width / (HATCH.CELL_W * window.Coquou.COLS);
  const img = HATCH.pixels(bmp);
  // Hatched without look rows: rows 9 and 10 repeat the first idle frame.
  const lookCells = [9, 10].flatMap((r) => Array.from({ length: 8 }, (_, c) => [r, c]));
  const withLook = typeof h?.withLook === "boolean" ? h.withLook : !HATCH.sameCells(img, [[0, 0], ...lookCells], k);
  const key0 = h?.key || HATCH.chooseKey(img);
  const read = async (path) => HATCH.bytesToB64(await petFile(key, path));
  const images = {
    atlas: atlasB64,
    base: h?.base ? await read(h.base) : await HATCH.framesSheet(HATCH.atlasFrames(img, [[0, 0]], k), { cols: 1, rows: 1, size: "1024x1024" }, key0),
  };
  // The reference to regenerate the look from: the original one, else the
  // look itself, so a changed description edits this pet instead of starting over.
  images.ref = h?.reference ? await read(h.reference) : images.base;
  const rows = {};
  for (const s of S) {
    if (s.mirrorOf || !wanted(s, withLook)) continue;
    const path = h?.rows?.[s.id];
    if (typeof path === "string") {
      try { images["row-" + s.id] = await read(path); continue; }
      catch (e) { console.warn("hatch sheet unreadable, using the atlas", s.id, String(e)); }
    }
    rows[s.id] = { status: "ok", atlas: true };
  }
  const meta = {
    version: 1, savedAt: new Date().toISOString(),
    name: manifest.displayName || manifest.id, desc: h?.description || "", style: h?.style || "auto",
    withLook, key: key0, profileId: $("profile").value, petId: manifest.id, rows,
    source: { key, id: manifest.id, name: entry.displayName, description: manifest.description || "", legacy: !h?.base },
  };
  if (persist) {
    await invoke("hatch_clear");
    await Promise.all(Object.entries(images).map(([name, png]) => invoke("hatch_put_image", { name, png })));
    await invoke("hatch_put_meta", { meta });
  }
  notes.clear();
  await restoreRun({ meta, images });
  console.info("editing", key, h ? "with hatch material" : "from the atlas only");
}

/** A generation is running (the look or any row). */
const busy = () => $("genBase").disabled || Object.values(st.rows).some((r) => r.status === "run");

async function startEdit(key) {
  const el = $("mainStatus");
  if (busy()) {
    setStatus(el, t("正在生成，等这一轮完成再载入。", "Still generating; load it once this round is done."), "bad");
    return;
  }
  if ((st.baseB64 || $("baseArea").classList.contains("resume"))
    && !(await ask(t("载入这只宠物？", "Load this pet?"), t("孵化页上现在的进度会被替换，已经生成的图片会删掉。", "What's on the hatch page now is replaced, and the images made so far are deleted."), t("载入", "Load")))) return;
  setStatus(el, t("正在载入…", "Loading…"));
  try { await editPet(key); }
  catch (e) { setStatus(el, t("载入失败：", "Could not load: ") + e, "bad"); console.error("edit failed", key, String(e)); }
}
// The only way in: the pet library's 「在孵化页修改」 button (app.js).
window.hatchEditPet = async (key) => {
  if (!hatched.some((p) => p.key === key)) await loadHatched();
  return startEdit(key);
};

/**
 * The notice that a pet is being changed, and the header button that ends
 * it (取消修改) or wipes a new hatch (清空重来). The button shows once there
 * is something to clear.
 */
function renderSource() {
  const src = st.source;
  $("editNote").hidden = !src;
  if (src) $("editNote").textContent = t(
    `正在修改「${src.name}」。不满意的动作在那一行写上想怎么改，点「重新生成」只重做那一行；改了描述或参考图后点「重新生成形象」，所有动作都要重做。改好后在「⑤ 安装」里替换原来的宠物或另存为新宠物。`,
    `You're changing ${src.name}. To fix one animation, say what to change on its row and click “Generate again”: only that row is redone. To change the look, edit the description or reference and click “Generate again” in step 3; then every animation is redone. When you're done, replace the old pet or save a new one in step 5.`);
  renderReset();
}
function renderReset() {
  const b = $("hatchReset");
  b.textContent = st.source ? t("取消修改", "Cancel changes") : t("清空重来", "Start over");
  b.hidden = !(st.source || st.baseB64 || st.refB64 || $("petName").value.trim() || $("petDesc").value.trim()
    || $("baseArea").classList.contains("resume"));
}

/** Back to an empty hatch page; the saved run (and its images) go too. */
async function resetHatch() {
  const el = $("mainStatus");
  if (busy()) {
    setStatus(el, t("正在生成，等这一轮完成再清空。", "Still generating; clear it once this round is done."), "bad");
    return;
  }
  const made = st.baseB64 || $("baseArea").classList.contains("resume");
  if (made && !(await (st.source
    ? ask(t(`取消对「${st.source.name}」的修改？`, `Cancel your changes to ${st.source.name}?`),
      t("这次生成的图片会删掉，无法恢复；原来的宠物不受影响。", "The images made this time are deleted for good; the pet itself is left as it was."), t("取消修改", "Cancel changes"))
    : ask(t("清空这次孵化？", "Clear this hatch?"),
      t("已经生成的图片会删掉，无法恢复；描述和参考图也会清空。", "The images made so far are deleted for good, and the description and reference are cleared."), t("清空", "Clear"))))) return;
  if (persist) await invoke("hatch_clear").catch((e) => console.warn("hatch clear failed", String(e)));
  Object.assign(st, { refB64: null, refData: null, key: "green", baseB64: null, rows: {}, atlasB64: null, hdB64: null, source: null, petAtlas: null });
  notes.clear();
  $("petName").value = $("petDesc").value = $("petId").value = "";
  $("style").value = "auto";
  $("withLook").checked = true;
  updateEstimate();
  renderDropHint();
  renderBaseHint();
  renderGenBase();
  renderInstallMode();
  $("rowsArea").hidden = $("finalArea").hidden = true;
  for (const id of ["mainStatus", "rowsStatus", "installStatus"]) setStatus($(id), "");
  renderSource();
  window.coquouHatchPending?.(false);
}
$("hatchReset").addEventListener("click", resetHatch);
// The header button appears as soon as the form has something in it.
for (const id of ["petName", "petDesc"]) $(id).addEventListener("input", renderReset);
listen("pet-installed", loadHatched);
listen("pets-changed", loadHatched);

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
    // A new base starts a new run: old sheets (and an edited pet's atlas
    // rows) no longer match it.
    st.petAtlas = null;
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
  renderReset();
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
  if (st.source?.legacy && st.petAtlas) {
    const note = document.createElement("p"); note.className = "hint";
    note.textContent = t(
      "这只宠物孵化时还没有保存原图，所以左边的形象是从图集里待机第 1 帧铺上抠像底色做的，比原图小、细节少一些；生成新动作时以它为参考，画风可能和原来的动作略有出入。没重新生成的动作直接沿用图集里的帧，只能整行重新生成，没法在原图上微调。",
      "This pet was hatched before Coquou kept the original images, so the look on the left is the first idle frame from its atlas on the key colour: smaller and less detailed than the original. New animations use it as their reference and may differ a little in style from the old ones. Animations you don't regenerate keep their frames from the atlas; they can only be regenerated as a whole row, not touched up.");
    info.append(note);
  }
  wrap.append(raw, cut, info);
  area.append(wrap);
  st.rows = {};
  renderCards();
  $("rowsArea").hidden = false;
  $("finalArea").hidden = true;
  renderReset();
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
    if (r.atlas && r.status === "ok") pill[0] = t("沿用图集", "From the atlas");
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
  setRow(s, { status: out.ok && !out.warnings.length ? "ok" : out.ok ? "warn" : "bad", frames: out.frames, cells: out.cells, warnings: out.warnings, raw,
    error: out.ok ? "" : out.errors.concat(out.warnings).join(t("；", "; ")) });
  if (save) { saveImage("row-" + s.id, raw); saveMeta(); }
  return st.rows[s.id].status;
}

/**
 * Frames for state `s` from the atlas of the pet being edited. The look
 * anchors have no atlas row; they come from the up / right / down / left
 * cells of the two look rows.
 */
function applyAtlas(s) {
  const { img, k } = st.petAtlas;
  const cells = s.cardinal ? [[9, 0], [9, 4], [10, 0], [10, 4]] : Array.from({ length: s.frames }, (_, c) => [s.row, c]);
  const frames = HATCH.atlasFrames(img, cells, k);
  const errors = frames.every(Boolean) ? HATCH.atlasErrors(frames) : [t("图集里这一行缺帧", "This row of the atlas is missing frames")];
  setRow(s, { status: errors.length ? "bad" : "ok", frames, cells: [], warnings: [], atlas: true,
    error: errors.length ? errors.join(t("；", "; ")) + t("，请重新生成这一行", "; generate this row again") : "" });
}

/** Store a row (and its mirror, which shares the frames) and lay all rows out again. */
function setRow(s, row) {
  st.rows[s.id] = row;
  for (const m of S.filter((x) => x.mirrorOf === s.id))
    st.rows[m.id] = { status: row.status === "bad" ? "bad" : "ok", frames: row.frames, cells: [], warnings: [], error: row.error };
  relayout();
}

/**
 * The sheet a dependent row is generated from: the model's own, or for a
 * row taken from the atlas its frames drawn back onto a key-colour grid.
 */
async function sheetOf(id) {
  const r = st.rows[id];
  return r.raw || (r.sheet ??= await HATCH.framesSheet(r.frames, HATCH.layoutFor(r.frames.length), st.key));
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
    const refs = [st.baseB64, ...(await Promise.all(depsOf(s).map(sheetOf)))];
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

// ---------- install ----------
// Editing a pet: replace it (same id, so the name and lines the user gave it
// in Coquou stay) or save a new one next to it.
const installMode = () => document.querySelector('input[name="installMode"]:checked')?.value;
const replacing = () => !!st.source && installMode() === "replace";
async function coquouIds() {
  const list = await invoke("list_pets").catch(() => []);
  return new Set(list.filter((p) => p.source === "coquou").map((p) => p.id));
}
function renderInstallMode() {
  const src = st.source;
  $("installMode").hidden = !src;
  if (!src) { $("petId").disabled = false; return; }
  // Presets and Codex pets are read-only: they can only be saved as a new pet.
  const canReplace = src.key.startsWith("coquou:");
  $("modeReplace").hidden = !canReplace;
  renderReplaceText();
  document.querySelector(`input[name="installMode"][value="${canReplace ? "replace" : "new"}"]`).checked = true;
  applyInstallMode();
}
function renderReplaceText() {
  $("modeReplaceText").textContent = t(`替换原来的「${st.source.name}」（保留你给 TA 改的名字和台词）`, `Replace ${st.source.name} (keeps the name and lines you gave them)`);
}
async function applyInstallMode() {
  $("petId").disabled = replacing();
  if (replacing()) { $("petId").value = st.source.id; return; }
  if (st.source && $("petId").value === st.source.id) {
    const taken = await coquouIds();
    let n = 2;
    while (taken.has(`${st.source.id}-${n}`)) n++;
    $("petId").value = `${st.source.id}-${n}`;
  }
}
for (const r of document.querySelectorAll('input[name="installMode"]')) r.addEventListener("change", applyInstallMode);

/** What the pet was made from, saved with it so it can be edited later (doc/pet-format.md). */
function hatchMaterial() {
  const files = { base: st.baseB64 };
  const hatch = { key: st.key, style: $("style").value, description: $("petDesc").value.trim(), withLook: $("withLook").checked, base: "hatch/base.png", rows: {} };
  // The stand-in reference of an edited pet is its own look; no need to keep it twice.
  if (st.refB64 && st.refB64 !== st.baseB64) { files.ref = st.refB64; hatch.reference = "hatch/ref.png"; }
  for (const s of S) {
    const raw = st.rows[s.id]?.raw;
    if (s.mirrorOf || !raw || !wanted(s, hatch.withLook)) continue;
    files["row-" + s.id] = raw;
    hatch.rows[s.id] = `hatch/row-${s.id}.png`;
  }
  return { files, hatch };
}

/**
 * The replaced pet's coquou.json with the new atlas and hatch material:
 * lines, translations, author and the like stay. Extra sheets aren't carried
 * over, so states drawn from one are dropped.
 */
async function keptExt(fresh) {
  const extFile = hatched.find((p) => p.key === st.source.key)?.extFile;
  let old = null;
  try { if (extFile) old = await petJson(st.source.key, extFile); }
  catch (e) { console.warn("cannot read the replaced pet's", extFile, String(e)); }
  if (!old || typeof old !== "object") return fresh;
  const merged = { ...old, ...fresh, tags: [...new Set([...(Array.isArray(old.tags) ? old.tags : []), ...fresh.tags])] };
  const states = Object.entries(old.states || {}).filter(([, v]) => !v?.sheet);
  if (states.length) merged.states = Object.fromEntries(states); else delete merged.states;
  return merged;
}

async function install() {
  const el = $("installStatus");
  try {
    const p = currentProfile();
    const replace = replacing();
    const id = replace ? st.source.id : $("petId").value.trim();
    if (st.source && !replace && (await coquouIds()).has(id))
      throw new Error(t("已经有宠物在用这个 ID 了，换一个吧", "Another pet already uses this ID; pick another one"));
    const { files, hatch } = hatchMaterial();
    let ext = { format: "coquou/1", hd: "spritesheet@2x.png", tags: ["ai-hatched"], generator: { tool: "coquou-hatch", protocol: p?.protocol, model: p?.model }, hatch };
    if (replace) ext = await keptExt(ext);
    const done = await invoke("install_generated", {
      id,
      displayName: $("petName").value.trim() || id,
      description: (replace ? st.source.description : $("petDesc").value.trim()) || "AI hatched pet",
      spritesheetPng: st.atlasB64,
      hdPng: st.hdB64,
      coquouJson: JSON.stringify(ext, null, 2),
      hatchFiles: files,
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
  if (st.source) renderReplaceText();
  renderSource();
};

// ---------- start ----------
renderKeyHint();
renderDropHint();
renderBaseHint();
renderGenBase();
updateEstimate();
loadHatched();
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
    if (!afterRows()) throw new Error("rows incomplete: " + rowStates());
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
    if (!done) throw new Error("install failed");
    await selftestEdit(done);
    console.info("selftest PASS " + done.key);
  } catch (e) {
    console.error("selftest FAIL", String(e));
  } finally {
    await invoke("ai_delete_profile", { id: saved.id }).catch(() => {});
    if (persist) await invoke("hatch_clear").catch(() => {});
  }
}

const rowStates = () => JSON.stringify(Object.fromEntries(Object.entries(st.rows).map(([k, v]) => [k, v.status + (v.atlas ? "(atlas)" : "") + (v.error ? ":" + v.error : "")])));

// Self-test of continuing from an installed pet: once with its hatch
// material (the atlas must come back byte for byte; one row regenerated,
// the pet replaced), once as a pet hatched before the material was kept
// (rows cut from its atlas; the idle row and a look row, whose anchors come
// from the atlas, regenerated; saved as a new pet).
async function selftestEdit(done) {
  const installed = st.atlasB64, installedHd = st.hdB64;
  const fresh = () => { st.rows = {}; st.baseB64 = null; st.atlasB64 = null; st.hdB64 = null; };
  const regenerate = async (ids) => {
    for (const id of ids) { notes.set(id, "make the motion bigger"); await genRow(S.find((s) => s.id === id)); }
    if (!afterRows()) throw new Error("rows incomplete after regenerating: " + rowStates());
    await buildFinal();
  };

  await loadHatched();
  fresh();
  await editPet(done.key);
  await buildFinal();
  if (st.atlasB64 !== installed) throw new Error("editing the installed pet gave a different atlas");
  await regenerate(["waving"]);
  if (!replacing()) throw new Error("replacing is not the default");
  const replaced = await install();
  if (!replaced?.replaced || replaced.key !== done.key) throw new Error("replacing the pet failed");
  console.info("selftest edit OK", done.key);

  const oldId = done.key.split(":")[1] + "-old";
  const old = await invoke("install_generated", {
    id: oldId, displayName: "自测旧宠物", description: "AI hatched pet", spritesheetPng: installed, hdPng: installedHd,
    coquouJson: JSON.stringify({ format: "coquou/1", hd: "spritesheet@2x.png", tags: ["ai-hatched"], generator: { tool: "coquou-hatch" } }),
  });
  let copy = null;
  try {
    await loadHatched();
    fresh();
    await editPet(old.key);
    if (!st.source?.legacy || !afterRows()) throw new Error("the old pet did not load from its atlas: " + rowStates());
    await regenerate(["idle", "look-a"]);
    if (!st.rows["look-cardinals"].atlas) throw new Error("look anchors should still come from the atlas");
    document.querySelector('input[name="installMode"][value="new"]').checked = true;
    await applyInstallMode();
    copy = await install();
    if (!copy || copy.replaced || copy.key !== `coquou:${oldId}-2`) throw new Error("saving as a new pet failed: " + JSON.stringify(copy));
    const ext = await petJson(copy.key, "coquou.json");
    const kept = Object.keys(ext.hatch?.rows || {}).sort().join(",");
    if (kept !== "idle,look-a") throw new Error("saved hatch rows: " + kept);
    console.info("selftest edit old OK", old.key);
  } finally {
    for (const k of [old.key, copy?.key]) if (k) await invoke("delete_pet", { key: k }).catch((e) => console.warn("cleanup", k, String(e)));
  }
}
})();
