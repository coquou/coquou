// Pet overlay: state machine, physics, per-pixel click-through, speech.
//
// The native side feeds the global cursor (`cursor` events) because a
// click-through window gets no mouse events; this page hit-tests the pet's
// sprite buffer and turns click-through off only over opaque pet pixels.
// Preferences live in Rust (prefs.rs): read with `prefs_get`, changes arrive
// as `prefs` events, the pet choice is saved with `prefs_set`.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const { defaultLines, platform, t } = window.Coquou;
window.Coquou.mirrorConsole("overlay");

const stage = document.getElementById("stage");
const ctx = stage.getContext("2d");

// The pet is drawn into a small sprite buffer first; hit tests read alpha
// from this buffer, so only opaque pet pixels catch the mouse.
// Nothing is drawn until the first pet has loaded (renderers: sprite.js).
const NO_PET = { name: "", S: 1, FOOT: 0, width: 0, height: 0, ownLines: () => null, has: () => false, draw() {} };
let renderer = NO_PET;
const buf = document.createElement("canvas");
const bctx = buf.getContext("2d", { willReadFrequently: true });

const GRAVITY = 2600;    // px/s²
const WALK_SPEED = 70;   // px/s
const ACTIVITY = {
  quiet:  { walk: 0.3, pause: [4, 9] },
  normal: { walk: 0.6, pause: [2, 6] },
  lively: { walk: 0.85, pause: [1, 3] },
};

// Saved preferences (prefs.rs); defaults until `prefs_get` answers.
let prefs = { zone: "corners", activity: "normal", sleepAfter: 1800, chatter: false, size: 1, firstRunDone: true };
const pause = () => { const [a, b] = ACTIVITY[prefs.activity].pause; return a + Math.random() * (b - a); };

const pet = {
  x: 300, y: -120, vx: 0, vy: 0,
  facing: 1, face: 1, state: "air", stateT: 0, nextDecision: 2,
  happyPending: false, happyUntil: 0, loved: 0, lookAngle: null, swing: 0, lastTouch: 0,
  task: null, taskSince: 0, taskEnd: 0, // see "activity" below
  reaction: null, reactionPending: null, reactionUntil: 0,
  target: null, // x the pet walks back to when it is outside its zone
};

let W = 0, H = 0, dpr = 1;
function resize() {
  dpr = window.devicePixelRatio || 1;
  W = window.innerWidth; H = window.innerHeight;
  stage.width = Math.round(W * dpr); stage.height = Math.round(H * dpr);
  sizeBuffer();
  pet.x = Math.min(Math.max(pet.x, 60), W - 60);
  if (pet.state !== "held") pet.y = Math.min(pet.y, H);
}
function sizeBuffer() {
  buf.width = Math.round(renderer.S * dpr); buf.height = Math.round(renderer.S * dpr);
}

// facing: the walk / flight direction. face: the way poses without a
// direction of their own look (sprite.js mirrors them). Settling down, the
// pet faces the middle of the screen, so in a corner it looks inward; near
// the middle it keeps the way it was going.
function setState(s) {
  pet.state = s; pet.stateT = 0;
  if (s !== "walk" && s !== "air") {
    const d = W / 2 - pet.x;
    pet.face = Math.abs(d) > W * 0.1 ? Math.sign(d) : pet.facing;
  }
}
function touch(t) { pet.lastTouch = t; }
// Stop strolling or sleeping to pay attention (a walk back to the zone
// resumes from idle on its own).
function settle() { if (pet.state === "walk" || pet.state === "sleep") setState("idle"); }

// ---------- particles (hearts, Zzz, !) ----------
const particles = [];
function emit(text, color, size, x, y, vx, vy, life) {
  particles.push({ text, color, size, x, y, vx, vy, life, age: 0 });
}
function hearts(n) {
  for (let i = 0; i < n; i++)
    emit("♥", "#ff6b8b", 16 + Math.random() * 8, pet.x + (Math.random() - 0.5) * 50,
      pet.y - renderer.height - 10, (Math.random() - 0.5) * 60, -60 - Math.random() * 50, 1.2);
}

