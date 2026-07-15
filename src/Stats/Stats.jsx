import { useState, useEffect } from "react";
import { loggedInvoke, logError } from "../logger";
import { ResourceCard, GroupTypeBadge, ConfirmDelete } from "../UIUtils";
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

function computeMetrics(groupStats, todoStats) {
  const studyMins = groupStats.reduce((s, r) => s + r.time_spent_minutes, 0);
  const todoMins  = todoStats.reduce((s, r) => s + r.time_spent_minutes, 0);
  // Sum of num_new = unique cards studied
  const cardsStudied = groupStats.reduce((s, r) => s + r.num_new, 0);
  const todosDone = todoStats.length;

  let totalP = 0, totalD = 0;
  groupStats.forEach(r => { totalP += r.num_promote; totalD += r.num_demote; });
  const avgRetention = (totalP + totalD) > 0 ? totalP / (totalP + totalD) : null;

  return { studyMins, todoMins, cardsStudied, todosDone, avgRetention };
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

function buildOverTimeData(groupStats, unit = "day") {
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

  const dates = Object.keys(byDate).sort();
  const labels = dates;

  const barData = {
    labels,
    datasets: [
      { label: "New",      data: dates.map(d => byDate[d].new),     backgroundColor: BLUE_BG,  stack: "s" },
      { label: "Promoted", data: dates.map(d => byDate[d].promote), backgroundColor: GREEN_BG, stack: "s" },
      { label: "Demoted",  data: dates.map(d => byDate[d].demote),  backgroundColor: RED_BG,   stack: "s" },
    ],
  };

  const retentionLabels = [];
  const retentionData   = [];
  dates.forEach(d => {
    const p = byDate[d].p;
    const total = p + byDate[d].d;
    if (total > 0) {
      retentionLabels.push(d);
      retentionData.push(Math.round((p / total) * 100));
    }
  });

  const lineData = {
    labels: retentionLabels,
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

  return { barData, lineData };
}

function buildTimeSpentData(groupStats, todoStats, unit = "day") {
  const byDate = {};
  const add = (r, kind) => {
    const key = bucketKey(r.date, unit);
    if (!byDate[key]) byDate[key] = { todo: 0, deck: 0 };
    byDate[key][kind] += r.time_spent_minutes;
  };
  todoStats.forEach(r => add(r, "todo"));
  groupStats.forEach(r => add(r, "deck"));

  const dates = Object.keys(byDate).sort();
  const toHours = m => Math.round((m / 60) * 10) / 10;

  return {
    labels: dates,
    datasets: [
      { label: "Todos", data: dates.map(d => toHours(byDate[d].todo)), backgroundColor: YELLOW_BG, stack: "s" },
      { label: "Decks", data: dates.map(d => toHours(byDate[d].deck)), backgroundColor: BLUE_BG,   stack: "s" },
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
  { label: "7d",  days: 7 },
  { label: "30d", days: 30 },
  { label: "90d", days: 90 },
  { label: "All", days: null },
];

function ChartPanel({ groupStats, todoStats }) {
  const [tab, setTab] = useState("bytime");
  const [range,  setRange]  = useState(30);
  const [offset, setOffset] = useState(0);

  // Snap back to the most recent window when the underlying data changes (e.g. plan switch)
  useEffect(() => setOffset(0), [groupStats, todoStats]);

  // Each chart windows over its own date domain
  function computeWindow(allDates) {
    const minDate = allDates[0] ?? null;
    const maxDate = allDates[allDates.length - 1] ?? null;
    let start = null, end = null;
    if (range !== null && maxDate) {
      end   = addDays(maxDate, -offset * range);
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
  const { barData, lineData } = buildOverTimeData(groupStats.filter(inWindow(overWin)), overWin.unit);

  const timeWin = computeWindow([...new Set([...groupStats, ...todoStats].map(r => r.date))].sort());
  const timeData = buildTimeSpentData(
    groupStats.filter(inWindow(timeWin)),
    todoStats.filter(inWindow(timeWin)),
    timeWin.unit,
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
                    {lineData.labels.length > 0 && (
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

function DeckSessionsTab({ groupStats, planId, onDeleted, setToast }) {
  const [deckFilter, setDeckFilter]   = useState("all");
  const [expanded, setExpanded]       = useState({});

  const deckNames = [...new Set(groupStats.map(r => r.group_name))]
    .sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));

  const visible = deckFilter === "all"
    ? groupStats
    : groupStats.filter(r => r.group_name === deckFilter);

  const byDeck = {};
  visible.forEach(r => {
    if (!byDeck[r.group_name]) byDeck[r.group_name] = [];
    byDeck[r.group_name].push(r);
  });

  const toggle = name => setExpanded(e => ({ ...e, [name]: !e[name] }));

  const deleteRow = async (id) => {
    try {
      await loggedInvoke("delete_group_stat", { id });
      setToast("Session deleted.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to delete session.", "error"); }
  };

  const deleteAll = async (groupName) => {
    try {
      await loggedInvoke("delete_group_stats_for_deck", { groupName, planId });
      setToast("Deck stats deleted.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to delete deck stats.", "error"); }
  };

  if (deckNames.length === 0) {
    return <div className="empty-bubble" style={{ marginTop: 16 }}>No deck sessions recorded yet.</div>;
  }

  const deletedNames = new Set(
    groupStats.filter(r => r.group_id === null).map(r => r.group_name)
  );

  return (
    <div>
      <div className="st-pills" style={{ marginBottom: 12 }}>
        <button className={`st-pill${deckFilter === "all" ? " active" : ""}`} onClick={() => setDeckFilter("all")}>All</button>
        {deckNames.map(n => {
          const deleted = deletedNames.has(n);
          const isActive = deckFilter === n;
          return (
            <button
              key={n}
              className={`st-pill${isActive ? " active" : ""}${deleted && !isActive ? " st-deck-pill-deleted" : ""}`}
              onClick={() => setDeckFilter(n)}>
              {n}
            </button>
          );
        })}
      </div>

      {deckNames.filter(name => byDeck[name]).map(name => {
        const rows = byDeck[name];
        const isOpen = !!expanded[name];
        const totalTime = rows.reduce((s, r) => s + r.time_spent_minutes, 0);
        const totalN    = rows.reduce((s, r) => s + r.num_new, 0);
        const totalP    = rows.reduce((s, r) => s + r.num_promote, 0);
        const totalD    = rows.reduce((s, r) => s + r.num_demote, 0);
        const avgRet    = (totalP + totalD) > 0 ? totalP / (totalP + totalD) : null;

        // Group rows by date for tinting
        const dateOrder = [];
        const dateToRows = {};
        rows.forEach(r => {
          if (!dateToRows[r.date]) { dateOrder.push(r.date); dateToRows[r.date] = []; }
          dateToRows[r.date].push(r);
        });

        const isDeleted = rows[0].group_id === null;

        return (
          <div key={name} className="st-deck-card">
            <div className="st-deck-header" onClick={() => toggle(name)} style={{ cursor: "pointer" }}>
              <span style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 8 }}>
                <span className="st-deck-name">{name}</span>
                {isDeleted && <span className="st-badge st-badge-deleted">Deleted</span>}
              </span>
              <span className="st-deck-meta">
                <span className="st-meta-pill">{rows.length} session{rows.length !== 1 ? "s" : ""}</span>
                <span className="st-meta-pill st-meta-pill--new">{totalN} new</span>
                <span className="st-meta-pill st-meta-pill--promote">+{totalP}</span>
                <span className="st-meta-pill st-meta-pill--demote">−{totalD}</span>
                {avgRet !== null && <span className={`st-meta-pill ${retentionPillClass(avgRet)}`}>{Math.round(avgRet * 100)}% ret.</span>}
                <span className="st-meta-pill st-meta-pill--time">{fmtTime(totalTime)}</span>
              </span>
              <span style={{ marginLeft: "auto", display: "flex", gap: 6, alignItems: "center" }} onClick={e => e.stopPropagation()}>
                <ConfirmDelete label="Delete all" small onConfirm={() => deleteAll(name)} />
                <span className="st-caret">{isOpen ? "▾" : "▸"}</span>
              </span>
            </div>

            {isOpen && (
              <table className="st-table">
                <colgroup>
                  <col /><col /><col /><col /><col /><col /><col />
                </colgroup>
                <thead>
                  <tr>
                    <th>Date</th>
                    <th style={{ color: "var(--t-blue)" }}>New</th>
                    <th style={{ color: "var(--t-green)" }}>Promoted</th>
                    <th style={{ color: "var(--t-red)" }}>Demoted</th>
                    <th>Retention</th>
                    <th>Time</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {dateOrder.map((date, di) => {
                    const dateRows = dateToRows[date];
                    const tinted = di % 2 === 1;
                    return dateRows.map(r => (
                      <tr key={r.id} style={tinted ? { background: "var(--t-surface-2)" } : {}}>
                        <td style={{ fontSize: 12, fontVariantNumeric: "tabular-nums" }}>{r.date}</td>
                        <td><span className="st-badge" style={{ background: "var(--t-blue-bg)", color: "var(--t-blue)" }}>{r.num_new}</span></td>
                        <td><span className="st-badge" style={{ background: "var(--t-green-bg)", color: "var(--t-green)" }}>{r.num_promote}</span></td>
                        <td><span className="st-badge" style={{ background: "var(--t-red-bg)", color: "var(--t-red)" }}>{r.num_demote}</span></td>
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
                        <td style={{ fontSize: 12, color: "var(--t-text-3)" }}>{fmtTime(r.time_spent_minutes)}</td>
                        <td style={{ minWidth: 70 }}><ConfirmDelete small onConfirm={() => deleteRow(r.id)} /></td>
                      </tr>
                    ));
                  })}
                </tbody>
              </table>
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

function TodosTab({ todoStats, today, onDeleted, setToast, allGroups, planResources, onOpenDeck }) {
  const [catFilter, setCatFilter] = useState("all");
  const [expanded,  setExpanded]  = useState({});
  const [dateFrom,  setDateFrom]  = useState("");
  const [dateTo,    setDateTo]    = useState("");
  const [editingId, setEditingId] = useState(null);
  const [editForm,  setEditForm]  = useState(null);

  const applyPreset = (days) => {
    if (days === null) { setDateFrom(""); setDateTo(""); return; }
    const d = new Date(today);
    d.setDate(d.getDate() - (days - 1));
    setDateFrom(d.toISOString().slice(0, 10));
    setDateTo(today);
  };

  let visible = todoStats;
  if (dateFrom) visible = visible.filter(r => r.date >= dateFrom);
  if (dateTo)   visible = visible.filter(r => r.date <= dateTo);
  if (catFilter !== "all") visible = visible.filter(r => parseCategories(r.category).includes(catFilter));

  const toggle = id => setExpanded(e => ({ ...e, [id]: !e[id] }));

  const deleteRow = async (id) => {
    try {
      await loggedInvoke("delete_todo_stat", { id });
      setToast("Todo entry deleted.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to delete todo entry.", "error"); }
  };

  const startEdit = (r) => {
    setEditingId(r.id);
    setEditForm({
      text: r.text,
      categoryMap: categoryStringToMap(r.category),
      details: r.details || "",
      timeSpent: r.time_spent_minutes,
      numUnit: r.num_unit || "",
      groups: r.groups.map(x => x.name),
      resources: r.resources.map(x => x.name),
      addGroupIds: [],
      addResourceIds: [],
    });
  };

  const cancelEdit = () => { setEditingId(null); setEditForm(null); };
  const editKey = (r) => (e) => { if (e.key === "Enter") saveEdit(r); if (e.key === "Escape") cancelEdit(); };

  const saveEdit = async (r) => {
    const trimmed = editForm.text.trim();
    if (!trimmed) return;
    const category = computeCategory(editForm.categoryMap);
    if (category === 0) { setToast("Select at least one category.", "error"); return; }
    const timeSpent = Math.max(0, Math.round(parseFloat(editForm.timeSpent) || 0));
    if (timeSpent <= 0) { setToast("Please log at least 1 minute.", "error"); return; }
    const removeGroups    = r.groups.map(x => x.name).filter(n => !editForm.groups.includes(n));
    const removeResources = r.resources.map(x => x.name).filter(n => !editForm.resources.includes(n));
    try {
      await loggedInvoke("update_todo_stat", {
        id: r.id,
        text: trimmed,
        category,
        details: editForm.details.trim() || null,
        timeSpentMinutes: timeSpent,
        numUnit: editForm.numUnit.trim() || null,
        removeGroupNames: removeGroups,
        removeResourceNames: removeResources,
        addGroupIds: editForm.addGroupIds,
        addResourceIds: editForm.addResourceIds,
      });
      setEditingId(null);
      setEditForm(null);
      setToast("Todo entry updated.");
      onDeleted();
    } catch (e) { logError("catch", e); setToast("Failed to update todo entry.", "error"); }
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
      <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap", marginBottom: 8 }}>
        <span style={{ fontSize: 12, color: "var(--t-text-3)", fontWeight: 500 }}>Date:</span>
        {[{ label: "All", days: null }, { label: "7d", days: 7 }, { label: "30d", days: 30 }, { label: "90d", days: 90 }].map(({ label, days }) => (
          <button key={label} className="st-btn-sm" onClick={() => applyPreset(days)}>{label}</button>
        ))}
        <input type="date" value={dateFrom} onChange={e => setDateFrom(e.target.value)}
          style={{ fontSize: 12, padding: "2px 4px", border: "1px solid var(--t-border)", borderRadius: "var(--t-r)" }} />
        <span style={{ fontSize: 12, color: "var(--t-text-3)" }}>-</span>
        <input type="date" value={dateTo} onChange={e => setDateTo(e.target.value)}
          style={{ fontSize: 12, padding: "2px 4px", border: "1px solid var(--t-border)", borderRadius: "var(--t-r)" }} />
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
                    {r.num_unit && <span className="st-meta-pill st-todo-unit" title={r.num_unit}>{r.num_unit}</span>}
                    <span className="st-meta-pill st-meta-pill--time">{fmtTime(r.time_spent_minutes)}</span>
                  </span>
                </div>
              </div>

              {isOpen && !isEditing && (
                <div className="st-todo-expanded">
                  {r.details && (
                    <div className="st-todo-section">
                      <div className="st-todo-section-label">Details</div>
                      <p className="st-todo-notes">{r.details}</p>
                    </div>
                  )}
                  {r.resources.length > 0 && (
                    <div className="st-todo-section">
                      <div className="st-todo-section-label">Tagged Resources</div>
                      <div className="st-resource-cards">
                        {r.resources.map((res, i) => <ResourceCard key={i} res={res} />)}
                      </div>
                    </div>
                  )}
                  {r.groups.length > 0 && (
                    <div className="st-todo-section">
                      <div className="st-todo-section-label">Tagged Decks/Notebooks</div>
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
                  <div style={{ display: "flex", gap: 8 }}>
                    <button className="st-btn-sm" onClick={() => startEdit(r)}>Edit</button>
                    <ConfirmDelete small onConfirm={() => deleteRow(r.id)} />
                  </div>
                </div>
              )}

              {isEditing && editForm && (
                <div className="st-todo-expanded" style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                  <div>
                    <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Name</div>
                    <input
                      value={editForm.text}
                      autoFocus
                      onKeyDown={editKey(r)}
                      onChange={e => setEditForm(f => ({ ...f, text: e.target.value }))}
                      style={{ width: "100%", boxSizing: "border-box", padding: "5px 8px", border: "1px solid var(--t-border-2)", borderRadius: "var(--t-r)", background: "var(--t-surface)", color: "var(--t-text)", fontSize: 13 }}
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
                        style={{ width: "100%", boxSizing: "border-box", padding: "5px 8px", border: "1px solid var(--t-border-2)", borderRadius: "var(--t-r)", background: "var(--t-surface)", color: "var(--t-text)", fontSize: 13 }}
                      />
                    </div>
                    <div style={{ flex: 1 }}>
                      <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Units completed (optional)</div>
                      <input
                        value={editForm.numUnit}
                        onKeyDown={editKey(r)}
                        onChange={e => setEditForm(f => ({ ...f, numUnit: e.target.value }))}
                        placeholder="e.g. 5 pages, 2 articles, 4 chapters"
                        style={{ width: "100%", boxSizing: "border-box", padding: "5px 8px", border: "1px solid var(--t-border-2)", borderRadius: "var(--t-r)", background: "var(--t-surface)", color: "var(--t-text)", fontSize: 13 }}
                      />
                    </div>
                  </div>
                  <div>
                    <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Details (optional)</div>
                    <textarea
                      value={editForm.details}
                      onChange={e => setEditForm(f => ({ ...f, details: e.target.value }))}
                      rows={3}
                      style={{ width: "100%", boxSizing: "border-box", padding: "5px 8px", border: "1px solid var(--t-border)", borderRadius: "var(--t-r)", background: "var(--t-surface)", color: "var(--t-text)", fontSize: 13, resize: "vertical", fontFamily: "inherit" }}
                    />
                  </div>
                  {(() => {
                    const addableResources = planResources.filter(pr => !editForm.resources.includes(pr.name));
                    if (editForm.resources.length === 0 && addableResources.length === 0) return null;
                    return (
                      <div>
                        <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Tagged Resources</div>
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                          {editForm.resources.map((res, i) => {
                            const dead = !planResources.some(pr => pr.name === res);
                            return (
                              <span key={i} className={`pill pill-clay${dead ? " pill-dead" : ""}`}>
                                {res}
                                <button onClick={() => setEditForm(f => ({ ...f, resources: f.resources.filter(n => n !== res) }))}
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
                    const keptLiveIds = r.groups
                      .filter(x => editForm.groups.includes(x.name) && x.group_id != null)
                      .map(x => x.group_id);
                    const addableGroups = allGroups.filter(g => !keptLiveIds.includes(g.id));
                    if (editForm.groups.length === 0 && addableGroups.length === 0) return null;
                    return (
                      <div>
                        <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Tagged Decks/Notebooks</div>
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                          {editForm.groups.map((g, i) => {
                            const info = r.groups.find(x => x.name === g);
                            const dead = info?.group_id == null;
                            const fam = info?.group_type === "notebook" ? "plum" : "blue";
                            return (
                              <span key={i} className={`pill pill-${fam}${dead ? " pill-dead" : ""}`}>
                                {g}
                                {info?.group_type && <GroupTypeBadge type={info.group_type} />}
                                <button onClick={() => setEditForm(f => ({ ...f, groups: f.groups.filter(n => n !== g) }))}
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
                  <div style={{ fontSize: 11, color: "var(--t-text-3)" }}>
                    Deleted materials can be removed here, but they cannot be added back.
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
    ]).then(([gs, ts, si, res]) => {
      setGroupStats(gs);
      setTodoStats(ts);
      setStreakInfo(si);
      setPlanResources(res);
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

  const metrics = computeMetrics(groupStats, todoStats);
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
                  className={`st-pill st-deck-pill-deleted${selectedPlanId === p.id ? " active" : ""}`}
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
              label="Avg. Card Retention"
              value={metrics.avgRetention !== null ? `${Math.round(metrics.avgRetention * 100)}%` : "-"}
              color={metrics.avgRetention !== null ? retColor : GRAY}
            />
            <MetricCard label="Cards Studied" value={metrics.cardsStudied} color="var(--t-blue)" />
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
          <ChartPanel groupStats={groupStats} todoStats={todoStats} />

          {/* Content tabs */}
          <div className="st-tabs">
            <button className={`st-tab st-tab--decks${contentTab === "decks" ? " active" : ""}`} onClick={() => setContentTab("decks")}>Decks</button>
            <button className={`st-tab st-tab--todos${contentTab === "todos" ? " active" : ""}`} onClick={() => setContentTab("todos")}>Todos</button>
          </div>

          {contentTab === "decks" && (
            <DeckSessionsTab
              groupStats={groupStats}
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

