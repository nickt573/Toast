import { useState, useEffect, useLayoutEffect, useRef } from "react";
import { loggedInvoke, logError } from "../logger";
import { open } from "@tauri-apps/plugin-dialog";
import { CardFace, AudioPlayer, renderAnkiHtml, stripHtml } from "./CardFace";
import { Tip, ConfirmDelete } from "../UIUtils";
import "./Decks.css";

const VIEW_DECKS = "decks";
const VIEW_CARDS = "cards";

async function pickFile(extensions) {
  try {
    const path = await open({ multiple: false, filters: [{ name: "File", extensions }] });
    return path ?? null;
  } catch { return null; }
}

function emptyNewCard(groupId) {
  return { group_id: groupId, front: "", back: "", is_searchable: true, is_uploaded: false, support: "", imported_support: null, front_image: null, back_image: null, front_audio: null, back_audio: null };
}

const parseFile = (file) => {
  if (!file) return "";
  return file.split(/[\\/]/).pop();
};

// Matches the backend's ORDER BY name COLLATE NOCASE
const byName = (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });

// ─── Deck List ────────────────────────────────────────────────────────────────

function DeckList({ setToast, onOpenDeck }) {
  const [decks, setDecks] = useState([]);
  const [loading, setLoading] = useState(true);
  const [plans, setPlans] = useState([]);
  const [cardCounts, setCardCounts] = useState({});
  const [newName, setNewName] = useState("");
  const [editingId, setEditingId] = useState(null);
  const [editingName, setEditingName] = useState("");
  const [ankiPending, setAnkiPending] = useState(null);
  const [ankiFrontIndices, setAnkiFrontIndices] = useState([]);
  const [ankiBackIndices, setAnkiBackIndices] = useState([]);
  const [ankiSupportIndices, setAnkiSupportIndices] = useState([]);
  const [ankiCreateFlipped, setAnkiCreateFlipped] = useState(false);
  const [ankiMakeSearchable, setAnkiMakeSearchable] = useState(false);
  const [merging, setMerging] = useState(false);
  const [mergeDeckA, setMergeDeckA] = useState(null);
  const [mergeDeckB, setMergeDeckB] = useState(null);
  const [mergeName, setMergeName] = useState("");
  const [mergeReset, setMergeReset] = useState(false);

  useEffect(() => {
    loggedInvoke("get_decks").then(setDecks).catch(e => logError("catch", e)).finally(() => setLoading(false));
    loggedInvoke("get_plans").then(setPlans).catch(e => logError("catch", e));
    loggedInvoke("get_deck_card_counts")
      .then(rows => setCardCounts(Object.fromEntries(rows)))
      .catch(e => logError("catch", e));
  }, []);

  const getPlanName = (planId) => plans.find((p) => p.id === planId)?.name ?? null;

  const createDeck = async () => {
    const name = newName.trim();
    if (!name) return;
    try {
      const deck = await loggedInvoke("create_deck", { name });
      setDecks((prev) => [...prev, deck].sort(byName));
      setToast(`${deck.name} successfully created.`);
      setNewName("");
    } catch (e) { logError("catch", e); setToast("Unable to create new deck.", "error"); }
  };

  const pickAnkiFile = async () => {
    const path = await pickFile(["apkg"]);
    if (!path) return;
    try {
      const fields = await loggedInvoke("peek_anki_fields", { path });
      setAnkiPending({ path, fields });
      setAnkiFrontIndices(fields.length > 0 ? [0] : []);
      setAnkiBackIndices(fields.length > 1 ? [1] : []);
      setAnkiSupportIndices([]);
      setAnkiCreateFlipped(false);
      setAnkiMakeSearchable(false);
    } catch (e) { logError("catch", e); setToast(`Failed to read deck: ${e}`, "error"); }
  };

  const confirmAnkiImport = async () => {
    if (!ankiPending) return;
    if (ankiFrontIndices.length === 0) { setToast("Please select at least one front field."); return; }
    if (ankiBackIndices.length === 0) { setToast("Please select at least one back field."); return; }
    try {
      const [, count] = await loggedInvoke("import_anki_deck", {
        path: ankiPending.path,
        frontFieldIndices: ankiFrontIndices,
        backFieldIndices: ankiBackIndices,
        supportFieldIndices: ankiSupportIndices,
        createFlipped: ankiCreateFlipped,
        isSearchable: ankiMakeSearchable,
      });
      await loggedInvoke("cleanup_orphaned_media");
      const [updatedDecks, counts] = await Promise.all([
        loggedInvoke("get_decks"),
        loggedInvoke("get_deck_card_counts"),
      ]);
      setDecks(updatedDecks);
      setCardCounts(Object.fromEntries(counts));
      setToast(`Imported ${count} cards successfully.`);
    } catch (e) { logError("catch", e); setToast(`Import failed: ${e}`, "error"); }
    finally { setAnkiPending(null); }
  };

  const startEdit = (deck, e) => { e.stopPropagation(); setEditingId(deck.id); setEditingName(deck.name); };

  const confirmEdit = async (id) => {
    const name = editingName.trim();
    if (!name) { setEditingId(null); setToast("Please choose a valid name."); return; }
    try {
      await loggedInvoke("update_deck", { deck: { id, name, group_type: "deck" } });
      setDecks((d) => d.map((dk) => dk.id === id ? { ...dk, name } : dk).sort(byName));
      setToast(`${editingName} successfully updated.`);
    } catch (e) { logError("catch", e); setToast(`Failed to update ${editingName}`, "error"); }
    setEditingId(null);
  };

  const deleteDeck = async (id) => {
    const target = decks.find((d) => d.id === id);
    try {
      await loggedInvoke("delete_deck", { id });
      setDecks((d) => d.filter((dk) => dk.id !== id));
      setToast(`${target?.name ?? "Deck"} successfully deleted.`);
    } catch (e) { logError("catch", e); setToast(`Failed to delete ${target?.name ?? "Deck"}.`, "error"); }
  };

  const startMerge = () => {
    setMerging(true);
    setMergeDeckA(decks.length > 0 ? decks[0].id : null);
    setMergeDeckB(decks.length > 1 ? decks[1].id : null);
    setMergeName("");
    setMergeReset(false);
  };

  const confirmMerge = async () => {
    if (!mergeDeckA || !mergeDeckB) { setToast("Please select two decks."); return; }
    if (mergeDeckA === mergeDeckB) { setToast("Please select two different decks."); return; }
    if (!mergeName.trim()) { setToast("Please enter a name for the merged deck."); return; }
    try {
      const newDeck = await loggedInvoke("merge_decks", {
        deckAId: mergeDeckA,
        deckBId: mergeDeckB,
        newName: mergeName.trim(),
        reset: mergeReset,
      });
      const [updatedDecks, counts] = await Promise.all([
        loggedInvoke("get_decks"),
        loggedInvoke("get_deck_card_counts"),
      ]);
      setDecks(updatedDecks);
      setCardCounts(Object.fromEntries(counts));
      setToast(`Decks merged into ${newDeck.name}.`);
      setMerging(false);
    } catch (e) { logError("catch", e); setToast("Failed to merge decks.", "error"); }
  };

  return (
    <>
      <div className="landing-hdr landing-hdr--deck">
        <h2>Decks</h2>
        <button onClick={startMerge} disabled={decks.length < 2}>Merge Decks</button>
        <button onClick={pickAnkiFile}>Import Anki</button>
      </div>

      {merging && (
        <div className="dk-merge-panel">
          <div style={{ fontSize: 13, fontWeight: 500, color: "var(--t-text)" }}>Merge two decks</div>
          <div className="dk-merge-row">
            <select value={mergeDeckA ?? ""} onChange={(e) => setMergeDeckA(Number(e.target.value))}>
              {decks.filter(d => d.id !== mergeDeckB).map(d => <option key={d.id} value={d.id}>{d.name}</option>)}
            </select>
            <span style={{ fontSize: 12, color: "var(--t-text-3)" }}>+</span>
            <select value={mergeDeckB ?? ""} onChange={(e) => setMergeDeckB(Number(e.target.value))}>
              {decks.filter(d => d.id !== mergeDeckA).map(d => <option key={d.id} value={d.id}>{d.name}</option>)}
            </select>
          </div>
          <input type="text" placeholder="New deck name..." value={mergeName}
            onChange={(e) => setMergeName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") confirmMerge(); if (e.key === "Escape") setMerging(false); }} />
          <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 13, cursor: "pointer", color: "var(--t-text-2)" }}>
            <input type="checkbox" checked={mergeReset} onChange={(e) => setMergeReset(e.target.checked)} />
            Reset progress on merged cards
          </label>
          <div style={{ fontSize: 11, color: "var(--t-text-3)" }}>
            The two source decks will be deleted after their cards move into the new deck. Stats history is preserved.
          </div>
          <div style={{ display: "flex", gap: 8 }}>
            <button className="primary" onClick={confirmMerge}>Merge</button>
            <button onClick={() => setMerging(false)}>Cancel</button>
          </div>
        </div>
      )}

      {ankiPending && (
        <div className="dk-merge-panel">
          <div style={{ fontSize: 13, fontWeight: 500, color: "var(--t-text)" }}>Map fields</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 5 }}>
            <div style={{ display: "flex", alignItems: "center", fontSize: 11, color: "var(--t-text-3)", paddingBottom: 4, borderBottom: "1px solid var(--t-border)" }}>
              <span style={{ flex: 1 }}>Field</span>
              <span style={{ width: 50, textAlign: "center" }}>Front</span>
              <span style={{ width: 50, textAlign: "center" }}>Back</span>
              <span style={{ width: 56, textAlign: "center", display: "inline-flex", justifyContent: "center", alignItems: "center", gap: 2 }}>
                Support
                <Tip text="Support fields are shown after flipping a card but stay out of similar-card matching. Map example sentences, mnemonics, or notes here to keep the similar cards panel clean. Flipped copies keep the same support. Fields mapped here can't be edited after import, but you can always add your own notes alongside them." />
              </span>
            </div>
            {ankiPending.fields.map((f, i) => (
              <div key={i} style={{ display: "flex", alignItems: "center", background: i % 2 === 1 ? "rgba(26,18,8,0.03)" : "transparent", borderRadius: 4, padding: "2px 0" }}>
                <span style={{ flex: 1, fontSize: 13, color: "var(--t-text)" }}>
                  <span style={{ color: "var(--t-text-3)", fontSize: 11, marginRight: 6, fontVariantNumeric: "tabular-nums" }}>{i + 1}.</span>
                  {f}
                </span>
                <label style={{ width: 50, display: "flex", justifyContent: "center", alignItems: "center", cursor: "pointer" }}>
                  <input type="checkbox" checked={ankiFrontIndices.includes(i)}
                    onChange={() => setAnkiFrontIndices(prev => prev.includes(i) ? prev.filter(x => x !== i) : [...prev, i])} />
                </label>
                <label style={{ width: 50, display: "flex", justifyContent: "center", alignItems: "center", cursor: "pointer" }}>
                  <input type="checkbox" checked={ankiBackIndices.includes(i)}
                    onChange={() => setAnkiBackIndices(prev => prev.includes(i) ? prev.filter(x => x !== i) : [...prev, i])} />
                </label>
                <label style={{ width: 56, display: "flex", justifyContent: "center", alignItems: "center", cursor: "pointer" }}>
                  <input type="checkbox" checked={ankiSupportIndices.includes(i)}
                    onChange={() => setAnkiSupportIndices(prev => prev.includes(i) ? prev.filter(x => x !== i) : [...prev, i])} />
                </label>
              </div>
            ))}
          </div>
          <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 13, cursor: "pointer", color: "var(--t-text-2)" }}>
            <input type="checkbox" checked={ankiMakeSearchable} onChange={(e) => setAnkiMakeSearchable(e.target.checked)} />
            Make all searchable
            <Tip text="Searchable cards appear in the similar cards panel shown during study sessions. Similar cards are based on any matching terms separated by commas and ignore anything in parentheses." />
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 13, cursor: "pointer", color: "var(--t-text-2)" }}>
            <input type="checkbox" checked={ankiCreateFlipped} onChange={(e) => setAnkiCreateFlipped(e.target.checked)} />
            Create flipped copies
          </label>
          <div style={{ display: "flex", gap: 8 }}>
            <button className="primary" onClick={confirmAnkiImport}>Import</button>
            <button onClick={() => setAnkiPending(null)}>Cancel</button>
          </div>
          <div style={{ fontSize: 11, color: "var(--t-text-3)" }}>
            Cards may not appear exactly one-to-one as in Anki.
          </div>
        </div>
      )}

      <div className="dk-list">
        {!loading && decks.length === 0 && <div className="landing-empty">No decks yet. Create one below.</div>}
        {decks.map((deck) => (
          <div className="landing-card landing-card--deck" key={deck.id} onClick={() => onOpenDeck(deck)}>
            <div className="landing-card-body">
              {editingId === deck.id ? (
                <input className="dk-deck-name-input" value={editingName} autoFocus
                  onClick={(e) => e.stopPropagation()}
                  onChange={(e) => setEditingName(e.target.value)}
                  onKeyDown={(e) => { if (e.key === "Enter") confirmEdit(deck.id); if (e.key === "Escape") setEditingId(null); }}
                  onBlur={() => confirmEdit(deck.id)} />
              ) : (
                <>
                  <span className="dk-deck-name">{deck.name}</span>
                  <div className="landing-card-stats">
                    <span className="landing-stat landing-stat--card">
                      <b>{cardCounts[deck.id] ?? 0}</b> {(cardCounts[deck.id] ?? 0) === 1 ? "card" : "cards"}
                    </span>
                    {deck.plan_id && getPlanName(deck.plan_id) && (
                      <span className="landing-stat landing-stat--plan">{getPlanName(deck.plan_id)}</span>
                    )}
                  </div>
                </>
              )}
            </div>
            <div className="landing-card-actions" onClick={(e) => e.stopPropagation()}>
              <button onClick={(e) => startEdit(deck, e)}>Edit</button>
              <ConfirmDelete onConfirm={() => deleteDeck(deck.id)} small />
            </div>
          </div>
        ))}
      </div>
      <div className="dk-new-deck">
        <input type="text" placeholder="New deck name..." value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && createDeck()} />
        <button className="primary" onClick={createDeck}>Create</button>
      </div>
    </>
  );
}