// ---------- input: native cursor feed + pointer events ----------
let cursor = null;      // latest cursor, CSS px relative to the window
let passthrough = true; // mirrors the native ignore-cursor-events flag
let drag = null;        // { offX, offY, samples: [[x, y, t]], moved }
let clock = 0, lastCursorMove = -10;

listen("cursor", ({ payload }) => {
  if (drag) return; // pointer events are exact while dragging
  if (payload && (!cursor || Math.hypot(payload[0] - cursor[0], payload[1] - cursor[1]) > 2)) lastCursorMove = clock;
  cursor = payload;
});

function bufferPoint(x, y) {
  const { S, FOOT } = renderer;
  return [x - (pet.x - S / 2), y - (pet.y - (S - FOOT))];
}
function hitTest(x, y) {
  const [bx, by] = bufferPoint(x, y);
  if (bx < 0 || by < 0 || bx >= renderer.S || by >= renderer.S) return false;
  return bctx.getImageData(Math.floor(bx * dpr), Math.floor(by * dpr), 1, 1).data[3] > 24;
}
function setPassthrough(on) {
  if (on === passthrough) return;
  passthrough = on;
  invoke("set_passthrough", { enabled: on }).catch((e) => console.warn("passthrough", e));
  stage.style.cursor = on ? "default" : "grab";
}

stage.addEventListener("pointerdown", (e) => {
  // Only a plain primary press drags; right / Ctrl-click opens the menu below.
  if (e.button !== 0 || e.ctrlKey || !hitTest(e.clientX, e.clientY)) return;
  stage.setPointerCapture(e.pointerId);
  drag = { offX: e.clientX - pet.x, offY: e.clientY - pet.y, samples: [[e.clientX, e.clientY, performance.now() / 1000]], moved: 0 };
  stage.style.cursor = "grabbing";
});
stage.addEventListener("pointermove", (e) => {
  cursor = [e.clientX, e.clientY];
  if (!drag) return;
  const now = performance.now() / 1000;
  const last = drag.samples[drag.samples.length - 1];
  drag.moved += Math.hypot(e.clientX - last[0], e.clientY - last[1]);
  drag.samples.push([e.clientX, e.clientY, now]);
  while (drag.samples.length > 2 && now - drag.samples[0][2] > 0.08) drag.samples.shift();
  if (drag.moved > 6 && pet.state !== "held") { pet.target = null; setState("held"); touch(clock); chat("held"); }
  if (pet.state === "held") {
    const nx = e.clientX - drag.offX;
    pet.swing += (nx - pet.x) * 0.02;
    pet.x = nx; pet.y = e.clientY - drag.offY;
  }
});
stage.addEventListener("pointerup", (e) => {
  if (!drag) return;
  const d = drag; drag = null;
  stage.style.cursor = "grab";
  touch(clock);
  if (pet.state !== "held") { onClick(); return; }
  // Released outside this display's overlay → carry the pet to that display.
  if (e.clientX < 0 || e.clientX > W || e.clientY < 0 || e.clientY > H) {
    hopScreens().then((moved) => {
      if (!moved) { pet.x = Math.min(Math.max(pet.x, 60), W - 60); pet.vx = 0; pet.vy = 0; setState("air"); }
    });
    return;
  }
  // Throw with the velocity of the last ~80 ms of movement.
  const a = d.samples[0], b = d.samples[d.samples.length - 1];
  const dt = Math.max(0.016, b[2] - a[2]);
  const clamp = (v) => Math.max(-3000, Math.min(3000, v));
  pet.vx = clamp((b[0] - a[0]) / dt); pet.vy = clamp((b[1] - a[1]) / dt);
  setState("air");
});

// Right-click on the pet: the same menu as the tray (tray.rs).
stage.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  if (drag || !hitTest(e.clientX, e.clientY)) return;
  invoke("show_pet_menu").catch((err) => console.warn("pet menu", err));
});

