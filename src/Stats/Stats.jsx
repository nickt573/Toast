import { useState, useEffect, Fragment } from "react";
import { loggedInvoke, logError } from "../logger";
import { ResourceCard, GroupTypeBadge, ArchivedBadge, ConfirmDelete, Linkify } from "../UIUtils";
import { CategoryPicker, computeCategory, CATEGORIES, CATEGORY_COLOR_BY_LABEL } from "../Plans/PlanUtils";
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

// Constants:

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

// Category colors are defined once in PlanUtils and shared with Todos.
const CATEGORY_COLORS = CATEGORY_COLOR_BY_LABEL;

// Helpers:

function fmtTime(minutes) {
  if (!minutes) return "0m";
  const h = Math.floor(minutes / 60);
  const m = Math.round(minutes % 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
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

// "Jul 8" style, for the ends of a session window
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
    // "Other" was renamed to "Culture" (bit 64), alias old stat rows
    .map(s => s === "Other" ? "Culture" : s);
}

function categoryStringToMap(catStr) {
  const names = parseCategories(catStr);
  const map = {};
  CATEGORIES.forEach(({ label, bit }) => { map[bit] = names.includes(label); });
  return map;
}

// An archived row was either copied into a merged deck or set aside by a reset, so
// either way something else is the record now. Every aggregate reads through this,
// otherwise a merge inflates the plan.
function counted(groupStats) {
  return groupStats.filter(r => !r.is_archived);
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

  return { studyMins, todoMins, newCardsStudied, totalCardsStudied, todosDone, avgRetention };
}

// Chart data builders:

// Bucket key per unit: "day" = the date itself (raw daily bars, unchanged),
// "week" = the Monday of that week, "month" = "YYYY-MM". Labels keep the year.
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

// Every bucket between the ends gets a label, empty or not. Otherwise a day nothing was
// studied on simply vanishes and the bars either side of it read as consecutive. The
// ends come from the window being shown when there is one, so a 30d page stays 30 wide
// however little of it was studied, and from the data itself for "All".
function bucketRange(keys, unit, from = null, to = null) {
  const sorted = [...keys].sort();
  const start = from ?? sorted[0];
  const end   = to   ?? sorted[sorted.length - 1];
  if (!start || !end) return [];
  const out = [];
  for (let k = start; k <= end; k = nextBucket(k, unit)) out.push(k);
  return out;
}

// A window starts at its first bucket with something in it, so two sessions in the last
// three days read as three days rather than four blanks and then three. Blanks after
// that stay, including a run at the end: not having studied yet today is worth seeing.
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

  // Retention rides the same labels as the bars so the two line up, but a bucket with
  // nothing reviewed has no rate to plot, so it goes in as a null and the line breaks.
  const retentionData = dates.map(d => {
    const { p, d: dem } = at(d);
    return (p + dem) > 0 ? Math.round((p / (p + dem)) * 100) : null;
  });
  const hasRetention = retentionData.some(v => v !== null);

  const lineData = {
    labels: dates,
    datasets: [
      {
        label: "Retention %",
        data: retentionData,
        borderColor: AMBER,
        backgroundColor: "rgba(196,154,68,0.16)",
        tension: 0.3,
        fill: true,
        pointRadius: 3,
      },
    ],
  };

  return { barData, lineData, hasRetention };
}

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

// Shared chart options:

// Caps how many date labels render as history grows, the bars themselves are unaffected.
const DATE_TICKS = { autoSkip: true, maxTicksLimit: 12, maxRotation: 30, font: { size: 10 } };

// Wraps a label onto word-boundary lines, only truncates past the line limit.
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

// Deck names are categorical: never skip a label (every bar stays identified), never
// rotate, wrap long names instead. Tooltips still show the full name.
const DECK_TICKS = {
  autoSkip: false,
  maxRotation: 0,
  font: { size: 10 },
  callback(value) {
    return wrapTickLabel(this.getLabelForValue(value));
  },
};

const barOpts = (stacked = false, yLabel = "", xTicks = null) => ({
  responsive: true,
  maintainAspectRatio: false,
  plugins: { legend: { display: false } },
  scales: {
    x: { stacked, grid: { display: false }, ...(xTicks ? { ticks: xTicks } : {}) },
    y: {
      stacked,
      beginAtZero: true,
      ticks: { stepSize: 1, font: { size: 10 } },
      title: { display: !!yLabel, text: yLabel, font: { size: 10 }, color: "var(--t-text-3)" },
    },
  },
});