// ─── Card Editor ──────────────────────────────────────────────────────────────

function CardEditor({ setToast, card, onSaved, onDeleted, onMarkedForReview, inPlan }) {
  const [form, setForm] = useState(null);
  const [previewing, setPreviewing] = useState(false);
  const [previewFlipped, setPreviewFlipped] = useState(false);
  const [cardLog, setCardLog] = useState([]);
  const paneRef = useRef(null);
  const scrollTopRef = useRef(0);
  const prevCardIdRef = useRef(null);

  useEffect(() => {
    if (!card) { setForm(null); setPreviewing(false); setPreviewFlipped(false); setCardLog([]); return; }
    setForm({
      ...card,
      support: card.support ?? "",
      front_image: card.front_image ?? null,
      back_image: card.back_image ?? null,
      front_audio: card.front_audio ?? null,
      back_audio: card.back_audio ?? null,
    });
    let cancelled = false;
    loggedInvoke("get_card_grade_log", { cardId: card.id })
      .then((log) => { if (!cancelled) setCardLog(log); })
      .catch((e) => { logError("get_card_grade_log", e); if (!cancelled) setCardLog([]); });
    return () => { cancelled = true; };
  }, [card?.id]);

  useEffect(() => {
    if (!card || !form || card.id !== form.id) return;
    setForm(f => ({ ...f, is_paused: card.is_paused }));
  }, [card?.is_paused]);

  useEffect(() => {
    if (!card || !form || card.id !== form.id) return;
    setForm(f => ({ ...f, is_searchable: card.is_searchable }));
  }, [card?.is_searchable]);

  useLayoutEffect(() => {
    if (!paneRef.current) return;
    if (prevCardIdRef.current !== (form?.id ?? null)) {
      paneRef.current.scrollTop = 0;
      scrollTopRef.current = 0;
      prevCardIdRef.current = form?.id ?? null;
    } else {
      paneRef.current.scrollTop = scrollTopRef.current;
    }
  });

  if (!form) return <div className="dk-editor-none">Select a card to edit.</div>;

  const set = (key, val) => setForm((f) => ({ ...f, [key]: val }));

  const pickFrontImage = async () => { const p = await pickFile(["png","jpg","jpeg","gif","webp"]); if (p) set("front_image", p); };
  const pickBackImage  = async () => { const p = await pickFile(["png","jpg","jpeg","gif","webp"]); if (p) set("back_image", p); };
  const pickFrontAudio = async () => { const p = await pickFile(["mp3","wav","ogg","m4a"]); if (p) set("front_audio", p); };
  const pickBackAudio  = async () => { const p = await pickFile(["mp3","wav","ogg","m4a"]); if (p) set("back_audio", p); };

  const save = async () => {
    try {
      await loggedInvoke("update_card", { card: { ...form, support: form.support || null, front_image: form.front_image || null, back_image: form.back_image || null, front_audio: form.front_audio || null, back_audio: form.back_audio || null } });
      setToast("Card successfully updated.");
      onSaved({ ...form });
    } catch (e) { logError("catch", e); setToast("Failed to update card.", "error"); }
  };

  const deleteCard = async () => {
    try {
      await loggedInvoke("delete_card", { id: form.id });
      onDeleted(form.id);
      setToast("Card successfully deleted.");
    } catch (e) { logError("catch", e); setToast("Failed to delete card.", "error"); }
  };

  const markForReview = async () => {
    try {
      await loggedInvoke("mark_for_review", { cardId: form.id });
      setToast("Card marked for review.");
      onMarkedForReview(form.id);
    } catch (e) { logError("catch", e); setToast("Failed to mark card for review.", "error"); }
  };

  const handlePaneScroll = (e) => { scrollTopRef.current = e.currentTarget.scrollTop; };

  const cardLogSummary = cardLog.length > 0 ? (() => {
    const total = cardLog.length;
    const retentionTotal = cardLog.filter(e => e.old_tier != 0).length;
    const correct = cardLog.filter(e => (e.grade >= 2 && e.old_tier != 0)).length;
    const isReview = cardLog.some(e => e.old_tier != 0);
    const retention = Math.round((correct / retentionTotal) * 100);
    const last = cardLog[0]?.graded_at;
    return (
      <div className="dk-card-log">
        {form.tier === 0 ? "New" : "Review"}
        {` · Review retention: ${isReview ? retention + "%" : "N/A"}`}
        {total === 1 ? ` · Seen ${total} time` : ` · Seen ${total} times`}
        {last && ` · Last seen: ${last}`}
      </div>
    );
  })() : (
    <div className="dk-card-log">New · Unseen</div>
  );

  if (previewing) {
    return (
      <div className="dk-editor-pane" ref={paneRef} onScroll={handlePaneScroll}>
        <div className="dk-editor-topbar">
          <button className="quiet" onClick={() => setPreviewing(false)}>← Back to Edit</button>
          {form.is_uploaded && <span className="dk-uploaded-badge">Anki Import</span>}
          <button style={{ marginLeft: "auto" }} onClick={() => setPreviewFlipped((f) => !f)}>
            {previewFlipped ? "Show Front" : "Show All"}
          </button>
        </div>
        <div className="dk-preview">
          <div className="dk-preview-label">{previewFlipped ? "Front + Back" : "Front"}</div>
          <div className="dk-preview-card">
            <CardFace card={form} showBack={previewFlipped} />
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="dk-editor-pane" ref={paneRef} onScroll={handlePaneScroll}>
      <div className="dk-editor-topbar">
        <button onClick={() => { setPreviewing(true); setPreviewFlipped(false); }}>Preview</button>
        {form.is_uploaded && <span className="dk-uploaded-badge">Anki Import</span>}
      </div>

      <div className="dk-field">
        <label>Front {form.is_uploaded && <span style={{ fontWeight: 400, textTransform: "none", fontSize: 11, color: "var(--t-text-3)" }}>(read-only)</span>}</label>
        {form.is_uploaded ? (
          <div className="dk-field-uploaded" dangerouslySetInnerHTML={{ __html: renderAnkiHtml(form.front) }} />
        ) : (
          <textarea rows={3} value={form.front} onChange={(e) => set("front", e.target.value)} />
        )}
      </div>

      {!form.is_uploaded && (
        <div className="dk-field">
          <label>Front Image</label>
          <div className="dk-file-row">
            <input type="text" value={parseFile(form.front_image ?? "")} readOnly placeholder="No file selected" />
            <button onClick={pickFrontImage}>Browse</button>
            {form.front_image && <button onClick={() => set("front_image", null)}>Clear</button>}
          </div>
        </div>
      )}

      {!form.is_uploaded && (
        <div className="dk-field">
          <label>Front Audio</label>
          {form.front_audio && (
            <div style={{ marginBottom: 6, display: "flex", flexDirection: "column", gap: 4 }}>
              <AudioPlayer path={form.front_audio} style={{ alignSelf: "flex-start" }} />
              <div className="dk-file-row">
                <input type="text" value={parseFile(form.front_audio)} readOnly style={{ flex: 1 }} />
                <button onClick={() => set("front_audio", null)}>Clear</button>
              </div>
            </div>
          )}
          <button onClick={pickFrontAudio} style={{ alignSelf: "flex-start" }}>
            {form.front_audio ? "Replace" : "+ Add Front Audio"}
          </button>
        </div>
      )}

      <div className="dk-field">
        <label>Back {form.is_uploaded && <span style={{ fontWeight: 400, textTransform: "none", fontSize: 11, color: "var(--t-text-3)" }}>(read-only)</span>}</label>
        {form.is_uploaded ? (
          <div className="dk-field-uploaded" dangerouslySetInnerHTML={{ __html: renderAnkiHtml(form.back) }} />
        ) : (
          <textarea rows={3} value={form.back} onChange={(e) => set("back", e.target.value)} />
        )}
      </div>

      {!form.is_uploaded && (
        <div className="dk-field">
          <label>Back Image</label>
          <div className="dk-file-row">
            <input type="text" value={parseFile(form.back_image ?? "")} readOnly placeholder="No file selected" />
            <button onClick={pickBackImage}>Browse</button>
            {form.back_image && <button onClick={() => set("back_image", null)}>Clear</button>}
          </div>
        </div>
      )}

      {!form.is_uploaded && (
        <div className="dk-field">
          <label>Back Audio</label>
          {form.back_audio && (
            <div style={{ marginBottom: 6, display: "flex", flexDirection: "column", gap: 4 }}>
              <AudioPlayer path={form.back_audio} style={{ alignSelf: "flex-start" }} />
              <div className="dk-file-row">
                <input type="text" value={parseFile(form.back_audio)} readOnly style={{ flex: 1 }} />
                <button onClick={() => set("back_audio", null)}>Clear</button>
              </div>
            </div>
          )}
          <button onClick={pickBackAudio} style={{ alignSelf: "flex-start" }}>
            {form.back_audio ? "Replace" : "+ Add Back Audio"}
          </button>
        </div>
      )}

      {form.imported_support && (
        <div className="dk-field">
          <label>Anki Support <span style={{ fontWeight: 400, textTransform: "none", fontSize: 11, color: "var(--t-text-3)" }}>(read-only)</span></label>
          <div className="dk-field-uploaded" dangerouslySetInnerHTML={{ __html: renderAnkiHtml(form.imported_support) }} />
        </div>
      )}

      <div className="dk-field">
        <label>{form.imported_support ? "Your Support" : "Support"} <span style={{ fontWeight: 400, textTransform: "none", fontSize: 11, color: "var(--t-text-3)" }}>{form.imported_support ? "(optional, shown after flip together with Anki support)" : "(optional, shown after flip)"}</span></label>
        <textarea rows={2} value={form.support} onChange={(e) => set("support", e.target.value)}/>
      </div>

      <div className="dk-field-row dk-field-checks">
        <div className="dk-new-card-check">
          <input type="checkbox" id="ce_searchable" checked={form.is_searchable} onChange={(e) => set("is_searchable", e.target.checked)} />
          <label htmlFor="ce_searchable">Searchable</label>
          <Tip text="Searchable cards appear in the similar cards panel shown during study sessions. Similar cards are based on any matching terms separated by commas and ignore anything in parentheses." />
        </div>
        <div className="dk-new-card-check">
          <input type="checkbox" id="ce_paused" checked={form.is_paused} onChange={(e) => set("is_paused", e.target.checked)} />
          <label htmlFor="ce_paused">Pause</label>
          <Tip text="Paused cards are skipped during SRS study sessions and do not update their due countdown until unpaused. Pausing a due card will cause a replacement card to be scheduled in its place." />
        </div>
      </div>

      {cardLogSummary}

      <div className="dk-editor-actions">
        <button className="primary" onClick={save}>Save</button>
        <ConfirmDelete onConfirm={deleteCard} />
        {inPlan && (
          <button className="btn-amber" onClick={markForReview} style={{ marginLeft: "auto" }}>
            Mark for Review
          </button>
        )}
      </div>
    </div>
  );
}

// ─── New Card Form ────────────────────────────────────────────────────────────

export function NewCardForm({ setToast, groupId, onCreated, deckSelector = null }) {
  const blank = () => emptyNewCard(groupId);
  const [form, setForm] = useState(blank);
  const [createFlipped, setCreateFlipped] = useState(false);
  const [flipMedia, setFlipMedia] = useState(false);
  const [priorityAdd, setPriorityAdd] = useState(false);
  const [flipPriorityAdd, setFlipPriorityAdd] = useState(false);
  const set = (key, val) => setForm((f) => ({ ...f, [key]: val }));

  useEffect(() => { set("group_id", groupId); }, [groupId]);

  const pickFrontImage = async () => { const p = await pickFile(["png","jpg","jpeg","gif","webp"]); if (p) set("front_image", p); };
  const pickBackImage  = async () => { const p = await pickFile(["png","jpg","jpeg","gif","webp"]); if (p) set("back_image", p); };
  const pickFrontAudio = async () => { const p = await pickFile(["mp3","wav","ogg","m4a"]); if (p) set("front_audio", p); };
  const pickBackAudio  = async () => { const p = await pickFile(["mp3","wav","ogg","m4a"]); if (p) set("back_audio", p); };

  const submit = async () => {
    if (!form.group_id) { setToast("Please select a deck."); return; }
    if (!form.front.trim() || !form.back.trim()) { setToast("Please enter a valid front and back side."); return; }
    const payload = {
      ...form,
      support: form.support || null,
      front_image: form.front_image || null,
      back_image:  form.back_image  || null,
      front_audio: form.front_audio || null,
      back_audio:  form.back_audio  || null,
    };
    try {
      const card = await loggedInvoke("create_card", { card: payload });
      if (priorityAdd) {
        await loggedInvoke("prioritize_card", { cardId: card.id });
        onCreated({ ...card, sequence: -9999 });
      } else {
        onCreated(card);
      }
      setToast("Card successfully created.");
      if (createFlipped) {
        const flipped = {
          ...payload,
          front: payload.back,
          back:  payload.front,
          ...(flipMedia ? {
            front_image: payload.back_image,  back_image:  payload.front_image,
            front_audio: payload.back_audio,  back_audio:  payload.front_audio,
          } : {}),
        };
        const flippedCard = await loggedInvoke("create_card", { card: flipped });
        if (flipPriorityAdd) {
          await loggedInvoke("prioritize_card", { cardId: flippedCard.id });
          onCreated({ ...flippedCard, sequence: -9999 });
        } else {
          onCreated(flippedCard);
        }
      }
      setForm(blank());
      setCreateFlipped(false);
      setFlipMedia(false);
      setPriorityAdd(false);
      setFlipPriorityAdd(false);
    } catch (e) { logError("catch", e); setToast("Failed to create card.", "error"); }
  };

  return (
    <div className="dk-new-card">
      {deckSelector}
      <div className="dk-new-card-row"><label>Front</label><textarea rows={2} value={form.front} onChange={(e) => set("front", e.target.value)} /></div>
      <div className="dk-new-card-row">
        <label>Front Image</label>
        <input type="text" value={parseFile(form.front_image ?? "")} readOnly placeholder="No file" style={{ flex: 1 }} />
        <button onClick={pickFrontImage}>Browse</button>
        {form.front_image && <button onClick={() => set("front_image", null)}>Clear</button>}
      </div>
      <div className="dk-new-card-row">
        <label>Front Audio</label>
        <input type="text" value={parseFile(form.front_audio ?? "")} readOnly placeholder="No file" style={{ flex: 1 }} />
        <button onClick={pickFrontAudio}>Browse</button>
        {form.front_audio && <button onClick={() => set("front_audio", null)}>Clear</button>}
      </div>
      <div className="dk-new-card-row"><label>Back</label><textarea rows={2} value={form.back} onChange={(e) => set("back", e.target.value)} /></div>
      <div className="dk-new-card-row">
        <label>Back Image</label>
        <input type="text" value={parseFile(form.back_image ?? "")} readOnly placeholder="No file" style={{ flex: 1 }} />
        <button onClick={pickBackImage}>Browse</button>
        {form.back_image && <button onClick={() => set("back_image", null)}>Clear</button>}
      </div>
      <div className="dk-new-card-row">
        <label>Back Audio</label>
        <input type="text" value={parseFile(form.back_audio ?? "")} readOnly placeholder="No file" style={{ flex: 1 }} />
        <button onClick={pickBackAudio}>Browse</button>
        {form.back_audio && <button onClick={() => set("back_audio", null)}>Clear</button>}
      </div>
      <div className="dk-new-card-row">
        <label>Support <Tip text="Always shown after flipping a card. Most useful when creating flipped copies. Great for example sentences, pronunciation guides, context notes, or mnemonics. " /></label>
        <textarea rows={2} value={form.support} onChange={(e) => set("support", e.target.value)} />
      </div>
      <div className="dk-new-card-row dk-new-card-actions">
        <div className="dk-new-card-checks">
          <div className="dk-new-card-checkrow">
            <div className="dk-new-card-check">
              <input type="checkbox" id="nc_searchable" checked={form.is_searchable} onChange={(e) => set("is_searchable", e.target.checked)} />
              <label htmlFor="nc_searchable">Searchable</label>
              <Tip text="Searchable cards appear in the similar cards panel shown during study sessions. Similar cards are based on any matching terms separated by commas and ignore anything in parentheses." />
            </div>
            <div className="dk-new-card-check">
              <input type="checkbox" id="nc_priority" checked={priorityAdd} onChange={(e) => setPriorityAdd(e.target.checked)} />
              <label htmlFor="nc_priority">Priority add</label>
              <Tip text="Gives this card priority for your next study session so it won't get buried behind existing cards in the queue." />
            </div>
            <div className="dk-new-card-check">
              <input type="checkbox" id="nc_flipped" checked={createFlipped} onChange={(e) => { setCreateFlipped(e.target.checked); if (!e.target.checked) { setFlipMedia(false); setFlipPriorityAdd(false); } }} />
              <label htmlFor="nc_flipped">Create flipped copy</label>
            </div>
          </div>
          <div className="dk-new-card-checkrow">
            <div className={`dk-new-card-check${createFlipped ? "" : " disabled"}`}>
              <input type="checkbox" id="nc_flip_media" checked={flipMedia} disabled={!createFlipped} onChange={(e) => setFlipMedia(e.target.checked)} />
              <label htmlFor="nc_flip_media">Swap media</label>
              <Tip text="Swap the front image and audio with the back image and audio on the flipped copy of this card." />
            </div>
            <div className={`dk-new-card-check${createFlipped ? "" : " disabled"}`}>
              <input type="checkbox" id="nc_flip_priority" checked={flipPriorityAdd} disabled={!createFlipped} onChange={(e) => setFlipPriorityAdd(e.target.checked)} />
              <label htmlFor="nc_flip_priority">Priority add copy</label>
              <Tip text="Gives the flipped copy card priority for your next study session so it won't get buried behind existing cards in the queue." />
            </div>
          </div>
        </div>
        <button className="primary" onClick={submit}>Add Card</button>
      </div>
    </div>
  );
}

// ─── Deck Actions Dropdown ────────────────────────────────────────────────────

function DeckActions({ onPauseAll, onUnpauseAll, onAllSearchable, onAllNotSearchable, onResetRequest }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);

  useEffect(() => {
    if (!open) return;
    function handle(e) { if (!ref.current?.contains(e.target)) setOpen(false); }
    document.addEventListener("mousedown", handle);
    return () => document.removeEventListener("mousedown", handle);
  }, [open]);

  function act(fn) { fn(); setOpen(false); }

  return (
    <div style={{ position: "relative" }} ref={ref}>
      <button onClick={() => setOpen(o => !o)} title="Deck actions" style={{ letterSpacing: 2 }}>...</button>
      {open && (
        <div className="dk-actions-menu">
          <button className="dk-menu-item" onClick={() => act(onPauseAll)}>Pause All</button>
          <button className="dk-menu-item" onClick={() => act(onUnpauseAll)}>Unpause All</button>
          <button className="dk-menu-item" onClick={() => act(onAllSearchable)}>All Searchable</button>
          <button className="dk-menu-item" onClick={() => act(onAllNotSearchable)}>All Not Searchable</button>
          <button className="dk-menu-item danger" onClick={() => act(onResetRequest)}>Reset Progress...</button>
        </div>
      )}
    </div>
  );
}