async function hopScreens() {
  const resized = new Promise((r) => window.addEventListener("resize", r, { once: true }));
  if (!(await invoke("follow_cursor_monitor"))) return false;
  await Promise.race([resized, new Promise((r) => setTimeout(r, 600))]);
  const c = await invoke("cursor_local");
  if (c) { pet.x = c[0]; pet.y = Math.min(c[1], H); }
  pet.vx = 0; pet.vy = 0; setState("air");
  return true;
}

function onClick() {
  if (pet.state === "sleep") {
    emit("!", "#ff9f1c", 28, pet.x + 20, pet.y - renderer.height - 20, 0, -40, 0.8);
    chat("wake", true);
  } else {
    pet.happyPending = true; // shown once TA has landed (think)
    hearts(3);
    chat("click");
  }
  pet.vx = 0;
  // On the ground the jump starts with the row's own crouch and take-off
  // (jumping frames 0–1, state "hop"); in mid-air it is a plain boost.
  if (["idle", "walk", "sleep", "land"].includes(pet.state)) setState("hop");
  else if (pet.state !== "hop") { pet.vy = -620; setState("air"); }
}

// ---------- power: hidden while a fullscreen app runs, 30 fps on battery ----------
let power = { paused: false, battery: false };
let loopRunning = true, lastFrame = performance.now();
function applyPower(next) {
  power = next;
  console.info("power:", JSON.stringify(next));
  if (!power.paused && !loopRunning) {
    loopRunning = true;
    lastFrame = performance.now();
    requestAnimationFrame(frame);
  }
}
listen("power", ({ payload }) => applyPower(payload));
invoke("power_state").then(applyPower).catch(() => {});

// ---------- speech bubble ----------
// System messages queue up so several results (a multi-file drop) are all
// read; chatter (the pet's lines) only speaks when nothing else is showing.
let bubble = null;
const bubbleQueue = [];
function say(text, seconds) { bubbleQueue.push({ text, seconds }); }

const CHAT_GAP = 6; // s between two lines
let lastChat = -100, nextIdleChat = 120;
function chat(kind, force = false) {
  if (!prefs.chatter) return;
  if (bubble || bubbleQueue.length) return;
  if (!force && clock - lastChat < CHAT_GAP) return;
  // The user's lines (prefs `custom`), else the pet's own (coquou.json), else the defaults.
  const pool = prefs.custom?.[petKey]?.lines?.[kind] ?? renderer.ownLines(kind) ?? defaultLines(kind);
  if (!pool?.length) return;
  lastChat = clock;
  bubble = { text: pool[Math.floor(Math.random() * pool.length)], until: clock + 3.2 };
}

// Greedy line breaking: between any two CJK characters, and at spaces
// elsewhere (a word wider than a whole line is split anywhere).
const CJK = "\u2e80-\u9fff\uf900-\ufaff\uff00-\uffef";
const TOKENS = new RegExp(`[${CJK}][，。！？、：；）」』～…]*|[^\\s${CJK}]+\\s*|\\s+`, "g");
function wrap(text, maxW) {
  const lines = [];
  let line = "";
  const fits = (s) => ctx.measureText(s.trimEnd()).width <= maxW;
  for (const token of text.match(TOKENS) || []) {
    if (fits(line + token)) { line += token; continue; }
    if (line.trim()) lines.push(line.trimEnd());
    line = "";
    for (const ch of token.trimStart()) {
      if (!fits(line + ch)) { lines.push(line); line = ""; }
      line += ch;
    }
  }
  lines.push(line.trimEnd());
  return lines;
}

