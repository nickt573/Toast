import { useState, useEffect, useRef } from "react";
import { loggedInvoke, logError } from "./logger";

// Amount field plus a unit chooser. A unit is a name with any number of alternate names; you
// pick a unit and step ‹ › through its names to choose which one shows on this entry. The
// list lets you add, edit, merge, or delete units. Units are global, shared by every picker.
export default function UnitPicker({ value, variantId, onChange, setToast, onUnitsChanged, autoFocusValue = false }) {
  const warn = (msg) => setToast ? setToast(msg, "warn") : setErr(msg);
  // Reload the picker's own list, then let the parent refresh anything showing unit names so
  // a rename, merge, or delete lands in the stat table and graph at once, never stale.
  const reload = async () => { const u = await load(); onUnitsChanged?.(); return u; };
  const [units, setUnits] = useState([]);
  const [open, setOpen] = useState(false);
  const [view, setView] = useState("list");   // "list" | "create" | groupId (editing)
  const [draft, setDraft] = useState([""]);    // names while creating; first is the primary
  const [addName, setAddName] = useState("");  // new alternate name while editing a unit
  const [nameEdits, setNameEdits] = useState({}); // in-progress name text while editing, keyed by variant id
  const [merging, setMerging] = useState(false);
  const [mergeSel, setMergeSel] = useState([]); // group ids to merge, in pick order; first is main
  const [pendingDel, setPendingDel] = useState(null); // { kind, id, name, uses } awaiting confirm
  const [err, setErr] = useState("");
  const wrapRef = useRef(null);

  const load = async () => {
    try { const u = await loggedInvoke("get_units"); setUnits(u); return u; }
    catch (e) { logError("catch", e); return units; }
  };
  useEffect(() => { load(); }, []);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e) => { if (wrapRef.current && !wrapRef.current.contains(e.target)) closeMenu(); };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const closeMenu = () => { setOpen(false); setView("list"); setErr(""); setAddName(""); setNameEdits({}); setMerging(false); setMergeSel([]); };
  const backToList = () => { setView("list"); setErr(""); setAddName(""); setNameEdits({}); setPendingDel(null); };
  // Editing a unit is done, not saved-or-discarded, so leaving only checks nothing is blank.
  const finishEdit = () => {
    if (editUnit && editUnit.variants.some(v => (nameEdits[v.id] ?? v.name).trim() === "")) {
      warn("A name can't be empty."); return;
    }
    backToList();
  };
  const startMerge = () => { setMerging(true); setMergeSel([]); setErr(""); };
  const cancelMerge = () => { setMerging(false); setMergeSel([]); };
  const toggleMerge = (groupId) => setMergeSel(sel => sel.includes(groupId) ? sel.filter(g => g !== groupId) : [...sel, groupId]);

  const selUnit = units.find(u => u.variants.some(v => v.id === variantId)) || null;
  const selVariant = selUnit ? selUnit.variants.find(v => v.id === variantId) : null;
  const editUnit = typeof view === "number" ? units.find(u => u.id === view) : null;

  const pick = (vid) => onChange({ value, variantId: vid });
  const clearUnit = (e) => { e?.stopPropagation(); onChange({ value, variantId: null }); };

  const cycle = (dir) => {
    if (!selUnit || selUnit.variants.length < 2) return;
    const vs = selUnit.variants;
    const i = vs.findIndex(v => v.id === variantId);
    pick(vs[(i + dir + vs.length) % vs.length].id);
  };

  const errorFor = (e, fallback) => {
    const s = String(e);
    if (s.includes("used by")) return "That name is in use, so it can't be deleted.";
    if (s.includes("at least one")) return "A unit needs at least one name.";
    return fallback;
  };

  const submitCreate = async () => {
    const names = draft.map(s => s.trim());
    // Every field must hold a name: a blank one warns and stops rather than being dropped.
    if (names.some(s => !s)) { warn("Enter a name for the unit."); return; }
    try {
      const groupId = await loggedInvoke("create_unit", { names });
      await reload();
      onChange({ value, variantId: groupId }); // the primary name's id names the group
      setDraft([""]); closeMenu();
    } catch (e) { logError("catch", e); setErr("Could not create that unit."); }
  };

  const renameVariant = async (id, name) => {
    if (!name.trim()) return;
    try { await loggedInvoke("rename_variant", { id, name: name.trim() }); await reload(); }
    catch (e) { logError("catch", e); setErr("Could not rename that name."); }
  };

  const addVariant = async (groupId) => {
    const name = addName.trim();
    if (!name) { warn("Enter a name to add."); return; }
    try { await loggedInvoke("add_variant", { groupId, name }); setAddName(""); await reload(); }
    catch (e) { logError("catch", e); setErr("Could not add that name."); }
  };

  const makeMain = async (id) => {
    try { await loggedInvoke("set_main_variant", { id }); await reload(); }
    catch (e) { logError("catch", e); setErr("Could not change the main name."); }
  };

  // Deleting a name or unit that entries chose clears it off them, so it asks first when the
  // count is above zero; an unused one goes straight away.
  const askDeleteVariant = (v) => v.uses > 0
    ? setPendingDel({ kind: "variant", id: v.id, name: v.name, uses: v.uses })
    : doDeleteVariant(v.id);
  const askDeleteUnit = (u) => {
    const uses = u.variants.reduce((s, v) => s + v.uses, 0);
    if (uses > 0) setPendingDel({ kind: "unit", id: u.id, name: u.variants[0].name, uses });
    else doDeleteUnit(u.id);
  };
  const confirmPendingDelete = async () => {
    const p = pendingDel;
    setPendingDel(null);
    if (p.kind === "variant") await doDeleteVariant(p.id);
    else await doDeleteUnit(p.id);
  };

  const doDeleteVariant = async (id) => {
    try {
      await loggedInvoke("delete_variant", { id });
      if (variantId === id) onChange({ value, variantId: null });
      await reload();
    } catch (e) { logError("catch", e); setErr(errorFor(e, "Could not delete that name.")); }
  };

  const doDeleteUnit = async (groupId) => {
    try {
      await loggedInvoke("delete_unit", { groupId });
      if (selUnit && selUnit.id === groupId) onChange({ value, variantId: null });
      await reload();
      backToList();
    } catch (e) { logError("catch", e); setErr(errorFor(e, "Could not delete that unit.")); }
  };

  // The first unit picked is the main; the rest fold into it, their names becoming its
  // alternates. Entries keep the name they logged, now counting under the main.
  const submitMerge = async () => {
    if (mergeSel.length < 2) return;
    const [main, ...rest] = mergeSel;
    try {
      for (const from of rest) await loggedInvoke("merge_units", { fromGroup: from, intoGroup: main });
      // The chosen name keeps its id through a merge, so the selection stays valid on its own,
      // still showing the exact spelling that was picked, now grouped under the main.
      await reload();
      cancelMerge();
    } catch (e) { logError("catch", e); setErr("Could not merge those units."); }
  };

  return (
    <div className="unit-picker" ref={wrapRef}>
      <input
        type="number" min="0" step="any"
        value={value}
        autoFocus={autoFocusValue}
        onChange={e => onChange({ value: e.target.value, variantId })}
        placeholder="0"
        className="unit-picker-value"
      />

      <div className="unit-picker-control">
        {selVariant && selUnit.variants.length > 1 && (
          <button type="button" className="unit-picker-arrow" title="Previous name" onClick={() => cycle(-1)}>‹</button>
        )}
        <button type="button" className={`unit-picker-select${selVariant ? " has-unit" : ""}`}
          onClick={() => { setOpen(o => !o); setView("list"); setErr(""); }}>
          <span className="unit-picker-select-label">{selVariant ? selVariant.name : "No unit"}</span>
          {selVariant
            ? <span className="unit-picker-clear" title="Clear unit" onClick={clearUnit}>×</span>
            : <span className="unit-picker-caret">▾</span>}
        </button>
        {selVariant && selUnit.variants.length > 1 && (
          <button type="button" className="unit-picker-arrow" title="Next name" onClick={() => cycle(1)}>›</button>
        )}
      </div>

      {open && (
        <div className="unit-picker-menu">
          {view === "list" && (
            <>
              <div className="unit-picker-list">
                {!merging && (
                  <div className="unit-picker-row">
                    <button type="button" className={`unit-picker-opt${variantId === null ? " active" : ""}`}
                      onClick={() => { onChange({ value, variantId: null }); closeMenu(); }}>
                      No unit
                    </button>
                  </div>
                )}
                {units.map(u => {
                  const mi = mergeSel.indexOf(u.id);
                  const selected = merging ? mi >= 0 : (selUnit && selUnit.id === u.id);
                  return (
                    <div key={u.id} className={`unit-picker-row${selected ? " active" : ""}`}>
                      <button type="button" className="unit-picker-opt"
                        onClick={() => merging ? toggleMerge(u.id) : (pick(u.variants[0].id), closeMenu())}>
                        {merging && mi >= 0 && <span className="unit-picker-tag">{mi === 0 ? "main" : mi + 1}</span>}
                        <span className="unit-picker-optname">{u.variants[0].name}</span>
                        {u.variants.length > 1 && <span className="unit-picker-count">+{u.variants.length - 1}</span>}
                      </button>
                      {!merging && (
                        <button type="button" className="st-btn-sm" onClick={() => { setView(u.id); setErr(""); setAddName(""); setNameEdits({}); }}>Edit</button>
                      )}
                    </div>
                  );
                })}
                {units.length === 0 && <div className="unit-picker-hint">No units yet.</div>}
              </div>
              {err && <div className="unit-picker-err">{err}</div>}
              {merging ? (
                <div className="unit-picker-actions col">
                  <div className="unit-picker-actionrow">
                    <button type="button" className="primary" disabled={mergeSel.length < 2} onClick={submitMerge}>Merge ({mergeSel.length})</button>
                    <button type="button" onClick={cancelMerge}>Cancel</button>
                  </div>
                </div>
              ) : (
                <div className="unit-picker-actions">
                  <button type="button" className="primary" onClick={() => { setDraft([""]); setView("create"); setErr(""); }}>+ Add Unit</button>
                  {units.length >= 2 && <button type="button" onClick={startMerge}>Merge</button>}
                </div>
              )}
            </>
          )}

          {view === "create" && (
            <>
              <div className="unit-picker-form">
                <div className="unit-picker-form-title">New Unit</div>
                {draft.map((name, i) => (
                  <div key={i} className={`unit-picker-fieldrow${i === 0 ? " is-main" : ""}`} title={i === 0 ? "The name shown by default" : undefined}>
                    <input autoFocus={i === 0} className="unit-picker-field" placeholder={i === 0 ? "Name" : "Alternate name"} value={name}
                      onChange={e => setDraft(d => d.map((x, j) => j === i ? e.target.value : x))}
                      onKeyDown={e => { if (e.key === "Enter") submitCreate(); }} />
                    {i !== 0 && (
                      <button type="button" className="unit-picker-swap" title="Make main"
                        onClick={() => setDraft(d => { const a = [...d]; const [x] = a.splice(i, 1); a.unshift(x); return a; })}>⇅</button>
                    )}
                    {draft.length > 1 && (
                      <button type="button" className="unit-picker-del" title="Remove"
                        onClick={() => setDraft(d => d.filter((_, j) => j !== i))}>×</button>
                    )}
                  </div>
                ))}
                <button type="button" className="st-btn-sm unit-picker-addalt" onClick={() => setDraft(d => [...d, ""])}>+ Add Alternate Name</button>
                {err && <div className="unit-picker-err">{err}</div>}
              </div>
              <div className="unit-picker-actions">
                <button type="button" className="primary" onClick={submitCreate}>Save</button>
                <button type="button" onClick={backToList}>Cancel</button>
              </div>
            </>
          )}

          {editUnit && (
            <>
              <div className="unit-picker-form">
                <div className="unit-picker-form-title">Edit unit</div>
                {editUnit.variants.map((v, i) => (
                  <div key={v.id} className={`unit-picker-fieldrow${i === 0 ? " is-main" : ""}`} title={i === 0 ? "The name shown by default" : undefined}>
                    <input value={nameEdits[v.id] ?? v.name} className="unit-picker-field"
                      onChange={e => setNameEdits(m => ({ ...m, [v.id]: e.target.value }))}
                      onBlur={e => {
                        const val = e.target.value.trim();
                        if (!val) { warn("A name can't be empty."); return; }
                        if (val !== v.name) renameVariant(v.id, val);
                      }}
                      onKeyDown={e => { if (e.key === "Enter") e.target.blur(); }} />
                    {i !== 0 && (
                      <button type="button" className="unit-picker-swap" title="Make main" onClick={() => makeMain(v.id)}>⇅</button>
                    )}
                    {editUnit.variants.length > 1 && (
                      <button type="button" className="unit-picker-del" title="Delete name" onClick={() => askDeleteVariant(v)}>×</button>
                    )}
                  </div>
                ))}
                <div className="unit-picker-fieldrow">
                  <input value={addName} className="unit-picker-field" placeholder="Alternate name"
                    onChange={e => setAddName(e.target.value)}
                    onKeyDown={e => { if (e.key === "Enter") addVariant(editUnit.id); }} />
                  <button type="button" className="primary" onClick={() => addVariant(editUnit.id)}>+ Add</button>
                </div>
                {err && <div className="unit-picker-err">{err}</div>}
              </div>
              {pendingDel ? (
                <div className="unit-picker-actions col">
                  <div className="unit-picker-confirm-msg">
                    "{pendingDel.name}" is used by {pendingDel.uses} {pendingDel.uses === 1 ? "entry" : "entries"}. Delete it?
                  </div>
                  <div className="unit-picker-actionrow">
                    <button type="button" onClick={() => setPendingDel(null)}>Cancel</button>
                    <button type="button" className="danger" onClick={confirmPendingDelete}>Delete</button>
                  </div>
                </div>
              ) : (
                <div className="unit-picker-actions">
                  <button type="button" className="primary" onClick={finishEdit}>Done</button>
                  <button type="button" className="danger" onClick={() => askDeleteUnit(editUnit)}>Delete Unit</button>
                </div>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
