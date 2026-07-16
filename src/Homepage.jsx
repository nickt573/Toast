import { useState, useEffect, useRef } from "react";
import { loggedInvoke, logError } from "./logger";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getVersion } from "@tauri-apps/api/app";

import { CardFace, renderAnkiHtml, stripAudioTags } from "./Decks/CardFace";
import { ResourceCard, GroupTypeBadge } from "./UIUtils";
import { computeCategory, maskToCategories, CategoryPicker, CategoryPills } from "./Plans/PlanUtils";
import "./Homepage.css";

// Shared pill helpers

function GroupPill({ group, onClick }) {
    const colorClass = group.group_type === "notebook" ? "pill-plum" : "pill-blue";
    return (
        <span
            className={`pill ${colorClass}${onClick ? " pill-clickable" : ""}`}
            onClick={onClick}
        >
            {group.name}
            <GroupTypeBadge type={group.group_type} />
        </span>
    );
}

function ResourcePill({ resource }) {
    return (
        <span
            className={`pill pill-clay${resource.url ? " pill-clickable" : ""}`}
            onClick={() => resource.url && openUrl(resource.url.startsWith("http") ? resource.url : `https://${resource.url}`)}
        >
            {resource.name}{resource.url && <span style={{ opacity: 0.55, marginLeft: 2, fontSize: 9 }}>↗</span>}
        </span>
    );
}

const DEFAULT_CATEGORY = () => ({ 1: false, 2: false, 4: false, 8: false, 16: false, 32: false, 64: false });

// Study Timer
// Module-level so timers survive navigation.
// Elapsed time goes to localStorage (not the DB) so closing the app pauses each timer and it restores on relaunch.

export const TIMER_STORE_KEY = "toast-study-timers";

function loadStudyTimers() {
    try {
        const stored = JSON.parse(localStorage.getItem(TIMER_STORE_KEY)) || {};
        const timers = {};
        Object.entries(stored).forEach(([planId, ms]) => {
            timers[planId] = { accumulatedMs: ms, runningSince: null };
        });
        return timers;
    } catch { return {}; }
}

const studyTimers = loadStudyTimers();
let timerPersistInterval = null;

function getStudyTimer(planId) {
    if (!studyTimers[planId]) studyTimers[planId] = { accumulatedMs: 0, runningSince: null };
    return studyTimers[planId];
}

function timerElapsedMs(planId) {
    const t = getStudyTimer(planId);
    return t.accumulatedMs + (t.runningSince ? Date.now() - t.runningSince : 0);
}

function persistStudyTimers() {
    const out = {};
    Object.keys(studyTimers).forEach(planId => {
        const ms = timerElapsedMs(planId);
        if (ms > 0) out[planId] = ms;
    });
    try { localStorage.setItem(TIMER_STORE_KEY, JSON.stringify(out)); } catch { /* storage full/unavailable */ }
}

// Persist on every action and on a heartbeat while running so a closed app loses at most a few seconds.
function syncTimerPersistence() {
    persistStudyTimers();
    const anyRunning = Object.values(studyTimers).some(t => t.runningSince !== null);
    if (anyRunning && !timerPersistInterval) {
        timerPersistInterval = setInterval(persistStudyTimers, 5000);
    } else if (!anyRunning && timerPersistInterval) {
        clearInterval(timerPersistInterval);
        timerPersistInterval = null;
    }
}

function startStudyTimer(planId) {
    const t = getStudyTimer(planId);
    if (!t.runningSince) t.runningSince = Date.now();
    syncTimerPersistence();
}

function pauseStudyTimer(planId) {
    const t = getStudyTimer(planId);
    if (t.runningSince) {
        t.accumulatedMs += Date.now() - t.runningSince;
        t.runningSince = null;
    }
    syncTimerPersistence();
}

function resetStudyTimer(planId) {
    studyTimers[planId] = { accumulatedMs: 0, runningSince: null };
    syncTimerPersistence();
}

// Todo time is always whole minutes
function timerMinutesRounded(planId) {
    return Math.round(timerElapsedMs(planId) / 60000);
}

