import { useState, useEffect } from "react";
import { loggedInvoke, logError } from "../logger";
import { openUrl } from "@tauri-apps/plugin-opener";

import Todos from "./Todos";
import { computeFrequency, computeCategory, FrequencyPicker, CategoryPicker } from "./PlanUtils";
import { Tip, ConfirmDelete, GroupTypeBadge } from "../UIUtils";
import "./Plans.css";

const DEFAULT_CATEGORY = () => ({ 1: false, 2: false, 4: false, 8: false, 16: false, 32: false, 64: false });

const DAY_LABELS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const DAY_NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const activeOnDay = (todo, day) => (todo.frequency & (1 << day)) !== 0;

// Matches the backend's ORDER BY name COLLATE NOCASE
const byName = (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });

// SRS Group Row

function SrsGroupRow({ group, scheduler, onClamp, onClampMax, onRemove, loadData, srsGroups, setToast, onNavigateToGroup }) {
    const [removing, setRemoving] = useState(false);
    const [editing, setEditing] = useState(false);
    const [maxNew, setMaxNew] = useState(scheduler.max_new);
    const [maxReview, setMaxReview] = useState(scheduler.max_review);
    const [canOverflow, setCanOverflow] = useState(scheduler.can_overflow);
    const [dueCount, setDueCount] = useState([]);

    async function saveSettings() {
        try {
            await loggedInvoke("update_scheduler", {
                scheduler: {
                    group_id: group.id,
                    studied_new: scheduler.studied_new,
                    studied_review: scheduler.studied_review,
                    max_new: maxNew,
                    max_review: maxReview,
                    can_overflow: canOverflow,
                }
            });
            loadData();
            setEditing(false);
            setToast("Deck settings saved.")
        } catch (e) {
            logError("catch", e);
            setToast("Failed to save deck settings.", "error");
        }
    }

    useEffect(() => {
        async function loadDueCount() {
            const count = await loggedInvoke("count_due_items", { groupId: group.id});
            setDueCount(count);
        }
        loadDueCount();
    }, [srsGroups]);

    return (
        <div className="plan-srs-row">
            <div className="plan-srs-header">
                <span
                    className={`plan-srs-name${onNavigateToGroup ? " clickable" : ""}`}
                    onClick={() => onNavigateToGroup?.(group, { menu: "plans", label: "Plans" })}
                >
                    {group.name}
                </span>
                <span className="plan-srs-counts">
                    New: {dueCount[0]}/{scheduler.max_new} · Review: {dueCount[1]}/{scheduler.max_review}
                </span>
            </div>
            <div className="plan-srs-actions">
                {editing && (
                    <div className="plan-srs-settings">
                        <label className="plan-srs-settings-label">
                            Max New
                            <Tip text="Maximum new (unseen) cards introduced per study session, excluding overdue cards" />
                            <input type="number" min={0} value={maxNew}
                                onChange={(e) => setMaxNew(Number(e.target.value))}
                                onKeyDown={(e) => { if (e.key === "Enter") saveSettings(); }}
                                className="plan-srs-settings-input" />
                        </label>
                        <label className="plan-srs-settings-label">
                            Max Review
                            <Tip text="Maximum review cards shown per study session, excluding overdue cards" />
                            <input type="number" min={0} value={maxReview}
                                onChange={(e) => setMaxReview(Number(e.target.value))}
                                onKeyDown={(e) => { if (e.key === "Enter") saveSettings(); }}
                                className="plan-srs-settings-input" />
                        </label>
                        <label className="plan-srs-settings-label with-gap">
                            <input type="checkbox" checked={canOverflow}
                                onChange={(e) => setCanOverflow(e.target.checked)} />
                            Overflow
                            <Tip text="When enabled, leftover cards that weren't studied today carry over to tomorrow without counting towards tomorrow's scheduled maximum. When disabled, the total due cards tomorrow will not exceed the set max." />
                        </label>
                        <button className="primary" onClick={saveSettings}>Save</button>
                    </div>
                )}
                {removing && (
                    <div className="plan-srs-confirm">
                        <span className="plan-srs-confirm-sub">
                            Preserve maintains all progress. Reset wipes all progress.
                        </span>
                        <button onClick={() => { onRemove(false); setRemoving(false); }}>Remove &amp; Preserve</button>
                        <button className="danger" onClick={() => { onRemove(true); setRemoving(false); }}>Remove &amp; Reset</button>
                    </div>
                )}
                <span className="plan-srs-tip-wrap">
                    <button className="btn-amber" onClick={onClamp}>Trim</button>
                    <Tip text="Updates the due queue to the max minus cards already studied today. Note that overflow cards may be unscheduled." />
                </span>
                <span className="plan-srs-tip-wrap">
                    <button className="btn-blue" onClick={onClampMax}>Fill</button>
                    <Tip text="Resets the due queue to the full max, ignoring cards already studied today. Note that overflow cards may be unscheduled." />
                </span>
                <button onClick={() => { setEditing((e) => !e); setRemoving(false); }}>{editing ? "Cancel" : "Settings"}</button>
                <button className="danger" onClick={() => { setRemoving((r) => !r); setEditing(false); }}>{removing ? "Cancel" : "Remove"}</button>
            </div>
        </div>
    );
}