function drawBubble() {
  if (bubble && bubble.until < clock) bubble = null;
  if (!bubble && bubbleQueue.length) {
    const next = bubbleQueue.shift();
    bubble = { text: next.text, until: clock + next.seconds };
  }
  if (!bubble) return;
  const maxW = 240, pad = 9, lineH = 18;
  ctx.font = `13px -apple-system, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif`;
  const lines = wrap(bubble.text, maxW);
  const w = Math.max(...lines.map((l) => ctx.measureText(l).width)) + pad * 2;
  const h = lines.length * lineH + pad * 2 - 4;
  const x = Math.min(Math.max(pet.x - w / 2, 6), W - w - 6);
  const y = Math.max(6, pet.y - renderer.height - h - 18);
  // Tail pointing at the pet's head.
  const tx = Math.min(Math.max(pet.x, x + 14), x + w - 14);
  ctx.globalAlpha = Math.min(1, (bubble.until - clock) * 3);
  ctx.fillStyle = "rgba(255,255,255,0.96)";
  ctx.strokeStyle = "rgba(0,0,0,0.12)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, 10);
  ctx.moveTo(tx - 7, y + h - 0.5); ctx.lineTo(tx, y + h + 8); ctx.lineTo(tx + 7, y + h - 0.5);
  ctx.fill(); ctx.stroke();
  ctx.fillStyle = "rgba(255,255,255,0.96)";
  ctx.fillRect(tx - 6, y + h - 2, 12, 2);
  ctx.fillStyle = "#333";
  lines.forEach((l, i) => ctx.fillText(l, x + pad, y + pad + 10 + i * lineH));
  ctx.globalAlpha = 1;
}

// Reactions every pet shares, applied around its renderer (feet at the
// origin): swinging while held or thrown, pivoting at mid-body, and a squash
// on landing unless the pet brings its own landing animation.
function bodyMotion(g) {
  if (pet.state === "held" || pet.state === "air") {
    const mid = -renderer.height / 2;
    g.translate(0, mid); g.rotate(Math.max(-0.4, Math.min(0.4, pet.swing))); g.translate(0, -mid);
  }
  if (pet.state === "land" && !renderer.has("land")) {
    const k = Math.max(0, 1 - pet.stateT / 0.28);
    g.scale(1 + 0.12 * k, 1 - 0.15 * k);
  }
}

// ---------- pet commands (tray, app window) ----------
listen("pet-command", ({ payload }) => {
  touch(clock);
  if (payload === "reset") {
    pet.x = nearestHome(W); pet.y = -100; pet.vx = 0; pet.vy = 0; setState("air");
  } else if (payload === "sleep") {
    pet.vx = 0; setState("sleep");
  } else if (payload.startsWith("pet:")) {
    usePet(payload.slice(4));
  }
});

// ---------- size (one setting for every pet) ----------
let baseRenderer = renderer, petKey = "", sizeMul = 1;
function scaled(r, mul) {
  if (mul === 1) return r;
  return {
    ...r,
    S: Math.ceil(r.S * mul), FOOT: r.FOOT * mul, width: r.width * mul, height: r.height * mul,
    draw: (g, p, t) => { g.save(); g.scale(mul, mul); r.draw(g, p, t); g.restore(); },
  };
}
function applySize(mul) {
  sizeMul = mul;
  renderer = scaled(baseRenderer, mul);
  sizeBuffer();
}

// ---------- preferences ----------
function applyPrefs(next) {
  const old = prefs;
  prefs = next;
  if (next.activity !== old.activity) pet.nextDecision = Math.min(pet.nextDecision, pause());
  if (next.sleepAfter === 0 && pet.state === "sleep") setState("idle");
  if (next.zone !== old.zone && !zoneOf(pet.x) && ["idle", "walk", "sleep"].includes(pet.state)) {
    pet.target = nearestHome(pet.x); setState("walk");
  }
  const mul = next.size ?? 1;
  if (mul !== sizeMul) {
    applySize(mul);
    pet.y = Math.min(pet.y, H);
    if (pet.state !== "held") { pet.vy = -200; setState("air"); }
  }
}
listen("prefs", ({ payload }) => applyPrefs(payload));

