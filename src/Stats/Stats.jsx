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

// ─── Constants ────────────────────────────────────────────────────────────────

// Themed palette — matches the app's cozy feature families (slate / forest / clay …)
const BLUE   = "#5A7A90";  // slate — new cards
const GREEN  = "#4A8C5E";  // forest — promoted / good retention
const RED    = "#B85454";  // terracotta — demoted / poor retention
const AMBER  = "#C49A44";  // amber — mid retention
const GRAY   = "#9A8488";  // warm grey — neutral

const BLUE_BG   = "rgba(90,122,144,0.78)";
const GREEN_BG  = "rgba(74,140,94,0.78)";
const RED_BG    = "rgba(184,84,84,0.78)";

// Category colors are defined once in PlanUtils and shared with Todos.
const CATEGORY_COLORS = CATEGORY_COLOR_BY_LABEL;

// ─── Helpers ──────────────────────────────────────────────────────────────────

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

function addDays(dateStr, n) {
  const d = new Date(dateStr + "T00:00:00Z");
  d.setUTCDate(d.getUTCDate() + n);
  return d.toISOString().slice(0, 10);
}

function parseCategories(catStr) {
  if (!catStr) return [];
  return catStr.split(",").map(s => s.trim()).filter(Boolean)
    // "Other" was renamed to "Culture" (bit 64) — alias old stat rows
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
  const cardsReviewed = groupStats.reduce((s, r) => s + r.num_new + r.num_promote + r.num_demote, 0);
  const todosDone = todoStats.length;

  let totalP = 0, totalD = 0;
  groupStats.forEach(r => { totalP += r.num_promote; totalD += r.num_demote; });
  const avgRetention = (totalP + totalD) > 0 ? totalP / (totalP + totalD) : null;

  return { studyMins, todoMins, cardsReviewed, todosDone, avgRetention };
}

// ─── Chart data builders ──────────────────────────────────────────────────────

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

  // Canonical category order first; any unrecognized legacy labels go last
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

// ─── Shared chart options ─────────────────────────────────────────────────────

// Caps how many date labels render as history grows; the bars themselves are unaffected.
const DATE_TICKS = { autoSkip: true, maxTicksLimit: 12, maxRotation: 30, font: { size: 10 } };