// SRS Section

function SrsSection({ planId, setToast, onNavigateToGroup }) {
    const [srsGroups, setSrsGroups] = useState([]);
    const [unassigned, setUnassigned] = useState([]);
    const [selectedGroupId, setSelectedGroupId] = useState(null);
    const [maxNew, setMaxNew] = useState(20);
    const [maxReview, setMaxReview] = useState(50);
    const [canOverflow, setCanOverflow] = useState(false);
    const [adding, setAdding] = useState(false);

    useEffect(() => { loadData(); }, [planId]);

    async function loadData() {
        try {
            const [srs, unass] = await Promise.all([
                loggedInvoke("get_plan_srs_groups", { planId }),
                loggedInvoke("get_unassigned_groups"),
            ]);
            setSrsGroups(srs);
            setUnassigned(unass);
            setSelectedGroupId(unass.length > 0 ? unass[0].id : null);
        } catch (e) { logError("catch", e); setToast("Failed to load SRS data.", "error"); }
    }

    async function addGroup() {
        if (!selectedGroupId) { setToast("Please select a deck."); return; }
        try {
            await loggedInvoke("add_group_to_plan", {
                groupId: selectedGroupId,
                planId,
                scheduler: { group_id: selectedGroupId, max_new: maxNew, max_review: maxReview, can_overflow: canOverflow },
            });
            setToast("Deck added to plan.");
            setAdding(false);
            setMaxNew(20); setMaxReview(50); setCanOverflow(false);
            await loadData();
        } catch (e) { logError("catch", e); setToast("Failed to add group.", "error"); }
    }

    async function removeGroup(groupId, reset) {
        try {
            await loggedInvoke("remove_group_from_plan", { groupId, reset });
            setToast(reset ? "Deck removed and progress reset." : "Deck removed, progress preserved.");
            await loadData();
        } catch (e) { logError("catch", e); setToast("Failed to remove group.", "error"); }
    }

    async function clampGroup(groupId) {
        try {
            await loggedInvoke("clamp_group", { groupId });
            setToast("Queue trimmed to today's remaining capacity.");
            await loadData();
        } catch (e) { logError("catch", e); setToast("Failed to clamp deck.", "error"); }
    }

    async function clampGroupMax(groupId) {
        try {
            await loggedInvoke("max_clamp_group", { groupId });
            setToast("Queue filled to max capacity.");
            await loadData();
        } catch (e) { logError("catch", e); setToast("Failed to clamp deck.", "error"); }
    }

    return (
        <div>
            {srsGroups.length === 0 && (
                <div className="empty-bubble">No decks linked yet.</div>
            )}
            {srsGroups.map(([group, scheduler]) => (
                <SrsGroupRow key={group.id} group={group} scheduler={scheduler}
                    onClamp={() => clampGroup(group.id)}
                    onClampMax={() => clampGroupMax(group.id)}
                    onRemove={(reset) => removeGroup(group.id, reset)}
                    loadData={() => loadData()}
                    srsGroups={srsGroups}
                    setToast={setToast}
                    onNavigateToGroup={onNavigateToGroup} />
            ))}
            {!adding ? (
                <button onClick={() => setAdding(true)} disabled={unassigned.length === 0}>+ Add Deck</button>
            ) : (
                <div className="plan-srs-add">
                    <div className="plan-section-title">Link Deck to SRS</div>
                    <select value={selectedGroupId ?? ""}
                        onChange={(e) => setSelectedGroupId(e.target.value ? Number(e.target.value) : null)}>
                        <option value="">Select a deck</option>
                        {unassigned.filter(g => g.group_type === "deck").map(g =>
                            <option key={g.id} value={g.id}>{g.name}</option>)}
                    </select>
                    <div className="plan-srs-settings-fields">
                        <label className="plan-srs-settings-label">
                            Max New
                            <Tip text="Maximum new (unseen) cards introduced per study session." />
                            <input type="number" min={0} value={maxNew}
                                onChange={(e) => setMaxNew(Number(e.target.value))}
                                className="plan-srs-settings-input" />
                        </label>
                        <label className="plan-srs-settings-label">
                            Max Review
                            <Tip text="Maximum review cards shown per study session." />
                            <input type="number" min={0} value={maxReview}
                                onChange={(e) => setMaxReview(Number(e.target.value))}
                                className="plan-srs-settings-input" />
                        </label>
                        <label className="plan-srs-settings-label with-gap">
                            <input type="checkbox" checked={canOverflow}
                                onChange={(e) => setCanOverflow(e.target.checked)} />
                            Overflow
                            <Tip text="When enabled, cards that weren't studied today carry over to tomorrow without counting against tomorrow's scheduled maximum.  When disabled, the total due cards tomorrow will not exceed the set max." />
                        </label>
                    </div>
                    <div className="plan-form-actions">
                        <button className="primary" onClick={addGroup}>+ Add</button>
                        <button onClick={() => setAdding(false)}>Cancel</button>
                    </div>
                </div>
            )}
        </div>
    );
}

