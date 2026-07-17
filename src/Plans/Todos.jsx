import { useState, useEffect } from "react";
import { loggedInvoke, logError } from "../logger";
import { openUrl } from "@tauri-apps/plugin-opener";
import { computeFrequency, maskToArray, computeCategory, maskToCategories, FrequencyPicker, CategoryPicker, CategoryPills } from "./PlanUtils";
import { ConfirmDelete, GroupTypeBadge } from "../UIUtils";

export default function Todos({ todo, setToast, refresh, onNavigateToGroup, planResources, allGroups, planName }) {
    const [editing, setEditing] = useState(false);
    const [text, setText] = useState(todo.text);
    const [orderNum, setOrderNum] = useState(todo.position ?? "");
    const [frequency, setFrequency] = useState(() => maskToArray(todo.frequency ?? 127));
    const [categoryMap, setCategoryMap] = useState(() => maskToCategories(todo.category ?? 64));
    const [linkedGroups, setLinkedGroups] = useState([]);
    const [linkedResources, setLinkedResources] = useState([]);
    const [selectedGroupIds, setSelectedGroupIds] = useState([]);
    const [selectedResourceIds, setSelectedResourceIds] = useState([]);

    // Reload whenever the plan's resources/groups change so linked pills update live, no leave/return needed.
    useEffect(() => { loadLinks(); }, [planResources, allGroups]);

    // The backend clamps and shifts order numbers, show the value it settled on
    useEffect(() => { setOrderNum(todo.position ?? ""); }, [todo.position]);

    async function loadLinks() {
        try {
            const [g, r] = await Promise.all([
                loggedInvoke("get_todo_groups", { todoId: todo.id }),
                loggedInvoke("get_todo_resources", { todoId: todo.id }),
            ]);
            setLinkedGroups(g);
            setLinkedResources(r);
            setSelectedGroupIds(g.map(g => g.id));
            setSelectedResourceIds(r.map(r => r.id));
        } catch (e) { logError("catch", e); }
    }

    function toggleFrequency(i) {
        setFrequency(prev => { const copy = [...prev]; copy[i] = !copy[i]; return copy; });
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

    async function updateTodo() {
        if (!text.trim()) { setToast("Todo description cannot be empty."); return; }
        const category = computeCategory(categoryMap);
        if (category === 0) { setToast("Please select at least one category.", "error"); return; }
        try {
            await loggedInvoke("update_todo", { todo: {
                id: todo.id, plan_id: todo.plan_id, text: text.trim(),
                is_done: todo.is_done, is_disabled: todo.is_disabled,
                frequency: computeFrequency(frequency), category,
            }});
            await Promise.all([
                loggedInvoke("set_todo_groups", { todoId: todo.id, groupIds: selectedGroupIds }),
                loggedInvoke("set_todo_resources", { todoId: todo.id, resourceIds: selectedResourceIds }),
            ]);
            const parsed = parseInt(orderNum, 10);
            await loggedInvoke("set_todo_position", { todoId: todo.id, position: Number.isNaN(parsed) ? null : parsed });
            await loadLinks();
            await refresh();
            setToast("Todo updated.");
            setEditing(false);
        } catch (err) { logError("catch", err); setToast("Failed to update todo.", "error"); }
    }

    async function deleteTodo() {
        try {
            await loggedInvoke("delete_todo", { id: todo.id });
            await refresh();
            setToast("Todo deleted.");
        } catch (err) { logError("catch", err); setToast("Failed to delete todo.", "error"); }
    }

    if (!editing) {
        return (
            <div style={{ border: "1px solid var(--t-yellow-bdr)", borderRadius: "var(--t-r)", padding: "12px", marginBottom: "10px", opacity: todo.is_disabled && !todo.is_skipped ? 0.5 : 1, background: "linear-gradient(280deg, var(--t-yellow-bg) 0%, var(--t-surface) 55%)" }}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: "12px" }}>
                    <div style={{ flex: 1 }}>
                        <div style={{ fontSize: 17, fontWeight: 600, display: "flex", alignItems: "center", gap: 8 }}>
                            {todo.text}
                        </div>

                        <div className="todo-section">
                            <div className="todo-section-label">Categories</div>
                            <div className="todo-section-pills">
                                <CategoryPills mask={todo.category} />
                            </div>
                        </div>

                        {(linkedGroups.length > 0 || linkedResources.length > 0) && (
                            <div className="todo-section">
                                <div className="todo-section-label">Resources / Decks / Notebooks</div>
                                <div className="todo-section-pills">
                                    {linkedResources.map(r => (
                                        <span key={r.id}
                                            onClick={() => r.url && openUrl(r.url.startsWith("http") ? r.url : `https://${r.url}`)}
                                            className={`pill pill-clay${r.url ? " pill-clickable" : ""}`}>
                                            {r.name}{r.url && <span style={{ opacity: 0.55, marginLeft: 2, fontSize: 9 }}>↗</span>}
                                        </span>
                                    ))}
                                    {linkedGroups.map(g => (
                                        <span key={g.id}
                                            onClick={() => onNavigateToGroup(g, { menu: "plans", label: "Plans" })}
                                            className={`pill ${g.group_type === "notebook" ? "pill-plum" : "pill-blue"} pill-clickable`}>
                                            {g.name}
                                            <GroupTypeBadge type={g.group_type} />
                                        </span>
                                    ))}
                                </div>
                            </div>
                        )}

                        {(() => {
                            const days = ["Sun","Mon","Tue","Wed","Thu","Fri","Sat"]
                                .filter((_, i) => (todo.frequency & (1 << i)) !== 0)
                                .join(" · ");
                            return days && <div style={{ marginTop: 8, fontSize: 10, color: "var(--t-text-3)" }}>{days}</div>;
                        })()}
                    </div>

                    <div style={{ display: "flex", gap: "8px", flexShrink: 0 }}>
                        <button onClick={() => setEditing(true)}>Edit</button>
                        <ConfirmDelete onConfirm={deleteTodo} small />
                    </div>
                </div>
            </div>
        );
    }

    return (
        <div style={{ border: "1px solid var(--t-yellow-bdr)", borderRadius: "var(--t-r)", padding: "12px", marginBottom: "10px", background: "linear-gradient(280deg, var(--t-yellow-bg) 0%, var(--t-surface) 62%)" }}>
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <div>
                    <div style={{ fontSize: 12, marginBottom: 4 }}>Order</div>
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
                    <div style={{ fontSize: 12, marginBottom: 4 }}>Description</div>
                    <input value={text} onChange={(e) => setText(e.target.value)}
                        onKeyDown={(e) => { if (e.key === "Enter") updateTodo(); if (e.key === "Escape") setEditing(false); }}
                        style={{ fontSize: 14, width: "100%", boxSizing: "border-box" }} />
                </div>

                <div>
                    <div style={{ fontSize: 12, marginBottom: 2 }}>Categories</div>
                    <CategoryPicker categoryMap={categoryMap} onChange={toggleCategory} />
                </div>

                {planResources && planResources.length > 0 && (
                    <div>
                        <div style={{ fontSize: 12, marginBottom: 2 }}>Resources</div>
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

                {allGroups && allGroups.length > 0 && (
                    <div>
                        <div style={{ fontSize: 12, marginBottom: 2 }}>Decks / Notebooks</div>
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                            {allGroups.map(g => {
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
                    <div style={{ fontSize: 12, marginBottom: 2 }}>Frequency</div>
                    <FrequencyPicker frequency={frequency} onChange={toggleFrequency} />
                </div>

                <div style={{ display: "flex", gap: "8px" }}>
                    <button className="primary" onClick={updateTodo}>Save</button>
                    <button onClick={() => setEditing(false)}>Cancel</button>
                </div>
            </div>
        </div>
    );
}