// ---------- installing pets (drop a .pet onto the pet / double-click) ----------
let dropHover = false;
listen("drop-hover", ({ payload }) => {
  dropHover = payload;
  if (payload) { touch(clock); settle(); }
});
listen("pet-installed", async ({ payload }) => {
  touch(clock);
  if (await usePet(payload.key)) {
    hearts(5);
    say(payload.replaced ? t(`更新好了：${payload.displayName}`, `Updated: ${payload.displayName}`) : t(`安装好了：${payload.displayName}`, `Installed: ${payload.displayName}`), 3);
    // Tell the pet's author about animations nothing will ever play.
    const unused = payload.unusedStates ?? [];
    if (unused.length) say(t(`这些动作 Coquou 用不上：${unused.join("、")}`, `Coquou never plays these animations: ${unused.join(", ")}`), 5);
  }
});
function installFailed({ file, error }) {
  say(t(`装不上${file ? " " + file : ""}：`, `Cannot install${file ? " " + file : ""}: `) + error, 5);
}
listen("pet-install-failed", ({ payload }) => installFailed(payload));
listen("update-available", ({ payload }) => {
  react("waiting");
  say(t(`Coquou 有新版本 ${payload.version} 啦，点托盘图标就能更新`, `Coquou ${payload.version} is out! Update from the tray icon`), 6);
});
// A pet was deleted in the app window: if it was ours, pick another.
listen("pets-changed", async ({ payload }) => {
  if (payload?.removed !== petKey) return;
  await useFallback(await invoke("list_pets").catch(() => []), payload.removed);
});

// ---------- activity (activity.rs): what Coquou itself is busy with ----------
// While the AI is generating, the pet works (`running`); while a pet package
// is checked and installed, TA looks it over (`review`). Each event carries
// the whole state, so one missed before this page listened is corrected by
// the next. A task stays on screen at least TASK_MIN, so a quick install
// shows a full loop instead of a flicker.
//
// Reactions are one-offs: sad when something failed (`failed`), asking for
// the user when something waits for them (`waiting`: a hatch step is ready
// to look at, an update is out). They are timed from when TA stands idle
// again, so one that arrives while the pet is held or falling (as at
// start-up) is still seen, and the pet stays put meanwhile.
const TASK_MIN = 1.5; // s
const REACTIONS = { failed: 2.4, waiting: 3 }; // s on screen
listen("pet-activity", ({ payload }) => {
  const want = payload.running ? "running" : payload.review ? "review" : null;
  if (want) {
    if (pet.task !== want) { pet.task = want; pet.taskSince = clock; chat("working"); }
    pet.taskEnd = Infinity;
    touch(clock); settle();
  } else if (pet.task) {
    pet.taskEnd = Math.max(clock, pet.taskSince + TASK_MIN);
  }
});
listen("pet-react", ({ payload }) => react(payload));
function react(name) {
  if (!Object.hasOwn(REACTIONS, name)) return console.warn("unknown reaction", name);
  pet.reactionPending = name; touch(clock); settle();
}

// ---------- pet switching ----------
// Loading takes a moment; only the most recent request may take effect
// (switching A then B quickly must end on B).
let loadSeq = 0;
// `remember: false` for automatic fallbacks, so a missing pet doesn't
// overwrite the user's own choice.
async function usePet(key, remember = true) {
  const seq = ++loadSeq;
  let next;
  try {
    const entry = (await invoke("list_pets")).find((p) => p.key === key);
    if (!entry) throw new Error(`pet ${key} not found`);
    next = await window.loadSpritePet(entry);
  } catch (e) {
    if (seq !== loadSeq) return false;
    console.error("failed to load pet", key, e);
    emit("?", "#ff6b6b", 28, pet.x, pet.y - renderer.height - 20, 0, -40, 1.2);
    react("failed");
    return false;
  }
  if (seq !== loadSeq) return false; // superseded by a newer switch
  baseRenderer = next;
  petKey = key;
  applySize(prefs.size ?? 1);
  invoke("report_current_pet", { key }).catch((e) => console.warn("report pet", e));
  if (remember && prefs.pet !== key) invoke("prefs_set", { patch: { pet: key } }).catch((e) => console.warn("save pet", e));
  console.info("pet:", key, renderer.name, `S=${renderer.S} h=${Math.round(renderer.height)}`);
  // Drop the new pet in from above.
  pet.y = -40; pet.vx = 0; pet.vy = 0; setState("air");
  setTimeout(() => chat("hello"), 900);
  return true;
}