// Resources Section

function ResourcesSection({ planId, plans, setToast, onChanged }) {
    const [resources, setResources] = useState([]);
    const [adding, setAdding] = useState(false);
    const [editingId, setEditingId] = useState(null);
    // Resources on every other plan, offered as autofill templates in the add
    // form so a resource can be copied into this plan from wherever it lives
    const [otherResources, setOtherResources] = useState([]);
    const [copyFrom, setCopyFrom] = useState("");

    const otherPlans = (plans ?? []).filter((p) => p.id !== planId);
    const [newName, setNewName] = useState("");
    const [newType, setNewType] = useState("");
    const [newUrl, setNewUrl] = useState("");
    const [newNotes, setNewNotes] = useState("");
    const [editName, setEditName] = useState("");
    const [editType, setEditType] = useState("");
    const [editUrl, setEditUrl] = useState("");
    const [editNotes, setEditNotes] = useState("");

    useEffect(() => { loadResources(); }, [planId]);

    useEffect(() => {
        if (!adding || otherPlans.length === 0) return;
        (async () => {
            try {
                const lists = await Promise.all(otherPlans.map((p) => loggedInvoke("get_resources", { planId: p.id })));
                setOtherResources(lists.flat());
            } catch (e) { logError("catch", e); }
        })();
    }, [adding, planId]);

    async function loadResources() {
        try {
            const r = await loggedInvoke("get_resources", { planId });
            setResources(r);
        } catch (e) { logError("catch", e); setToast("Failed to load resources.", "error"); }
    }

    async function createResource() {
        if (!newName.trim()) { setToast("Resource name is required."); return; }
        try {
            await loggedInvoke("create_resource", {
                resource: {
                    plan_id: planId,
                    name: newName.trim(),
                    resource_type: newType.trim() || null,
                    url: newUrl.trim() || null,
                    notes: newNotes.trim() || null,
                }
            });
            closeAdd();
            setToast("Resource created.");
            await loadResources();
            onChanged?.();
        } catch (e) { logError("catch", e); setToast("Failed to create resource.", "error"); }
    }

    function startEdit(r) {
        setEditingId(r.id);
        setEditName(r.name);
        setEditType(r.resource_type ?? "");
        setEditUrl(r.url ?? "");
        setEditNotes(r.notes ?? "");
    }

    async function saveEdit(r) {
        if (!editName.trim()) { setToast("Resource name is required."); return; }
        try {
            await loggedInvoke("update_resource", {
                resource: {
                    id: r.id,
                    plan_id: r.plan_id,
                    name: editName.trim(),
                    resource_type: editType.trim() || null,
                    url: editUrl.trim() || null,
                    notes: editNotes.trim() || null,
                }
            });
            setEditingId(null);
            setToast("Resource updated.");
            await loadResources();
            onChanged?.();
        } catch (e) { logError("catch", e); setToast("Failed to update resource.", "error"); }
    }

    async function deleteResource(id) {
        try {
            await loggedInvoke("delete_resource", { id });
            setToast("Resource deleted.");
            await loadResources();
            onChanged?.();
        } catch (e) { logError("catch", e); setToast("Failed to delete resource.", "error"); }
    }

    function pickCopyFrom(value) {
        setCopyFrom(value);
        const src = otherResources.find((r) => r.id === Number(value));
        if (src) {
            setNewName(src.name);
            setNewType(src.resource_type ?? "");
            setNewUrl(src.url ?? "");
            setNewNotes(src.notes ?? "");
        } else {
            setNewName(""); setNewType(""); setNewUrl(""); setNewNotes("");
        }
    }

    function closeAdd() {
        setAdding(false);
        setCopyFrom("");
        setNewName(""); setNewType(""); setNewUrl(""); setNewNotes("");
    }

    return (
        <div>
            {resources.length === 0 && (
                <div className="empty-bubble">No resources yet.</div>
            )}
            {resources.map((r) => (
                <div key={r.id} className="plan-resource-row">
                    {editingId === r.id ? (
                        <div className="plan-resource-edit-form">
                            <input value={editName} onChange={(e) => setEditName(e.target.value)} placeholder="Name"
                                onKeyDown={(e) => { if (e.key === "Enter") saveEdit(r); if (e.key === "Escape") setEditingId(null); }} />
                            <input value={editType} onChange={(e) => setEditType(e.target.value)} placeholder="Type (book, website, video…)"
                                onKeyDown={(e) => { if (e.key === "Enter") saveEdit(r); if (e.key === "Escape") setEditingId(null); }} />
                            <input value={editUrl} onChange={(e) => setEditUrl(e.target.value)} placeholder="URL (optional)"
                                onKeyDown={(e) => { if (e.key === "Enter") saveEdit(r); if (e.key === "Escape") setEditingId(null); }} />
                            <textarea value={editNotes} onChange={(e) => setEditNotes(e.target.value)} placeholder="Notes (optional)" rows={2}
                                style={{ fontFamily: "inherit", resize: "none" }} />
                            <div className="plan-resource-edit-actions">
                                <button className="primary" onClick={() => saveEdit(r)}>Save</button>
                                <button onClick={() => setEditingId(null)}>Cancel</button>
                            </div>
                        </div>
                    ) : (
                        <div className="plan-resource-view">
                            <div className="plan-resource-body">
                                <div style={{ display: "flex", alignItems: "baseline", gap: 8, flexWrap: "wrap" }}>
                                    <div className="plan-resource-name">{r.name}</div>
                                    {r.url &&
                                        <a href={r.url} className="plan-resource-url"
                                            onClick={(e) => { e.preventDefault(); openUrl(r.url.startsWith("http") ? r.url : `https://${r.url}`); }}>
                                            ↗
                                        </a>}
                                    {r.resource_type && <span className="st-resource-card-type">{r.resource_type}</span>}
                                </div>
                                {r.notes && <div className="plan-resource-notes">{r.notes}</div>}
                            </div>
                            <div className="plan-resource-actions">
                                <button onClick={() => startEdit(r)}>Edit</button>
                                <ConfirmDelete onConfirm={() => deleteResource(r.id)} small />
                            </div>
                        </div>
                    )}
                </div>
            ))}
            {!adding ? (
                <button onClick={() => setAdding(true)}>+ Add Resource</button>
            ) : (
                <div className="plan-resource-add">
                    <div className="plan-section-title">New Resource</div>
                    {otherResources.length > 0 && (
                        <div className="plan-resource-copyfrom">
                            <span>Copy from</span>
                            <select value={copyFrom} onChange={(e) => pickCopyFrom(e.target.value)}>
                                <option value="">None (new)</option>
                                {otherPlans.map((p) => {
                                    const rs = otherResources.filter((r) => r.plan_id === p.id);
                                    return rs.length > 0 && (
                                        <optgroup key={p.id} label={p.name}>
                                            {rs.map((r) => <option key={r.id} value={r.id}>{r.name}</option>)}
                                        </optgroup>
                                    );
                                })}
                            </select>
                        </div>
                    )}
                    <input value={newName} onChange={(e) => setNewName(e.target.value)} placeholder="Name (e.g. Duolingo, Genki I)" autoFocus
                        onKeyDown={(e) => { if (e.key === "Enter") createResource(); if (e.key === "Escape") closeAdd(); }} />
                    <input value={newType} onChange={(e) => setNewType(e.target.value)} placeholder="Type (e.g. book, website, video)"
                        onKeyDown={(e) => { if (e.key === "Enter") createResource(); if (e.key === "Escape") closeAdd(); }} />
                    <input value={newUrl} onChange={(e) => setNewUrl(e.target.value)} placeholder="URL (optional)"
                        onKeyDown={(e) => { if (e.key === "Enter") createResource(); if (e.key === "Escape") closeAdd(); }} />
                    <textarea value={newNotes} onChange={(e) => setNewNotes(e.target.value)} placeholder="Notes (optional)" rows={2}
                        style={{ fontFamily: "inherit", resize: "none" }} />
                    <div className="plan-form-actions">
                        <button className="primary" onClick={createResource}>+ Add</button>
                        <button onClick={closeAdd}>Cancel</button>
                    </div>
                </div>
            )}
        </div>
    );
}

