// Records a narrated walkthrough of the crowding planner as demo/out/crowding-demo.mp4.
// Needs the crowding service (with the web app) on DEMO_URL, default http://localhost:8090.
// Run: cd demo && npm install && npx playwright install chromium && npm run record
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, renameSync, rmSync } from "node:fs";
import ffmpeg from "ffmpeg-static";
import { chromium } from "playwright";

const APP = process.env.DEMO_URL ?? "http://localhost:8090/";
const W = 1440;
const H = 900;
const OUT = new URL("./out/", import.meta.url).pathname;
rmSync(OUT + "raw", { recursive: true, force: true });
mkdirSync(OUT + "raw", { recursive: true });

const browser = await chromium.launch({ args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
const ctx = await browser.newContext({
  viewport: { width: W, height: H },
  recordVideo: { dir: OUT + "raw", size: { width: W, height: H } },
});
const t0 = Date.now();
const page = await ctx.newPage();
const wait = (ms) => page.waitForTimeout(ms);

// ---------- overlay: cursor, captions, cards ----------
const OVERLAY = `
  #demo-cursor { position: fixed; z-index: 99999; width: 22px; height: 22px; margin: -11px 0 0 -11px;
    border-radius: 50%; background: rgba(31,78,140,.35); border: 2px solid #1f4e8c; pointer-events: none;
    transition: transform .12s; left: -50px; top: -50px; }
  #demo-cursor.down { transform: scale(.7); background: rgba(31,78,140,.6); }
  #demo-cap { position: fixed; z-index: 99998; left: 50%; bottom: 34px; transform: translateX(-50%);
    max-width: 980px; padding: 14px 22px; border-radius: 12px; background: rgba(17,24,39,.88); color: #fff;
    font: 600 21px/1.35 system-ui, -apple-system, sans-serif; text-align: center; opacity: 0;
    transition: opacity .35s; pointer-events: none; box-shadow: 0 8px 30px rgba(0,0,0,.25); }
  #demo-cap.on { opacity: 1; }
  #demo-cap small { display: block; font-weight: 400; font-size: 16px; opacity: .85; margin-top: 4px; }
  #demo-card { position: fixed; inset: 0; z-index: 99997; display: flex; flex-direction: column;
    justify-content: center; align-items: center; background: #0f1b2d; color: #fff; opacity: 0;
    transition: opacity .6s; pointer-events: none; font-family: system-ui, -apple-system, sans-serif; }
  #demo-card.on { opacity: 1; }
  #demo-card h1 { font-size: 54px; margin: 0 0 14px; letter-spacing: -.5px; }
  #demo-card p { font-size: 24px; margin: 0; opacity: .85; max-width: 1000px; text-align: center; line-height: 1.4; }
  #demo-card ul { font-size: 23px; line-height: 1.6; margin: 24px 0 0; max-width: 1050px; }
  #demo-card .tag { margin-top: 30px; font-size: 16px; opacity: .6; }
`;
async function installOverlay() {
  await page.evaluate((css) => {
    if (document.getElementById("demo-cursor")) return;
    const st = document.createElement("style");
    st.textContent = css;
    document.head.appendChild(st);
    for (const id of ["demo-cursor", "demo-cap", "demo-card"]) {
      const d = document.createElement("div");
      d.id = id;
      document.body.appendChild(d);
    }
    const c = document.getElementById("demo-cursor");
    addEventListener("mousemove", (e) => { c.style.left = e.clientX + "px"; c.style.top = e.clientY + "px"; }, true);
    addEventListener("mousedown", () => c.classList.add("down"), true);
    addEventListener("mouseup", () => c.classList.remove("down"), true);
  }, OVERLAY);
}
async function caption(html, ms = 0) {
  await page.evaluate((h) => {
    const c = document.getElementById("demo-cap");
    c.innerHTML = h;
    c.classList.toggle("on", !!h);
  }, html);
  if (ms) await wait(ms);
}
async function card(html, ms) {
  await page.evaluate((h) => {
    const c = document.getElementById("demo-card");
    c.innerHTML = h;
    c.classList.add("on");
  }, html);
  await wait(ms);
}
async function hideCard() {
  await page.evaluate(() => document.getElementById("demo-card").classList.remove("on"));
  await wait(700);
}
let mouse = { x: W / 2, y: H / 2 };
async function moveTo(x, y, steps = 25) {
  await page.mouse.move(x, y, { steps });
  mouse = { x, y };
}
async function clickOn(locator, pause = 250) {
  await locator.scrollIntoViewIfNeeded();
  const b = await locator.boundingBox();
  await moveTo(b.x + b.width / 2, b.y + b.height / 2);
  await wait(pause);
  await page.mouse.down();
  await wait(90);
  await page.mouse.up();
}
async function setRange(locator, value) {
  const b = await locator.boundingBox();
  const [min, max] = await locator.evaluate((el) => [+el.min, +el.max]);
  const x = b.x + 8 + ((value - min) / (max - min)) * (b.width - 16);
  await moveTo(x, b.y + b.height / 2, 18);
  await locator.evaluate((el, v) => {
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
    set.call(el, String(v));
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }, value);
}
async function choosePlace(label, query, expect) {
  const input = page.locator("label.place", { hasText: label }).locator("input");
  await clickOn(input);
  await input.fill("");
  await input.pressSequentially(query, { delay: 70 });
  const item = page.locator(".suggest li", { hasText: expect }).first();
  await item.waitFor({ timeout: 15000 });
  await wait(400);
  await clickOn(item, 150);
  await wait(300);
}
async function search() {
  await clickOn(page.getByRole("button", { name: "Find buses" }));
  await page.getByRole("button", { name: "Find buses" }).waitFor({ timeout: 90000 });
  await page.locator(".option").first().waitFor({ timeout: 90000 });
  await wait(1500);
}

// ---------- 0. title ----------
await page.goto(APP, { waitUntil: "networkidle" });
await installOverlay();
await moveTo(W / 2, H / 2, 1);
const tStart = (Date.now() - t0) / 1000;
await card(
  `<h1>Delhi bus crowding</h1>
   <p>From 2.56 million October e-tickets to a journey planner that shows how full each bus usually is</p>
   <div class="tag">Delhi Transport Stack hackathon · Problem 2</div>`,
  4200,
);
await hideCard();

// ---------- 1. the brief's example: fastest crowded, standing, leave later with seats ----------
await caption("A commuter plans a weekday trip at 08:30<small>Sangam Vihar → Nehru Place</small>", 1800);
await choosePlace("From", "Sangam Vihar", "Hamdard Nagar / Sangam Vihar");
await choosePlace("To", "Nehru Place", "Nehru Place");
const time = page.locator('input[type="time"]');
await clickOn(time);
await time.fill("08:30");
await setRange(page.locator('input[type="range"]'), 0.25);
await wait(400);
await search();
await caption(
  "Every option shows time <i>and</i> how full the bus usually is<small>Fastest is usually crowded · leave at 08:42 for a seat</small>",
  6500,
);
await clickOn(page.locator(".option").nth(0));
await caption("On the map, each stretch of the ride is coloured by its usual crowding", 4000);
const later = page.locator(".option", { hasText: "Leave later" }).first();
if (await later.count()) {
  await clickOn(later);
  await caption("Leaving 10 minutes later: seats likely the whole way", 3500);
}

// ---------- 2. the comfort slider changes the recommendation ----------
await caption("Another trip: Palam Vihar → Dwarka Sector 21, 08:30<small>First with “Get there fastest”</small>", 1500);
await choosePlace("From", "Palam Vihar More", "Palam Vihar More");
await choosePlace("To", "Dwarka Sec 21", "Dwarka Sec 21");
await clickOn(time);
await time.fill("08:30");
await setRange(page.locator('input[type="range"]'), 0);
await search();
await caption("Fastest arrives 09:11 but is usually crowded<small>“Feels like” prices the crowding in minutes</small>", 5000);
await caption("Now slide towards “Avoid crowds”…", 1200);
await setRange(page.locator('input[type="range"]'), 1);
await wait(500);
await search();
await caption("…and the planner recommends a bus with seats that arrives at the same time<small>Confidence dots say how many tickets are behind each estimate</small>", 6000);
await clickOn(page.locator(".option").first());
await wait(2500);

// ---------- 3. network map ----------
await caption("");
await clickOn(page.getByRole("button", { name: "Crowding map" }));
await wait(2500);
await caption("For planners: crowding on every stretch of the network, half hour by half hour", 2500);
const slider = page.locator('input[type="range"]');
const times = [];
for (let m = 5 * 60; m <= 22 * 60; m += 30) times.push(m);
const idx = (m) => (m - 4 * 60) / 30;
for (const m of times) {
  await setRange(slider, idx(m));
  if (m === 8 * 60 + 30) await caption("Morning peak: standing and crowded stretches build up into the centre");
  if (m === 13 * 60) await caption("Midday: most riders find a seat");
  if (m === 18 * 60 + 30) await caption("Evening peak: the reverse flow, a little lighter");
  await wait(m === 9 * 60 || m === 19 * 60 ? 2200 : 650);
}
await setRange(slider, idx(9 * 60));
await wait(1200);
await clickOn(page.getByText("Only show standing and crowded stretches"));
await caption("09:00: two in three riders stand or are in a crowded bus; here only those stretches", 4500);
const scen = page.locator("select").nth(1);
await clickOn(scen);
await scen.selectOption("high");
await caption("Ridership is an assumption, so every number comes as a range: here 5 M riders a day…", 3500);
await scen.selectOption("low");
await caption("…and here 3 M. The shares move, the hotspots stay", 3500);
await scen.selectOption("mid");
await wait(800);

// ---------- 4. operators ----------
await caption("");
await clickOn(page.getByRole("button", { name: "For operators" }));
await page.locator("table tbody tr").first().waitFor({ timeout: 30000 });
await wait(1200);
await caption("For operators: which routes need more buses, and which stretches are overloaded", 4500);
await clickOn(page.locator("table").first().locator("tbody tr").nth(3));
await wait(800);
await page.locator(".heat").waitFor({ timeout: 30000 });
await page.locator(".heat").evaluate((el) => el.scrollIntoView({ behavior: "smooth", block: "start" }));
await caption("One route, every stretch × every half hour<small>Pale cells rest on few tickets</small>", 5500);
await page.locator(".operator").evaluate((el) => el.scrollTo({ top: 0, behavior: "smooth" }));
await wait(900);
const win = page.locator(".toolbar select").nth(1);
await clickOn(win);
await win.selectOption("pm");
await caption("Switch to the evening peak", 3500);

// ---------- 5. closing ----------
await caption("");
await card(
  `<h1>Grounded in the tickets</h1>
   <ul>
     <li>Riders and buses both counted from the tickets: 2.0 M tickets matched to routes, 5,300 buses seen running each weekday</li>
     <li>9 AM: 37% of riders seated, 31% standing, 32% crowded; toward the centre the average bus is full</li>
     <li>Peaks at 09:00 and 19:00; toward the centre 1.4× heavier in the morning, away 1.5× in the evening</li>
     <li>Oct 1–24 predicts Oct 25–31: rank correlation 0.87, same crowding level on 76% of stretches</li>
     <li>Crowding inside OpenTripPlanner routing, GTFS-Realtime occupancy, live ticket adjustment</li>
   </ul>`,
  9000,
);

// ---------- encode ----------
const tEnd = (Date.now() - t0) / 1000;
await ctx.close();
await browser.close();
const raw = readdirSync(OUT + "raw").find((f) => f.endsWith(".webm"));
renameSync(OUT + "raw/" + raw, OUT + "raw/demo.webm");
const start = Math.max(0, tStart - 0.3).toFixed(2);
const dur = (tEnd - tStart + 0.3).toFixed(2);
execFileSync(ffmpeg, [
  "-y", "-loglevel", "error", "-ss", start, "-t", dur, "-i", OUT + "raw/demo.webm",
  "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20", "-preset", "medium", "-movflags", "+faststart",
  OUT + "crowding-demo.mp4",
]);
console.log(`wrote ${OUT}crowding-demo.mp4 (${dur}s)`);