// ---------- activity zone ----------
// Keep the pet out of the middle of the screen, where input boxes and
// content usually are. Outside its zone (after being dropped or thrown
// there) it walks back to the nearest one.
function zones() {
  const m = 50;                                         // keep off the very edge
  const cw = Math.min(Math.max(W * 0.16, 180), 360);   // corner zone width
  switch (prefs.zone) {
    case "full": return [[m, W - m]];
    case "left": return [[m, m + cw]];
    case "right": return [[W - m - cw, W - m]];
    default: return [[m, m + cw], [W - m - cw, W - m]];
  }
}
function zoneOf(x) { return zones().find(([a, b]) => x >= a && x <= b) || null; }
function nearestHome(x) {
  let best = x, bestD = Infinity;
  for (const [a, b] of zones()) {
    const tx = Math.min(Math.max(x, a + 30), b - 30);
    if (Math.abs(tx - x) < bestD) { bestD = Math.abs(tx - x); best = tx; }
  }
  return best;
}

// ---------- behaviour ----------
function think(t, dt) {
  pet.stateT += dt;
  // The click jump takes longer than the joy lasts: start it on landing.
  if (pet.happyPending && pet.state === "idle") { pet.happyPending = false; pet.happyUntil = t + 1.2; }
  if (pet.reactionPending && pet.state === "idle") {
    pet.reaction = pet.reactionPending; pet.reactionPending = null;
    pet.reactionUntil = t + REACTIONS[pet.reaction];
    chat(pet.reaction);
  }
  // Done: linger a moment before strolling off, so the short gaps between
  // one AI request and the next don't send the pet walking back and forth.
  if (pet.task && t >= pet.taskEnd) {
    pet.task = null;
    if (pet.state === "idle") { pet.stateT = 0; pet.nextDecision = pause(); }
  }

  // Pets with look rows (Codex v2) turn toward a recently moved, nearby
  // cursor. 0° = up, clockwise; a deadzone around the head falls back to idle.
  if (cursor) {
    const hy = pet.y - renderer.height * 0.6;
    const ldx = cursor[0] - pet.x, ldy = cursor[1] - hy, ld = Math.hypot(ldx, ldy);
    const recent = t - lastCursorMove < 2.5;
    pet.lookAngle = recent && ld > renderer.width * 0.6 && ld < 500
      ? (Math.atan2(ldx, -ldy) * 180 / Math.PI + 360) % 360 : null;
  } else {
    pet.lookAngle = null;
  }

  switch (pet.state) {
    case "idle":
      if (!zoneOf(pet.x)) { pet.target = nearestHome(pet.x); setState("walk"); break; }
      if (pet.task || pet.reactionPending || pet.reactionUntil > t) break; // stays put while busy or reacting
      if (prefs.sleepAfter > 0 && t - pet.lastTouch > prefs.sleepAfter && Math.random() < dt * 0.2) { setState("sleep"); break; }
      if (t > nextIdleChat) { nextIdleChat = t + 120 + Math.random() * 180; chat("idle"); }
      if (pet.stateT > pet.nextDecision) {
        pet.nextDecision = pause();
        if (Math.random() < ACTIVITY[prefs.activity].walk) {
          pet.facing = Math.random() < 0.5 ? -1 : 1;
          setState("walk");
        } else setState("idle");
      }
      break;
    case "walk": {
      if (pet.target != null) { // heading back to its zone
        const dx = pet.target - pet.x, step = WALK_SPEED * 1.6 * dt;
        if (dx) pet.facing = Math.sign(dx);
        if (Math.abs(dx) <= step) { pet.x = pet.target; pet.target = null; pet.nextDecision = 1 + Math.random() * 2; setState("idle"); }
        else pet.x += Math.sign(dx) * step;
        break;
      }
      const [a, b] = zoneOf(pet.x) || [50, W - 50];
      pet.x += pet.facing * WALK_SPEED * dt;
      if (pet.x < a || pet.x > b) { pet.facing *= -1; pet.x = Math.min(Math.max(pet.x, a), b); }
      if (pet.stateT > pet.nextDecision) { pet.nextDecision = pause(); setState("idle"); }
      break;
    }
    case "sleep":
      if (Math.random() < dt * 0.8) emit("z", "#7f8fa6", 14 + Math.random() * 8, pet.x + renderer.width * 0.4, pet.y - renderer.height * 0.8, 15, -30, 2);
      break;
    case "held":
      pet.swing *= Math.pow(0.02, dt);
      break;
    case "air":
      pet.vy += GRAVITY * dt;
      pet.x += pet.vx * dt; pet.y += pet.vy * dt;
      pet.swing = Math.max(-0.4, Math.min(0.4, pet.vx / 3000));
      if (Math.abs(pet.vx) > 30) pet.facing = pet.face = Math.sign(pet.vx);
      if (pet.x < 50) { pet.x = 50; pet.vx = -pet.vx * 0.5; }
      if (pet.x > W - 50) { pet.x = W - 50; pet.vx = -pet.vx * 0.5; }
      if (pet.y < renderer.height + 10) { pet.y = renderer.height + 10; pet.vy = Math.abs(pet.vy) * 0.3; }
      if (pet.y >= H) {
        pet.y = H;
        if (pet.vy > 500) { pet.vy = -pet.vy * 0.35; pet.vx *= 0.7; }
        else { pet.vx = 0; pet.vy = 0; pet.swing = 0; setState("land"); }
      }
      break;
    case "hop":
      if (pet.stateT >= renderer.frameTime("jumping", 2)) { pet.vy = -620; setState("air"); }
      break;
    case "land":
      if (pet.stateT > 0.35) { pet.nextDecision = 1 + Math.random() * 2; setState("idle"); }
      break;
  }
}