// Todo Creator

function TodoCreator({ planId, groups, planResources, setToast, onCreated }) {
    const [open, setOpen] = useState(false);
    const [text, setText] = useState("");
    const [orderNum, setOrderNum] = useState("");
    const [selectedGroupIds, setSelectedGroupIds] = useState([]);
    const [selectedResourceIds, setSelectedResourceIds] = useState([]);
    const [frequency, setFrequency] = useState([true, true, true, true, true, true, true]);
    const [categoryMap, setCategoryMap] = useState(DEFAULT_CATEGORY());

    function toggleFrequency(i) {
        setFrequency(prev => { const c = [...prev]; c[i] = !c[i]; return c; });
    }
    function toggleCategory(bit) {
        setCategoryMap(prev => ({ ...prev, [bit]: !prev[bit] }));
    }
    function toggleGroup(id) {
        setSelectedGroupIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
    }
    function toggleResource(id) {
        setSelectedResourceIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
    }

    async function submit() {
        if (!text.trim()) { setToast("Todo description cannot be empty."); return; }
        const category = computeCategory(categoryMap);
        if (category === 0) { setToast("Please select at least one category.", "error"); return; }

        const todo = {
            plan_id: planId,
            text: text.trim(),
            category,
            frequency: computeFrequency(frequency),
        };

        try {
            const created = await loggedInvoke("create_todo", { todo });
            await Promise.all([
                selectedGroupIds.length > 0
                    ? loggedInvoke("set_todo_groups", { todoId: created.id, groupIds: selectedGroupIds })
                    : Promise.resolve(),
                selectedResourceIds.length > 0
                    ? loggedInvoke("set_todo_resources", { todoId: created.id, resourceIds: selectedResourceIds })
                    : Promise.resolve(),
            ]);
            const parsed = parseInt(orderNum, 10);
            if (!Number.isNaN(parsed)) {
                await loggedInvoke("set_todo_position", { todoId: created.id, position: parsed });
            }
            setToast("Todo created.");
            setText(""); setOrderNum(""); setSelectedGroupIds([]); setSelectedResourceIds([]);
            setFrequency([true, true, true, true, true, true, true]);
            setCategoryMap(DEFAULT_CATEGORY());
            setOpen(false);
            onCreated();
        } catch (e) { logError("catch", e); setToast("Failed to create todo.", "error"); }
    }

    return (
        <div className="plan-todo-creator">
            {!open ? (
                <button onClick={() => setOpen(true)}>+ Add Todo</button>
            ) : (
                <div className="plan-todo-form">
                    <div className="plan-section-title">New Todo</div>
                    <div>
                        <div className="plan-form-sublabel">Order</div>
                        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                            <input type="number" min="1" step="1" value={orderNum}
                                onChange={(e) => setOrderNum(e.target.value)}
                                placeholder="None" style={{ width: 70 }} />
                            <span style={{ fontSize: 11, color: "var(--t-text-3)" }}>
                                Numbered todos are listed first and the rest follow alphabetically.
                            </span>
                        </div>
                    </div>
                    <div>
                        <div className="plan-form-sublabel">Description</div>
                        <input value={text} onChange={(e) => setText(e.target.value)} autoFocus
                            onKeyDown={(e) => { if (e.key === "Enter") submit(); if (e.key === "Escape") setOpen(false); }}
                            style={{ width: "100%", boxSizing: "border-box" }} />
                    </div>

                    <div>
                        <div className="plan-form-sublabel">Categories</div>
                        <CategoryPicker categoryMap={categoryMap} onChange={toggleCategory} />
                    </div>

                    {planResources.length > 0 && (
                        <div>
                            <div className="plan-form-sublabel">Resources</div>
                            <div className="plan-pill-group">
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

                    {groups.length > 0 && (
                        <div>
                            <div className="plan-form-sublabel">Decks / Notebooks</div>
                            <div className="plan-pill-group">
                                {groups.map(g => {
                                    const active = selectedGroupIds.includes(g.id);
                                    const fam = g.group_type === "notebook" ? " active-notebook" : " active-deck";
                                    return (
                                        <label key={g.id} className={`picker-pill${active ? fam : ""}`}>
                                            <input type="checkbox" checked={active}
                                                onChange={() => toggleGroup(g.id)} style={{ margin: 0 }} />
                                            {g.name}
                                            <GroupTypeBadge type={g.group_type} />
                                        </label>
                                    );
                                })}
                            </div>
                        </div>
                    )}

                    <div>
                        <div className="plan-form-sublabel">Frequency</div>
                        <FrequencyPicker frequency={frequency} onChange={toggleFrequency} />
                    </div>

                    <div className="plan-form-actions">
                        <button className="primary" onClick={submit}>+ Add</button>
                        <button onClick={() => setOpen(false)}>Cancel</button>
                    </div>
                </div>
            )}
        </div>
    );
}

// Plans

export default function Plans({ setToast, onNavigateToGroup, returnContext, onConsumeReturnContext }) {
    const [plans, setPlans] = useState([]);
    const [loading, setLoading] = useState(true);
    const [name, setName] = useState("");
    const [editingPlan, setEditingPlan] = useState(null);
    const [todos, setTodos] = useState([]);
    const [groups, setGroups] = useState([]);
    const [planResources, setPlanResources] = useState([]);
    const [editingId, setEditingId] = useState(null);
    const [editingName, setEditingName] = useState("");
    const [collapsed, setCollapsed] = useState({});
    const [summaries, setSummaries] = useState({});
    // null = full week; 0..6 previews only that day's active todos
    const [weekDay, setWeekDay] = useState(null);
    // "only" hides todos off that day; "all" keeps them but dims them
    const [dayMode, setDayMode] = useState("only");

    const toggleSection = (key) => setCollapsed(c => ({ ...c, [key]: !c[key] }));

    useEffect(() => {
        getPlans();
        loadSummaries();
        loggedInvoke("get_groups")
            .then(setGroups)
            .catch((err) => { logError("catch", err); setToast("Failed to load groups.", "error"); });
    }, []);

    async function loadSummaries() {
        try {
            const rows = await loggedInvoke("get_plan_summaries");
            setSummaries(Object.fromEntries(rows.map(([id, todos, resources, decks]) => [id, { todos, resources, decks }])));
        } catch (err) { logError("catch", err); }
    }

    useEffect(() => {
        if (returnContext?.plan) {
            loadPlanData(returnContext.plan);
            onConsumeReturnContext();
        }
    }, [returnContext]);

    async function getPlans() {
        try { setPlans(await loggedInvoke("get_plans")); }
        catch (err) { logError("catch", err); setToast("Failed to load plans.", "error"); }
        finally { setLoading(false); }
    }

    async function loadPlanData(plan) {
        setEditingPlan(plan);
        setWeekDay(null);
        setDayMode("only");
        try {
            const [t, r] = await Promise.all([
                loggedInvoke("get_todos", { planId: plan.id }),
                loggedInvoke("get_resources", { planId: plan.id }),
            ]);
            setTodos(t);
            setPlanResources(r);
        } catch (err) { logError("catch", err); setToast("Failed to load plan data.", "error"); }
    }

    async function refreshResources() {
        if (!editingPlan) return;
        try { setPlanResources(await loggedInvoke("get_resources", { planId: editingPlan.id })); }
        catch (e) { logError("catch", e); }
    }

    async function confirmEdit(id) {
        const trimmed = editingName.trim();
        if (!trimmed) { setEditingId(null); setToast("Please enter a valid name."); return; }
        try {
            await loggedInvoke("update_plan", { id, name: trimmed });
            setPlans((prev) => prev.map((p) => p.id === id ? { ...p, name: trimmed } : p).sort(byName));
            setToast("Plan updated.");
        } catch (e) { logError("catch", e); setToast("Failed to update plan.", "error"); }
        setEditingId(null);
    }

    async function deletePlan(plan) {
        try {
            await loggedInvoke("delete_plan", { id: plan.id });
            await getPlans();
            if (editingPlan?.id === plan.id) setEditingPlan(null);
            setToast(`${plan.name} deleted.`);
        } catch (err) { logError("catch", err); setToast(`Failed to delete ${plan.name}.`, "error"); }
    }

    async function createPlan() {
        if (!name.trim()) { setToast("Please enter a valid name."); return; }
        try {
            await loggedInvoke("create_plan", { name: name.trim() });
            await getPlans();
            setToast(`${name} created.`);
            setName("");
        } catch (err) { logError("catch", err); setToast(`Failed to create ${name}.`, "error"); }
    }

    async function getTodos(planId) {
        try { setTodos(await loggedInvoke("get_todos", { planId })); }
        catch (err) { logError("catch", err); setToast("Failed to load todos.", "error"); }
    }

    const navigateFromPlan = editingPlan
        ? (group, origin) => onNavigateToGroup(group, {
              ...origin,
              menu: "plans",
              label: editingPlan.name,
              plansContext: { plan: editingPlan },
          })
        : onNavigateToGroup;

    // A day preview hides todos disabled that day, or keeps them dimmed in "all" mode.
    const displayTodos = weekDay === null || dayMode === "all" ? todos : todos.filter((t) => activeOnDay(t, weekDay));

    if (editingPlan) {
        return (
            <div className="plans-root">
                <div className="landing-hdr landing-hdr--plan">
                    <button className="quiet" onClick={() => { setEditingPlan(null); setPlanResources([]); loadSummaries(); }}>← Back</button>
                    <h2>{editingPlan.name}</h2>
                </div>
                <div className="plans-scroll">
                    <div className="plan-builder-cols">
                        <div className="plan-col-main">
                            <div className="plan-col-label plan-col-label--todos plan-col-label--toggle" onMouseDown={(e) => e.preventDefault()} onClick={() => toggleSection("todos")}>
                                Todos <span className="plan-col-label-caret">{collapsed.todos ? "▸" : "▾"}</span>
                            </div>
                            {!collapsed.todos && <>
                                {todos.length === 0 && <div className="empty-bubble">No todos yet.</div>}
                                {todos.length > 0 && (
                                    <div className="todo-filters">
                                        <div className="todo-filter-seg">
                                            <button className={weekDay === null ? "active" : ""} onClick={() => setWeekDay(null)}>All</button>
                                            {DAY_LABELS.map((d, i) => (
                                                <button key={d} className={weekDay === i ? "active" : ""} onClick={() => setWeekDay(i)}>{d}</button>
                                            ))}
                                        </div>
                                        {weekDay !== null && (
                                            <div className="todo-filter-seg">
                                                <button className={dayMode === "only" ? "active" : ""} onClick={() => setDayMode("only")}>Active only</button>
                                                <button className={dayMode === "all" ? "active" : ""} onClick={() => setDayMode("all")}>Show all</button>
                                            </div>
                                        )}
                                    </div>
                                )}
                                {todos.length > 0 && displayTodos.length === 0 && (
                                    <div className="empty-bubble">No todos on {DAY_NAMES[weekDay]}.</div>
                                )}
                                {displayTodos.map(todo => (
                                    <Todos
                                        key={todo.id}
                                        todo={todo}
                                        dimmed={weekDay !== null && dayMode === "all" && !activeOnDay(todo, weekDay)}
                                        setToast={setToast}
                                        refresh={() => getTodos(editingPlan.id)}
                                        onNavigateToGroup={navigateFromPlan}
                                        planResources={planResources}
                                        allGroups={groups}
                                        planName={editingPlan.name}
                                    />
                                ))}
                                <TodoCreator
                                    planId={editingPlan.id}
                                    groups={groups}
                                    planResources={planResources}
                                    setToast={setToast}
                                    onCreated={() => getTodos(editingPlan.id)}
                                />
                            </>}
                            <div className="plan-col-label plan-col-label--resources plan-col-label--toggle" style={{ marginTop: 24 }} onMouseDown={(e) => e.preventDefault()} onClick={() => toggleSection("resources")}>
                                Resources <span className="plan-col-label-caret">{collapsed.resources ? "▸" : "▾"}</span>
                            </div>
                            {!collapsed.resources && (
                                <div className="plan-resources-indent">
                                    <ResourcesSection planId={editingPlan.id} plans={plans} setToast={setToast} onChanged={refreshResources} />
                                </div>
                            )}
                        </div>

                        <div className="plan-col-side">
                            <div className="plan-col-label plan-col-label--srs plan-col-label--toggle" onMouseDown={(e) => e.preventDefault()} onClick={() => toggleSection("srs")}>
                                Decks <span className="plan-col-label-caret">{collapsed.srs ? "▸" : "▾"}</span>
                            </div>
                            {!collapsed.srs && <SrsSection planId={editingPlan.id} setToast={setToast} onNavigateToGroup={navigateFromPlan} />}
                        </div>
                    </div>
                </div>
            </div>
        );
    }

    return (
        <div className="plans-root">
            <div className="landing-hdr landing-hdr--plan">
                <h2>Plans</h2>
            </div>
            <div className="landing-body">
                {!loading && plans.length === 0 && <div className="landing-empty">No plans yet. Create one below.</div>}
                {plans.map(plan => (
                    <div key={plan.id} className="landing-card landing-card--plan"
                        onClick={() => editingId !== plan.id && loadPlanData(plan)}>
                        <div className="landing-card-body">
                            {editingId === plan.id ? (
                                <input value={editingName} autoFocus
                                    onClick={e => e.stopPropagation()}
                                    onChange={(e) => setEditingName(e.target.value)}
                                    onKeyDown={(e) => {
                                        if (e.key === "Enter") confirmEdit(plan.id);
                                        if (e.key === "Escape") setEditingId(null);
                                    }}
                                    onBlur={() => confirmEdit(plan.id)}
                                />
                            ) : (
                                <>
                                    <div className="landing-card-name">{plan.name}</div>
                                    <div className="landing-card-stats">
                                        <span className="landing-stat landing-stat--todos"><b>{summaries[plan.id]?.todos ?? 0}</b><span>{(summaries[plan.id]?.todos ?? 0) === 1 ? "todo" : "todos"}</span></span>
                                        <span className="landing-stat-divider" />
                                        <span className="landing-stat landing-stat--resources"><b>{summaries[plan.id]?.resources ?? 0}</b><span>{(summaries[plan.id]?.resources ?? 0) === 1 ? "resource" : "resources"}</span></span>
                                        <span className="landing-stat-divider" />
                                        <span className="landing-stat landing-stat--deck"><b>{summaries[plan.id]?.decks ?? 0}</b><span>{(summaries[plan.id]?.decks ?? 0) === 1 ? "deck" : "decks"}</span></span>
                                    </div>
                                </>
                            )}
                        </div>
                        <div className="landing-card-actions" onClick={e => e.stopPropagation()}>
                            {editingId === plan.id ? (
                                <>
                                    <button className="primary" onMouseDown={(e) => e.preventDefault()} onClick={() => confirmEdit(plan.id)}>Save</button>
                                    <button onMouseDown={(e) => e.preventDefault()} onClick={() => setEditingId(null)}>Cancel</button>
                                </>
                            ) : (
                                <>
                                    <button onClick={(e) => { e.stopPropagation(); setEditingId(plan.id); setEditingName(plan.name); }}>Edit</button>
                                    <ConfirmDelete onConfirm={() => deletePlan(plan)} small />
                                </>
                            )}
                        </div>
                    </div>
                ))}
            </div>
            <div className="landing-footer">
                <input placeholder="New plan name..." value={name}
                    onChange={(e) => setName(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && createPlan()} />
                <button className="primary" onClick={createPlan}>+ Create</button>
            </div>
        </div>
    );
}