// ─── Card View ────────────────────────────────────────────────────────────────

function CardView({ setToast, deck, onBack, returnTo, onReturnToOrigin }) {
  const [cards, setCards] = useState([]);
  const [selectedId, setSelectedId] = useState(null);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [sort, setSort] = useState("id");
  const [plans, setPlans] = useState([]);
  const [confirmReset, setConfirmReset] = useState(false);
  const [dateSince, setDateSince] = useState("");
  const [today, setToday] = useState(null);
  const [lastSeenMap, setLastSeenMap] = useState({});
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [creatorOpen, setCreatorOpen] = useState(false);

  useEffect(() => {
    loggedInvoke("get_cards", { deckId: deck.id })
      .then((c) => { setCards(c); if (c.length > 0) setSelectedId(c[0].id); })
      .catch(e => logError("catch", e));
    loggedInvoke("get_plans").then(setPlans).catch(e => logError("catch", e));
    loggedInvoke("get_current_date").then(setToday).catch(e => logError("catch", e));
    loggedInvoke("get_card_last_seen_dates", { deckId: deck.id })
      .then(pairs => setLastSeenMap(Object.fromEntries(pairs)))
      .catch(e => logError("catch", e));
  }, [deck.id]);

  const planName = deck.plan_id ? (plans.find((p) => p.id === deck.plan_id)?.name ?? null) : null;

  const pauseAll = async () => {
    try {
      await loggedInvoke("pause_all", { groupId: deck.id });
      const fresh = await loggedInvoke("get_cards", { deckId: deck.id });
      setCards(fresh);
      setToast("All cards paused.");
    } catch (e) { logError("catch", e); setToast("Failed to pause cards.", "error"); }
  };

  const unpauseAll = async () => {
    try {
      await loggedInvoke("unpause_all", { groupId: deck.id });
      const fresh = await loggedInvoke("get_cards", { deckId: deck.id });
      setCards(fresh);
      setToast("All cards unpaused.");
    } catch (e) { logError("catch", e); setToast("Failed to unpause cards.", "error"); }
  };

  const setAllSearchable = async (searchable) => {
    try {
      await loggedInvoke("set_all_searchable", { groupId: deck.id, searchable });
      setCards((prev) => prev.map((c) => ({ ...c, is_searchable: searchable })));
      setToast(searchable ? "All cards set to searchable." : "All cards set to not searchable.");
    } catch (e) { logError("catch", e); setToast("Failed to update cards.", "error"); }
  };

  const resetDeck = async () => {
    try {
      await loggedInvoke("reset_deck", { groupId: deck.id });
      const updated = await loggedInvoke("get_cards", { deckId: deck.id });
      setCards(updated);
      setToast("Deck progress reset.");
      setConfirmReset(false);
    } catch (e) { logError("catch", e); setToast("Failed to reset deck.", "error"); }
  };

  let filtered = cards.filter((c) => {
    if (!search.trim()) return true;
    const q = search.toLowerCase();
    const front = c.is_uploaded ? stripHtml(c.front) : c.front;
    const back = c.is_uploaded ? stripHtml(c.back) : c.back;
    return front.toLowerCase().includes(q) || back.toLowerCase().includes(q);
  });

  if (filter === "paused") filtered = filtered.filter(c => c.is_paused);
  else if (filter === "unpaused") filtered = filtered.filter(c => !c.is_paused);
  else if (filter === "due") filtered = deck.plan_id ? filtered.filter(c => c.is_due) : [];
  else if (filter === "overdue") filtered = deck.plan_id ? filtered.filter(c => c.is_overdue === true) : [];
  else if (filter === "new") filtered = filtered.filter(c => c.tier == 0);
  else if (filter === "review") filtered = filtered.filter(c => c.tier != 0);
  else if (filter === "custom") filtered = filtered.filter(c => !c.is_uploaded);
  else if (filter === "uploaded") filtered = filtered.filter(c => c.is_uploaded);

  if (dateSince) filtered = filtered.filter(c => lastSeenMap[c.id] && lastSeenMap[c.id] >= dateSince);

  if (sort === "sequence") filtered = [...filtered].sort((a, b) => a.sequence - b.sequence);
  // else: trust backend ORDER BY (position ASC NULLS LAST, id ASC) for merged-deck zipper order

  const selectedCard = cards.find((c) => c.id === selectedId) ?? null;
  const handleCreated = (card) => { setCards((prev) => [...prev, card]); setSelectedId(card.id); };
  const handleSaved = async (updated) => {
    const old = cards.find(c => c.id === updated.id);
    if (old?.is_paused !== updated.is_paused) {
      const fresh = await loggedInvoke("get_cards", { deckId: deck.id });
      setCards(fresh);
    } else {
      setCards((prev) => prev.map((c) => c.id === updated.id ? updated : c));
    }
  };
  const handleDeleted = async (id) => {
    const fresh = await loggedInvoke("get_cards", { deckId: deck.id });
    setCards(fresh);
    setSelectedId(fresh.length > 0 ? fresh[0].id : null);
  };
  const handleMarkedForReview = (id) => {
    setCards((prev) => prev.map((c) => c.id === id ? { ...c, sequence: -9999, is_due: true, is_paused: false } : c));
  };

  return (
    <div className="dk-cards-root">
      <div className="dk-cards-header">
        {returnTo ? (
          <button className="quiet" onClick={onReturnToOrigin}>← Back to {returnTo.label}</button>
        ) : (
          <button className="quiet" onClick={onBack}>← Decks</button>
        )}
        <h2>{deck.name}</h2>
        {planName && <span className="dk-cards-plan">{planName}</span>}
        <span style={{ fontSize: 12, color: "var(--t-text-3)" }}>{cards.length} card{cards.length !== 1 ? "s" : ""}</span>
        <DeckActions onPauseAll={pauseAll} onUnpauseAll={unpauseAll}
          onAllSearchable={() => setAllSearchable(true)} onAllNotSearchable={() => setAllSearchable(false)}
          onResetRequest={() => setConfirmReset(true)} />
      </div>

      {confirmReset && (
        <div className="dk-confirm-bar">
          <span style={{ flex: 1 }}>Reset all SRS progress on this deck? This cannot be undone.</span>
          <button className="danger" onClick={resetDeck}>Reset</button>
          <button onClick={() => setConfirmReset(false)}>Cancel</button>
        </div>
      )}

      <div className="dk-cards-body">
        <div className="dk-table-pane">
          <div className="dk-table-search">
            <input type="text" placeholder="Search cards..." value={search} onChange={(e) => setSearch(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Escape") setSearch(""); }} />
            <div className="dk-sort-filter-row">
              <div className="dk-sort-seg">
                <button className={sort === "id" ? "active" : ""} onClick={() => setSort("id")}>Created Date</button>
                <button className={sort === "sequence" ? "active" : ""} onClick={() => setSort("sequence")}>Due Date</button>
              </div>
              <button className={`dk-filter-toggle${filtersOpen || filter !== "all" ? " active" : ""}`} onClick={() => setFiltersOpen(o => !o)}>
                Filters {filtersOpen ? "▾" : "▸"}
              </button>
              {today && (
                <div className="dk-date-filter">
                  <span>Last seen:</span>
                  <input type="date" value={dateSince} onChange={e => setDateSince(e.target.value)} />
                  {dateSince && <button style={{ fontSize: 11, padding: "1px 5px" }} onClick={() => setDateSince("")}>Clear</button>}
                </div>
              )}
            </div>
            {filtersOpen && (
              <div className="dk-filter-bar">
                {[
                  { key: "all",     label: "All"      },
                  { key: "due",     label: "Due" },
                  { key: "overdue", label: "Overdue"   },
                  { key: "new",     label: "New"       },
                  { key: "review",  label: "Review"    },
                ].map(f => (
                  <button key={f.key}
                    className={`dk-filter-btn${filter === f.key ? " active" : ""}`}
                    onClick={() => setFilter(f.key)}>
                    {f.label}
                  </button>
                ))}
                <span />
                {[
                  { key: "paused",   label: "Paused"   },
                  { key: "unpaused", label: "Unpaused" },
                  { key: "custom",   label: "Custom"   },
                  { key: "uploaded", label: "Uploaded" },
                ].map(f => (
                  <button key={f.key}
                    className={`dk-filter-btn${filter === f.key ? " active" : ""}`}
                    onClick={() => setFilter(f.key)}>
                    {f.label}
                  </button>
                ))}
              </div>
            )}
          </div>

          <div className="dk-table-scroll">
            {filtered.length === 0 ? (
              <div className="dk-table-empty">
                {(filter === "due" || filter === "overdue") && !deck.plan_id
                  ? "This deck isn't linked to a plan."
                  : search || filter !== "all" || dateSince
                    ? "No cards match your filters."
                    : "No cards yet, create some below!"}
              </div>
            ) : (
              <table className="dk-card-table">
                <thead>
                  <tr><th>Front</th><th>Back</th><th>Due</th><th>Paused</th></tr>
                </thead>
                <tbody>
                  {filtered.map((card) => {
                    const front = card.is_uploaded ? stripHtml(card.front) : card.front;
                    const back = card.is_uploaded ? stripHtml(card.back) : card.back;
                    return (
                      <tr key={card.id}
                        className={[
                          card.id === selectedId ? "selected" : "",
                          card.is_due && deck.plan_id
                            ? (card.tier === 0
                                ? "is-new-due"
                                : card.is_overdue === true
                                  ? "is-review-overdue"
                                  : "is-review-due")
                            : (!card.is_due && card.is_overdue === true ? "is-overdue-only" : ""),
                        ].filter(Boolean).join(" ")}
                        onClick={() => setSelectedId(card.id)}>
                        <td><div className="dk-cell-clamp">{front}</div></td>
                        <td><div className="dk-cell-clamp">{back}</div></td>
                        <td>{card.sequence > 0 ? `${card.sequence}d` : card.is_due ? "Today" : "ASAP"}</td>
                        <td>{card.is_paused ? "Yes" : "—"}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>

          <div className={`dk-creator-toggle-row${creatorOpen ? " open" : ""}`}>
            <button className="dk-filter-toggle" onClick={() => setCreatorOpen(o => !o)}>
              {creatorOpen ? "Hide New Card" : "New Card"}
              <span style={{ fontSize: 10, marginLeft: 5 }}>{creatorOpen ? "▾" : "▸"}</span>
            </button>
          </div>
          {creatorOpen && (
            <div className="dk-new-card-dark">
              <NewCardForm setToast={setToast} groupId={deck.id} onCreated={handleCreated} />
            </div>
          )}
        </div>

        <CardEditor
          setToast={setToast}
          card={selectedCard}
          onSaved={handleSaved}
          onDeleted={handleDeleted}
          onMarkedForReview={handleMarkedForReview}
          inPlan={!!deck.plan_id}
        />
      </div>
    </div>
  );
}

// ─── Root ─────────────────────────────────────────────────────────────────────

export default function Decks({ setToast, initialDeck, onClearInitial, returnTo, onReturnToOrigin }) {
  const [view, setView] = useState(initialDeck ? VIEW_CARDS : VIEW_DECKS);
  const [activeDeck, setActiveDeck] = useState(initialDeck ?? null);

  useEffect(() => {
    if (initialDeck) onClearInitial?.();
  }, []);

  const openDeck = (deck) => { setActiveDeck(deck); setView(VIEW_CARDS); };
  const goBack = () => { setActiveDeck(null); setView(VIEW_DECKS); };

  return (
    <div className="dk-root">
      {view === VIEW_DECKS && <DeckList setToast={setToast} onOpenDeck={openDeck} />}
      {view === VIEW_CARDS && activeDeck && (
        <CardView
          setToast={setToast}
          deck={activeDeck}
          onBack={goBack}
          returnTo={returnTo}
          onReturnToOrigin={onReturnToOrigin}
        />
      )}
    </div>
  );
}