// Rubbing: moving the cursor back and forth over the pet makes it happy.
let rub = 0, lastRubPos = null;
function petting(t, dt, hovering) {
  rub = Math.max(0, rub - 250 * dt);
  if (hovering && !drag && cursor && lastRubPos && pet.state !== "air") {
    rub += Math.hypot(cursor[0] - lastRubPos[0], cursor[1] - lastRubPos[1]);
    if (rub > 700) {
      rub = 0; touch(t);
      settle();
      pet.loved = t + 2.5; hearts(4);
      chat("pet");
    }
  }
  lastRubPos = cursor ? [...cursor] : null;
}

// ---------- render loop ----------
function frame(now) {
  if (power.paused) { loopRunning = false; return; } // resumed by applyPower
  if (power.battery && now - lastFrame < 1000 / 30 - 2) { requestAnimationFrame(frame); return; }
  const dt = Math.min(0.05, (now - lastFrame) / 1000);
  lastFrame = now; clock += dt;

  think(clock, dt);

  // Draw the pet into the sprite buffer (also the hit-test mask).
  bctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const { S, FOOT } = renderer;
  bctx.clearRect(0, 0, S, S);
  bctx.save(); bctx.translate(S / 2, S - FOOT); bodyMotion(bctx); renderer.draw(bctx, pet, clock); bctx.restore();

  const hovering = cursor ? hitTest(cursor[0], cursor[1]) : false;
  pet.dropHover = dropHover;
  setPassthrough(!(hovering || drag || dropHover));
  petting(clock, dt, hovering);

  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, W, H);
  // Soft shadow on the "floor".
  if (pet.state !== "held") {
    const lift = Math.max(0, H - pet.y);
    ctx.fillStyle = `rgba(0,0,0,${0.18 * Math.max(0.2, 1 - lift / 400)})`;
    ctx.beginPath(); ctx.ellipse(pet.x, H - 3, renderer.width * 0.45 * Math.max(0.4, 1 - lift / 600), 5, 0, 0, Math.PI * 2); ctx.fill();
  }
  ctx.drawImage(buf, pet.x - S / 2, pet.y - (S - FOOT), S, S);

  for (let i = particles.length - 1; i >= 0; i--) {
    const p = particles[i];
    p.age += dt; p.x += p.vx * dt; p.y += p.vy * dt;
    if (p.age > p.life) { particles.splice(i, 1); continue; }
    ctx.globalAlpha = 1 - p.age / p.life;
    ctx.fillStyle = p.color;
    ctx.font = `bold ${p.size}px -apple-system, "Segoe UI", sans-serif`;
    ctx.fillText(p.text, p.x, p.y);
  }
  ctx.globalAlpha = 1;
  drawBubble();

  requestAnimationFrame(frame);
}

