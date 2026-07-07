import { useState, useEffect, useRef } from "react";
import { loggedInvoke, logError } from "./logger";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getVersion } from "@tauri-apps/api/app";

import { CardFace, renderAnkiHtml, stripAudioTags } from "./Decks/CardFace";
import { ResourceCard, GroupTypeBadge } from "./UIUtils";
import { computeCategory, maskToCategories, CategoryPicker, CategoryPills } from "./Plans/PlanUtils";
import "./Homepage.css";

// ─── Shared pill helpers ──────────────────────────────────────────────────────

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


// ─── Grade Buttons ────────────────────────────────────────────────────────────

function GradeButtons({ onGrade, card }) {
    const gradeDeltas = card.tier > 0 ? [
        { label: "Nope",  tierDelta: -2, grade: 0, cls: "hp-grade-nope",  easeDelta: -0.12 },
        { label: "Rough", tierDelta: -1, grade: 1, cls: "hp-grade-rough", easeDelta: -0.05 },
        { label: "Fine",  tierDelta:  1, grade: 2, cls: "hp-grade-fine",  easeDelta:  0.04 },
        { label: "Easy",  tierDelta:  1, grade: 3, cls: "hp-grade-easy",  easeDelta:  0.10 },
    ] : [
        { label: "One More Time", tierDelta: -1, grade: 1, cls: "hp-grade-omt",   easeDelta: -0.05 },
        { label: "Got It",        tierDelta:  1, grade: 2, cls: "hp-grade-gotit", easeDelta:  0.04 },
    ];

    function calcNextSequence(grade, tierDelta, easeDelta) {
        if (!card) return null;
        const newTier = Math.min(30, Math.max(card.tier === 0 ? 0 : 1, card.tier + tierDelta));
        const newEase = Math.max(-0.35, Math.min(0.35, card.ease + easeDelta));
        if (newTier === 0) return 0;
        return Math.round(Math.pow(2, newTier - 1) * (1 + newEase));
    }

    return (
        <div className="hp-grade-bar">
            {gradeDeltas.map(({ label, tierDelta, grade, cls, easeDelta }) => {
                const nextSeq = calcNextSequence(grade, tierDelta, easeDelta);
                return (
                    <button key={grade} onClick={() => onGrade(grade)} className={`hp-grade-btn ${cls}`}>
                        <span>{label}</span>
                        {nextSeq !== null && (
                            <span className="hp-grade-btn-interval">
                                {nextSeq === 0 ? "Again" : `${nextSeq}d`}
                            </span>
                        )}
                    </button>
                );
            })}
        </div>
    );
}

// ─── Similar Items Navigator ──────────────────────────────────────────────────

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

// ─── Todo Complete Popup ──────────────────────────────────────────────────────

function TodoCompletePopup({ todo, todoGroups, todoResources, planResources, allGroups, onConfirm, onCancel, onNavigateToGroup }) {
    const [timeSpent, setTimeSpent] = useState(0);
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
                <div className="hp-popup-title">{todo.text}</div>

                <div>
                    <div className="hp-popup-label">Category</div>
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
                        <div className="hp-popup-label">Study materials</div>
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
                        )}>
                        Done
                    </button>
                </div>
            </div>
        </div>
    );
}

// ─── Free Todo Popup ──────────────────────────────────────────────────────────

function FreeTodoPopup({ planId, planResources, allGroups, onConfirm, onCancel, setToast }) {
    const [text, setText] = useState("");
    const [timeSpent, setTimeSpent] = useState(0);
    const [numUnit, setNumUnit] = useState("");
    const [details, setDetails] = useState("");
    const [selectedGroupIds, setSelectedGroupIds] = useState([]);
    const [selectedResourceIds, setSelectedResourceIds] = useState([]);
    const [categoryMap, setCategoryMap] = useState(DEFAULT_CATEGORY());

    function toggleGroup(id) {
        setSelectedGroupIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
    }
    function toggleResource(id) {
        setSelectedResourceIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
    }
    function toggleCategory(bit) {
        setCategoryMap(prev => ({ ...prev, [bit]: !prev[bit] }));
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
        });
    }

    return (
        <div className="hp-overlay">
            <div className="hp-popup">
                <div className="hp-popup-title">Log Extra Activity</div>

                <div>
                    <div className="hp-popup-label">What did you do?</div>
                    <input value={text} onChange={(e) => setText(e.target.value)}
                        placeholder="e.g. Read extra grammar notes"
                        style={{ width: "100%" }} />
                </div>

                <div>
                    <div className="hp-popup-label">Category</div>
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
                        <div className="hp-popup-label">Study materials</div>
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
                    <button onClick={onCancel}>Cancel</button>
                    <button className="primary" onClick={submit}>Log</button>
                </div>
            </div>
        </div>
    );
}

