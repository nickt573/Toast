import { useState, useEffect, useRef, Fragment } from "react";
import { loggedInvoke, logError } from "../logger";
import { ResourceCard, ItemBar, GroupTypeBadge, ArchivedBadge, DeckStateBadge, ConfirmDelete, Linkify, Tip, NotebookPageTag } from "../UIUtils";
import { CategoryPicker, computeCategory, CATEGORIES, CATEGORY_COLOR_BY_LABEL } from "../Plans/PlanUtils";
import UnitPicker from "../UnitPicker";
import { resolveUnitPair } from "../unitPair";
import {
  Chart as ChartJS,
  CategoryScale,
  LinearScale,
  BarElement,
  LineElement,
  PointElement,
  Tooltip,
  Legend,
} from "chart.js";
import { Bar, Line } from "react-chartjs-2";
import "./Stats.css";

ChartJS.register(CategoryScale, LinearScale, BarElement, LineElement, PointElement, Tooltip, Legend);

// Constants

// Themed palette matching the app's feature families
const BLUE   = "#5A7A90";  // slate, new cards
const GREEN  = "#4A8C5E";  // forest, promoted / good retention
const RED    = "#B85454";  // terracotta, demoted / poor retention
const AMBER  = "#C49A44";  // amber, mid retention
const GRAY   = "#9A8488";  // warm grey, neutral

const YELLOW = "#E0A92E"; // yellow, todos

const BLUE_BG   = "rgba(90,122,144,0.78)";
const GREEN_BG  = "rgba(74,140,94,0.78)";
const RED_BG    = "rgba(184,84,84,0.78)";
const YELLOW_BG = "rgba(224,169,46,0.78)";

// Category colors are defined once in PlanUtils and shared with Todos
const CATEGORY_COLORS = CATEGORY_COLOR_BY_LABEL;

// Helpers