// Deck names are categorical: never skip a label (every bar stays identified), never
// rotate, truncate long names instead — tooltips still show the full name.
const DECK_TICKS = {
  autoSkip: false,
  maxRotation: 0,
  font: { size: 10 },
  callback(value) {
    const label = this.getLabelForValue(value);
    return label.length > 14 ? label.slice(0, 13) + "…" : label;
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

// ─── Metric card ─────────────────────────────────────────────────────────────

function MetricCard({ label, value, color }) {
  return (
    <div className="st-metric">
      <div className="st-metric-value" style={color ? { color } : {}}>{value}</div>
      <div className="st-metric-label">{label}</div>
    </div>
  );
}

// ─── Chart panel ─────────────────────────────────────────────────────────────

const RANGES = [
  { label: "7d",  days: 7 },
  { label: "30d", days: 30 },
  { label: "90d", days: 90 },
  { label: "All", days: null },
];

function ChartPanel({ groupStats, todoStats }) {
  const [tab, setTab] = useState("overtime");
  const [range,  setRange]  = useState(30);
  const [offset, setOffset] = useState(0);

  // Snap back to the most recent window when the underlying data changes (e.g. plan switch)
  useEffect(() => setOffset(0), [groupStats]);

  const allDates = [...new Set(groupStats.map(r => r.date))].sort();
  const minDate = allDates[0] ?? null;
  const maxDate = allDates[allDates.length - 1] ?? null;

  let windowStats = groupStats;
  let windowStart = null, windowEnd = null;
  if (range !== null && maxDate) {
    windowEnd   = addDays(maxDate, -offset * range);
    windowStart = addDays(windowEnd, -(range - 1));
    windowStats = groupStats.filter(r => r.date >= windowStart && r.date <= windowEnd);
  }

  // "All" keeps every datapoint but widens the unit so a lifetime of history stays
  // readable: raw days up to 90 days of span, weekly totals to ~18 months, then monthly.
  let unit = "day";
  if (range === null && minDate && maxDate) {
    const spanDays = (new Date(maxDate) - new Date(minDate)) / 86400000 + 1;
    if (spanDays > 548) unit = "month";
    else if (spanDays > 90) unit = "week";
  }

  const { barData, lineData } = buildOverTimeData(windowStats, unit);
  const byDeckData    = buildByDeckData(groupStats);
  const byCatData     = buildByCategoryData(todoStats);

  const canGoOlder = range !== null && minDate !== null && windowStart > minDate;
  const canGoNewer = offset > 0;

  const tabs = [
    { key: "overtime", label: "Over Time" },
    { key: "bydeck",   label: "By Deck"   },
    { key: "bycat",    label: "By Category" },
  ];

  const legend = (
    <span className="st-legend">
      <span className="st-legend-dot" style={{ background: BLUE  }} />New
      <span className="st-legend-dot" style={{ background: GREEN }} />Promoted
      <span className="st-legend-dot" style={{ background: RED   }} />Demoted
    </span>
  );

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
        {tab !== "bycat" && legend}
      </div>

      {tab === "overtime" && (
        groupStats.length === 0
          ? <div className="empty-bubble">No deck study data yet.</div>
          : <div>
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
                  unit !== "day" && (
                    <span style={{ marginLeft: "auto", fontSize: 11, color: "var(--t-text-3)" }}>
                      {unit === "week" ? "weekly" : "monthly"} totals
                    </span>
                  )
                ) : (
                  <span style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 6 }}>
                    <button className="st-btn-sm" disabled={!canGoOlder} style={!canGoOlder ? { opacity: 0.4 } : {}}
                      onClick={() => setOffset(o => o + 1)}>‹</button>
                    <span style={{ fontSize: 11, color: "var(--t-text-3)", fontVariantNumeric: "tabular-nums" }}>
                      {windowStart} – {windowEnd}
                    </span>
                    <button className="st-btn-sm" disabled={!canGoNewer} style={!canGoNewer ? { opacity: 0.4 } : {}}
                      onClick={() => setOffset(o => o - 1)}>›</button>
                  </span>
                )}
              </div>
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

// ─── Deck Sessions tab ────────────────────────────────────────────────────────

function DeckSessionsTab({ groupStats, planId, onDeleted }) {
  const [deckFilter, setDeckFilter]   = useState("all");
  const [expanded, setExpanded]       = useState({});

  const deckNames = [...new Set(groupStats.map(r => r.group_name))]
    .sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));

  const visible = deckFilter === "all"
    ? groupStats
    : groupStats.filter(r => r.group_name === deckFilter);

  // Group by deck name
  const byDeck = {};
  visible.forEach(r => {
    if (!byDeck[r.group_name]) byDeck[r.group_name] = [];
    byDeck[r.group_name].push(r);
  });

  const toggle = name => setExpanded(e => ({ ...e, [name]: !e[name] }));

  const deleteRow = async (id) => {
    await loggedInvoke("delete_group_stat", { id });
    onDeleted();
  };

  const deleteAll = async (groupName) => {
    await loggedInvoke("delete_group_stats_for_deck", { groupName, planId });
    onDeleted();
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
                <span>{rows.length} session{rows.length !== 1 ? "s" : ""}</span>
                <span style={{ color: "var(--t-blue)" }}>{totalN} new</span>
                <span style={{ color: "var(--t-green)" }}>+{totalP}</span>
                <span style={{ color: "var(--t-red)" }}>−{totalD}</span>
                {avgRet !== null && <span style={{ color: retentionColor(avgRet) }}>{Math.round(avgRet * 100)}% ret.</span>}
                <span>{fmtTime(totalTime)}</span>
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
                                <span className="st-ret-pct" style={{ color: "var(--t-text-3)" }}>—</span>
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

// ─── Todos tab ────────────────────────────────────────────────────────────────
// Derived from PlanUtils so the filter pills match every category picker's order.
const ALL_CATEGORIES = CATEGORIES.map(c => c.label);

function TodosTab({ todoStats, today, onDeleted, setToast, allGroups, onOpenDeck }) {
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
    await loggedInvoke("delete_todo_stat", { id });
    onDeleted();
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
    });
  };

  const cancelEdit = () => { setEditingId(null); setEditForm(null); };
  const editKey = (r) => (e) => { if (e.key === "Enter") saveEdit(r); if (e.key === "Escape") cancelEdit(); };

  const saveEdit = async (r) => {
    const trimmed = editForm.text.trim();
    if (!trimmed) return;
    const category = computeCategory(editForm.categoryMap);
    if (category === 0) { setToast("Select at least one category.", "error"); return; }
    const timeSpent = Math.max(0, parseFloat(editForm.timeSpent) || 0);
    if (timeSpent <= 0) { setToast("Please log at least 1 minute.", "error"); return; }
    const removeGroups    = r.groups.map(x => x.name).filter(n => !editForm.groups.includes(n));
    const removeResources = r.resources.map(x => x.name).filter(n => !editForm.resources.includes(n));
    await loggedInvoke("update_todo_stat", {
      id: r.id,
      text: trimmed,
      category,
      details: editForm.details.trim() || null,
      timeSpentMinutes: timeSpent,
      numUnit: editForm.numUnit.trim() || null,
      removeGroupNames: removeGroups,
      removeResourceNames: removeResources,
    });
    setEditingId(null);
    setEditForm(null);
    onDeleted();
  };

  if (todoStats.length === 0) {
    return <div className="empty-bubble" style={{ marginTop: 16 }}>No todo history recorded yet.</div>;
  }

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
          : visible.map(r => {
          const isOpen    = !!expanded[r.id];
          const isEditing = editingId === r.id;
          const cats      = parseCategories(r.category);
          return (
            <div key={r.id} className="st-todo-row">
              <div className="st-todo-collapsed" onClick={() => toggle(r.id)} style={{ cursor: "pointer" }}>
                <span className="st-todo-text">{r.text}</span>
                <span style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
                  {cats.map(c => (
                    <span key={c} className="st-pill-tag" style={{ background: CATEGORY_COLORS[c] || GRAY, color: "var(--t-btn-fg)" }}>{c}</span>
                  ))}
                  {r.num_unit && <span className="st-pill-tag" style={{ background: "var(--t-surface-2)", color: "var(--t-text-2)" }}>{r.num_unit}</span>}
                </span>
                <span style={{ fontSize: 11, color: "var(--t-text-3)", fontVariantNumeric: "tabular-nums" }}>{r.date}</span>
                <span style={{ fontSize: 12, color: "var(--t-text-3)" }}>{fmtTime(r.time_spent_minutes)}</span>
                <button
                  className="st-btn-sm"
                  onClick={e => { e.stopPropagation(); if (isEditing) cancelEdit(); else { setExpanded(ex => ({ ...ex, [r.id]: true })); startEdit(r); } }}
                  style={{ marginLeft: "auto" }}
                >{isEditing ? "Cancel" : "Edit"}</button>
                <span className="st-caret">{isOpen ? "▾" : "▸"}</span>
              </div>

              {isOpen && !isEditing && (
                <div className="st-todo-expanded">
                  {r.details && <p className="st-todo-notes">{r.details}</p>}
                  {r.groups.length > 0 && (
                    <div className="st-todo-section">
                      <div className="st-todo-section-label">Study Materials</div>
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
                  {r.resources.length > 0 && (
                    <div className="st-todo-section">
                      <div className="st-todo-section-label">Resources</div>
                      <div className="st-resource-cards">
                        {r.resources.map((res, i) => <ResourceCard key={i} res={res} />)}
                      </div>
                    </div>
                  )}
                  <ConfirmDelete small onConfirm={() => deleteRow(r.id)} />
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
                    <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Notes</div>
                    <textarea
                      value={editForm.details}
                      onChange={e => setEditForm(f => ({ ...f, details: e.target.value }))}
                      rows={3}
                      style={{ width: "100%", boxSizing: "border-box", padding: "5px 8px", border: "1px solid var(--t-border)", borderRadius: "var(--t-r)", background: "var(--t-surface)", color: "var(--t-text)", fontSize: 13, resize: "vertical", fontFamily: "inherit" }}
                    />
                  </div>
                  {editForm.groups.length > 0 && (
                    <div>
                      <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Study Materials</div>
                      <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                        {editForm.groups.map((g, i) => {
                          const info = r.groups.find(x => x.name === g);
                          const fam = info?.group_type === "notebook" ? "plum" : "blue";
                          return (
                            <span key={i} className={`pill pill-${fam}`}>
                              {g}
                              {info?.group_type && <GroupTypeBadge type={info.group_type} />}
                              <button onClick={() => setEditForm(f => ({ ...f, groups: f.groups.filter(n => n !== g) }))}
                                style={{ background: "none", border: "none", cursor: "pointer", padding: 0, lineHeight: 1, color: "inherit", fontSize: 12 }}>×</button>
                            </span>
                          );
                        })}
                      </div>
                    </div>
                  )}
                  {editForm.resources.length > 0 && (
                    <div>
                      <div style={{ fontSize: 11, color: "var(--t-text-3)", marginBottom: 4 }}>Resources</div>
                      <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                        {editForm.resources.map((res, i) => (
                          <span key={i} className="pill pill-clay">
                            {res}
                            <button onClick={() => setEditForm(f => ({ ...f, resources: f.resources.filter(n => n !== res) }))}
                              style={{ background: "none", border: "none", cursor: "pointer", padding: 0, lineHeight: 1, color: "inherit", fontSize: 12 }}>×</button>
                          </span>
                        ))}
                      </div>
                    </div>
                  )}
                  <div style={{ display: "flex", gap: 8 }}>
                    <button className="st-btn-sm" onClick={() => saveEdit(r)}
                      style={{ border: "1px solid var(--t-stat-bdr)", color: "var(--t-stat)", background: "var(--t-stat-bg)" }}
                    >Save</button>
                    <button className="st-btn-sm" onClick={cancelEdit}>Cancel</button>
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ─── Root ─────────────────────────────────────────────────────────────────────

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
    ]).then(([gs, ts, si]) => {
      setGroupStats(gs);
      setTodoStats(ts);
      setStreakInfo(si);
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
              value={metrics.avgRetention !== null ? `${Math.round(metrics.avgRetention * 100)}%` : "—"}
              color={metrics.avgRetention !== null ? retColor : GRAY}
            />
            <MetricCard label="Cards Studied" value={metrics.cardsReviewed} color="var(--t-blue)" />
            <MetricCard label="Todos Done"    value={metrics.todosDone}     color="var(--t-yellow)" />
            <MetricCard
              label="Study Streak"
              value={`${streakInfo.streak}d`}
              color={streakInfo.streak === 0 ? GRAY : atRisk ? AMBER : "var(--t-green)"}
            />
            <MetricCard label="Deck Study Time" value={fmtTime(metrics.studyMins)} color="var(--t-text-2)" />
            <MetricCard label="Todo Time"  value={fmtTime(metrics.todoMins)}  color="var(--t-text-2)" />
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
            />
          )}
          {contentTab === "todos" && (
            <TodosTab
              todoStats={todoStats}
              today={today}
              onDeleted={() => loadStats(selectedPlanId)}
              setToast={setToast}
              allGroups={allGroups}
              onOpenDeck={openDeck}
            />
          )}
        </div>
      </div>
    </>
  );
}