// ─── SRS Study Session ────────────────────────────────────────────────────────

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
        setFlipped(true);
        const itemId = card?.id;
        if (!itemId) return;
        try {
            const similar = await loggedInvoke("get_similar_cards", { itemId });
            setSimilarItems(similar);
        } catch (e) { logError("catch", e); }
    }

    async function handleGrade(grade) {
        if (!isCard) throw new Error("Attempted Notebook SRS");
        const itemId = card?.id;
        if (!itemId) return;
        try {
            await loggedInvoke("grade_item", { itemId, grade });
            await fetchNext();
        } catch (e) { logError("catch", e); setToast("Failed to grade card", "error"); }
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

    useEffect(() => {
        function onKey(e) {
            if (e.target.tagName === "TEXTAREA" || e.target.tagName === "INPUT" || e.target.isContentEditable) return;
            if (e.key === " " && !flipped) { e.preventDefault(); handleFlip(); return; }
            if (flipped && card) {
                const gradeMap = card.tier > 0
                    ? { "1": 0, "2": 1, "3": 2, "4": 3 }
                    : { "1": 1, "2": 1, "3": 2, "4": 2 };
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

// ─── Plan Study Page ──────────────────────────────────────────────────────────

function PlanStudyPage({ plan, onBack, onStartSession, onNavigateToGroup, setToast }) {
    const [todos, setTodos] = useState([]);
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
                setCompletingTodoLinks({ groups: g, resources: r });
                setCompletingTodo(todo);
            } catch (e) { logError("catch", e); setToast("Failed to load todo links.", "error"); }
        }
    }

    async function confirmComplete(timeSpent, numUnit, details, resourceIds, groupIds, category) {
        if (!completingTodo) return;
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
            });
            setCompletingTodo(null);
            setCompletingTodoLinks({ groups: [], resources: [] });
            setToast("Done!");
            await loadData();
        } catch (e) { logError("catch", e); setToast("Failed to complete todo.", "error"); }
    }

    async function confirmFreeTodo({ text, category, details, timeSpent, numUnit, groupIds, resourceIds }) {
        if (!category || category === 0) { setToast("Select at least one category.", "error"); return; }
        if (timeSpent <= 0) { setToast("Please log at least 1 minute.", "error"); return; }
        try {
            await loggedInvoke("log_free_todo", {
                planId: plan.id, text, category, details,
                timeSpentMinutes: timeSpent, numUnit, groupIds, resourceIds,
            });
            setShowFreeTodo(false);
            setToast("Activity logged.");
        } catch (e) { logError("catch", e); setToast("Failed to log activity.", "error"); }
    }

    return (
        <div className="hp-root">
            <div className="hp-plan-page">
                <div className="hp-plan-back">
                    <button className="quiet" onClick={onBack}>← Back to Plans</button>
                    <h2>{plan.name}</h2>
                </div>

                {/* Todos */}
                <div className="hp-section-panel">
                    <div style={{ display: "flex", alignItems: "center", marginBottom: 6 }}>
                        <span className="hp-section-label hp-section-label--todos" style={{ marginBottom: 0, flex: 1 }}>Todos</span>
                        <button onClick={() => setShowFreeTodo(true)} style={{ fontSize: 11 }}>Log Extra</button>
                    </div>
                    {todos.length === 0 && (
                        <div className="empty-bubble">No todos today.</div>
                    )}
                    {todos.map(todo => {
                        const links = todoLinks[todo.id] ?? { groups: [], resources: [] };
                        return (
                            <div key={todo.id} className={`hp-todo-row${todo.is_done ? " done" : ""}`}>
                                <input type="checkbox" checked={todo.is_done}
                                    onChange={() => handleTodoCheck(todo)}
                                    style={{ marginTop: 3, cursor: "pointer", flexShrink: 0 }} />
                                <div style={{ flex: 1 }}>
                                    <div className={`hp-todo-text${todo.is_done ? " done" : ""}`}>
                                        {todo.text}
                                    </div>
                                    <CategoryPills mask={todo.category} style={{ marginTop: 5 }} />
                                    {(links.groups.length > 0 || links.resources.length > 0) && (
                                        <div style={{ marginTop: 5, display: "flex", gap: 5, flexWrap: "wrap" }}>
                                            {links.resources.map(r => (
                                                <ResourcePill key={r.id} resource={r} />
                                            ))}
                                            {links.groups.map(g => (
                                                <GroupPill key={g.id} group={g} onClick={() => navigateFromPlan(g)} />
                                            ))}
                                        </div>
                                    )}
                                </div>
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

                {/* Resources — quiet reference list, collapsed by default */}
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
                    />
                )}

                {showFreeTodo && (
                    <FreeTodoPopup
                        planId={plan.id}
                        planResources={planResources}
                        allGroups={allGroups}
                        onConfirm={confirmFreeTodo}
                        onCancel={() => setShowFreeTodo(false)}
                        setToast={setToast}
                    />
                )}
            </div>
        </div>
    );
}

// ─── Homepage ─────────────────────────────────────────────────────────────────

const VIEW_HOME    = "home";
const VIEW_PLAN    = "plan";
const VIEW_SESSION = "session";

export default function Homepage({ setToast, onNavigateToGroup, returnContext, onConsumeReturnContext, refreshDayCount, onRefreshDay }) {
    const [plans, setPlans] = useState([]);
    const [view, setView] = useState(VIEW_HOME);
    const [activePlan, setActivePlan] = useState(null);
    const [activeGroup, setActiveGroup] = useState(null);
    const [planCounts, setPlanCounts] = useState({});
    const [displayDate, setDisplayDate] = useState("");
    const [version, setVersion] = useState("");

    function loadDisplayDate() {
        loggedInvoke("get_current_date").then(ds => {
            const [y, m, d] = ds.split('-').map(Number);
            setDisplayDate(new Date(y, m - 1, d).toLocaleDateString("en-US", {
                weekday: "long", month: "long", day: "numeric"
            }));
        }).catch(e => logError("catch", e));
    }

    useEffect(() => {
        loadDisplayDate();
        loadPlans();
        getVersion().then(setVersion).catch(e => logError("getVersion", e));
    }, []);

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
                        <button className="hp-refresh-day" onClick={onRefreshDay}>Refresh Day</button>
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
                            return (
                                <div
                                    key={plan.id}
                                    className={`hp-plan-card${hasDue ? " has-due" : ""}`}
                                    onClick={() => { setActivePlan(plan); setView(VIEW_PLAN); }}
                                >
                                    <div className="hp-plan-card-top">
                                        <div className="hp-plan-name">{plan.name}</div>
                                        {streakInfo?.streak > 0 && (
                                            <span className={`hp-streak-chip${atRisk ? " at-risk" : ""}`}>
                                                {streakInfo.streak}d streak{atRisk ? " !" : ""}
                                            </span>
                                        )}
                                    </div>
                                    <div className="hp-plan-card-stats">
                                        <div className="hp-plan-stat-box">
                                            <span className="hp-stat-num">{counts?.todos ?? 0}</span>
                                            <span className="hp-stat-lbl">{(counts?.todos ?? 0) == 1 ? "todo due" : "todos due"}</span>
                                        </div>
                                        <div className="hp-plan-stat-divider" />
                                        <div className="hp-plan-stat-box">
                                            <span className="hp-stat-num">{counts?.cards ?? 0}</span>
                                            <span className="hp-stat-lbl">{(counts?.cards ?? 0) == 1 ? "card due" : "cards due"}</span>
                                        </div>
                                    </div>
                                    <div className="hp-plan-card-foot">
                                        <span className="hp-plan-open">Open plan →</span>
                                    </div>
                                </div>
                            );
                        })}
                    </div>
                )}
            </div>
            {version && <div className="hp-version">Toast v{version}</div>}
        </div>
    );
}