function fmtTime(minutes) {
  if (!minutes) return "0m";
  const h = Math.floor(minutes / 60);
  const m = Math.round(minutes % 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

function plural(n, word) {
  return `${n.toLocaleString()} ${word}${n === 1 ? "" : "s"}`;
}

// Formats a logged amount with its unit, empty unless both are present
function fmtUnit(value, name) {
  if (value === null || value === undefined || !name) return "";
  const n = Number.isInteger(value) ? value : Math.round(value * 100) / 100;
  return `${n.toLocaleString()} ${name}`;
}

function retentionColor(rate) {
  if (rate >= 0.8) return GREEN;
  if (rate >= 0.5) return AMBER;
  return RED;
}

function retentionPillClass(rate) {
  if (rate >= 0.8) return "st-meta-pill--ret-good";
  if (rate >= 0.5) return "st-meta-pill--ret-mid";
  return "st-meta-pill--ret-poor";
}

function daysBetween(from, to) {
  const ms = new Date(to + "T00:00:00Z") - new Date(from + "T00:00:00Z");
  return Math.round(ms / 86400000);
}

// Short month and day, for the ends of a session window
function fmtShortDay(dateStr) {
  const [y, m, d] = dateStr.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

function addDays(dateStr, n) {
  const d = new Date(dateStr + "T00:00:00Z");
  d.setUTCDate(d.getUTCDate() + n);
  return d.toISOString().slice(0, 10);
}

function parseCategories(catStr) {
  if (!catStr) return [];
  return catStr.split(",").map(s => s.trim()).filter(Boolean)
    // Other was renamed to Culture at bit 64, so alias the old stat rows
    .map(s => s === "Other" ? "Culture" : s);
}

function categoryStringToMap(catStr) {
  const names = parseCategories(catStr);
  const map = {};
  CATEGORIES.forEach(({ label, bit }) => { map[bit] = names.includes(label); });
  return map;
}

// An archived row was copied into a merged deck or set aside by a reset, so something
// else is the record now and counting it would inflate the plan
function counted(groupStats) {
  return groupStats.filter(r => !r.is_archived);
}

// How long the plan has run, from its first logged day through today counting that first
// day, and archived rows count since a merged or reset deck still dates the plan. A
// deleted plan stops there instead, ending on its last logged day, since nothing can be
// added to it any more
function totalPlanDays(groupStats, todoStats, today, deleted) {
  if (!today) return null;
  let earliest = null, latest = null;
  [...groupStats, ...todoStats].forEach(r => {
    if (earliest === null || r.date < earliest) earliest = r.date;
    if (latest === null || r.date > latest) latest = r.date;
  });
  if (earliest === null) return null;
  return daysBetween(earliest, deleted ? latest : today) + 1;
}

function computeMetrics(groupStats, todoStats) {
  const studyMins = groupStats.reduce((s, r) => s + r.time_spent_minutes, 0);
  const todoMins  = todoStats.reduce((s, r) => s + r.time_spent_minutes, 0);
  // New cards seen, versus every card touched including promotes and demotes
  const newCardsStudied = groupStats.reduce((s, r) => s + r.num_new, 0);
  const totalCardsStudied = groupStats.reduce((s, r) => s + r.num_new + r.num_promote + r.num_demote, 0);
  const todosDone = todoStats.length;

  let totalP = 0, totalD = 0;
  groupStats.forEach(r => { totalP += r.num_promote; totalD += r.num_demote; });
  const avgRetention = (totalP + totalD) > 0 ? totalP / (totalP + totalD) : null;

  // Average time per calendar day across the studied span, decks and todos together, and
  // the span ends on the last active day so an idle plan keeps the figure it had at its peak
  const studyDates = [...groupStats, ...todoStats].map(r => r.date).sort();
  const studySpan = studyDates.length
    ? daysBetween(studyDates[0], studyDates[studyDates.length - 1]) + 1
    : 0;
  const avgDailyStudy = studySpan > 0 ? (studyMins + todoMins) / studySpan : null;

  return { studyMins, todoMins, newCardsStudied, totalCardsStudied, todosDone, avgRetention, avgDailyStudy };
}

// Chart data builders

// Bucket key per unit, day is the date itself, week is that week's Monday, month is the
// year and month, and labels keep the year
function bucketKey(dateStr, unit) {
  if (unit === "week") {
    const dow = new Date(dateStr + "T00:00:00Z").getUTCDay();
    return addDays(dateStr, -((dow + 6) % 7));
  }
  if (unit === "month") return dateStr.slice(0, 7);
  return dateStr;
}

function nextBucket(key, unit) {
  if (unit === "month") {
    let [y, m] = key.split("-").map(Number);
    if (m === 12) { y += 1; m = 1; } else m += 1;
    return `${y}-${String(m).padStart(2, "0")}`;
  }
  return addDays(key, unit === "week" ? 7 : 1);
}

// Every bucket between the ends gets a label so an unstudied day doesn't vanish and leave
// the bars either side reading as consecutive, and the ends come from the shown window
function bucketRange(keys, unit, from = null, to = null) {
  const sorted = [...keys].sort();
  const start = from ?? sorted[0];
  const end   = to   ?? sorted[sorted.length - 1];
  if (!start || !end) return [];
  const out = [];
  for (let k = start; k <= end; k = nextBucket(k, unit)) out.push(k);
  return out;
}

// A window starts at its first bucket with something in it, so it doesn't open on blanks,
// but blanks after that stay, including a run at the end
function windowBuckets(byDate, unit, win) {
  const dates = bucketRange(
    Object.keys(byDate),
    unit,
    win?.start ? bucketKey(win.start, unit) : null,
    win?.end   ? bucketKey(win.end,   unit) : null,
  );
  const first = dates.findIndex(d => byDate[d]);
  return first === -1 ? [] : dates.slice(first);
}

function buildOverTimeData(groupStats, unit = "day", win = null) {
  const byDate = {};
  groupStats.forEach(r => {
    const key = bucketKey(r.date, unit);
    if (!byDate[key]) byDate[key] = { new: 0, promote: 0, demote: 0, p: 0, d: 0 };
    byDate[key].new     += r.num_new;
    byDate[key].promote += r.num_promote;
    byDate[key].demote  += r.num_demote;
    byDate[key].p       += r.num_promote;
    byDate[key].d       += r.num_demote;
  });

  const dates = windowBuckets(byDate, unit, win);
  const at = d => byDate[d] ?? { new: 0, promote: 0, demote: 0, p: 0, d: 0 };

  const barData = {
    labels: dates,
    datasets: [
      { label: "New",      data: dates.map(d => at(d).new),     backgroundColor: BLUE_BG,  stack: "s" },
      { label: "Promoted", data: dates.map(d => at(d).promote), backgroundColor: GREEN_BG, stack: "s" },
      { label: "Demoted",  data: dates.map(d => at(d).demote),  backgroundColor: RED_BG,   stack: "s" },
    ],
  };

  // Retention rides the same labels as the bars, and a bucket with nothing reviewed goes
  // in as null so the line runs straight over it with no dot to hover
  const rate = (p, d) => (p + d) > 0 ? Math.round((p / (p + d)) * 100) : null;
  const daily = dates.map(d => rate(at(d).p, at(d).d));
  // Every review up to and including that bucket, so the line shows the average settling
  // rather than how each day went on its own
  let runP = 0, runD = 0;
  const cumulative = dates.map(d => {
    const { p, d: dem } = at(d);
    runP += p;
    runD += dem;
    return (p + dem) > 0 ? rate(runP, runD) : null;
  });

  return { barData, dates, retention: { daily, cumulative }, hasRetention: daily.some(v => v !== null) };
}

const retentionLine = (dates, data) => ({
  labels: dates,
  datasets: [
    {
      label: "Retention %",
      data,
      borderColor: AMBER,
      // Spelled out because the flat runs at the edges are drawn by hand and have to match
      borderWidth: 3,
      tension: 0.3,
      fill: false,
      pointRadius: 3,
      spanGaps: true,
      // Without this the chart trims past its top edge and a 100% day loses half its dot
      clip: false,
    },
  ],
});

function buildTimeSpentData(groupStats, todoStats, unit = "day", win = null) {
  const byDate = {};
  const add = (r, kind) => {
    const key = bucketKey(r.date, unit);
    if (!byDate[key]) byDate[key] = { todo: 0, deck: 0 };
    byDate[key][kind] += r.time_spent_minutes;
  };
  todoStats.forEach(r => add(r, "todo"));
  groupStats.forEach(r => add(r, "deck"));

  const dates = windowBuckets(byDate, unit, win);
  const toHours = m => Math.round((m / 60) * 10) / 10;
  const at = d => byDate[d] ?? { todo: 0, deck: 0 };

  return {
    labels: dates,
    datasets: [
      { label: "Todos", data: dates.map(d => toHours(at(d).todo)), backgroundColor: YELLOW_BG, stack: "s" },
      { label: "Decks", data: dates.map(d => toHours(at(d).deck)), backgroundColor: BLUE_BG,   stack: "s" },
    ],
  };
}

function buildByDeckData(groupStats) {
  const byDeck = {};
  groupStats.forEach(r => {
    if (!byDeck[r.group_name]) byDeck[r.group_name] = { new: 0, promote: 0, demote: 0 };
    byDeck[r.group_name].new     += r.num_new;
    byDeck[r.group_name].promote += r.num_promote;
    byDeck[r.group_name].demote  += r.num_demote;
  });

  const decks = Object.keys(byDeck)
    .filter(d => byDeck[d].new + byDeck[d].promote + byDeck[d].demote > 0)
    .sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));

  return {
    labels: decks,
    datasets: [
      { label: "New",      data: decks.map(d => byDeck[d].new),     backgroundColor: BLUE_BG  },
      { label: "Promoted", data: decks.map(d => byDeck[d].promote), backgroundColor: GREEN_BG },
      { label: "Demoted",  data: decks.map(d => byDeck[d].demote),  backgroundColor: RED_BG   },
    ],
  };
}

function buildByCategoryData(todoStats) {
  const byCategory = {};
  todoStats.forEach(r => {
    const cats = parseCategories(r.category);
    cats.forEach(cat => {
      byCategory[cat] = (byCategory[cat] || 0) + r.time_spent_minutes;
    });
  });

  // Canonical category order first, any unrecognized legacy labels go last
  const order = CATEGORIES.map(c => c.label);
  const cats = [
    ...order.filter(c => byCategory[c] > 0),
    ...Object.keys(byCategory).filter(c => !order.includes(c) && byCategory[c] > 0),
  ];
  return {
    labels: cats,
    datasets: [
      {
        label: "Hours spent",
        data: cats.map(c => Math.round((byCategory[c] / 60) * 10) / 10),
        backgroundColor: cats.map(c => CATEGORY_COLORS[c] || GRAY),
      },
    ],
  };
}

// The distinct units in a plan's logged todos, labelled with the main spelling. Each carries
// the group's alternate spellings so a search on one surfaces the main
function unitOptionsFrom(todoStats, allUnits = []) {
  const altsByGroup = new Map(allUnits.map(u => [u.id, u.variants.map(v => v.name.toLowerCase())]));
  const seen = new Map();
  todoStats.forEach(r => {
    if (r.unit_group_id != null && r.unit_name && !seen.has(r.unit_group_id)) seen.set(r.unit_group_id, r.unit_name);
  });
  return [...seen.entries()]
    .map(([id, name]) => ({ id, name, alt: altsByGroup.get(id) ?? [name.toLowerCase()] }))
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}

// Shared chart options

// A day bucket is one date, a week or month bucket spans a range, given as its two ISO ends
function bucketSpan(key, unit) {
  if (unit === "week") return [key, addDays(key, 6)];
  if (unit === "month") {
    const [y, m] = key.split("-").map(Number);
    const last = new Date(y, m, 0).getDate();
    return [`${key}-01`, `${key}-${String(last).padStart(2, "0")}`];
  }
  return [key, null];
}

// The whole span on one line, for a tooltip title
function fmtBucketTip(key, unit) {
  const [start, end] = bucketSpan(key, unit);
  return end ? `${start} - ${end}` : start;
}

// Caps how many date labels render as history grows, the bars themselves are unaffected, and
// a range bucket stacks its two ends over a dash so the ISO dates aren't squeezed
function dateTicks(unit) {
  return {
    autoSkip: true, maxTicksLimit: 12, maxRotation: unit === "day" ? 30 : 0, font: { size: 10 },
    callback(value) {
      const [start, end] = bucketSpan(this.getLabelForValue(value), unit);
      return end ? [`${start} -`, end] : start;
    },
  };
}

// A tooltip title that names the bucket's full span, shared by every date chart
const dateTip = (unit) => ({ callbacks: { title: (items) => fmtBucketTip(items[0].label, unit) } });

// Wraps a label onto word-boundary lines, only truncates past the line limit
function wrapTickLabel(label, width = 14, maxLines = 2) {
  const lines = [];
  let line = "";
  for (let word of label.split(" ")) {
    while (word.length > width) {
      if (line) { lines.push(line); line = ""; }
      lines.push(word.slice(0, width));
      word = word.slice(width);
    }
    const next = line ? `${line} ${word}` : word;
    if (next.length > width) { lines.push(line); line = word; }
    else line = next;
  }
  if (line) lines.push(line);
  if (lines.length > maxLines) {
    lines.length = maxLines;
    lines[maxLines - 1] = lines[maxLines - 1].slice(0, width - 1) + "…";
  }
  return lines.length === 1 ? lines[0] : lines;
}

// Deck names are categorical, so never skip or rotate a label, wrap long names instead
const DECK_TICKS = {
  autoSkip: false,
  maxRotation: 0,
  font: { size: 10 },
  callback(value) {
    return wrapTickLabel(this.getLabelForValue(value));
  },
};

// The cards chart and the retention chart under it are read as one picture, so both fix
// the same y axis width, otherwise their labels size differently and the plots misalign
const PAIRED_AXIS_W = 46;
const pairedAxis = (scale) => { scale.width = PAIRED_AXIS_W; };

const barOpts = (stacked = false, yLabel = "", xTicks = null, pairedY = false, tipUnit = null) => ({
  responsive: true,
  maintainAspectRatio: false,
  plugins: { legend: { display: false }, ...(tipUnit ? { tooltip: dateTip(tipUnit) } : {}) },
  scales: {
    x: { stacked, grid: { display: false }, ...(xTicks ? { ticks: xTicks } : {}) },
    y: {
      stacked,
      beginAtZero: true,
      ticks: { stepSize: 1, font: { size: 10 } },
      // A literal, not the token, since canvas ignores an unresolved var with no fallback
      title: { display: !!yLabel, text: yLabel, font: { size: 10 }, color: GRAY },
      ...(pairedY ? { afterFit: pairedAxis } : {}),
    },
  },
});

const lineOpts = (unit) => ({
  responsive: true,
  maintainAspectRatio: false,
  layout: { padding: { top: 10 } },
  plugins: { legend: { display: false }, tooltip: dateTip(unit) },
  scales: {
    // A line starts hard against the left edge while a bar sits mid-slot, so offset gives
    // the line the same slots and a day lands under its own bars
    x: { offset: true, grid: { display: false, offset: true }, ticks: dateTicks(unit) },
    y: { beginAtZero: true, max: 100, afterFit: pairedAxis, ticks: { callback: v => v + "%", font: { size: 10 } } },
  },
});

// Sitting in the bars' slots leaves the line short of both edges, so this carries the
// first and last rate out flat, drawing nothing there so no dot can be hovered
const stretchRetention = {
  id: "stretchRetention",
  beforeDatasetsDraw(chart) {
    const points = chart.getDatasetMeta(0).data.filter(p => !p.skip);
    if (points.length === 0) return;
    const set = chart.data.datasets[0];
    const { left, right } = chart.chartArea;
    const first = points[0];
    const last = points[points.length - 1];
    const ctx = chart.ctx;
    ctx.save();
    ctx.strokeStyle = set.borderColor;
    ctx.lineWidth = set.borderWidth;
    for (const [x0, x1, y] of [[left, first.x, first.y], [last.x, right, last.y]]) {
      if (x1 - x0 < 0.5) continue;
      ctx.beginPath();
      ctx.moveTo(x0, y);
      ctx.lineTo(x1, y);
      ctx.stroke();
    }
    ctx.restore();
  },
};

// A dashed line across the By Time chart marking the average total hours per bucket
const avgTimeLine = {
  id: "avgTimeLine",
  afterDatasetsDraw(chart) {
    if (!chart.options.plugins?.avgTimeLine?.display) return;
    const n = chart.data.labels.length;
    if (!n) return;
    let sum = 0;
    for (let i = 0; i < n; i++) {
      for (const d of chart.data.datasets) sum += (d.data[i] ?? 0);
    }
    const avg = sum / n;
    if (!(avg > 0)) return;
    const { ctx, chartArea: { left, right, top }, scales: { y } } = chart;
    const yPix = y.getPixelForValue(avg);
    ctx.save();
    ctx.beginPath();
    ctx.setLineDash([5, 4]);
    ctx.lineWidth = 2;
    ctx.strokeStyle = RED;
    ctx.moveTo(left, yPix);
    ctx.lineTo(right, yPix);
    ctx.stroke();
    // Value pill riding the right end of the line, red fill so the number stays readable over bars
    ctx.setLineDash([]);
    const text = `${avg.toFixed(1)}h`;
    ctx.font = "700 12px 'Atkinson Hyperlegible', system-ui, sans-serif";
    const tw = ctx.measureText(text).width;
    const padX = 7, bw = tw + padX * 2, bh = 20;
    const bx = right - bw;
    let by = yPix - bh - 4;
    if (by < top) by = yPix + 4;
    ctx.fillStyle = RED;
    ctx.beginPath();
    ctx.roundRect(bx, by, bw, bh, 5);
    ctx.fill();
    ctx.fillStyle = "#fff";
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    ctx.fillText(text, bx + padX, by + bh / 2 + 0.5);
    ctx.restore();
  },
};

// Metric card
function MetricCard({ label, value, color, faces }) {
  const items = faces ?? [{ label, value, color }];
  const [i, setI] = useState(0);
  const idx = i % items.length;
  const cur = items[idx];
  const multi = items.length > 1;
  return (
    <div className={`st-metric${multi ? " st-metric--multi" : ""}`}
      onClick={multi ? () => setI(idx + 1) : undefined}>
      <div className="st-metric-body">
        <div className="st-metric-value" style={cur.color ? { color: cur.color } : {}}>{cur.value}</div>
        <div className="st-metric-label">{cur.label}</div>
      </div>
      {multi && (
        <div className="st-metric-dots">
          {items.map((_, k) => (
            <span key={k} className={`st-metric-dot${k === idx ? " active" : ""}`} />
          ))}
        </div>
      )}
    </div>
  );
}

// Chart panel

const RET_MODES = [
  { key: "daily",      label: "Daily" },
  { key: "cumulative", label: "Cumulative" },
];

const RANGES = [
  { label: "All", days: null },
  { label: "7d",  days: 7 },
  { label: "30d", days: 30 },
  { label: "90d", days: 90 },
];

function ChartPanel({ groupStats: allGroupStats, todoStats, today }) {
  const [tab, setTab] = useState("bytime");
  const [showAvg, setShowAvg] = useState(false);
  const [range,  setRange]  = useState(30);
  const [offset, setOffset] = useState(0);
  const [retMode, setRetMode] = useState("daily");

  const groupStats = counted(allGroupStats);

  // Snap back to the most recent window when the underlying data changes, keyed on the
  // prop since the filtered copy is new on every render
  useEffect(() => setOffset(0), [allGroupStats, todoStats]);

  function computeWindow(allDates) {
    const minDate = allDates[0] ?? null;
    const maxDate = allDates[allDates.length - 1] ?? null;
    // A fixed range ends today rather than on the last day studied, so a quiet stretch
    // since the last session shows as the blank days it is
    const anchor = today ?? maxDate;
    let start = null, end = null;
    if (range !== null && anchor) {
      end   = addDays(anchor, -offset * range);
      start = addDays(end, -(range - 1));
    }
    // All keeps every datapoint but widens the unit so long histories stay readable, raw
    // days up to 90 days of span, weekly totals to about 18 months, then monthly
    let unit = "day";
    if (range === null && minDate && maxDate) {
      const spanDays = (new Date(maxDate) - new Date(minDate)) / 86400000 + 1;
      if (spanDays > 548) unit = "month";
      else if (spanDays > 90) unit = "week";
    }
    const canGoOlder = range !== null && minDate !== null && start > minDate;
    return { start, end, unit, canGoOlder };
  }
  const inWindow = (win) => (r) => win.start === null || (r.date >= win.start && r.date <= win.end);

  const overWin = computeWindow([...new Set(groupStats.map(r => r.date))].sort());
  const { barData, dates, retention, hasRetention } = buildOverTimeData(
    groupStats.filter(inWindow(overWin)), overWin.unit, overWin,
  );
  const lineData = retentionLine(dates, retention[retMode]);

  const timeWin = computeWindow([...new Set([...groupStats, ...todoStats].map(r => r.date))].sort());
  const timeData = buildTimeSpentData(
    groupStats.filter(inWindow(timeWin)),
    todoStats.filter(inWindow(timeWin)),
    timeWin.unit,
    timeWin,
  );

  const byDeckData    = buildByDeckData(groupStats);
  const byCatData     = buildByCategoryData(todoStats);

  const canGoNewer = offset > 0;

  const tabs = [
    { key: "bytime",  label: "By Time" },
    { key: "bycards", label: "By Cards" },
    { key: "bydeck",  label: "By Deck" },
    { key: "bycat",   label: "By Category" },
  ];

  const legend = (
    <span className="st-legend">
      <span className="st-legend-dot" style={{ background: BLUE  }} />New
      <span className="st-legend-dot" style={{ background: GREEN }} />Promoted
      <span className="st-legend-dot" style={{ background: RED   }} />Demoted
    </span>
  );

  const timeLegend = (
    <span className="st-legend">
      <span className="st-legend-dot" style={{ background: YELLOW }} />Todos
      <span className="st-legend-dot" style={{ background: BLUE   }} />Decks
    </span>
  );

  const rangeControls = (win, extra) => (
    <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap", marginBottom: 10 }}>
      <div className="st-pills">
        {RANGES.map(({ label, days }) => (
          <button
            key={label}
            className={`st-pill${range === days ? " active" : ""}`}
            onClick={() => { setRange(days); setOffset(0); }}>
            {label}
          </button>
        ))}
      </div>
      {extra && <span style={{ marginLeft: 10, display: "inline-flex", alignItems: "center" }}>{extra}</span>}
      {range === null ? (
        win.unit !== "day" && (
          <span style={{ marginLeft: "auto", fontSize: 11, color: "var(--t-text-3)" }}>
            {win.unit === "week" ? "weekly" : "monthly"} totals
          </span>
        )
      ) : (
        <span style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 6 }}>
          <button className="st-btn-sm" disabled={!win.canGoOlder} style={!win.canGoOlder ? { opacity: 0.4 } : {}}
            onClick={() => setOffset(o => o + 1)}>‹</button>
          <span style={{ fontSize: 11, color: "var(--t-text-3)", fontVariantNumeric: "tabular-nums" }}>
            {win.start} – {win.end}
          </span>
          <button className="st-btn-sm" disabled={!canGoNewer} style={!canGoNewer ? { opacity: 0.4 } : {}}
            onClick={() => setOffset(o => o - 1)}>›</button>
        </span>
      )}
    </div>
  );

  // Hours are fractional, drop the whole-number tick step
  const timeOpts = (() => {
    const o = barOpts(true, "Hours", dateTicks(timeWin.unit), false, timeWin.unit);
    delete o.scales.y.ticks.stepSize;
    o.plugins = { ...o.plugins, avgTimeLine: { display: showAvg } };
    return o;
  })();

  return (
    <div className="st-chart-panel">
      <div className="st-chart-header">
        <div className="st-pills">
          {tabs.map(t => (
            <button key={t.key} className={`st-pill${tab === t.key ? " active" : ""}`} onClick={() => setTab(t.key)}>
              {t.label}
            </button>
          ))}
        </div>
        {(tab === "bycards" || tab === "bydeck") && legend}
        {tab === "bytime" && timeLegend}
      </div>

      {tab === "bytime" && (
        groupStats.length === 0 && todoStats.length === 0
          ? <div className="empty-bubble">No study time recorded yet.</div>
          : <div>
              {rangeControls(timeWin,
                <label style={{ display: "inline-flex", alignItems: "center", gap: 5, fontSize: 12, fontWeight: 600, color: "var(--t-text-2)", cursor: "pointer", userSelect: "none" }}>
                  <input type="checkbox" checked={showAvg} onChange={() => setShowAvg(a => !a)} />
                  Average Time
                </label>
              )}
              {timeData.labels.length === 0
                ? <div className="empty-bubble">No time recorded in this period.</div>
                : <div style={{ height: 200 }}>
                    <Bar data={timeData} options={timeOpts} plugins={[avgTimeLine]} />
                  </div>
              }
            </div>
      )}

      {tab === "bycards" && (
        groupStats.length === 0
          ? <div className="empty-bubble">No deck study data yet.</div>
          : <div>
              {rangeControls(overWin)}
              {barData.labels.length === 0
                ? <div className="empty-bubble">No study recorded in this period.</div>
                : <>
                    <div style={{ height: 200 }}>
                      <Bar data={barData} options={barOpts(true, "Cards", dateTicks(overWin.unit), true, overWin.unit)} />
                    </div>
                    {hasRetention && (
                      <div style={{ marginTop: 14 }}>
                        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
                          <span style={{ fontSize: 11, color: AMBER }}>Retention %</span>
                          <div className="st-pills" style={{ marginLeft: "auto" }}>
                            {RET_MODES.map(m => (
                              <button key={m.key} className={`st-pill${retMode === m.key ? " active" : ""}`}
                                onClick={() => setRetMode(m.key)}>
                                {m.label}
                              </button>
                            ))}
                          </div>
                        </div>
                        <div style={{ height: 150 }}>
                          <Line data={lineData} options={lineOpts(overWin.unit)} plugins={[stretchRetention]} />
                        </div>
                      </div>
                    )}
                  </>
              }
            </div>
      )}

      {tab === "bydeck" && (
        byDeckData.labels.length === 0
          ? <div className="empty-bubble">No deck study data yet.</div>
          : <div style={{ height: 220 }}>
              <Bar data={byDeckData} options={barOpts(false, "Cards", DECK_TICKS)} />
            </div>
      )}

      {tab === "bycat" && (
        byCatData.labels.length === 0
          ? <div className="empty-bubble">No todo data yet.</div>
          : <div style={{ height: 220 }}>
              <Bar data={byCatData} options={barOpts(false, "Hours")} />
            </div>
      )}

    </div>
  );
}