const lineOpts = {
  responsive: true,
  maintainAspectRatio: false,
  layout: { padding: { top: 10, right: 4 } },
  plugins: { legend: { display: false } },
  scales: {
    x: { grid: { display: false }, ticks: DATE_TICKS },
    y: { beginAtZero: true, max: 100, ticks: { callback: v => v + "%", font: { size: 10 } } },
  },
};

// Metric card:

function MetricCard({ label, value, color }) {
  return (
    <div className="st-metric">
      <div className="st-metric-value" style={color ? { color } : {}}>{value}</div>
      <div className="st-metric-label">{label}</div>
    </div>
  );
}

// Chart panel:

const RANGES = [
  { label: "All", days: null },
  { label: "7d",  days: 7 },
  { label: "30d", days: 30 },
  { label: "90d", days: 90 },
];

function ChartPanel({ groupStats: allGroupStats, todoStats, today }) {
  const [tab, setTab] = useState("bytime");
  const [range,  setRange]  = useState(30);
  const [offset, setOffset] = useState(0);

  const groupStats = counted(allGroupStats);

  // Snap back to the most recent window when the underlying data changes (e.g. plan
  // switch). Keyed on the prop, since the filtered copy is new on every render.
  useEffect(() => setOffset(0), [allGroupStats, todoStats]);

  // Each chart windows over its own date domain
  function computeWindow(allDates) {
    const minDate = allDates[0] ?? null;
    const maxDate = allDates[allDates.length - 1] ?? null;
    // A fixed range ends today rather than on the last day studied, so a quiet stretch
    // since the last session shows as the blank days it is.
    const anchor = today ?? maxDate;
    let start = null, end = null;
    if (range !== null && anchor) {
      end   = addDays(anchor, -offset * range);
      start = addDays(end, -(range - 1));
    }
    // "All" keeps every datapoint but widens the unit so a lifetime of history stays
    // readable: raw days up to 90 days of span, weekly totals to ~18 months, then monthly.
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
  const { barData, lineData, hasRetention } = buildOverTimeData(
    groupStats.filter(inWindow(overWin)), overWin.unit, overWin,
  );

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

  const rangeControls = (win) => (
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
    const o = barOpts(true, "Hours", DATE_TICKS);
    delete o.scales.y.ticks.stepSize;
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
              {rangeControls(timeWin)}
              {timeData.labels.length === 0
                ? <div className="empty-bubble">No time recorded in this period.</div>
                : <div style={{ height: 200 }}>
                    <Bar data={timeData} options={timeOpts} />
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
                      <Bar data={barData} options={barOpts(true, "Cards", DATE_TICKS)} />
                    </div>
                    {hasRetention && (
                      <div style={{ marginTop: 14 }}>
                        <div style={{ fontSize: 11, color: AMBER, marginBottom: 4 }}>Retention %</div>
                        <div style={{ height: 150 }}>
                          <Line data={lineData} options={lineOpts} />
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

// Deck Sessions tab:

// Archive toggle for a scope of stat rows. Reads Unarchive only when every row in
// scope is already archived, so a mix or none offers Archive first.
function ArchiveButton({ rows, onArchive, label = "Archive" }) {
  const allArchived = rows.length > 0 && rows.every(r => r.is_archived);
  return (
    <button className="st-archive-btn" onClick={() => onArchive(!allArchived)}>
      {allArchived ? `Un${label.toLowerCase()}` : label}
    </button>
  );
}

// How many days of sessions one page of a deck's table covers.
const WINDOW_DAYS = 14;

function DeckSessionsTab({ groupStats, planDecks, planId, onDeleted, setToast }) {
  const [deckFilter, setDeckFilter]   = useState("all");
  const [expanded, setExpanded]       = useState({});
  // How many windows back from the newest session each deck card is paged. 0 is the
  // most recent fortnight.
  const [windowBack, setWindowBack] = useState({});
  // The one session row a click has selected, so the card's foot acts on it alone.
  const [selectedRowId, setSelectedRowId] = useState(null);

  // One card per deck, a fortnight of days paged inside it.
  //
  // origin_group_id survives the deck being deleted, but it is a plain rowid and
  // SQLite reissues it to the next deck created, so it identifies a deck only while
  // that deck is alive. Rows whose deck is gone are bucketed apart by name, otherwise
  // a dead deck's history lands inside a brand new deck's table. Rows old enough to
  // predate origin_group_id have nothing but the name to go on either way.
  const deckId = r => (r.group_id !== null
    ? `live:${r.origin_group_id}`
    : `dead:${r.origin_group_id ?? "x"}:${r.group_name}`);

  // A deck only counts as archived once every one of its rows is, the same test
  // ArchiveButton uses to decide whether it reads Archive all or Unarchive all.
  const allArchived = rows => rows.length > 0 && rows.every(r => r.is_archived);

  // Why a deck stopped being live, or null while it still is. Archived outranks
  // gone because it is the state that decides whether any of this counts toward
  // your totals, and a deck can easily be both.
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

  // Decks in the plan that haven't been studied yet have no rows to derive a card
  // from, so seed them here. They show an empty table until a session opens one, and
  // drop off entirely if they leave the plan without ever being studied.
  planDecks.forEach(d => {
    const k = `live:${d.id}`;
    if (!byDeck[k]) byDeck[k] = [];
  });

  const deckName = key => (byDeck[key][0]?.group_name
    ?? planDecks.find(d => `live:${d.id}` === key)?.name
    ?? "");

  const deckKeys = Object.keys(byDeck)
    .sort((a, b) => deckName(a).localeCompare(deckName(b), undefined, { sensitivity: "base" }));

  // Deleting or merging the deck you had filtered to leaves the key pointing at
  // nothing, which would read as an empty page rather than a cleared filter
  const activeFilter = byDeck[deckFilter] ? deckFilter : "all";
  const visibleKeys = activeFilter === "all" ? deckKeys : deckKeys.filter(k => k === activeFilter);

  const toggle = key => setExpanded(e => ({ ...e, [key]: !e[key] }));

  const deleteRow = async (id) => {
    try {
      await loggedInvoke("delete_group_stat", { id });
      setToast("Session deleted.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to delete session.", "error"); }
  };

  // This page is what decides which rows make up a deck's card, so it hands over their
  // ids rather than a description the backend would have to group by all over again.
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
        {/* Read the name off the same row the deck's card does. Rows are newest
            first, and a rename or merge only ever updates the live deck, so older
            rows can still carry a name this deck hasn't gone by in a while. */}
        {Object.keys(byDeck).map(id => {
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

        // Rows arrive newest first, so the first one anchors the most recent
        // fortnight and paging walks backwards from there in whole windows.
        const anchor  = deckRows[0]?.date ?? null;
        const oldest  = deckRows[deckRows.length - 1]?.date ?? null;
        const maxBack = anchor ? Math.floor(daysBetween(oldest, anchor) / WINDOW_DAYS) : 0;
        const back    = Math.min(Math.max(windowBack[cardId] ?? 0, 0), maxBack);
        const winEnd   = anchor ? addDays(anchor, -back * WINDOW_DAYS) : null;
        const winStart = winEnd ? addDays(winEnd, -(WINDOW_DAYS - 1)) : null;
        const rows = anchor ? deckRows.filter(r => r.date <= winEnd && r.date >= winStart) : [];
        // Only a row on the current page counts as selected, so paging away drops the
        // selection and the foot falls back to acting on the whole deck.
        const selectedRow = rows.find(r => r.id === selectedRowId);
        // The reset line belongs to the run it introduces, so it is anchored on the
        // first row of the older run rather than on the row that opened the newer one.
        // Those two sit either side of the same boundary, so the line lands in the same
        // place whenever both are on one page, and follows the older rows onto the
        // previous page when the fortnight splits between them. It also means a reset
        // with nothing left underneath draws no line at all.
        const eraFirstRowIds = new Set(
          deckRows.filter((r, i) => i > 0 && deckRows[i - 1].starts_era).map(r => r.id)
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
                <span style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 8 }}>
                  <span className="st-deck-name">{name}</span>
                </span>
                <span className="st-caret">{isOpen ? "▾" : "▸"}</span>
              </div>
              <div className="st-deck-meta">
                <span className="st-meta-pill st-meta-pill--new">{totalN} new</span>
                <span className="st-meta-pill st-meta-pill--promote">+{totalP}</span>
                <span className="st-meta-pill st-meta-pill--demote">−{totalD}</span>
                {avgRet !== null && <span className={`st-meta-pill ${retentionPillClass(avgRet)}`}>{Math.round(avgRet * 100)}% ret.</span>}
                <span className="st-deck-meta-right">
                  {isGone && (wasMerged
                    ? <span className="st-badge st-badge-merged">Merged</span>
                    : <span className="st-badge st-badge-deleted">Deleted</span>)}
                  {isArchived && (
                    <span className="st-badge st-badge-archived" title="Every session in this deck is archived, so none of it counts toward your totals">Archived</span>
                  )}
                  <span className="st-meta-pill st-meta-pill--count">{deckRows.length} session{deckRows.length !== 1 ? "s" : ""}</span>
                  <span className="st-meta-pill st-meta-pill--time">{fmtTime(totalTime)}</span>
                </span>
              </div>
            </div>

            {isOpen && (
              <table className="st-table">
                <colgroup>
                  <col /><col /><col /><col /><col /><col /><col />
                </colgroup>
                <thead>
                  <tr>
                    <th style={{ color: "var(--t-blue)" }}>New</th>
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
                      {eraFirstRowIds.has(r.id) && (
                        <tr className="st-era-row">
                          <td colSpan={7} className="st-era-divider">Progress Reset</td>
                        </tr>
                      )}
                      <tr
                        className={`${i % 2 === 1 ? "st-row-alt" : ""}${selectedRowId === r.id ? " st-row-selected" : ""}`}
                        onClick={() => setSelectedRowId(id => (id === r.id ? null : r.id))}
                        style={{ cursor: "pointer" }}>
                        <td><span className="st-badge" style={{ background: "var(--t-blue)", color: "var(--t-accent-fg)" }}>{r.num_new}</span></td>
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

            {/* Every deck that has been studied gets the bar, whether or not there is a
                fortnight to page back to, so the deck's own actions always have a home
                at the foot of the card. */}
            {isOpen && deckRows.length > 0 && (
              <div className="st-window-nav">
                {selectedRow ? (
                  <>
                    <ArchiveButton rows={[selectedRow]} label="Archive"
                      onArchive={a => archiveRow(selectedRow.id, a)} />
                    <ConfirmDelete label="Delete" small onConfirm={() => deleteRow(selectedRow.id)} />
                  </>
                ) : (
                  <>
                    <ArchiveButton rows={deckRows} label="Archive all"
                      onArchive={a => archiveStats(deckRows, a)} />
                    <ConfirmDelete label="Delete all" small onConfirm={() => deleteStats(deckRows)} />
                  </>
                )}
                <span className="st-window-pager">
                  <button className="st-btn-sm" disabled={back >= maxBack}
                    onClick={() => step(1)} title="Earlier sessions">‹</button>
                  <span className="st-window-label">{fmtShortDay(winStart)} - {fmtShortDay(winEnd)}</span>
                  <button className="st-btn-sm" disabled={back <= 0}
                    onClick={() => step(-1)} title="Later sessions">›</button>
                </span>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

// Todos tab:
// Derived from PlanUtils so the filter pills match every category picker's order.
const ALL_CATEGORIES = CATEGORIES.map(c => c.label);

function fmtDayLabel(dateStr) {
  const [y, m, d] = dateStr.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString("en-US", {
    weekday: "long", month: "long", day: "numeric", year: "numeric",
  });
}

const SEARCH_SCOPES = [
  { key: "all",       label: "All" },
  { key: "description", label: "Description" },
  { key: "details",   label: "Details" },
  { key: "resources", label: "Resources" },
  { key: "groups",    label: "Decks / Notebooks" },
];

function TodosTab({ todoStats, today, onDeleted, setToast, allGroups, planResources, onOpenDeck }) {
  const [catFilter, setCatFilter] = useState("all");
  const [expanded,  setExpanded]  = useState({});
  const [dateFrom,  setDateFrom]  = useState("");
  const [dateTo,    setDateTo]    = useState("");
  const [editingId, setEditingId] = useState(null);
  const [editForm,  setEditForm]  = useState(null);
  const [search,    setSearch]    = useState("");
  const [scopes,    setScopes]    = useState(() => new Set(["all"]));
  const [preset,    setPreset]    = useState("All");

  // All stands alone: picking it clears the rest, picking anything else clears it
  const toggleScope = (key) => setScopes(prev => {
    if (key === "all") return new Set(["all"]);
    const next = new Set(prev);
    next.delete("all");
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next.size === 0 ? new Set(["all"]) : next;
  });

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
  if (catFilter !== "all") visible = visible.filter(r => parseCategories(r.category).includes(catFilter));

  const query = search.trim().toLowerCase();
  if (query) {
    const has = s => (s || "").toLowerCase().includes(query);
    const inScope = key => scopes.has("all") || scopes.has(key);
    visible = visible.filter(r =>
      (inScope("description") && has(r.text)) ||
      (scopes.has("all") && has(r.num_unit)) ||
      (inScope("details") && has(r.details)) ||
      // Resources match on name and description only, never the type or link
      (inScope("resources") && r.resources.some(res => has(res.name) || has(res.notes))) ||
      (inScope("groups") && r.groups.some(g => has(g.name)))
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
      categoryMap: categoryStringToMap(r.category),
      details: r.details || "",
      timeSpent: r.time_spent_minutes,
      numUnit: r.num_unit || "",
      // Kept lines are tracked by row id, never by name: the snapshot keeps whatever
      // a deck or resource was called when it was logged, so names repeat and drift.
      groups: r.groups.map(x => x.row_id),
      resources: r.resources.map(x => x.row_id),
      addGroupIds: [],
      addResourceIds: [],
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
    const removeGroupRowIds    = r.groups.map(x => x.row_id).filter(id => !editForm.groups.includes(id));
    const removeResourceRowIds = r.resources.map(x => x.row_id).filter(id => !editForm.resources.includes(id));
    try {
      await loggedInvoke("update_todo_stat", {
        id: r.id,
        text: trimmed,
        category,
        details: editForm.details.trim() || null,
        timeSpentMinutes: timeSpent,
        numUnit: editForm.numUnit.trim() || null,
        removeGroupRowIds,
        removeResourceRowIds,
        addGroupIds: editForm.addGroupIds,
        addResourceIds: editForm.addResourceIds,
      });
      setEditingId(null);
      setEditForm(null);
      setToast("Entry updated.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to update entry.", "error"); }
  };

  if (todoStats.length === 0) {
    return <div className="empty-bubble" style={{ marginTop: 16 }}>No todo history recorded yet.</div>;
  }

  // Consecutive same-date rows become one labeled day section (rows arrive date-sorted)
  const days = [];
  visible.forEach(r => {
    if (days.length === 0 || days[days.length - 1].date !== r.date) days.push({ date: r.date, rows: [] });
    days[days.length - 1].rows.push(r);
  });

  return (
    <div>
      {/* Grid so the search bar and the date pair share a column and end at the same edge */}
      <div style={{ display: "grid", gridTemplateColumns: "max-content 1fr", gap: "8px 6px", alignItems: "center", marginBottom: 8 }}>
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <input type="date" value={dateFrom} onChange={editDate(setDateFrom)}
            style={{ fontSize: 12, padding: "2px 4px", border: "1px solid var(--t-border)" }} />
          <span style={{ fontSize: 12, color: "var(--t-text-3)" }}>-</span>
          <input type="date" value={dateTo} onChange={editDate(setDateTo)}
            style={{ fontSize: 12, padding: "2px 4px", border: "1px solid var(--t-border)" }} />
        </div>
        <div className="st-pills">
          {[{ label: "All", days: null }, { label: "7d", days: 7 }, { label: "30d", days: 30 }, { label: "90d", days: 90 }].map(({ label, days }) => (
            <button key={label} className={`st-pill${preset === label ? " active" : ""}`} onClick={() => applyPreset(label, days)}>{label}</button>
          ))}
        </div>
        <input
          value={search}
          onChange={e => setSearch(e.target.value)}
          placeholder="Search todo history"
          size={1}
          style={{ fontSize: 12, padding: "3px 8px", border: "1px solid var(--t-border)", width: "100%" }}
        />
        <div className="st-pills">
          {SEARCH_SCOPES.map(s => (
            <button key={s.key} className={`st-pill${scopes.has(s.key) ? " active" : ""}`} onClick={() => toggleScope(s.key)}>
              {s.label}
            </button>
          ))}
        </div>
      </div>
      <div className="st-pills" style={{ marginBottom: 12 }}>
        <button className={`st-pill${catFilter === "all" ? " active" : ""}`} onClick={() => setCatFilter("all")}>All</button>
        {ALL_CATEGORIES.map(c => {
          const active = catFilter === c;
          const col = CATEGORY_COLORS[c] || GRAY;
          return (
            <button key={c} className="st-pill" onClick={() => setCatFilter(c)}
              style={active ? { background: col, borderColor: col, color: "var(--t-btn-fg)" } : {}}>
              {c}
            </button>
          );
        })}
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
          // Categories, time and units ride in the collapsed row, not in here, so an
          // entry can carry those and still have nothing to open up to.
          const isBare    = !r.details && r.resources.length === 0 && r.groups.length === 0;
          return (
            <div key={r.id} className="st-todo-row">
              <div className="st-todo-collapsed" onClick={() => toggle(r.id)}>
                <div className="st-todo-line">
                  <span className="st-todo-text">{r.text}</span>
                  <span className="st-caret">{isOpen ? "▾" : "▸"}</span>
                </div>
                <div className="st-todo-tags">
                  {cats.map(c => (
                    <span key={c} className="st-pill-tag" style={{ background: CATEGORY_COLORS[c] || GRAY, color: "var(--t-btn-fg)" }}>{c}</span>
                  ))}
                  <span className="st-todo-meta-right">
                    {r.num_unit && <span className="st-meta-pill st-meta-pill--count st-todo-unit" title={r.num_unit}>{r.num_unit}</span>}
                    <span className="st-meta-pill st-meta-pill--time">{fmtTime(r.time_spent_minutes)}</span>
                  </span>
                </div>
              </div>

              {isOpen && !isEditing && (
                <div className="st-todo-expanded">
                  {r.details && (
                    <div className="st-todo-section">
                      <div className="st-todo-section-label">Details</div>
                      <p className="st-todo-notes"><Linkify text={r.details} /></p>
                    </div>
                  )}
                  {r.resources.length > 0 && (
                    <div className="st-todo-section">
                      <div className="st-todo-section-label">Resources</div>
                      <div className="st-resource-cards">
                        {r.resources.map((res, i) => <ResourceCard key={i} res={res} />)}
                      </div>
                    </div>
                  )}
                  {r.groups.length > 0 && (
                    <div className="st-todo-section">
                      <div className="st-todo-section-label">Decks / Notebooks</div>
                      <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                        {r.groups.map((g, i) => {
                          const live = g.group_id != null ? allGroups.find(x => x.id === g.group_id) : null;
                          if (!live) {
                            if (g.group_type) {
                              const fam = g.group_type === "notebook" ? "pill-plum" : "pill-blue";
                              return (
                                <span key={i} className={`pill ${fam} pill-dead`}>
                                  {g.name}
                                  <GroupTypeBadge type={g.group_type} />
                                </span>
                              );
                            }
                            return (
                              <span key={i} className="pill pill-neutral">{g.name}</span>
                            );
                          }
                          return (
                            <span key={i}
                              className={`pill ${live.group_type === "notebook" ? "pill-plum" : "pill-blue"} pill-clickable`}
                              onClick={() => onOpenDeck(live)}>
                              {g.name}
                              <GroupTypeBadge type={live.group_type} />
                            </span>
                          );
                        })}
                      </div>
                    </div>
                  )}
                  {isBare && (
                    <div style={{ textAlign: "center", color: "var(--t-text-3)", fontStyle: "italic", padding: "10px 0" }}>
                      Nothing else recorded for this todo.
                    </div>
                  )}
                </div>
              )}

              {/* The foot of an open todo, holding its actions the way a deck card's
                  bar holds Archive all and Delete all. */}
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
                      <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Units completed (optional)</div>
                      <input
                        value={editForm.numUnit}
                        onKeyDown={editKey(r)}
                        onChange={e => setEditForm(f => ({ ...f, numUnit: e.target.value }))}
                        placeholder="e.g. 5 pages, 2 articles, 4 chapters"
                        style={{ width: "100%", boxSizing: "border-box", padding: "5px 8px", border: "1px solid var(--t-border-2)", background: "var(--t-surface)", color: "var(--t-text)", fontSize: 13 }}
                      />
                    </div>
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
                    const keptGroups = r.groups.filter(x => editForm.groups.includes(x.row_id));
                    const keptLiveIds = keptGroups.filter(x => x.group_id != null).map(x => x.group_id);
                    const addableGroups = allGroups.filter(g => !keptLiveIds.includes(g.id));
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
                  <div style={{ fontSize: 11, color: "var(--t-text-3)", fontStyle: "italic" }}>
                    Resources, decks, and notebooks that have been deleted can be removed here, but they cannot be added back.
                  </div>
                  <div style={{ display: "flex", gap: 8 }}>
                    <button className="primary" onClick={() => saveEdit(r)}>Save</button>
                    <button onClick={cancelEdit}>Cancel</button>
                  </div>
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

// Root:

export default function Stats({ setToast, onNavigateToGroup, returnContext, onConsumeReturnContext }) {
  const [activePlans,    setActivePlans]   = useState([]);
  const [deletedPlans,   setDeletedPlans]  = useState([]);
  const [selectedPlanId, setSelectedPlanId] = useState(null);
  const [groupStats,     setGroupStats]    = useState([]);
  const [todoStats,      setTodoStats]     = useState([]);
  const [streakInfo,     setStreakInfo]    = useState({ streak: 0, studied_today: false });
  const [contentTab,     setContentTab]    = useState(() => returnContext?.contentTab ?? "decks");
  const [today,          setToday]         = useState(null);
  const [loading,        setLoading]       = useState(true);
  const [allGroups,      setAllGroups]     = useState([]);
  const [planDecks,      setPlanDecks]     = useState([]);
  const [planResources,  setPlanResources] = useState([]);

  useEffect(() => {
    loggedInvoke("get_current_date").then(setToday).catch(e => logError("catch", e));
    Promise.all([
      loggedInvoke("get_plans"),
      loggedInvoke("get_deleted_plan_ids"),
      loggedInvoke("get_groups"),
    ]).then(([ps, deleted, gs]) => {
      const dp = deleted.map(([id, name]) => ({ id, name }));
      setActivePlans(ps);
      setDeletedPlans(dp);
      setAllGroups(gs);
      const firstId = returnContext?.selectedPlanId ?? ps[0]?.id ?? dp[0]?.id ?? null;
      setSelectedPlanId(firstId);
      if (returnContext) onConsumeReturnContext();
      setLoading(false);
    }).catch(e => { logError("catch", e); setLoading(false); });
  }, []);

  const openDeck = (group) => {
    onNavigateToGroup(group, {
      menu: "stats",
      label: "Stats",
      statsContext: { selectedPlanId, contentTab },
    });
  };

  const loadStats = (planId) => {
    if (!planId) return;
    Promise.all([
      loggedInvoke("get_group_stats",     { planId }),
      loggedInvoke("get_todo_stats", { planId }),
      loggedInvoke("get_plan_streak",     { planId }),
      loggedInvoke("get_resources",       { planId }),
      // A deck in the plan that hasn't been studied has no stat rows to be found in,
      // so its card comes from plan membership instead
      loggedInvoke("get_plan_srs_groups", { planId }),
    ]).then(([gs, ts, si, res, srs]) => {
      setGroupStats(gs);
      setTodoStats(ts);
      setStreakInfo(si);
      setPlanResources(res);
      setPlanDecks(srs.map(([g]) => g).filter(g => g.group_type === "deck"));
    }).catch(e => { logError("catch", e); setToast("Failed to load stats.", "error"); });
  };

  const deleteDeletedPlan = async (planId) => {
    try {
      await loggedInvoke("delete_deleted_plan_stats", { planId });
      const freshDeleted = await loggedInvoke("get_deleted_plan_ids");
      const dp = freshDeleted.map(([id, name]) => ({ id, name }));
      setDeletedPlans(dp);
      if (selectedPlanId === planId) {
        const next = activePlans[0]?.id ?? dp[0]?.id ?? null;
        setSelectedPlanId(next);
      }
      setToast("Plan stats deleted.");
    } catch(e) {
      logError("catch", e);
      setToast("Failed to delete plan stats.", "error");
    }
  };

  useEffect(() => {
    if (!selectedPlanId) {
      setGroupStats([]);
      setTodoStats([]);
      setStreakInfo({ streak: 0, studied_today: false });
      setPlanResources([]);
      return;
    }
    loadStats(selectedPlanId);
  }, [selectedPlanId]);

  const metrics = computeMetrics(counted(groupStats), todoStats);
  const retColor = metrics.avgRetention !== null ? retentionColor(metrics.avgRetention) : GRAY;
  const atRisk = streakInfo.streak > 0 && !streakInfo.studied_today;

  return (
    <>
      <div className="st-root">
        <div className="st-header">
          <div style={{ flex: 1 }}>
            <h2>Stats</h2>
          </div>
        </div>
        <div className="st-body">
          {/* Plan selector */}
          <div style={{ display: "flex", alignItems: "flex-start", gap: 8, marginBottom: 16 }}>
            <div className="st-plan-bar" style={{ flex: 1, marginBottom: 0 }}>
              {activePlans.map(p => (
                <button
                  key={p.id}
                  className={`st-pill${selectedPlanId === p.id ? " active" : ""}`}
                  onClick={() => setSelectedPlanId(p.id)}
                >
                  {p.name}
                </button>
              ))}
              {deletedPlans.map(p => (
                <button
                  key={`d-${p.id}`}
                  className={`st-pill st-pill-dead st-pill-dead--deleted${selectedPlanId === p.id ? " active" : ""}`}
                  onClick={() => setSelectedPlanId(p.id)}
                >{p.name}</button>
              ))}
              {!loading && activePlans.length === 0 && deletedPlans.length === 0 && <span style={{ color: "var(--t-text-3)", fontSize: 13 }}>No plans yet.</span>}
            </div>
            {selectedPlanId && deletedPlans.some(p => p.id === selectedPlanId) && (
              <div style={{ flexShrink: 0 }}>
                <ConfirmDelete label="Delete All Stats" onConfirm={() => deleteDeletedPlan(selectedPlanId)} />
              </div>
            )}
          </div>

          {/* Summary metrics */}
          <div className="st-metrics">
            <MetricCard
              label="Avg. Retention"
              value={metrics.avgRetention !== null ? `${Math.round(metrics.avgRetention * 100)}%` : "-"}
              color={metrics.avgRetention !== null ? retColor : GRAY}
            />
            <MetricCard label="New Cards Seen" value={metrics.newCardsStudied} color="var(--t-blue)" />
            <MetricCard label="Total Cards Seen" value={metrics.totalCardsStudied} color="var(--t-blue)" />
            <MetricCard label="Todos Done"    value={metrics.todosDone}     color="var(--t-yellow)" />
            <MetricCard
              label="Study Streak"
              value={`${streakInfo.streak}d`}
              color={streakInfo.streak === 0 ? GRAY : atRisk ? AMBER : "var(--t-green)"}
            />
            <MetricCard label="Deck Study Time" value={fmtTime(metrics.studyMins)} color="var(--t-time)" />
            <MetricCard label="Todo Time"  value={fmtTime(metrics.todoMins)}  color="var(--t-time)" />
          </div>

          {/* Chart panel */}
          <ChartPanel groupStats={groupStats} todoStats={todoStats} today={today} />

          {/* Content tabs */}
          <div className="st-tabs">
            <button className={`st-tab st-tab--decks${contentTab === "decks" ? " active" : ""}`} onClick={() => setContentTab("decks")}>Decks</button>
            <button className={`st-tab st-tab--todos${contentTab === "todos" ? " active" : ""}`} onClick={() => setContentTab("todos")}>Todos</button>
          </div>

          {contentTab === "decks" && (
            <DeckSessionsTab
              groupStats={groupStats}
              planDecks={planDecks}
              planId={selectedPlanId}
              onDeleted={() => loadStats(selectedPlanId)}
              setToast={setToast}
            />
          )}
          {contentTab === "todos" && (
            <TodosTab
              todoStats={todoStats}
              today={today}
              onDeleted={() => loadStats(selectedPlanId)}
              setToast={setToast}
              allGroups={allGroups}
              planResources={planResources}
              onOpenDeck={openDeck}
            />
          )}
        </div>
      </div>
    </>
  );
}