function StudyTimer({ planId }) {
    const [, setTick] = useState(0);
    const running = getStudyTimer(planId).runningSince !== null;

    useEffect(() => {
        if (!running) return;
        const interval = setInterval(() => setTick(t => t + 1), 500);
        return () => clearInterval(interval);
    }, [running, planId]);

    function toggle() {
        if (running) pauseStudyTimer(planId);
        else startStudyTimer(planId);
        setTick(t => t + 1);
    }

    function clear() {
        resetStudyTimer(planId);
        setTick(t => t + 1);
    }

    const elapsedMs = timerElapsedMs(planId);
    const totalSec = Math.floor(elapsedMs / 1000);
    const h = Math.floor(totalSec / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    const s = totalSec % 60;
    const display = h > 0
        ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
        : `${m}:${String(s).padStart(2, "0")}`;

    // A stopped timer with accumulated time is paused, not fresh. It shows Resume instead of Start.
    const paused = !running && elapsedMs > 0;
    const toggleClass = running ? "hp-timer-btn--pause" : paused ? "hp-timer-btn--resume" : "hp-timer-btn--start";

    return (
        <div className="hp-timer">
            <span className={`hp-timer-display${running ? " running" : ""}`}>{display}</span>
            <button className={`hp-timer-btn ${toggleClass}`} onClick={toggle}>
                {running ? "Pause" : paused ? "Resume" : "Start"}
            </button>
            <button className="hp-timer-btn hp-timer-btn--clear" onClick={clear}>Clear</button>
        </div>
    );
}


// Grade Buttons

function GradeButtons({ onGrade, card }) {
    // Rendering without a card would throw and unmount the whole app
    if (!card) return null;
    const gradeDeltas = card.tier > 0 ? [
        { label: "Nope",  tierDelta: -2, grade: 0, cls: "hp-grade-nope",  easeDelta: -0.12 },
        { label: "Rough", tierDelta: -1, grade: 1, cls: "hp-grade-rough", easeDelta: -0.08 },
        { label: "Fine",  tierDelta:  1, grade: 2, cls: "hp-grade-fine",  easeDelta: -0.08, easeFloorZero: true },
        { label: "Easy",  tierDelta:  1, grade: 3, cls: "hp-grade-easy",  easeDelta:  0.10 },
    ] : [
        { label: "One More Time", tierDelta: -1, grade: 4, cls: "hp-grade-omt",   easeDelta: -0.05 },
        { label: "Got It",        tierDelta:  1, grade: 5, cls: "hp-grade-gotit", easeDelta:  0.00 },
    ];

    function calcNextSequence(tierDelta, easeDelta, easeFloorZero) {
        if (!card) return null;
        const newTier = Math.min(10, Math.max(card.tier === 0 ? 0 : 1, card.tier + tierDelta));
        // Fine never pushes ease below 0 or deepens an already-negative ease.
        const easeFloor = easeFloorZero ? Math.min(0, card.ease) : -0.35;
        const newEase = Math.max(easeFloor, Math.min(0.35, card.ease + easeDelta));
        if (newTier === 0) return { base: 0, span: 0 };
        const raw = Math.pow(2, newTier - 1) * (1 + newEase);
        // Mirror the backend +-15% fuzz so the preview shows the range, not a false exact day.
        return { base: Math.round(raw), span: Math.round(raw * 0.15) };
    }

    return (
        <div className="hp-grade-bar">
            {gradeDeltas.map(({ label, tierDelta, grade, cls, easeDelta, easeFloorZero }) => {
                const nextSeq = calcNextSequence(tierDelta, easeDelta, easeFloorZero);
                return (
                    <button key={grade} onClick={() => onGrade(grade)} className={`hp-grade-btn ${cls}`}>
                        <span>{label}</span>
                        {nextSeq !== null && (
                            <span className="hp-grade-btn-interval">
                                {nextSeq.base === 0
                                    ? "Again"
                                    : nextSeq.span > 0
                                        ? `${Math.max(1, nextSeq.base - nextSeq.span)}-${nextSeq.base + nextSeq.span}d`
                                        : `${nextSeq.base}d`}
                            </span>
                        )}
                    </button>
                );
            })}
        </div>
    );
}

// Similar Items Navigator

function SimilarFace({ item, side }) {
    return (
        <div className={`hp-similar-face hp-similar-face-${side}`}>
            {item.is_uploaded ? (
                <div dangerouslySetInnerHTML={{ __html: renderAnkiHtml(stripAudioTags(item[side])) }} />
            ) : (
                <div style={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>{item[side]}</div>
            )}
        </div>
    );
}

function SimilarNavigator({ items, frontCount = 0, groupType }) {
    const [idx, setIdx] = useState(0);
    if (!items || items.length === 0) return null;
    const item = items[idx];
    const matchSide = idx < frontCount ? "front" : "back";

    return (
        <div className="hp-similar">
            <div className="hp-similar-hdr">
                <span>Similar</span>
                <span style={{ color: "var(--t-text-3)", fontWeight: 400, textTransform: "none", letterSpacing: 0 }}>
                    {idx + 1} / {items.length}
                </span>
                <span className={`pill pill-${matchSide === "front" ? "plum" : "pink"}`} style={{ textTransform: "lowercase", fontSize: 10 }}>
                    {matchSide}
                </span>
            </div>
            <div className="hp-similar-content">
                {groupType === "deck" ? (
                    <>
                        <SimilarFace item={item} side="front" />
                        <div className="hp-similar-divider" />
                        <SimilarFace item={item} side="back" />
                    </>
                ) : (
                    <>
                        <div className="hp-similar-front">{item.title}</div>
                        {item.description && <div className="hp-similar-back">{item.description}</div>}
                    </>
                )}
            </div>
            <div className="hp-similar-nav">
                <button onClick={() => setIdx(i => (i - 1 + items.length) % items.length)}>‹</button>
                <button onClick={() => setIdx(i => (i + 1) % items.length)}>›</button>
            </div>
        </div>
    );
}

// Todo Complete Popup

function TodoCompletePopup({ todo, todoGroups, todoResources, planResources, allGroups, onConfirm, onCancel, onNavigateToGroup, initialTime = 0 }) {
    const [text, setText] = useState(todo.text);
    const [timeSpent, setTimeSpent] = useState(initialTime);
    const [numUnit, setNumUnit] = useState("");
    const [details, setDetails] = useState("");
    const [categoryMap, setCategoryMap] = useState(() => maskToCategories(todo.category ?? 64));
    const [selectedResourceIds, setSelectedResourceIds] = useState(() => todoResources.map(r => r.id));
    const [selectedGroupIds, setSelectedGroupIds] = useState(() => todoGroups.map(g => g.id));

    function toggleResource(id) {
        setSelectedResourceIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
    }
    function toggleGroup(id) {
        setSelectedGroupIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
    }
    function toggleCategory(bit) {
        setCategoryMap(prev => ({ ...prev, [bit]: !prev[bit] }));
    }

    return (
        <div className="hp-overlay">
            <div className="hp-popup">
                <div className="hp-popup-title">Complete Todo</div>

                <div>
                    <div className="hp-popup-label">What did you do?</div>
                    <input value={text} onChange={(e) => setText(e.target.value)}
                        placeholder="e.g. Read extra grammar notes"
                        style={{ width: "100%" }} />
                </div>

                <div>
                    <div className="hp-popup-label">Categories</div>
                    <CategoryPicker categoryMap={categoryMap} onChange={toggleCategory} />
                </div>

                {planResources.length > 0 && (
                    <div>
                        <div className="hp-popup-label">Resources</div>
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                            {planResources.map(r => (
                                <label key={r.id} className={`picker-pill${selectedResourceIds.includes(r.id) ? " active-resource" : ""}`}>
                                    <input type="checkbox" checked={selectedResourceIds.includes(r.id)}
                                        onChange={() => toggleResource(r.id)} style={{ margin: 0 }} />
                                    {r.name}
                                </label>
                            ))}
                        </div>
                    </div>
                )}

                {allGroups.length > 0 && (
                    <div>
                        <div className="hp-popup-label">Decks / Notebooks</div>
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                            {allGroups.map(g => (
                                <label key={g.id} className={`picker-pill${selectedGroupIds.includes(g.id) ? (g.group_type === "notebook" ? " active-notebook" : " active-deck") : ""}`}>
                                    <input type="checkbox" checked={selectedGroupIds.includes(g.id)}
                                        onChange={() => toggleGroup(g.id)} style={{ margin: 0 }} />
                                    {g.name}
                                    <GroupTypeBadge type={g.group_type} />
                                </label>
                            ))}
                        </div>
                    </div>
                )}

                <div>
                    <div className="hp-popup-label">Time spent (minutes)</div>
                    <input type="number" min={1} step={1} value={timeSpent}
                        onChange={(e) => setTimeSpent(e.target.value)} placeholder="0"
                        style={{ width: "100%" }} />
                </div>

                <div>
                    <div className="hp-popup-label">Units completed (optional)</div>
                    <input value={numUnit} onChange={(e) => setNumUnit(e.target.value)}
                        placeholder="e.g. 5 pages, 2 articles, 4 chapters"
                        style={{ width: "100%" }} />
                </div>

                <div>
                    <div className="hp-popup-label">Details (optional)</div>
                    <textarea value={details} onChange={(e) => setDetails(e.target.value)}
                        placeholder="Any extra notes about what you did" rows={3}
                        style={{ width: "100%", fontFamily: "inherit" }} />
                </div>

                <div className="hp-popup-actions">
                    <button onClick={onCancel}>Cancel</button>
                    <button
                        className="primary"
                        onClick={() => onConfirm(
                            Math.round(parseFloat(timeSpent) || 0),
                            numUnit !== "" ? numUnit : null,
                            details !== "" ? details : null,
                            selectedResourceIds,
                            selectedGroupIds,
                            computeCategory(categoryMap),
                            text,
                        )}>
                        Done
                    </button>
                </div>
            </div>
        </div>
    );
}

// Free Todo Popup

function FreeTodoPopup({ planId, planResources, allGroups, todos = [], onConfirm, onCancel, setToast, initialTime = 0 }) {
    const [mode, setMode] = useState(todos.length > 0 ? "choose" : "form");
    const [text, setText] = useState("");
    const [timeSpent, setTimeSpent] = useState(initialTime);
    const [numUnit, setNumUnit] = useState("");
    const [details, setDetails] = useState("");
    const [selectedGroupIds, setSelectedGroupIds] = useState([]);
    const [selectedResourceIds, setSelectedResourceIds] = useState([]);
    const [categoryMap, setCategoryMap] = useState(DEFAULT_CATEGORY());
    const [today, setToday] = useState("");
    const [date, setDate] = useState("");

    useEffect(() => {
        loggedInvoke("get_current_date")
            .then(d => { setToday(d); setDate(d); })
            .catch(e => logError("catch", e));
    }, []);

    function toggleGroup(id) {
        setSelectedGroupIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
    }
    function toggleResource(id) {
        setSelectedResourceIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
    }
    function toggleCategory(bit) {
        setCategoryMap(prev => ({ ...prev, [bit]: !prev[bit] }));
    }

    function startBlank() {
        setText("");
        setCategoryMap(DEFAULT_CATEGORY());
        setSelectedGroupIds([]);
        setSelectedResourceIds([]);
        setMode("form");
    }

    // Autofill only. The logged stat never stores the todo's id.
    async function pickTodo(todo) {
        setText(todo.text);
        setCategoryMap(maskToCategories(todo.category ?? 64));
        try {
            const [g, r] = await Promise.all([
                loggedInvoke("get_todo_groups", { todoId: todo.id }),
                loggedInvoke("get_todo_resources", { todoId: todo.id }),
            ]);
            setSelectedGroupIds(g.map(x => x.id));
            setSelectedResourceIds(r.map(x => x.id));
        } catch (e) { logError("catch", e); }
        setMode("form");
    }

    async function submit() {
        if (!text.trim()) { setToast("Please enter a todo name."); return; }
        const category = computeCategory(categoryMap);
        await onConfirm({
            text: text.trim(),
            category,
            details: details || null,
            timeSpent: Math.round(parseFloat(timeSpent) || 0),
            numUnit: numUnit || null,
            groupIds: selectedGroupIds,
            resourceIds: selectedResourceIds,
            date: date && date !== today ? date : null,
        });
    }

    if (mode === "choose") {
        return (
            <div className="hp-overlay">
                {/* Keyed per mode: reusing the scrolled DOM node skews the short prescreen */}
                <div className="hp-popup" key="choose">
                    <div className="hp-popup-title">Log Extra Activity</div>
                    <button className="primary" onClick={startBlank}>Create My Own</button>
                    <div>
                        <div className="hp-popup-label">Or choose an existing todo</div>
                        <div style={{ display: "flex", flexDirection: "column", gap: 6, maxHeight: 260, overflowY: "scroll" }}>
                            {todos.map(t => (
                                <button key={t.id} className="hp-free-todo-option" title={t.text} onClick={() => pickTodo(t)}>{t.text}</button>
                            ))}
                        </div>
                    </div>
                    <div className="hp-popup-actions">
                        <button onClick={onCancel}>Cancel</button>
                    </div>
                </div>
            </div>
        );
    }

    return (
        <div className="hp-overlay">
            <div className="hp-popup" key="form">
                <div className="hp-popup-title">Log Extra Activity</div>

                <div>
                    <div className="hp-popup-label">Date</div>
                    <input type="date" value={date} max={today}
                        onChange={(e) => setDate(e.target.value > today ? today : e.target.value)}
                        style={{ width: "100%" }} />
                </div>

                <div>
                    <div className="hp-popup-label">What did you do?</div>
                    <input value={text} onChange={(e) => setText(e.target.value)}
                        placeholder="e.g. Read extra grammar notes"
                        style={{ width: "100%" }} />
                </div>

                <div>
                    <div className="hp-popup-label">Categories</div>
                    <CategoryPicker categoryMap={categoryMap} onChange={toggleCategory} />
                </div>

                {planResources.length > 0 && (
                    <div>
                        <div className="hp-popup-label">Resources</div>
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                            {planResources.map(r => (
                                <label key={r.id} className={`picker-pill${selectedResourceIds.includes(r.id) ? " active-resource" : ""}`}>
                                    <input type="checkbox" checked={selectedResourceIds.includes(r.id)}
                                        onChange={() => toggleResource(r.id)} style={{ margin: 0 }} />
                                    {r.name}
                                </label>
                            ))}
                        </div>
                    </div>
                )}

                {allGroups.length > 0 && (
                    <div>
                        <div className="hp-popup-label">Decks / Notebooks</div>
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                            {allGroups.map(g => (
                                <label key={g.id} className={`picker-pill${selectedGroupIds.includes(g.id) ? (g.group_type === "notebook" ? " active-notebook" : " active-deck") : ""}`}>
                                    <input type="checkbox" checked={selectedGroupIds.includes(g.id)}
                                        onChange={() => toggleGroup(g.id)} style={{ margin: 0 }} />
                                    {g.name}
                                    <GroupTypeBadge type={g.group_type} />
                                </label>
                            ))}
                        </div>
                    </div>
                )}

                <div>
                    <div className="hp-popup-label">Time spent (minutes)</div>
                    <input type="number" min={1} step={1} value={timeSpent}
                        onChange={(e) => setTimeSpent(e.target.value)} placeholder="0"
                        style={{ width: "100%" }} />
                </div>

                <div>
                    <div className="hp-popup-label">Units completed (optional)</div>
                    <input value={numUnit} onChange={(e) => setNumUnit(e.target.value)}
                        placeholder="e.g. 5 pages, 2 articles, 4 chapters"
                        style={{ width: "100%" }} />
                </div>

                <div>
                    <div className="hp-popup-label">Details (optional)</div>
                    <textarea value={details} onChange={(e) => setDetails(e.target.value)}
                        placeholder="Any extra notes about what you did"
                        rows={3}
                        style={{ width: "100%", fontFamily: "inherit" }} />
                </div>

                <div className="hp-popup-actions">
                    {todos.length > 0 && (
                        <button style={{ marginRight: "auto" }} onClick={() => setMode("choose")}>← Back</button>
                    )}
                    <button onClick={onCancel}>Cancel</button>
                    <button className="primary" onClick={submit}>Log</button>
                </div>
            </div>
        </div>
    );
}

// SRS Study Session

function StudySession({ group, onBack, setToast }) {
    const [card, setCard] = useState(null);
    const [flipped, setFlipped] = useState(false);
    const [workspace, setWorkspace] = useState("");
    const [similarItems, setSimilarItems] = useState({ front: [], back: [] });
    const [newCount, setNewCount] = useState(0);
    const [reviewCount, setReviewCount] = useState(0);
    const [done, setDone] = useState(false);
    const lastShownId = useRef(null);
    const lastFlush = useRef(Date.now());
    // Drops re-entrant grades so a double-press can't grade the same card twice
    const grading = useRef(false);
    const isCard = group.group_type === "deck";

    async function fetchNext() {
        try {
            const counts = await loggedInvoke("count_due_items", { groupId: group.id });
            const totalDue = counts[0] + counts[1];
            const next = await loggedInvoke("get_next_due_card", {
                groupId: group.id,
                excludeId: totalDue > 1 ? lastShownId.current : null,
            });
            setNewCount(counts[0]);
            setReviewCount(counts[1]);
            if (!next) { setDone(true); return; }
            lastShownId.current = next.id;
            setCard(next);
            setFlipped(false);
            setWorkspace("");
            setSimilarItems({ front: [], back: [] });
        } catch (e) { logError("catch", e); setToast("Failed to fetch next item.", "error"); }
    }

    async function handleFlip() {
        if (!isCard) throw new Error("Attempted Notebook SRS");
        const itemId = card?.id;
        if (!itemId) return;
        setFlipped(true);
        try {
            const similar = await loggedInvoke("get_similar_cards", { itemId });
            setSimilarItems(similar);
        } catch (e) { logError("catch", e); }
    }

    async function handleGrade(grade) {
        if (!isCard) throw new Error("Attempted Notebook SRS");
        const itemId = card?.id;
        if (!itemId || grading.current) return;
        grading.current = true;
        try {
            await loggedInvoke("grade_item", { itemId, grade });
            await fetchNext();
        } catch (e) { logError("catch", e); setToast("Failed to grade card", "error"); }
        finally { grading.current = false; }
    }

    async function flushTime() {
        const now = Date.now();
        const elapsed = (now - lastFlush.current) / 60000;
        if (elapsed > 0.1) {
            lastFlush.current = now;
            try { await loggedInvoke("add_group_time", { groupId: group.id, minutes: elapsed }); }
            catch (e) { logError("catch", e); }
        }
    }

    async function handleBack() {
        await flushTime();
        onBack();
    }

    useEffect(() => {
        fetchNext();
        const interval = setInterval(flushTime, 20000);
        const onVisibility = () => { if (document.hidden) flushTime(); };
        document.addEventListener("visibilitychange", onVisibility);
        return () => {
            clearInterval(interval);
            document.removeEventListener("visibilitychange", onVisibility);
        };
    }, []);

    // WKWebView sometimes drops the repaint after a full card swap. Force a flush.
    useEffect(() => {
        if (!card) return;
        const el = document.querySelector(".hp-session");
        if (!el) return;
        el.style.opacity = "0.999";
        const id = requestAnimationFrame(() => { el.style.opacity = ""; });
        return () => cancelAnimationFrame(id);
    }, [card?.id, flipped]);

    useEffect(() => {
        function onKey(e) {
            if (e.repeat) return;
            if (e.target.tagName === "TEXTAREA" || e.target.tagName === "INPUT" || e.target.isContentEditable) return;
            if (e.key === " " && !flipped) { e.preventDefault(); handleFlip(); return; }
            if (flipped && card) {
                const gradeMap = card.tier > 0
                    ? { "1": 0, "2": 1, "3": 2, "4": 3 }
                    : { "1": 4, "2": 4, "3": 5, "4": 5 };
                if (gradeMap[e.key] !== undefined) handleGrade(gradeMap[e.key]);
            }
        }
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [flipped, card]);

    if (done) {
        return (
            <div className="hp-session">
                <div className="hp-session-inner">
                    <div className="hp-session-header" style={{ marginBottom: 24 }}>
                        <button className="quiet" onClick={handleBack}>← Back</button>
                        <h2>{group.name}</h2>
                    </div>
                    <div className="hp-done">
                        <div className="hp-done-title">All done for today!</div>
                        <div className="hp-done-sub">Come back tomorrow for more.</div>
                    </div>
                </div>
            </div>
        );
    }

    return (
        <div className="hp-session">
            <div className="hp-session-inner">
                <div className="hp-session-header">
                    <button className="quiet" onClick={handleBack}>← Back</button>
                    <h2>{group.name}</h2>
                    <div className="hp-session-counts">
                        <span style={{ color: "var(--t-blue)", opacity: newCount > 0 ? 1 : 0.45 }}>New: {newCount}</span>
                        <span style={{ color: "var(--t-green)", opacity: reviewCount > 0 ? 1 : 0.45 }}>Review: {reviewCount}</span>
                    </div>
                </div>

                {card && (
                    <div style={{ marginBottom: 8 }}>
                        {card.tier > 0
                            ? <span className="pill pill-green">Review</span>
                            : <span className="pill pill-blue">New</span>}
                    </div>
                )}

                <div className="hp-card-box">
                    {isCard && card && <CardFace card={card} showBack={flipped} />}
                </div>

                <textarea
                    className="hp-workspace"
                    value={workspace}
                    onChange={(e) => setWorkspace(e.target.value)}
                    placeholder="Workspace to write your answer or practice"
                    rows={3}
                    autoCorrect="off"
                    autoCapitalize="off"
                    spellCheck={false}
                />

                {!flipped ? (
                    <button className="hp-flip-btn" onClick={handleFlip}>Flip</button>
                ) : (
                    <GradeButtons onGrade={handleGrade} card={card} />
                )}

                {flipped && (
                    <SimilarNavigator
                        items={[...similarItems.front, ...similarItems.back]}
                        frontCount={similarItems.front.length}
                        groupType={group.group_type}
                    />
                )}
            </div>
        </div>
    );
}

// Plan Study Page

function PlanStudyPage({ plan, onBack, onStartSession, onNavigateToGroup, setToast }) {
    const [todos, setTodos] = useState([]);
    const [allTodos, setAllTodos] = useState([]);
    const [srsGroups, setSrsGroups] = useState([]);
    const [completingTodo, setCompletingTodo] = useState(null);
    const [completingTodoLinks, setCompletingTodoLinks] = useState({ groups: [], resources: [] });
    const [todoLinks, setTodoLinks] = useState({});
    const [dueCounts, setDueCounts] = useState({});
    const [showFreeTodo, setShowFreeTodo] = useState(false);
    const [planResources, setPlanResources] = useState([]);
    const [allGroups, setAllGroups] = useState([]);
    const [showResources, setShowResources] = useState(false);

    function navigateFromPlan(group, origin) {
        onNavigateToGroup(group, {
            ...origin,
            menu: "home",
            label: plan.name,
            homeContext: { plan },
        });
    }

    useEffect(() => { loadData(); }, [plan.id]);

    useEffect(() => {
        async function loadCounts() {
            const entries = await Promise.all(
                srsGroups.map(async ([group]) => {
                    const [newDue, reviewDue] = await loggedInvoke("count_due_items", { groupId: group.id });
                    return [group.id, { newDue, reviewDue }];
                })
            );
            setDueCounts(Object.fromEntries(entries));
        }
        loadCounts();
    }, [srsGroups]);

    async function loadData() {
        try {
            const [t, srs, r, g] = await Promise.all([
                loggedInvoke("get_todos", { planId: plan.id }),
                loggedInvoke("get_plan_srs_groups", { planId: plan.id }),
                loggedInvoke("get_resources", { planId: plan.id }),
                loggedInvoke("get_groups"),
            ]);
            const enabled = t.filter(todo => !todo.is_disabled);
            setTodos(enabled);
            setAllTodos(t);
            setSrsGroups(srs.filter(([group]) => group.group_type === "deck"));
            setPlanResources(r);
            setAllGroups(g);

            const links = {};
            await Promise.all(enabled.map(async (todo) => {
                const [tg, tr] = await Promise.all([
                    loggedInvoke("get_todo_groups", { todoId: todo.id }),
                    loggedInvoke("get_todo_resources", { todoId: todo.id }),
                ]);
                links[todo.id] = { groups: tg, resources: tr };
            }));
            setTodoLinks(links);
        } catch (e) { logError("catch", e); setToast("Failed to load plan data.", "error"); }
    }

    async function handleTodoCheck(todo) {
        if (todo.is_done) {
            try { await loggedInvoke("uncomplete_todo", { todoId: todo.id }); await loadData(); }
            catch (e) { logError("catch", e); setToast("Failed to uncomplete todo.", "error"); }
        } else {
            try {
                const [g, r] = await Promise.all([
                    loggedInvoke("get_todo_groups", { todoId: todo.id }),
                    loggedInvoke("get_todo_resources", { todoId: todo.id }),
                ]);
                pauseStudyTimer(plan.id);
                setCompletingTodoLinks({ groups: g, resources: r });
                setCompletingTodo(todo);
            } catch (e) { logError("catch", e); setToast("Failed to load todo links.", "error"); }
        }
    }

    async function confirmComplete(timeSpent, numUnit, details, resourceIds, groupIds, category, text) {
        if (!completingTodo) return;
        if (!text?.trim()) { setToast("Please enter a todo name."); return; }
        if (!category || category === 0) { setToast("Select at least one category.", "error"); return; }
        if (timeSpent <= 0) { setToast("Please log at least 1 minute.", "error"); return; }
        try {
            await loggedInvoke("complete_todo", {
                todoId: completingTodo.id,
                timeSpentMinutes: timeSpent,
                numUnit,
                details,
                resourceIds,
                groupIds,
                category,
                text: text.trim(),
            });
            setCompletingTodo(null);
            setCompletingTodoLinks({ groups: [], resources: [] });
            resetStudyTimer(plan.id);
            setToast("Done!");
            await loadData();
        } catch (e) { logError("catch", e); setToast("Failed to complete todo.", "error"); }
    }

    async function confirmFreeTodo({ text, category, details, timeSpent, numUnit, groupIds, resourceIds, date }) {
        if (!category || category === 0) { setToast("Select at least one category.", "error"); return; }
        if (timeSpent <= 0) { setToast("Please log at least 1 minute.", "error"); return; }
        try {
            await loggedInvoke("log_free_todo", {
                planId: plan.id, text, category, details,
                timeSpentMinutes: timeSpent, numUnit, groupIds, resourceIds, date,
            });
            setShowFreeTodo(false);
            resetStudyTimer(plan.id);
            setToast("Done!");
        } catch (e) {
            logError("catch", e);
            const msg = String(e).includes("future")
                ? "Can't log activity for a future date."
                : "Failed to log activity.";
            setToast(msg, "error");
        }
    }

    return (
        <div className="hp-root">
            <div className="hp-plan-page">
                <div className="hp-plan-back">
                    <button className="quiet" onClick={onBack}>← Back</button>
                    <h2>{plan.name}</h2>
                    <StudyTimer planId={plan.id} />
                </div>

                {/* Todos */}
                <div className="hp-section-panel">
                    <div style={{ display: "flex", alignItems: "center", marginBottom: 12 }}>
                        <span className="hp-section-label hp-section-label--todos" style={{ marginBottom: 0, flex: 1 }}>Todos</span>
                        <button onClick={() => { pauseStudyTimer(plan.id); setShowFreeTodo(true); }} style={{ fontSize: 11 }}>Log Extra</button>
                    </div>
                    {todos.length === 0 && (
                        <div className="empty-bubble">No todos today.</div>
                    )}
                    {todos.map(todo => {
                        const links = todoLinks[todo.id] ?? { groups: [], resources: [] };
                        return (
                            <div key={todo.id} className={`hp-todo-row${todo.is_done ? " done" : ""}`}>
                                <div style={{ overflow: "hidden" }}>
                                    <input type="checkbox" checked={todo.is_done}
                                        onChange={() => handleTodoCheck(todo)}
                                        style={{ float: "left", marginRight: 10, marginTop: 3, cursor: "pointer" }} />
                                    <div className={`hp-todo-text${todo.is_done ? " done" : ""}`}>
                                        {todo.text}
                                    </div>
                                </div>
                                <div className="todo-section">
                                    <div className="todo-section-label">Categories</div>
                                    <div className="todo-section-pills">
                                        <CategoryPills mask={todo.category} />
                                    </div>
                                </div>
                                {(links.groups.length > 0 || links.resources.length > 0) && (
                                    <div className="todo-section">
                                        <div className="todo-section-label">Resources / Decks / Notebooks</div>
                                        <div className="todo-section-pills">
                                            {links.resources.map(r => (
                                                <ResourcePill key={r.id} resource={r} />
                                            ))}
                                            {links.groups.map(g => (
                                                <GroupPill key={g.id} group={g} onClick={() => navigateFromPlan(g)} />
                                            ))}
                                        </div>
                                    </div>
                                )}
                            </div>
                        );
                    })}
                </div>

                {/* Study (SRS Groups) */}
                <div className="hp-section-panel">
                    <span className="hp-section-label hp-section-label--decks">Decks</span>
                    {srsGroups.length === 0 && (
                        <div className="empty-bubble">No decks linked for study.</div>
                    )}
                    {srsGroups.map(([group]) => {
                        const counts = dueCounts[group.id] || { newDue: 0, reviewDue: 0 };
                        const { newDue, reviewDue } = counts;
                        const isEmpty = newDue === 0 && reviewDue === 0;
                        return (
                            <div
                                key={group.id}
                                onClick={() => !isEmpty && onStartSession(group)}
                                className={`hp-deck-row${isEmpty ? " hp-deck-row--empty" : ""}`}
                            >
                                {isEmpty && <span className="hp-deck-check">✓</span>}
                                <span className="hp-deck-name">{group.name}</span>
                                {!isEmpty &&
                                    <span style={{ display: "flex", gap: 10 }}>
                                        <span className="hp-deck-new">{newDue > 0 && `New: ${newDue}`}</span>
                                        <span className="hp-deck-review">{reviewDue > 0 && `Review: ${reviewDue}`}</span>
                                    </span>
                                }
                            </div>
                        );
                    })}
                </div>

                {/* Resources, collapsed by default. */}
                {planResources.length > 0 && (
                    <div className="hp-resources">
                        <span className="hp-resources-toggle" onClick={() => setShowResources(s => !s)}>
                            Resources <span className="hp-resources-caret">{showResources ? "▾" : "▸"}</span>
                        </span>
                        {showResources && (
                            <div className="hp-resources-list">
                                {planResources.map(r => <ResourceCard key={r.id} res={r} />)}
                            </div>
                        )}
                    </div>
                )}

                {completingTodo && (
                    <TodoCompletePopup
                        todo={completingTodo}
                        todoGroups={completingTodoLinks.groups}
                        todoResources={completingTodoLinks.resources}
                        planResources={planResources}
                        allGroups={allGroups}
                        onConfirm={confirmComplete}
                        onCancel={() => { setCompletingTodo(null); setCompletingTodoLinks({ groups: [], resources: [] }); }}
                        onNavigateToGroup={navigateFromPlan}
                        initialTime={timerMinutesRounded(plan.id)}
                    />
                )}

                {showFreeTodo && (
                    <FreeTodoPopup
                        planId={plan.id}
                        planResources={planResources}
                        allGroups={allGroups}
                        todos={allTodos}
                        onConfirm={confirmFreeTodo}
                        onCancel={() => setShowFreeTodo(false)}
                        setToast={setToast}
                        initialTime={timerMinutesRounded(plan.id)}
                    />
                )}
            </div>
        </div>
    );
}

// Homepage

const VIEW_HOME    = "home";
const VIEW_PLAN    = "plan";
const VIEW_SESSION = "session";

export default function Homepage({ setToast, onNavigateToGroup, returnContext, onConsumeReturnContext, refreshDayCount, onRefreshDay, onOpenHelp }) {
    const [plans, setPlans] = useState([]);
    const [view, setView] = useState(VIEW_HOME);
    const [activePlan, setActivePlan] = useState(null);
    const [activeGroup, setActiveGroup] = useState(null);
    const [planCounts, setPlanCounts] = useState({});
    const [displayDate, setDisplayDate] = useState("");
    const [version, setVersion] = useState("");
    const [dayStale, setDayStale] = useState(false);

    function loadDisplayDate() {
        loggedInvoke("get_current_date").then(ds => {
            const [y, m, d] = ds.split('-').map(Number);
            setDisplayDate(new Date(y, m - 1, d).toLocaleDateString("en-US", {
                weekday: "long", month: "long", day: "numeric"
            }));
        }).catch(e => logError("catch", e));
        loggedInvoke("is_day_stale").then(setDayStale).catch(e => logError("catch", e));
    }

    useEffect(() => {
        loadDisplayDate();
        loadPlans();
        getVersion().then(setVersion).catch(e => logError("getVersion", e));
    }, []);

    useEffect(() => {
        if (view === VIEW_HOME) {
            loggedInvoke("is_day_stale").then(setDayStale).catch(e => logError("catch", e));
        }
    }, [view]);

    useEffect(() => {
        if (returnContext?.plan) {
            setActivePlan(returnContext.plan);
            setView(VIEW_PLAN);
            onConsumeReturnContext();
        }
    }, [returnContext]);

    useEffect(() => {
        if (refreshDayCount > 0) {
            loadPlans();
            loadDisplayDate();
        }
    }, [refreshDayCount]);

    async function loadPlans() {
        try {
            const p = await loggedInvoke("get_plans");
            setPlans(p);

            const counts = {};
            await Promise.all(p.map(async (plan) => {
                const [todos, srs, streakInfo] = await Promise.all([
                    loggedInvoke("get_todos", { planId: plan.id }),
                    loggedInvoke("get_plan_srs_groups", { planId: plan.id }),
                    loggedInvoke("get_plan_streak", { planId: plan.id }),
                ]);
                const todayTodos = todos.filter(t => !t.is_disabled && !t.is_done).length;
                const deckDueCounts = await Promise.all(
                    srs
                        .filter(([group]) => group.group_type === "deck")
                        .map(async ([group]) => {
                            const [n, r] = await loggedInvoke("count_due_items", { groupId: group.id });
                            return n + r;
                        })
                );
                const totalDue = deckDueCounts.reduce((a, b) => a + b, 0);
                counts[plan.id] = { todos: todayTodos, cards: totalDue, streakInfo };
            }));
            setPlanCounts(counts);
        } catch (e) { logError("catch", e); setToast("Failed to load plans.", "error"); }
    }

    if (view === VIEW_SESSION && activeGroup) {
        return (
            <StudySession
                group={activeGroup}
                onBack={() => { setView(VIEW_PLAN); setActiveGroup(null); }}
                setToast={setToast}
            />
        );
    }

    if (view === VIEW_PLAN && activePlan) {
        return (
            <PlanStudyPage
                plan={activePlan}
                onBack={async () => { setView(VIEW_HOME); setActivePlan(null); await loadPlans(); }}
                onStartSession={(group) => { setActiveGroup(group); setView(VIEW_SESSION); }}
                onNavigateToGroup={onNavigateToGroup}
                setToast={setToast}
            />
        );
    }

    return (
        <div className="hp-root">
            <div className="hp-home">
                <div className="hp-greeting-row">
                    <div>
                        <div className="hp-greeting">Welcome back</div>
                        <div className="hp-date">{displayDate}</div>
                    </div>
                    {onRefreshDay && (
                        <button className={`hp-refresh-day${dayStale ? " stale" : ""}`} onClick={onRefreshDay}>Refresh Day</button>
                    )}
                </div>

                {plans.length === 0 ? (
                    <div className="hp-empty-state">
                        <div className="hp-empty-icon">
                            <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                                <rect x="3" y="3" width="18" height="18" rx="2"/>
                                <path d="M3 9h18M9 21V9"/>
                            </svg>
                        </div>
                        <div className="hp-empty-title">No plans yet</div>
                        <div className="hp-empty-sub">Head to <strong>Plans</strong> to create your first study plan.</div>
                    </div>
                ) : (
                    <div>
                        {plans.map(plan => {
                            const counts = planCounts[plan.id];
                            const streakInfo = counts?.streakInfo;
                            const atRisk = streakInfo && streakInfo.streak > 0 && !streakInfo.studied_today;
                            const hasDue = counts && (counts.todos > 0 || counts.cards > 0);
                            // Nothing due but nothing studied: streak is still at risk so this must not read as done.
                            const idle = !!counts && !hasDue && streakInfo && !streakInfo.studied_today;
                            // Requires counts so a plan doesn't flash "done" while loading.
                            const isDone = !!counts && !hasDue && !idle;
                            return (
                                <div
                                    key={plan.id}
                                    className={`hp-plan-card${hasDue ? " has-due" : ""}${isDone ? " is-done" : ""}`}
                                    onClick={() => { setActivePlan(plan); setView(VIEW_PLAN); }}
                                >
                                    <div className="hp-plan-card-top">
                                        <div className="hp-plan-name-row">
                                            {isDone && <span className="hp-plan-check">✓</span>}
                                            <div className="hp-plan-name">{plan.name}</div>
                                        </div>
                                        {streakInfo?.streak > 0 && (
                                            <span className={`hp-streak-chip${atRisk ? " at-risk" : ""}`}>
                                                {streakInfo.streak}d streak{atRisk ? " !" : ""}
                                            </span>
                                        )}
                                    </div>
                                    <div className="hp-plan-card-stats">
                                        <div className="hp-plan-stat-box">
                                            <span className={`hp-stat-num ${isDone ? "hp-stat-num--zero" : "hp-stat-num--todos"}`}>{counts?.todos ?? 0}</span>
                                            <span className="hp-stat-lbl">{(counts?.todos ?? 0) == 1 ? "todo due" : "todos due"}</span>
                                        </div>
                                        <div className="hp-plan-stat-divider" />
                                        <div className="hp-plan-stat-box">
                                            <span className={`hp-stat-num ${isDone ? "hp-stat-num--zero" : "hp-stat-num--decks"}`}>{counts?.cards ?? 0}</span>
                                            <span className="hp-stat-lbl">{(counts?.cards ?? 0) == 1 ? "card due" : "cards due"}</span>
                                        </div>
                                    </div>
                                    <div className="hp-plan-card-foot">
                                        {idle && (
                                            <span className="hp-plan-idle-note">
                                                Nothing due today. Log an extra activity to {streakInfo.streak > 0 ? "keep" : "start"} your streak.
                                            </span>
                                        )}
                                        <span className="hp-plan-open">Open plan →</span>
                                    </div>
                                </div>
                            );
                        })}
                    </div>
                )}
            </div>
            {onOpenHelp && (
                <button className="hp-help-btn" onClick={onOpenHelp} title="How Toast works">?</button>
            )}
            {version && <div className="hp-version">Toast v{version}</div>}
        </div>
    );
}