// Deck Sessions tab

// Reads Unarchive only when every row in scope is already archived
function ArchiveButton({ rows, onArchive, label = "Archive" }) {
  const allArchived = rows.length > 0 && rows.every(r => r.is_archived);
  return (
    <button className="st-archive-btn" onClick={() => onArchive(!allArchived)}>
      {allArchived ? `Un${label.toLowerCase()}` : label}
    </button>
  );
}

// How many days of sessions one page of a deck's table covers
const WINDOW_DAYS = 14;

function DeckSessionsTab({ groupStats, deckResets, planDecks, planId, onDeleted, setToast }) {
  const [deckFilter, setDeckFilter]   = useState("all");
  const [expanded, setExpanded]       = useState({});
  // How many windows back from the newest session each deck card is paged, 0 being the
  // most recent fortnight
  const [windowBack, setWindowBack] = useState({});
  // The one session row a click has selected, so the card's foot acts on it alone
  const [selectedRowId, setSelectedRowId] = useState(null);

  // origin_group_id is a plain rowid SQLite reissues, so it identifies a deck only while
  // that deck is alive, and rows whose deck is gone are bucketed apart by name
  const deckId = r => (r.group_id !== null
    ? `live:${r.origin_group_id}`
    : `dead:${r.origin_group_id ?? "x"}:${r.group_name}`);

  // A deck only counts as archived once every one of its rows is
  const allArchived = rows => rows.length > 0 && rows.every(r => r.is_archived);

  // Why a deck stopped being live, or null while it still is, and archived outranks gone
  // since a deck can be both and archived is what decides whether it counts
  const deadState = rows => {
    if (rows.length === 0) return null;
    if (allArchived(rows)) return "archived";
    if (rows[0].group_id !== null) return null;
    return rows[0].is_merged ? "merged" : "deleted";
  };

  const byDeck = {};
  groupStats.forEach(r => {
    const k = deckId(r);
    if (!byDeck[k]) byDeck[k] = [];
    byDeck[k].push(r);
  });

  // Decks in the plan that haven't been studied have no rows to derive a card from, so
  // seed them here and they drop off if they leave the plan unstudied
  planDecks.forEach(d => {
    const k = `live:${d.id}`;
    if (!byDeck[k]) byDeck[k] = [];
  });

  const deckName = key => (byDeck[key][0]?.group_name
    ?? planDecks.find(d => `live:${d.id}` === key)?.name
    ?? "");

  const deckKeys = Object.keys(byDeck)
    .sort((a, b) => deckName(a).localeCompare(deckName(b), undefined, { sensitivity: "base" }));

  // Decks still in play lead, then the archived, deleted and merged ones, each group
  // alphabetical, and the filter row and the tables below share the order
  const orderedKeys = [...deckKeys]
    .sort((a, b) => (deadState(byDeck[a]) ? 1 : 0) - (deadState(byDeck[b]) ? 1 : 0));

  // Deleting or merging the filtered deck leaves the key pointing at nothing, which
  // would read as an empty page rather than a cleared filter
  const activeFilter = byDeck[deckFilter] ? deckFilter : "all";
  const visibleKeys = activeFilter === "all" ? orderedKeys : orderedKeys.filter(k => k === activeFilter);

  const toggle = key => setExpanded(e => ({ ...e, [key]: !e[key] }));

  const deleteRow = async (id) => {
    try {
      await loggedInvoke("delete_group_stat", { id });
      setToast("Session deleted.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to delete session.", "error"); }
  };

  // This page decides which rows make up a deck's card, so it hands over their ids rather
  // than a description the backend would have to group by all over again
  const deleteStats = async (rows) => {
    try {
      await loggedInvoke("delete_group_stats", { ids: rows.map(r => r.id) });
      setToast("Deck stats deleted.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to delete deck stats.", "error"); }
  };

  const archiveRow = async (id, archived) => {
    try {
      await loggedInvoke("set_group_stat_archived", { id, archived });
      setToast(archived ? "Session archived." : "Session unarchived.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to archive session.", "error"); }
  };

  const archiveStats = async (rows, archived) => {
    try {
      await loggedInvoke("set_group_stats_archived", { ids: rows.map(r => r.id), archived });
      setToast(archived ? "Stats archived." : "Stats unarchived.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to archive stats.", "error"); }
  };

  if (deckKeys.length === 0) {
    return <div className="empty-bubble" style={{ marginTop: 16 }}>No decks in this plan yet.</div>;
  }

  return (
    <div>
      <div className="st-pills" style={{ marginBottom: 12 }}>
        <button className={`st-pill${activeFilter === "all" ? " active" : ""}`} onClick={() => setDeckFilter("all")}>All</button>
        {/* Read the name off the same row the deck's card does, since a rename or merge
            only updates the live deck and older rows can carry a stale name */}
        {orderedKeys.map(id => {
          const dead = deadState(byDeck[id]);
          const isActive = activeFilter === id;
          return (
            <button
              key={id}
              className={`st-pill st-pill--name${isActive ? " active" : ""}${dead ? ` st-pill-dead st-pill-dead--${dead}` : ""}`}
              onClick={() => setDeckFilter(id)}
              title={deckName(id)}>
              {deckName(id)}
            </button>
          );
        })}
      </div>

      {visibleKeys.map(cardId => {
        const deckRows = byDeck[cardId];
        const name = deckName(cardId);
        const isOpen = !!expanded[cardId];

        // Rows arrive newest first, so the first anchors the most recent fortnight and
        // paging walks backwards from there in whole windows
        const anchor  = deckRows[0]?.date ?? null;
        const oldest  = deckRows[deckRows.length - 1]?.date ?? null;
        const maxBack = anchor ? Math.floor(daysBetween(oldest, anchor) / WINDOW_DAYS) : 0;
        const back    = Math.min(Math.max(windowBack[cardId] ?? 0, 0), maxBack);
        const winEnd   = anchor ? addDays(anchor, -back * WINDOW_DAYS) : null;
        const winStart = winEnd ? addDays(winEnd, -(WINDOW_DAYS - 1)) : null;
        const rows = anchor ? deckRows.filter(r => r.date <= winEnd && r.date >= winStart) : [];
        // Only a row on the current page counts as selected, so paging away drops the
        // selection and the foot falls back to acting on the whole deck
        const selectedRow = rows.find(r => r.id === selectedRowId);
        const deckOrigin = deckRows[0]?.origin_group_id ?? null;
        // A reset marks the highest line id at the time, so its boundary anchors on the
        // newest row at or below that mark and draws nothing with no run to separate
        const resetRowIds = new Set(
          deckResets
            .filter(x => deckOrigin !== null && x.origin_group_id === deckOrigin)
            .filter(x => deckRows[0]?.id > x.after_stat_id)
            .map(x => deckRows.find(r => r.id <= x.after_stat_id)?.id)
            .filter(id => id !== undefined)
        );

        const totalTime = deckRows.reduce((s, r) => s + r.time_spent_minutes, 0);
        const totalN    = deckRows.reduce((s, r) => s + r.num_new, 0);
        const totalP    = deckRows.reduce((s, r) => s + r.num_promote, 0);
        const totalD    = deckRows.reduce((s, r) => s + r.num_demote, 0);
        const avgRet    = (totalP + totalD) > 0 ? totalP / (totalP + totalD) : null;

        // A missing deck was either deleted outright or merged into another one
        const isGone = deckRows.length > 0 && deckRows[0].group_id === null;
        const wasMerged = deckRows[0]?.is_merged;
        const isArchived = allArchived(deckRows);
        const step = n => setWindowBack(v => ({ ...v, [cardId]: back + n }));

        return (
          <div key={cardId} className="st-deck-card">
            <div className="st-deck-header" onClick={() => toggle(cardId)} style={{ cursor: "pointer" }}>
              <div className="st-deck-line">
                <span style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 6 }}>
                  {(isGone || isArchived) && (
                    <span style={{ display: "inline-flex", gap: 3 }}>
                      {isGone && <DeckStateBadge state={wasMerged ? "merged" : "deleted"} />}
                      {isArchived && <DeckStateBadge state="archived" />}
                    </span>
                  )}
                  <span className="st-deck-name">{name}</span>
                </span>
                <span className="t-caret">{isOpen ? "▾" : "▸"}</span>
              </div>
              {!isOpen && (
              <div className="st-deck-meta">
                <span className="st-meta-pill st-meta-pill--new">{totalN} new</span>
                <span className="st-meta-pill st-meta-pill--promote">+{totalP}</span>
                <span className="st-meta-pill st-meta-pill--demote">−{totalD}</span>
                {avgRet !== null && <span className={`st-meta-pill ${retentionPillClass(avgRet)}`}>{Math.round(avgRet * 100)}% ret.</span>}
                <span className="st-deck-meta-right">
                  <span className="st-meta-pill st-meta-pill--count">{deckRows.length} session{deckRows.length !== 1 ? "s" : ""}</span>
                  <span className="st-meta-pill st-meta-pill--time">{fmtTime(totalTime)}</span>
                </span>
              </div>
              )}
            </div>

            {isOpen && (
              <table className="st-table">
                <colgroup>
                  <col /><col /><col /><col /><col /><col /><col />
                </colgroup>
                <thead>
                  <tr>
                    <th style={{ color: "var(--t-new)" }}>New</th>
                    <th style={{ color: "var(--t-green)" }}>Promoted</th>
                    <th style={{ color: "var(--t-red)" }}>Demoted</th>
                    <th>Retention</th>
                    <th></th>
                    <th>Date</th>
                    <th>Time</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.length === 0 && (
                    <tr><td colSpan={7} style={{ textAlign: "center", color: "var(--t-text-3)", padding: "10px 0" }}>
                      No sessions yet.
                    </td></tr>
                  )}
                  {rows.map((r, i) => (
                    <Fragment key={r.id}>
                      {resetRowIds.has(r.id) && (
                        <tr className="st-era-row">
                          <td colSpan={7} className="st-era-divider">Progress Reset</td>
                        </tr>
                      )}
                      <tr
                        className={`${i % 2 === 1 ? "st-row-alt" : ""}${selectedRowId === r.id ? " st-row-selected" : ""}`}
                        onClick={() => setSelectedRowId(id => (id === r.id ? null : r.id))}
                        style={{ cursor: "pointer" }}>
                        <td><span className="st-badge" style={{ background: "var(--t-new)", color: "var(--t-accent-fg)" }}>{r.num_new}</span></td>
                        <td><span className="st-badge" style={{ background: "var(--t-green)", color: "var(--t-accent-fg)" }}>{r.num_promote}</span></td>
                        <td><span className="st-badge" style={{ background: "var(--t-red)", color: "var(--t-accent-fg)" }}>{r.num_demote}</span></td>
                        <td>
                          {(r.num_promote + r.num_demote) === 0 ? (
                            <div className="st-ret-bar-wrap">
                              <div className="st-ret-bar-track">
                                <span className="st-ret-pct" style={{ color: "var(--t-text-3)" }}>-</span>
                              </div>
                            </div>
                          ) : (
                            <div className="st-ret-bar-wrap">
                              <div className="st-ret-bar-track">
                                <div className="st-ret-bar-fill" style={{
                                  width: `${Math.round(r.retention_rate * 100)}%`,
                                  background: retentionColor(r.retention_rate),
                                }} />
                                <span className="st-ret-pct">
                                  {Math.round(r.retention_rate * 100)}%
                                </span>
                              </div>
                            </div>
                          )}
                        </td>
                        <td></td>
                        <td className="st-date-cell">
                          <span className="st-date">
                            {r.date}
                            {r.is_archived && (
                              <span className="st-date-arch" title="Archived, so it isn't counted toward your totals"><ArchivedBadge /></span>
                            )}
                          </span>
                        </td>
                        <td style={{ fontSize: 12, color: "var(--t-text-3)" }}>{fmtTime(r.time_spent_minutes)}</td>
                      </tr>
                    </Fragment>
                  ))}
                </tbody>
              </table>
            )}

            {/* Every studied deck gets the bar even with no fortnight to page back to, so
                the deck's own actions always have a home at the foot of the card */}
            {isOpen && deckRows.length > 0 && (
              <div className="st-window-nav">
                <span className="st-window-actions">
                  {selectedRow ? (
                    <>
                      <ArchiveButton rows={[selectedRow]} label="Archive"
                        onArchive={a => archiveRow(selectedRow.id, a)} />
                      <ConfirmDelete label="Delete" small onConfirm={() => deleteRow(selectedRow.id)} />
                    </>
                  ) : (
                    <>
                      <ArchiveButton rows={deckRows} label="Archive All"
                        onArchive={a => archiveStats(deckRows, a)} />
                      <ConfirmDelete label="Delete All" small onConfirm={() => deleteStats(deckRows)} />
                    </>
                  )}
                </span>
                {/* Equal tracks flank the pager so it sits dead centre of the card, and the
                    session count and time hold the same corner they take when it's closed */}
                <span className="st-window-pager">
                  <button className="st-btn-sm" disabled={back >= maxBack}
                    onClick={() => step(1)} title="Earlier sessions">‹</button>
                  <span className="st-window-label">{fmtShortDay(winStart)} - {fmtShortDay(winEnd)}</span>
                  <button className="st-btn-sm" disabled={back <= 0}
                    onClick={() => step(-1)} title="Later sessions">›</button>
                </span>
                <span className="st-deck-meta-right">
                  <span className="st-meta-pill st-meta-pill--count">{deckRows.length} session{deckRows.length !== 1 ? "s" : ""}</span>
                  <span className="st-meta-pill st-meta-pill--time">{fmtTime(totalTime)}</span>
                </span>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

// Todos tab
// Derived from PlanUtils so the filter pills match every category picker's order
const ALL_CATEGORIES = CATEGORIES.map(c => c.label);

function fmtDayLabel(dateStr) {
  const [y, m, d] = dateStr.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString("en-US", {
    weekday: "long", month: "long", day: "numeric", year: "numeric",
  });
}

const WEEKDAY_LABELS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const SEARCH_SCOPES = [
  { key: "all",       label: "All" },
  { key: "description", label: "Description" },
  { key: "details",   label: "Details" },
];

function tagValue(kind, live, key) { return `${kind}:${live ? "live" : "dead"}:${key}`; }

function matchTag(r, value) {
  const [kind, life, ...rest] = value.split(":");
  const key = rest.join(":");
  if (kind === "resource") {
    return r.resources.some(res => life === "live"
      ? String(res.resource_id) === key
      : res.resource_id == null && res.name === key);
  }
  return r.groups.some(g =>
    g.group_type === kind &&
    (life === "live" ? String(g.group_id) === key : g.group_id == null && g.name === key));
}

function buildTagSections(todoStats, allGroups, planResources) {
  const byName = (a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: "base" });
  const liveGroups = (type) => allGroups.filter(g => g.group_type === type)
    .map(g => ({ value: tagValue(type, true, String(g.id)), label: g.name, dead: false }))
    .sort(byName);
  const deadGroups = (type) => {
    const seen = new Map();
    todoStats.forEach(r => r.groups.forEach(g => {
      if (g.group_type === type && g.group_id == null && !seen.has(g.name)) {
        seen.set(g.name, { value: tagValue(type, false, g.name), label: g.name, dead: true });
      }
    }));
    return [...seen.values()].sort(byName);
  };
  const liveResources = planResources
    .map(pr => ({ value: tagValue("resource", true, String(pr.id)), label: pr.name, dead: false }))
    .sort(byName);
  const deadResources = (() => {
    const seen = new Map();
    todoStats.forEach(r => r.resources.forEach(res => {
      if (res.resource_id == null && !seen.has(res.name)) {
        seen.set(res.name, { value: tagValue("resource", false, res.name), label: res.name, dead: true });
      }
    }));
    return [...seen.values()].sort(byName);
  })();
  return [
    { kind: "resource", title: "Resources", items: [...liveResources, ...deadResources] },
    { kind: "deck",     title: "Decks",     items: [...liveGroups("deck"), ...deadGroups("deck")] },
    { kind: "notebook", title: "Notebooks", items: [...liveGroups("notebook"), ...deadGroups("notebook")] },
  ].filter(s => s.items.length > 0);
}

function FilterDropdown({ label, active, groups, value, onSelect, className }) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const ref = useRef(null);
  useEffect(() => {
    if (!open) return;
    const onDoc = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);
  const close = () => { setOpen(false); setSearch(""); };

  const q = search.trim().toLowerCase();
  // Match the label or any alternate spelling it carries
  const matches = (it) => it.label.toLowerCase().includes(q) || (it.alt ?? []).some(a => a.includes(q));
  const shownGroups = groups
    .map(g => ({ ...g, items: q ? g.items.filter(it => it.value !== "all" && matches(it)) : g.items }))
    .filter(g => g.items.length > 0);
  const total = groups.reduce((n, g) => n + g.items.filter(it => it.value !== "all").length, 0);

  return (
    <div className={`st-dd${className ? " " + className : ""}`} ref={ref}>
      <button className={`st-dd-btn${active ? " active" : ""}`} onClick={() => open ? close() : setOpen(true)}>
        <span className="st-dd-label">{label}</span>
        <span className="st-dd-caret">▾</span>
      </button>
      {open && (
        <div className="st-dd-menu">
          {total > 1 && (
            <input className="st-dd-search" placeholder="Search" value={search} onChange={e => setSearch(e.target.value)} />
          )}
          {shownGroups.map((g, gi) => (
            <div key={gi} className="st-dd-group">
              {g.title && <div className="st-dd-head">{g.title}</div>}
              {g.items.map(it => (
                <button key={it.value}
                  className={`st-dd-opt${it.dead ? " dead" : ""}${value === it.value ? " active" : ""}`}
                  onClick={() => { onSelect(it.value); close(); }}>{it.label}</button>
              ))}
            </div>
          ))}
          {shownGroups.length === 0 && <div className="st-dd-hint">No matches.</div>}
        </div>
      )}
    </div>
  );
}

function TodosTab({ todoStats, today, onDeleted, setToast, allGroups, planResources, allUnits, onOpenDeck }) {
  const [catFilter, setCatFilter] = useState(() => new Set(["all"]));
  const [expanded,  setExpanded]  = useState({});
  const [dateFrom,  setDateFrom]  = useState("");
  const [dateTo,    setDateTo]    = useState("");
  const [editingId, setEditingId] = useState(null);
  const [editForm,  setEditForm]  = useState(null);
  const [search,    setSearch]    = useState("");
  const [scopes,    setScopes]    = useState(() => new Set(["all"]));
  const [preset,    setPreset]    = useState("All");
  const [unitFilter, setUnitFilter] = useState("all");
  const [minutes,   setMinutes]   = useState("");
  const [minutesOp, setMinutesOp] = useState(">=");
  const [tagFilter, setTagFilter] = useState(null);
  const [dayFilter, setDayFilter] = useState(() => new Set(["all"]));

  const unitOptions = unitOptionsFrom(todoStats, allUnits);

  // All stands alone, picking it clears the rest and picking anything else clears it
  const toggleIn = (setter) => (key) => setter(prev => {
    if (key === "all") return new Set(["all"]);
    const next = new Set(prev);
    next.delete("all");
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next.size === 0 ? new Set(["all"]) : next;
  });
  const toggleScope = toggleIn(setScopes);
  const toggleCat   = toggleIn(setCatFilter);
  const toggleDay   = toggleIn(setDayFilter);

  const applyPreset = (label, days) => {
    setPreset(label);
    if (days === null) { setDateFrom(""); setDateTo(""); return; }
    const d = new Date(today);
    d.setDate(d.getDate() - (days - 1));
    setDateFrom(d.toISOString().slice(0, 10));
    setDateTo(today);
  };

  // A hand-picked date no longer matches whatever preset was highlighted
  const editDate = (setter) => (e) => { setter(e.target.value); setPreset(null); };

  let visible = todoStats;
  if (dateFrom) visible = visible.filter(r => r.date >= dateFrom);
  if (dateTo)   visible = visible.filter(r => r.date <= dateTo);
  if (!catFilter.has("all")) visible = visible.filter(r => parseCategories(r.category).some(c => catFilter.has(c)));
  if (!dayFilter.has("all")) visible = visible.filter(r => dayFilter.has(WEEKDAY_LABELS[new Date(r.date + "T00:00:00").getDay()]));
  if (unitFilter !== "all") visible = visible.filter(r => r.unit_group_id === unitFilter);
  if (tagFilter) visible = visible.filter(r => matchTag(r, tagFilter));

  const minutesNum = minutes === "" ? null : parseInt(minutes, 10);
  if (minutesNum !== null && !Number.isNaN(minutesNum)) {
    visible = visible.filter(r => minutesOp === ">=" ? r.time_spent_minutes >= minutesNum : r.time_spent_minutes <= minutesNum);
  }

  const query = search.trim().toLowerCase();
  if (query) {
    const has = s => (s || "").toLowerCase().includes(query);
    const inScope = key => scopes.has("all") || scopes.has(key);
    visible = visible.filter(r =>
      (inScope("description") && has(r.text)) ||
      (inScope("details") && has(r.details))
    );
  }

  const toggle = id => setExpanded(e => ({ ...e, [id]: !e[id] }));

  const deleteRow = async (id) => {
    try {
      await loggedInvoke("delete_todo_stat", { id });
      setToast("Entry deleted.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to delete entry.", "error"); }
  };

  const startEdit = (r) => {
    setEditingId(r.id);
    setEditForm({
      text: r.text,
      date: r.date,
      categoryMap: categoryStringToMap(r.category),
      details: r.details || "",
      timeSpent: r.time_spent_minutes,
      numValue: r.num_value ?? "",
      variantId: r.variant_id ?? null,
      // Kept lines are tracked by row id, never by name, since the snapshot keeps whatever
      // a deck or resource was called when it was logged and names repeat and drift
      groups: r.groups.map(x => x.row_id),
      resources: r.resources.map(x => x.row_id),
      addGroupIds: [],
      addResourceIds: [],
      // Tagged page per notebook, keyed by group id since a notebook links a stat only once
      pageByGroup: Object.fromEntries(
        r.groups.filter(g => g.group_type === "notebook" && g.group_id != null)
          .map(g => [g.group_id, g.page_id ?? null])
      ),
    });
  };

  const cancelEdit = () => { setEditingId(null); setEditForm(null); };
  const editKey = (r) => (e) => { if (e.key === "Enter") saveEdit(r); if (e.key === "Escape") cancelEdit(); };

  const saveEdit = async (r) => {
    const trimmed = editForm.text.trim();
    if (!trimmed) { setToast("Please enter a todo name.", "warn"); return; }
    const category = computeCategory(editForm.categoryMap);
    if (category === 0) { setToast("Select at least one category.", "warn"); return; }
    const timeSpent = Math.max(0, Math.round(parseFloat(editForm.timeSpent) || 0));
    if (timeSpent <= 0) { setToast("Please log at least 1 minute.", "warn"); return; }
    if (!editForm.date) { setToast("Please pick a date.", "warn"); return; }
    if (today && editForm.date > today) { setToast("An entry can't be dated in the future.", "warn"); return; }
    const unit = resolveUnitPair(editForm.numValue, editForm.variantId);
    if (unit.error) { setToast(unit.error, "warn"); return; }
    const removeGroupRowIds    = r.groups.map(x => x.row_id).filter(id => !editForm.groups.includes(id));
    const removeResourceRowIds = r.resources.map(x => x.row_id).filter(id => !editForm.resources.includes(id));
    // A page on a newly added notebook rides in with the insert, one on a kept link is set
    // after the update since its row already exists
    const addGroupPages = [];
    const keptPageUpdates = [];
    Object.entries(editForm.pageByGroup).forEach(([gidStr, pid]) => {
      const gid = Number(gidStr);
      const kept = r.groups.find(g => g.group_id === gid && editForm.groups.includes(g.row_id));
      if (kept) {
        if ((pid ?? null) !== (kept.page_id ?? null)) keptPageUpdates.push([kept.row_id, pid ?? null]);
      } else if (editForm.addGroupIds.includes(gid) && pid != null) {
        addGroupPages.push([gid, pid]);
      }
    });
    try {
      await loggedInvoke("update_todo_stat", {
        id: r.id,
        date: editForm.date,
        text: trimmed,
        category,
        details: editForm.details.trim() || null,
        timeSpentMinutes: timeSpent,
        numValue: unit.numValue,
        variantId: unit.variantId,
        removeGroupRowIds,
        removeResourceRowIds,
        addGroupIds: editForm.addGroupIds,
        addGroupPages,
        addResourceIds: editForm.addResourceIds,
      });
      for (const [rowId, pageId] of keptPageUpdates) {
        await loggedInvoke("set_todo_stat_group_page", { rowId, pageId });
      }
      setEditingId(null);
      setEditForm(null);
      setToast("Entry updated.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to update entry.", "error"); }
  };

  if (todoStats.length === 0) {
    return <div className="empty-bubble" style={{ marginTop: 16 }}>No todo history recorded yet.</div>;
  }

  // Consecutive same-date rows become one labeled day section, rows arrive date-sorted
  const days = [];
  visible.forEach(r => {
    if (days.length === 0 || days[days.length - 1].date !== r.date) days.push({ date: r.date, rows: [] });
    days[days.length - 1].rows.push(r);
  });

  // Any deviation from the untouched defaults counts as filtering, which is when the running
  // tally appears against the full history
  const filtersActive = !!(dateFrom || dateTo || !catFilter.has("all") || !dayFilter.has("all") || query || unitFilter !== "all" || minutes !== "" || tagFilter);

  // The tally also carries the matching todos' total time, and the picked unit's summed amount
  const visibleMinutes = visible.reduce((s, r) => s + r.time_spent_minutes, 0);
  const unitName = unitFilter === "all" ? null : (unitOptions.find(u => u.id === unitFilter)?.name ?? null);
  const unitSum = unitName ? visible.reduce((s, r) => s + (r.num_value ?? 0), 0) : null;
  const visibleUnits = unitSum === null ? null : (Number.isInteger(unitSum) ? unitSum : Math.round(unitSum * 100) / 100);

  return (
    <div>
      <div className="st-filters">
        {/* Each control group wears a quiet label the way a todo's Categories and Resources
            sections do. The panel is one grid, the wide date and search fields share the
            first column, the two dropdowns stack in the middle column so the rows line up,
            and the category pills take the full-width bottom row */}
        <div className="st-field">
          <div className="st-field-label">Date range</div>
          <div className="st-field-row">
            <input type="date" className="st-input" value={dateFrom} onChange={editDate(setDateFrom)} />
            <span className="st-affix">-</span>
            <input type="date" className="st-input" value={dateTo} onChange={editDate(setDateTo)} />
            <div className="st-pills">
              {[{ label: "All", days: null }, { label: "7d", days: 7 }, { label: "30d", days: 30 }, { label: "90d", days: 90 }].map(({ label, days }) => (
                <button key={label} className={`st-pill${preset === label ? " active" : ""}`} onClick={() => applyPreset(label, days)}>{label}</button>
              ))}
            </div>
          </div>
        </div>
        <div className="st-field">
          <div className="st-field-label">Unit</div>
          <div className="st-field-row">
            <FilterDropdown
              label={unitFilter === "all" ? "All" : (unitOptions.find(u => u.id === unitFilter)?.name ?? "All")}
              active={unitFilter !== "all"}
              value={unitFilter === "all" ? "all" : String(unitFilter)}
              groups={[{ items: [{ value: "all", label: "All units" }, ...unitOptions.map(u => ({ value: String(u.id), label: u.name, alt: u.alt }))] }]}
              onSelect={v => setUnitFilter(v === "all" ? "all" : Number(v))} />
          </div>
        </div>
        <div className="st-field">
          <div className="st-field-label">Study time</div>
          <div className="st-field-row">
            <div className="st-pills">
              <button className={`st-pill${minutesOp === ">=" ? " active" : ""}`} onClick={() => setMinutesOp(">=")}>≥</button>
              <button className={`st-pill${minutesOp === "<=" ? " active" : ""}`} onClick={() => setMinutesOp("<=")}>≤</button>
            </div>
            <input
              className="st-input st-input--minutes"
              value={minutes}
              onChange={e => { const v = e.target.value; if (v === "" || /^\d+$/.test(v)) setMinutes(v); }}
              placeholder="0"
              inputMode="numeric"
            />
            <span className="st-affix">min</span>
          </div>
        </div>
        <div className="st-field">
          <div className="st-field-label">Search</div>
          <div className="st-field-row">
            <input
              className="st-search-input"
              value={search}
              onChange={e => setSearch(e.target.value)}
              placeholder="Search todo history"
              size={1}
            />
            <div className="st-pills">
              {SEARCH_SCOPES.map(s => (
                <button key={s.key} className={`st-pill${scopes.has(s.key) ? " active" : ""}`} onClick={() => toggleScope(s.key)}>
                  {s.label}
                </button>
              ))}
            </div>
          </div>
        </div>
        <div className="st-field">
          <div className="st-field-label">Resource / Deck / Notebook</div>
          <div className="st-field-row">
            {(() => {
              const sections = buildTagSections(todoStats, allGroups, planResources);
              const selected = tagFilter ? sections.flatMap(s => s.items).find(it => it.value === tagFilter) : null;
              return (
                <FilterDropdown
                  label={selected ? selected.label : "All"}
                  active={!!tagFilter}
                  value={tagFilter ?? "all"}
                  groups={[{ items: [{ value: "all", label: "All" }] }, ...sections.map(s => ({ title: s.title, items: s.items }))]}
                  onSelect={v => setTagFilter(v === "all" ? null : v)} />
              );
            })()}
          </div>
        </div>
        <div className="st-field">
          <div className="st-field-label">Day of week</div>
          <div className="st-field-row">
            <div className="st-pills st-pills--tight">
              <button className={`st-pill${dayFilter.has("all") ? " active" : ""}`} onClick={() => toggleDay("all")}>All</button>
              {WEEKDAY_LABELS.map(d => (
                <button key={d} className={`st-pill${dayFilter.has(d) ? " active" : ""}`} onClick={() => toggleDay(d)}>{d}</button>
              ))}
            </div>
          </div>
        </div>
        <div className="st-field st-field--cats">
          <div className="st-field-label">Categories</div>
          <div className="st-field-row">
            <div className="st-pills">
              <button className={`st-pill${catFilter.has("all") ? " active" : ""}`} onClick={() => toggleCat("all")}>All</button>
              {ALL_CATEGORIES.map(c => {
                const active = catFilter.has(c);
                const col = CATEGORY_COLORS[c] || GRAY;
                return (
                  <button key={c} className="st-pill" onClick={() => toggleCat(c)}
                    style={active ? { background: col, borderColor: col, color: "var(--t-btn-fg)" } : {}}>
                    {c}
                  </button>
                );
              })}
            </div>
            <span className={`st-count-box${filtersActive ? "" : " off"}`} title="Matching todos out of all completed todos, with their total study time">
              {visible.length}/{todoStats.length} <span className="st-count-label">todos</span>
              <span className="st-count-sep">·</span>
              {fmtTime(visibleMinutes)}
              {unitName && (
                <>
                  <span className="st-count-sep">·</span>
                  {visibleUnits.toLocaleString()}{" "}
                  <span className="st-count-unit" title={unitName}>({unitName})</span>
                </>
              )}
            </span>
          </div>
        </div>
      </div>

      <div className="st-todo-list">
        {visible.length === 0
          ? <div className="empty-bubble">No todos match your filters.</div>
          : days.map(day => (
          <div key={day.date} className="st-day-group">
            <div className="st-day-divider"><span>{fmtDayLabel(day.date)}</span></div>
            {day.rows.map(r => {
          const isOpen    = !!expanded[r.id];
          const isEditing = editingId === r.id;
          const cats      = parseCategories(r.category);
          return (
            <div key={r.id} className="st-todo-row">
              <div className="st-todo-collapsed" onClick={() => { if (isEditing) cancelEdit(); toggle(r.id); }}>
                <div className="st-todo-line">
                  <span className="st-todo-text">{r.text}</span>
                  <span className="t-caret">{isOpen ? "▾" : "▸"}</span>
                </div>
                {!isOpen && (
                <div className="st-todo-tags">
                  {cats.map(c => (
                    <span key={c} className="st-pill-tag" style={{ background: CATEGORY_COLORS[c] || GRAY, color: "var(--t-btn-fg)" }}>{c}</span>
                  ))}
                  <span className="st-todo-meta-right">
                    {r.unit_label && (
                      <span className="st-meta-pill st-meta-pill--count st-todo-unit" title={fmtUnit(r.num_value, r.unit_label)}>
                        {fmtUnit(r.num_value, r.unit_label)}
                      </span>
                    )}
                    <span className="st-meta-pill st-meta-pill--time">{fmtTime(r.time_spent_minutes)}</span>
                  </span>
                </div>
                )}
              </div>

              {isOpen && !isEditing && (
                <div className="st-todo-expanded">
                  <div className="st-todo-section st-todo-summary">
                    {cats.length > 0 && (
                      <div className="st-todo-summary-cats">
                        <div className="st-todo-section-label">Categories</div>
                        <div className="st-todo-section-pills">
                          {cats.map(c => (
                            <span key={c} className="st-pill-tag" style={{ background: CATEGORY_COLORS[c] || GRAY, color: "var(--t-btn-fg)" }}>{c}</span>
                          ))}
                        </div>
                      </div>
                    )}
                    <div className="st-todo-summary-meta">
                      {r.unit_label && (
                        <div className="st-todo-summary-field">
                          <div className="st-todo-section-label">Units</div>
                          <span className="st-meta-pill st-meta-pill--count st-todo-unit" title={fmtUnit(r.num_value, r.unit_label)}>
                            {fmtUnit(r.num_value, r.unit_label)}
                          </span>
                        </div>
                      )}
                      <div className="st-todo-summary-field">
                        <div className="st-todo-section-label">Time</div>
                        <span className="st-meta-pill st-meta-pill--time">{fmtTime(r.time_spent_minutes)}</span>
                      </div>
                    </div>
                  </div>
                  {(r.resources.length > 0 || r.groups.length > 0) && (
                    <div className="st-todo-section">
                      <div className="st-todo-section-label">Resources / Decks / Notebooks</div>
                      <div className="st-item-bars">
                        {[...r.resources]
                          .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }))
                          .map((res, i) => <ResourceCard key={`res-${i}`} res={res} />)}
                        {/* Decks lead notebooks, each alphabetical, so the order matches a plan's todo */}
                        {[...r.groups]
                          .sort((a, b) => (a.group_type === "notebook" ? 1 : 0) - (b.group_type === "notebook" ? 1 : 0)
                            || a.name.localeCompare(b.name, undefined, { sensitivity: "base" }))
                          .map(g => {
                            const live = g.group_id != null ? allGroups.find(x => x.id === g.group_id) : null;
                            return (
                              <ItemBar key={`g-${g.row_id}`} name={g.name}
                                family={g.group_type === "notebook" ? "notebook" : "deck"}
                                pageNumber={g.group_type === "notebook" ? g.page_number : null}
                                onOpen={live ? () => onOpenDeck(live, g.group_type === "notebook" ? g.page_id : null) : undefined} />
                            );
                          })}
                      </div>
                    </div>
                  )}
                  {r.details && (
                    <div className="st-todo-section">
                      <div className="st-todo-section-label">Details</div>
                      <p className="st-todo-notes"><Linkify text={r.details} /></p>
                    </div>
                  )}
                </div>
              )}

              {isOpen && !isEditing && (
                <div className="st-todo-foot">
                  <button className="st-btn-sm" onClick={() => startEdit(r)}>Edit</button>
                  <ConfirmDelete small onConfirm={() => deleteRow(r.id)} />
                </div>
              )}

              {isEditing && editForm && (
                <div className="st-todo-expanded" style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                  <div>
                    <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Description</div>
                    <input
                      value={editForm.text}
                      autoFocus
                      onKeyDown={editKey(r)}
                      onChange={e => setEditForm(f => ({ ...f, text: e.target.value }))}
                      style={{ width: "100%", boxSizing: "border-box", padding: "5px 8px", border: "1px solid var(--t-border-2)", background: "var(--t-surface)", color: "var(--t-text)", fontSize: 13 }}
                    />
                  </div>
                  <div>
                    <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Categories</div>
                    <CategoryPicker
                      categoryMap={editForm.categoryMap}
                      onChange={bit => setEditForm(f => ({ ...f, categoryMap: { ...f.categoryMap, [bit]: !f.categoryMap[bit] } }))}
                    />
                  </div>
                  <div style={{ display: "flex", gap: 10 }}>
                    <div style={{ flex: 1 }}>
                      <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Date</div>
                      <input
                        type="date"
                        value={editForm.date}
                        max={today ?? undefined}
                        onKeyDown={editKey(r)}
                        onChange={e => setEditForm(f => ({ ...f, date: e.target.value }))}
                        style={{ width: "100%", boxSizing: "border-box", padding: "5px 8px", border: "1px solid var(--t-border-2)", background: "var(--t-surface)", color: "var(--t-text)", fontSize: 13 }}
                      />
                    </div>
                    <div style={{ flex: 1 }}>
                      <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Time (minutes)</div>
                      <input
                        type="number" min="0" step="1"
                        value={editForm.timeSpent}
                        onKeyDown={editKey(r)}
                        onChange={e => setEditForm(f => ({ ...f, timeSpent: e.target.value }))}
                        style={{ width: "100%", boxSizing: "border-box", padding: "5px 8px", border: "1px solid var(--t-border-2)", background: "var(--t-surface)", color: "var(--t-text)", fontSize: 13 }}
                      />
                    </div>
                    <div style={{ flex: 1 }}>
                      <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Units (optional)</div>
                      <UnitPicker
                        value={editForm.numValue}
                        variantId={editForm.variantId}
                        setToast={setToast}
                        onUnitsChanged={onDeleted}
                        onChange={({ value, variantId }) => setEditForm(f => ({ ...f, numValue: value, variantId }))}
                      />
                    </div>
                  </div>
                  {(() => {
                    const keptResources = r.resources.filter(x => editForm.resources.includes(x.row_id));
                    const addableResources = planResources.filter(pr => !keptResources.some(k => k.name === pr.name));
                    if (keptResources.length === 0 && addableResources.length === 0) return null;
                    return (
                      <div>
                        <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Resources</div>
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                          {keptResources.map(res => {
                            const dead = !planResources.some(pr => pr.name === res.name);
                            return (
                              <span key={res.row_id} className={`pill pill-clay${dead ? " pill-dead" : ""}`}>
                                {res.name}
                                <button onClick={() => setEditForm(f => ({ ...f, resources: f.resources.filter(id => id !== res.row_id) }))}
                                  style={{ background: "none", border: "none", cursor: "pointer", padding: 0, lineHeight: 1, color: "inherit", fontSize: 12 }}>×</button>
                              </span>
                            );
                          })}
                          {addableResources.map(pr => (
                            <label key={pr.id} className={`picker-pill${editForm.addResourceIds.includes(pr.id) ? " active-resource" : ""}`}>
                              <input type="checkbox" checked={editForm.addResourceIds.includes(pr.id)}
                                onChange={() => setEditForm(f => ({
                                  ...f,
                                  addResourceIds: f.addResourceIds.includes(pr.id)
                                    ? f.addResourceIds.filter(x => x !== pr.id)
                                    : [...f.addResourceIds, pr.id],
                                }))}
                                style={{ margin: 0 }} />
                              {pr.name}
                            </label>
                          ))}
                        </div>
                      </div>
                    );
                  })()}
                  {(() => {
                    // Decks lead notebooks, each alphabetical, matching the read-only view and a plan's todo
                    const byGroup = (a, b) => (a.group_type === "notebook" ? 1 : 0) - (b.group_type === "notebook" ? 1 : 0)
                      || a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
                    const keptGroups = r.groups.filter(x => editForm.groups.includes(x.row_id)).sort(byGroup);
                    const keptLiveIds = keptGroups.filter(x => x.group_id != null).map(x => x.group_id);
                    const addableGroups = allGroups.filter(g => !keptLiveIds.includes(g.id)).sort(byGroup);
                    if (keptGroups.length === 0 && addableGroups.length === 0) return null;
                    return (
                      <div>
                        <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Decks / Notebooks</div>
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                          {keptGroups.map(info => {
                            const dead = info.group_id == null;
                            const fam = info.group_type === "notebook" ? "plum" : "blue";
                            return (
                              <span key={info.row_id} className={`pill pill-${fam}${dead ? " pill-dead" : ""}`}>
                                {info.name}
                                {info.group_type && <GroupTypeBadge type={info.group_type} />}
                                <button onClick={() => setEditForm(f => ({ ...f, groups: f.groups.filter(id => id !== info.row_id) }))}
                                  style={{ background: "none", border: "none", cursor: "pointer", padding: 0, lineHeight: 1, color: "inherit", fontSize: 12 }}>×</button>
                              </span>
                            );
                          })}
                          {addableGroups.map(g => {
                            const active = editForm.addGroupIds.includes(g.id);
                            const fam = g.group_type === "notebook" ? " active-notebook" : " active-deck";
                            return (
                              <label key={g.id} className={`picker-pill${active ? fam : ""}`}>
                                <input type="checkbox" checked={active}
                                  onChange={() => setEditForm(f => ({
                                    ...f,
                                    addGroupIds: active
                                      ? f.addGroupIds.filter(x => x !== g.id)
                                      : [...f.addGroupIds, g.id],
                                  }))}
                                  style={{ margin: 0 }} />
                                {g.name}
                                <GroupTypeBadge type={g.group_type} />
                              </label>
                            );
                          })}
                        </div>
                      </div>
                    );
                  })()}
                  {(() => {
                    const nbs = [];
                    const seen = new Set();
                    r.groups.filter(g => g.group_type === "notebook" && g.group_id != null && editForm.groups.includes(g.row_id))
                      .forEach(g => { if (!seen.has(g.group_id)) { seen.add(g.group_id); nbs.push({ id: g.group_id, name: g.name }); } });
                    allGroups.filter(g => g.group_type === "notebook" && editForm.addGroupIds.includes(g.id))
                      .forEach(g => { if (!seen.has(g.id)) { seen.add(g.id); nbs.push({ id: g.id, name: g.name }); } });
                    if (nbs.length === 0) return null;
                    return (
                      <div>
                        <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Notebook pages (optional)</div>
                        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                          {nbs.map(nb => (
                            <div key={nb.id} style={{ display: "flex", alignItems: "center", gap: 8 }}>
                              <span style={{ fontSize: 13, color: "var(--t-text-2)", minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{nb.name}</span>
                              <NotebookPageTag notebookId={nb.id} pageId={editForm.pageByGroup[nb.id] ?? null}
                                onChange={pid => setEditForm(f => ({ ...f, pageByGroup: { ...f.pageByGroup, [nb.id]: pid } }))} />
                            </div>
                          ))}
                        </div>
                      </div>
                    );
                  })()}
                  <div style={{ fontSize: 11, color: "var(--t-text-3)", fontStyle: "italic" }}>
                    Resources, decks, and notebooks that have been deleted can be removed here, but they cannot be added back.
                  </div>
                  <div>
                    <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Details (optional)</div>
                    <textarea
                      value={editForm.details}
                      onChange={e => setEditForm(f => ({ ...f, details: e.target.value }))}
                      rows={3}
                      style={{ width: "100%", boxSizing: "border-box", padding: "5px 8px", border: "1px solid var(--t-border)", background: "var(--t-surface)", color: "var(--t-text)", fontSize: 13, resize: "vertical", fontFamily: "inherit" }}
                    />
                  </div>
                </div>
              )}

              {isEditing && editForm && (
                <div className="st-todo-foot">
                  <button className="primary" onClick={() => saveEdit(r)}>Save</button>
                  <button onClick={cancelEdit}>Cancel</button>
                </div>
              )}
            </div>
          );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}

// Root

// Plans still in play lead, then the disabled and deleted ones together, each group
// alphabetical, and the page opens on whichever one this puts first
function orderPlanPills(active, deleted) {
  return [
    ...active.map(p => ({ ...p, dead: p.is_disabled ? "archived" : null })),
    ...deleted.map(p => ({ ...p, dead: "deleted", deleted: true })),
  ].sort((a, b) => (a.dead ? 1 : 0) - (b.dead ? 1 : 0)
    || a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}

export default function Stats({ setToast, onNavigateToGroup, returnContext, onConsumeReturnContext }) {
  const [activePlans,    setActivePlans]   = useState([]);
  const [deletedPlans,   setDeletedPlans]  = useState([]);
  const [selectedPlanId, setSelectedPlanId] = useState(null);
  const [groupStats,     setGroupStats]    = useState([]);
  const [deckResets,     setDeckResets]    = useState([]);
  const [todoStats,      setTodoStats]     = useState([]);
  const [streakInfo,     setStreakInfo]    = useState({ streak: 0, studied_today: false, longest: 0 });
  const [contentTab,     setContentTab]    = useState(() => returnContext?.contentTab ?? "decks");
  const [today,          setToday]         = useState(null);
  const [loading,        setLoading]       = useState(true);
  const [allGroups,      setAllGroups]     = useState([]);
  const [planDecks,      setPlanDecks]     = useState([]);
  const [planResources,  setPlanResources] = useState([]);
  const [totals,         setTotals]        = useState(null);
  // Units are global, so the todo filter's spelling search can reach every alternate name a
  // unit carries, not only the ones that happen to appear in this plan's logged rows
  const [allUnits,       setAllUnits]      = useState([]);

  useEffect(() => {
    loggedInvoke("get_current_date").then(setToday).catch(e => logError("catch", e));
    Promise.all([
      loggedInvoke("get_plans"),
      loggedInvoke("get_deleted_plan_ids"),
      loggedInvoke("get_groups"),
      loggedInvoke("get_units"),
    ]).then(([ps, deleted, gs, units]) => {
      const dp = deleted.map(([id, name]) => ({ id, name }));
      setActivePlans(ps);
      setDeletedPlans(dp);
      setAllGroups(gs);
      setAllUnits(units);
      loadTotals();
      const firstId = returnContext?.selectedPlanId ?? orderPlanPills(ps, dp)[0]?.id ?? null;
      setSelectedPlanId(firstId);
      if (returnContext) onConsumeReturnContext();
      setLoading(false);
    }).catch(e => { logError("catch", e); setLoading(false); });
  }, []);

  const openDeck = (group, pageId = null) => {
    onNavigateToGroup(group, {
      menu: "stats",
      label: "Stats",
      statsContext: { selectedPlanId, contentTab },
    }, pageId);
  };

  const loadStats = (planId) => {
    if (!planId) return;
    Promise.all([
      loggedInvoke("get_group_stats",     { planId }),
      loggedInvoke("get_todo_stats", { planId }),
      loggedInvoke("get_plan_streak",     { planId }),
      loggedInvoke("get_resources",       { planId }),
      // A deck in the plan that hasn't been studied has no stat rows, so its card comes
      // from plan membership instead
      loggedInvoke("get_plan_srs_groups", { planId }),
      loggedInvoke("get_plan_resets",     { planId }),
    ]).then(([gs, ts, si, res, srs, rst]) => {
      setGroupStats(gs);
      setTodoStats(ts);
      setStreakInfo(si);
      setPlanResources(res);
      setPlanDecks(srs.map(([g]) => g).filter(g => g.group_type === "deck"));
      setDeckResets(rst);
    }).catch(e => { logError("catch", e); setToast("Failed to load stats.", "error"); });
  };

  // The header speaks for the whole record, so the backend sums across every plan, and the
  // oldest first record sets the day count even after that plan is put down
  const loadTotals = () => {
    loggedInvoke("get_record_totals")
      .then(t => setTotals({ deckMins: t.deck_mins, todoMins: t.todo_mins, earliest: t.earliest }))
      .catch(e => logError("catch", e));
  };

  const refreshStats = () => {
    loadStats(selectedPlanId);
    loadTotals();
  };

  const deleteDeletedPlan = async (planId) => {
    try {
      await loggedInvoke("delete_deleted_plan_stats", { planId });
      const freshDeleted = await loggedInvoke("get_deleted_plan_ids");
      const dp = freshDeleted.map(([id, name]) => ({ id, name }));
      setDeletedPlans(dp);
      if (selectedPlanId === planId) {
        const next = orderPlanPills(activePlans, dp)[0]?.id ?? null;
        setSelectedPlanId(next);
      }
      loadTotals();
      setToast("Plan stats deleted.");
    } catch(e) {
      logError("catch", e);
      setToast("Failed to delete plan stats.", "error");
    }
  };

  useEffect(() => {
    if (!selectedPlanId) {
      setGroupStats([]);
      setDeckResets([]);
      setTodoStats([]);
      setStreakInfo({ streak: 0, studied_today: false, longest: 0 });
      setPlanResources([]);
      return;
    }
    loadStats(selectedPlanId);
  }, [selectedPlanId]);

  const planDeleted = deletedPlans.some(p => p.id === selectedPlanId);
  const planDisabled = activePlans.some(p => p.id === selectedPlanId && p.is_disabled);
  // A disabled plan can't be studied, so it reads like a deleted one: no live streak
  const planDormant = planDeleted || planDisabled;
  const metrics = computeMetrics(counted(groupStats), todoStats);
  const totalDays = totalPlanDays(groupStats, todoStats, today, planDormant);
  const recordDays = totals?.earliest && today ? daysBetween(totals.earliest, today) + 1 : null;
  const retColor = metrics.avgRetention !== null ? retentionColor(metrics.avgRetention) : GRAY;
  const atRisk = streakInfo.streak > 0 && !streakInfo.studied_today;

  const planPills = orderPlanPills(activePlans, deletedPlans);

  return (
    <>
      <div className="st-root">
        <div className="st-header">
          <div style={{ flex: 1 }}>
            <h2>Stats</h2>
          </div>
          {totals?.earliest && (
            <span className="hdr-context">
              {[
                `${fmtTime(totals.deckMins + totals.todoMins)} study time`,
                recordDays !== null ? plural(recordDays, "day") : null,
              ].filter(Boolean).join(" · ")}
            </span>
          )}
        </div>
        <div className="st-body">
          <div style={{ display: "flex", alignItems: "flex-start", gap: 8, marginBottom: 16 }}>
            <div className="st-plan-bar" style={{ flex: 1, marginBottom: 0 }}>
              {planPills.map(p => (
                <button
                  key={p.deleted ? `d-${p.id}` : p.id}
                  className={`st-pill${p.dead ? ` st-pill-dead st-pill-dead--${p.dead}` : ""}${selectedPlanId === p.id ? " active" : ""}`}
                  onClick={() => setSelectedPlanId(p.id)}
                >
                  {p.name}
                </button>
              ))}
              {!loading && planPills.length === 0 && <span style={{ color: "var(--t-text-3)", fontSize: 13 }}>No plans yet.</span>}
            </div>
            {selectedPlanId && planDeleted && (
              <div style={{ flexShrink: 0 }}>
                <ConfirmDelete label="Delete All Stats" onConfirm={() => deleteDeletedPlan(selectedPlanId)} />
              </div>
            )}
          </div>

          <div className="st-metrics">
            <MetricCard
              label="Avg. Retention"
              value={metrics.avgRetention !== null ? `${Math.round(metrics.avgRetention * 100)}%` : "-"}
              color={metrics.avgRetention !== null ? retColor : GRAY}
            />
            <MetricCard faces={[
              { label: "Unique Cards Studied", value: metrics.newCardsStudied, color: "var(--t-blue)" },
              { label: "Total Cards Studied", value: metrics.totalCardsStudied, color: "var(--t-blue)" },
            ]} />
            <MetricCard label="Todos Done"    value={metrics.todosDone}     color="var(--t-yellow)" />
            <MetricCard faces={[
              { label: "Deck Time", value: fmtTime(metrics.studyMins), color: "var(--t-time)" },
              { label: "Todo Time", value: fmtTime(metrics.todoMins), color: "var(--t-time)" },
              { label: "Total Time", value: fmtTime(metrics.studyMins + metrics.todoMins), color: "var(--t-time)" },
            ]} />
            <MetricCard
              label="Avg. Daily Time"
              value={metrics.avgDailyStudy !== null ? fmtTime(Math.round(metrics.avgDailyStudy)) : "-"}
              color={metrics.avgDailyStudy !== null ? "var(--t-time)" : GRAY}
            />
            {/* A dormant plan can't be studied, so only the longest streak says anything */}
            {planDormant ? (
              <MetricCard
                label="Longest Streak"
                value={`${streakInfo.longest}d`}
                color={streakInfo.longest === 0 ? GRAY : "var(--t-green)"}
              />
            ) : (
              <MetricCard faces={[
                {
                  label: "Current Streak",
                  value: `${streakInfo.streak}d`,
                  color: streakInfo.streak === 0 ? GRAY : atRisk ? AMBER : "var(--t-green)",
                },
                {
                  label: "Longest Streak",
                  value: `${streakInfo.longest}d`,
                  color: streakInfo.longest === 0 ? GRAY : "var(--t-green)",
                },
              ]} />
            )}
            <MetricCard
              label="Total Days"
              value={totalDays !== null ? `${totalDays}d` : "-"}
              color={totalDays !== null ? "var(--t-time)" : GRAY}
            />
          </div>

          <ChartPanel groupStats={groupStats} todoStats={todoStats} today={today} />

          <div className="st-tabs">
            <button className={`st-tab st-tab--decks${contentTab === "decks" ? " active" : ""}`} onClick={() => setContentTab("decks")}>Decks</button>
            <button className={`st-tab st-tab--todos${contentTab === "todos" ? " active" : ""}`} onClick={() => setContentTab("todos")}>Todos</button>
          </div>

          {contentTab === "decks" && (
            <DeckSessionsTab
              groupStats={groupStats}
              deckResets={deckResets}
              planDecks={planDecks}
              planId={selectedPlanId}
              onDeleted={refreshStats}
              setToast={setToast}
            />
          )}
          {contentTab === "todos" && (
            <TodosTab
              todoStats={todoStats}
              today={today}
              onDeleted={refreshStats}
              setToast={setToast}
              allGroups={allGroups}
              planResources={planResources}
              allUnits={allUnits}
              onOpenDeck={openDeck}
            />
          )}
        </div>
      </div>
    </>
  );
}