window.addEventListener("resize", resize);
resize();
pet.x = nearestHome(W);
requestAnimationFrame(frame);

// ---------- start-up ----------
// Default order: a pet the user already has (e.g. from Codex), then the
// preset; hidden pets only when nothing else is left.
function defaultPets(pets, except) {
  const usable = pets.filter((p) => p.key !== except && !p.hidden);
  const own = usable.filter((p) => p.source !== "preset");
  const presets = usable.filter((p) => p.source === "preset");
  const hidden = pets.filter((p) => p.key !== except && p.hidden);
  return [...own, ...presets, ...hidden].map((p) => p.key);
}
// Try the defaults without overwriting the user's remembered choice. The
// preset ships inside the app, so ending up with nothing means a damaged
// install.
async function useFallback(pets, except) {
  for (const key of defaultPets(pets, except)) if (await usePet(key, false)) return;
  console.error("no pet could be loaded");
  say(t("一只宠物都加载不出来，安装可能损坏了，请重新安装 Coquou", "No pet could be loaded. The install may be damaged; please reinstall Coquou"), 30);
}

// Interface language (i18n.rs), for speech: system messages and the default lines.
const loadLang = () => invoke("get_settings").then((s) => window.Coquou.setLang(s.lang)).catch((e) => console.warn("language", e));
listen("settings-changed", loadLang);

// Pet: one installed or named with --pet before the page was listening,
// else the remembered one, else the defaults.
(async () => {
  await loadLang();
  // The feed only reports movement; start from where the cursor is now.
  invoke("cursor_local").then((c) => { if (!cursor) cursor = c; }).catch(() => {});
  applyPrefs(await invoke("prefs_get"));
  const pets = await invoke("list_pets").catch(() => []);
  console.info("pets found:", pets.map((p) => p.key).join(", ") || "(none)");
  // Installs from files given at launch may have finished (or failed)
  // before this page was listening; library.rs kept them for now.
  const early = await invoke("overlay_ready").catch((e) => { console.warn("overlay ready", e); return null; });
  const first = early?.pet ?? prefs.pet;
  if (!first || !(await usePet(first))) await useFallback(pets, first);
  // Their `pet-react` was sent before anyone listened, so react here.
  for (const f of early?.failed ?? []) { installFailed(f); react("failed"); }

  // First run: show the app window once and say where Coquou lives.
  if (!prefs.firstRunDone) {
    invoke("prefs_set", { patch: { firstRunDone: true } }).catch(() => {});
    invoke("open_window", { name: "app" }).catch(() => {});
    setTimeout(() => say(platform === "macos"
      ? t("你好呀！我住在屏幕右上角的菜单栏里，点那里的图标或者右键点我，可以叫我回来、哄我睡觉",
        "Hi there! I live in the menu bar at the top right. Click the icon there, or right-click me, to call me back or put me to sleep")
      : t("你好呀！我住在屏幕右下角的通知区域，单击图标打开主窗口，右键图标或者右键点我，可以叫我回来、哄我睡觉",
        "Hi there! I live in the notification area at the bottom right. Click the icon to open the main window; right-click it, or me, to call me back or put me to sleep"), 8), 1500);
  }
})();
