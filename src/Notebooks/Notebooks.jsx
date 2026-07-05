import { useState, useEffect, useRef, useCallback } from "react";
import { NewCardForm } from "../Decks/Decks";
import { ConfirmDelete } from "../UIUtils";
import { convertFileSrc } from "@tauri-apps/api/core";
import { loggedInvoke, logError } from "../logger";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Link } from "@tiptap/extension-link";
import { useEditor, EditorContent } from "@tiptap/react";
import { StarterKit } from "@tiptap/starter-kit";
import { Image } from "@tiptap/extension-image";
import { Table } from "@tiptap/extension-table";
import { TableRow } from "@tiptap/extension-table-row";
import { TableCell } from "@tiptap/extension-table-cell";
import { TableHeader } from "@tiptap/extension-table-header";
import { TextAlign } from "@tiptap/extension-text-align";
import { Color } from "@tiptap/extension-color";
import { TextStyle } from "@tiptap/extension-text-style";
import { Highlight } from "@tiptap/extension-highlight";
import { RevealBlock } from "./CustomExtentions";
import { rewriteContentForDisplay, rewriteContentForSave } from "./NotebookUtils";
import "./Notebooks.css";

const VIEW_NOTEBOOKS = "notebooks";
const VIEW_PAGES     = "pages";

async function pickFile(extensions) {
    try {
        const path = await open({ multiple: false, filters: [{ name: "File", extensions }] });
        return path ?? null;
    } catch { return null; }
}

// Matches the backend's ORDER BY name COLLATE NOCASE
const byName = (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });

// ─── Notebook List ────────────────────────────────────────────────────────────

function NotebookList({ setToast, onOpenNotebook }) {
    const [notebooks, setNotebooks] = useState([]);
    const [loading, setLoading] = useState(true);
    const [pageCounts, setPageCounts] = useState({});
    const [newName, setNewName] = useState("");
    const [editingId, setEditingId] = useState(null);
    const [editingName, setEditingName] = useState("");

    const [merging, setMerging] = useState(false);
    const [mergeNotebookA, setMergeNotebookA] = useState(null);
    const [mergeNotebookB, setMergeNotebookB] = useState(null);
    const [mergeName, setMergeName] = useState("");

    useEffect(() => {
        loggedInvoke("get_notebooks").then(setNotebooks).catch(e => logError("catch", e)).finally(() => setLoading(false));
        loggedInvoke("get_notebook_page_counts")
            .then(rows => setPageCounts(Object.fromEntries(rows)))
            .catch(e => logError("catch", e));
    }, []);

    const createNotebook = async () => {
        const name = newName.trim();
        if (!name) {setToast("Please enter a valid name."); return; };
        try {
            const nb = await loggedInvoke("create_notebook", { name });
            setNotebooks((prev) => [...prev, nb].sort(byName));
            setToast(`${nb.name} successfully created.`);
            setNewName("");
        } catch (e) { logError("catch", e); setToast("Failed to create notebook.", "error"); }
    };

    const startEdit = (nb, e) => { e.stopPropagation(); setEditingId(nb.id); setEditingName(nb.name); };

    const confirmEdit = async (id) => {
        const name = editingName.trim();
        if (!name) { setEditingId(null); setToast("Please enter a valid name."); return; }
        try {
            await loggedInvoke("update_notebook", { notebook: { id, plan_id: null, name, group_type: "notebook" } });
            setNotebooks((prev) => prev.map((n) => n.id === id ? { ...n, name } : n).sort(byName));
            setToast(`${editingName} successfully updated.`);
        } catch (e) { logError("catch", e); setToast("Failed to update notebook.", "error"); }
        setEditingId(null);
    };

    const deleteNotebook = async (id) => {
        const target = notebooks.find((n) => n.id === id);
        try {
            await loggedInvoke("delete_notebook", { id });
            setNotebooks((prev) => prev.filter((n) => n.id !== id));
            setToast(`${target?.name ?? "Notebook"} deleted.`);
        } catch (e) { logError("catch", e); setToast("Failed to delete notebook.", "error"); }
    };

    const startMerge = () => {
        setMerging(true);
        setMergeNotebookA(notebooks.length > 0 ? notebooks[0].id : null);
        setMergeNotebookB(notebooks.length > 1 ? notebooks[1].id : null);
        setMergeName("");
    };

    const confirmMerge = async () => {
        if (!mergeNotebookA || !mergeNotebookB) { setToast("Please select two notebooks."); return; }
        if (mergeNotebookA === mergeNotebookB) { setToast("Please select two different notebooks."); return; }
        if (!mergeName.trim()) { setToast("Please enter a name for the merged notebook."); return; }
        try {
            const newNb = await loggedInvoke("merge_notebooks", {
                notebookAId: mergeNotebookA,
                notebookBId: mergeNotebookB,
                newName: mergeName.trim(),
            });
            const updatedNotebooks = await loggedInvoke("get_notebooks");
            setNotebooks(updatedNotebooks);
            setToast(`Notebooks merged into ${newNb.name}.`);
            setMerging(false);
        } catch (e) { logError("catch", e); setToast("Failed to merge notebooks.", "error"); }
    };

    return (
        <>
            <div className="landing-hdr landing-hdr--notebook">
                <h2>Notebooks</h2>
                <button onClick={startMerge} disabled={notebooks.length < 2}>Merge Notebooks</button>
            </div>
            {merging && (
                <div className="nb-merge-panel">
                    <div style={{ fontSize: 13, fontWeight: 500 }}>Merge two notebooks</div>
                    <div className="nb-merge-row">
                        <select value={mergeNotebookA ?? ""} onChange={(e) => setMergeNotebookA(Number(e.target.value))}>
                            {notebooks.filter(n => n.id !== mergeNotebookB).map(n => <option key={n.id} value={n.id}>{n.name}</option>)}
                        </select>
                        <span style={{ fontSize: 12, color: "var(--t-text-3)" }}>+</span>
                        <select value={mergeNotebookB ?? ""} onChange={(e) => setMergeNotebookB(Number(e.target.value))}>
                            {notebooks.filter(n => n.id !== mergeNotebookA).map(n => <option key={n.id} value={n.id}>{n.name}</option>)}
                        </select>
                    </div>
                    <input type="text" placeholder="New notebook name..." value={mergeName}
                        onChange={(e) => setMergeName(e.target.value)} />
                    <div style={{ fontSize: 11, color: "var(--t-text-3)" }}>
                        The two source notebooks will be deleted after their pages move into the new notebook. Pages are ordered by date created. If either notebook is linked to a plan, that link will be removed.
                    </div>
                    <div style={{ display: "flex", gap: 8 }}>
                        <button className="primary" onClick={confirmMerge}>Merge</button>
                        <button onClick={() => setMerging(false)}>Cancel</button>
                    </div>
                </div>
            )}
            <div className="nb-list">
                {!loading && notebooks.length === 0 && <div className="landing-empty">No notebooks yet. Create one below.</div>}
                {notebooks.map((nb) => (
                    <div className="landing-card landing-card--notebook" key={nb.id} onClick={() => onOpenNotebook(nb)}>
                        <div className="landing-card-body">
                            {editingId === nb.id ? (
                                <input className="nb-notebook-name-input" value={editingName} autoFocus
                                    onClick={(e) => e.stopPropagation()}
                                    onChange={(e) => setEditingName(e.target.value)}
                                    onKeyDown={(e) => {
                                        if (e.key === "Enter") confirmEdit(nb.id);
                                        if (e.key === "Escape") setEditingId(null);
                                    }}
                                    onBlur={() => confirmEdit(nb.id)} />
                            ) : (
                                <>
                                    <span className="nb-notebook-name">{nb.name}</span>
                                    <div className="landing-card-stats">
                                        <span className="landing-stat landing-stat--page">
                                            <b>{pageCounts[nb.id] ?? 0}</b> {(pageCounts[nb.id] ?? 0) === 1 ? "page" : "pages"}
                                        </span>
                                    </div>
                                </>
                            )}
                        </div>
                        <div className="landing-card-actions" onClick={(e) => e.stopPropagation()}>
                            <button onClick={(e) => startEdit(nb, e)}>Edit</button>
                            <ConfirmDelete onConfirm={() => deleteNotebook(nb.id)} small />
                        </div>
                    </div>
                ))}
            </div>
            <div className="nb-new-notebook">
                <input type="text" placeholder="New notebook name..." value={newName}
                    onChange={(e) => setNewName(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && createNotebook()} />
                <button className="primary" onClick={createNotebook}>Create</button>
            </div>
        </>
    );
}

// ─── Audio Recorder ───────────────────────────────────────────────────────────

// Recording lives in a hook so its compact controls can render inside the
// toolbar while the (line-hungry) audio player renders on its own bar only when
// a clip actually exists.
function useAudioRecorder({ audioFile, onAudioChange }) {
    const [recording, setRecording] = useState(false);
    const [paused, setPaused] = useState(false);
    const mediaRecorderRef = useRef(null);
    const chunksRef = useRef([]);
    const streamRef = useRef(null);

    useEffect(() => {
        return () => { streamRef.current?.getTracks().forEach(t => t.stop()); };
    }, []);

    async function startRecording() {
        try {
            const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
            streamRef.current = stream;
            const mr = new MediaRecorder(stream);
            chunksRef.current = [];
            mr.ondataavailable = (e) => { if (e.data.size > 0) chunksRef.current.push(e.data); };
            mr.onstop = async () => {
                const blob = new Blob(chunksRef.current, { type: "audio/webm" });
                stream.getTracks().forEach(t => t.stop());

                // Save to pages/audio/ via Tauri
                const arrayBuffer = await blob.arrayBuffer();
                const uint8 = new Uint8Array(arrayBuffer);
                try {
                    const path = await loggedInvoke("save_page_audio", { data: Array.from(uint8) });
                    onAudioChange(path);
                } catch (e) { logError("save_page_audio", e); }
            };
            mediaRecorderRef.current = mr;
            mr.start();
            setRecording(true);
            setPaused(false);
        } catch (e) { logError("microphone_access", e); }
    }

    function pauseRecording() {
        if (mediaRecorderRef.current?.state === "recording") {
            mediaRecorderRef.current.pause();
            setPaused(true);
        }
    }

    function resumeRecording() {
        if (mediaRecorderRef.current?.state === "paused") {
            mediaRecorderRef.current.resume();
            setPaused(false);
        }
    }

    function stopRecording() {
        mediaRecorderRef.current?.stop();
        setRecording(false);
        setPaused(false);
    }

    async function deleteAudio() {
        await loggedInvoke("delete_page_audio", { path: audioFile });
        onAudioChange(null);
    }

    return { recording, paused, startRecording, pauseRecording, resumeRecording, stopRecording, deleteAudio };
}

// Compact recording controls that slot into the toolbar
function AudioControls({ audioFile, audio }) {
    return (
        <>
            {!audio.recording ? (
                <button className="nb-tb-btn record" onClick={audio.startRecording}>
                    ● {audioFile ? "Re-record" : "Record"}
                </button>
            ) : (
                <>
                    {!audio.paused ? (
                        <button className="nb-tb-btn record recording" onClick={audio.pauseRecording}>⏸ Pause</button>
                    ) : (
                        <button className="nb-tb-btn record recording" onClick={audio.resumeRecording}>▶ Resume</button>
                    )}
                    <button className="nb-tb-btn" onClick={audio.stopRecording}>■ Stop</button>
                </>
            )}
            {audioFile && !audio.recording && (
                <button className="nb-tb-btn danger" onClick={audio.deleteAudio}>Delete Audio</button>
            )}
        </>
    );
}

// ─── Page Editor (TipTap) ─────────────────────────────────────────────────────

export function PageEditor({ content, onChange, editable, audioFile, onAudioChange }) {
    const [linkPrompt, setLinkPrompt] = useState(false);
    const [linkInput, setLinkInput] = useState("");
    const audio = useAudioRecorder({ audioFile, onAudioChange: onAudioChange ?? (() => {}) });

    // Track toolbar overflow so we can fade the edges the user can still scroll to
    const toolbarRef = useRef(null);
    const [tbScroll, setTbScroll] = useState({ l: false, r: false });
    const updateTbScroll = useCallback(() => {
        const el = toolbarRef.current;
        if (!el) return;
        const l = el.scrollLeft > 1;
        const r = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
        setTbScroll((prev) => (prev.l === l && prev.r === r ? prev : { l, r }));
    }, []);

    const editor = useEditor({
        extensions: [
            StarterKit.configure({ code: false, codeBlock: false, link: false }),
            TextStyle,
            Color,
            Highlight.configure({ multicolor: true }),
            Link.configure({ openOnClick: false, inclusive: false, autolink: false }),
            Image.extend({
                addAttributes() {
                    return { ...this.parent?.(), rawPath: { default: null } };
                },
            }).configure({ allowBase64: false }),
            Table.configure({ resizable: true }),
            TableRow,
            TableCell,
            TableHeader,
            TextAlign.configure({ types: ["heading", "paragraph"] }),
            RevealBlock,
        ],
        content: content ?? null,
        editable,
        onUpdate: ({ editor }) => { onChange(editor.getJSON()); },
    }, [editable]);

    useEffect(() => {
        if (!editor) return;
        const current = JSON.stringify(editor.getJSON());
        const incoming = JSON.stringify(content);
        if (current !== incoming) {
            setTimeout(() => { editor.commands.setContent(content ?? null, false); }, 0);
        }
    }, [content]);

    useEffect(() => {
        if (!editable || !editor) return;
        updateTbScroll();
        const el = toolbarRef.current;
        if (!el) return;
        const ro = new ResizeObserver(updateTbScroll);
        ro.observe(el);
        return () => ro.disconnect();
    }, [editable, editor, audioFile, audio.recording, audio.paused, updateTbScroll]);

    const insertImage = useCallback(async () => {
        if (!editor) return;
        const path = await pickFile(["png", "jpg", "jpeg", "gif", "webp"]);
        if (path) {
            editor.chain().focus().setImage({ src: convertFileSrc(path), rawPath: path }).run();
        }
    }, [editor]);

    const confirmLink = useCallback(() => {
        setLinkPrompt(false);
        const val = linkInput.trim();
        if (!val) { setLinkInput(""); return; }
        const mdMatch = val.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
        const text = mdMatch ? mdMatch[1] : val;
        const url = mdMatch ? mdMatch[2] : val;
        const href = url.startsWith("http") ? url : `https://${url}`;
        editor.chain().focus()
            .insertContent({ type: "text", marks: [{ type: "link", attrs: { href } }], text })
            .insertContent({ type: "text", text: " " })
            .run();
        setLinkInput("");
    }, [editor, linkInput]);

    const insertRevealBlock = useCallback(() => {
        if (!editor) return;
        editor.chain().focus().insertContent({ type: "revealBlock", attrs: { prompt: "", answer: "" } }).run();
    }, [editor]);

    const handleViewClick = useCallback((e) => {
        if (!editable) {
            const a = e.target.closest("a");
            if (a?.href) { e.preventDefault(); openUrl(a.href); }
        }
    }, [editable]);

    if (!editor) return null;

    return (
        <div className="nb-editor-wrap">
            {/* Audio player gets its own bar only when a clip exists; the record
                controls live in the toolbar so no line is wasted otherwise */}
            {audioFile && (
                <div className="nb-audio-bar">
                    <audio controls src={convertFileSrc(audioFile)} />
                </div>
            )}

            {editable && (
                <>
                    {linkPrompt && (
                        <div className="nb-inline-prompt">
                            <input type="text" autoFocus
                                placeholder="[display text](url) or just a URL"
                                value={linkInput}
                                onChange={(e) => setLinkInput(e.target.value)}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter") confirmLink();
                                    if (e.key === "Escape") { setLinkPrompt(false); setLinkInput(""); }
                                }} />
                            <button className="nb-tb-btn" onClick={confirmLink}>Insert</button>
                            <button className="nb-tb-btn" onClick={() => { setLinkPrompt(false); setLinkInput(""); }}>Cancel</button>
                        </div>
                    )}
                    <div className={`nb-toolbar-wrap${tbScroll.l ? " scroll-l" : ""}${tbScroll.r ? " scroll-r" : ""}`}>
                    <div className="nb-toolbar" ref={toolbarRef} onScroll={updateTbScroll}>
                        <AudioControls audioFile={audioFile} audio={audio} />
                        <div className="nb-tb-sep" />
                        <button className={`nb-tb-btn${editor.isActive("bold") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleBold().run()}><b>B</b></button>
                        <button className={`nb-tb-btn${editor.isActive("italic") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleItalic().run()}><i>I</i></button>
                        <button className={`nb-tb-btn${editor.isActive("underline") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleUnderline().run()}><u>U</u></button>
                        <button className={`nb-tb-btn${editor.isActive("strike") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleStrike().run()}><s>S</s></button>
                        <button className={`nb-tb-btn${editor.isActive("highlight") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleHighlight().run()}><mark>H</mark></button>
                        <div className="nb-tb-sep" />
                        <select style={{ fontSize: 12, padding: "2px 4px", border: "1px solid var(--t-border)", borderRadius: "var(--t-r)", background: "var(--t-surface)" }}
                            value={[1,2,3].find((l) => editor.isActive("heading", { level: l })) || 0}
                            onChange={(e) => {
                                const v = parseInt(e.target.value);
                                if (v === 0) editor.chain().focus().setParagraph().run();
                                else editor.chain().focus().toggleHeading({ level: v }).run();
                            }}>
                            <option value={0}>Paragraph</option>
                            <option value={1}>H1</option>
                            <option value={2}>H2</option>
                            <option value={3}>H3</option>
                        </select>
                        <div className="nb-tb-sep" />
                        <button className="nb-tb-btn" onClick={() => editor.chain().focus().setTextAlign("left").run()}>≡L</button>
                        <button className="nb-tb-btn" onClick={() => editor.chain().focus().setTextAlign("center").run()}>≡C</button>
                        <button className="nb-tb-btn" onClick={() => editor.chain().focus().setTextAlign("right").run()}>≡R</button>
                        <div className="nb-tb-sep" />
                        <button className={`nb-tb-btn${editor.isActive("bulletList") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleBulletList().run()}>• List</button>
                        <button className={`nb-tb-btn${editor.isActive("orderedList") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleOrderedList().run()}>1. List</button>
                        <div className="nb-tb-sep" />
                        <button className={`nb-tb-btn${editor.isActive("blockquote") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleBlockquote().run()}>❝</button>
                        <button className="nb-tb-btn" onClick={() => editor.chain().focus().setHorizontalRule().run()}>―</button>
                        <div className="nb-tb-sep" />
                        <button className="nb-tb-btn" onClick={() => editor.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()}>⊞ Table</button>
                        {editor.isActive("table") && <>
                            <button className="nb-tb-btn" onClick={() => editor.chain().focus().addRowAfter().run()}>+Row</button>
                            <button className="nb-tb-btn" onClick={() => editor.chain().focus().addColumnAfter().run()}>+Col</button>
                            <button className="nb-tb-btn" onClick={() => editor.chain().focus().deleteRow().run()}>-Row</button>
                            <button className="nb-tb-btn" onClick={() => editor.chain().focus().deleteColumn().run()}>-Col</button>
                            <button className="nb-tb-btn" onClick={() => editor.chain().focus().deleteTable().run()}>✕ Table</button>
                        </>}
                        <div className="nb-tb-sep" />
                        <button className="nb-tb-btn" onClick={insertImage}>Image</button>
                        <button className="nb-tb-btn" onClick={() => setLinkPrompt((p) => !p)}>Link</button>
                        <div className="nb-tb-sep" />
                        <button className="nb-tb-btn" onClick={insertRevealBlock}>Reveal</button>
                        <div className="nb-tb-sep" />
                        <button className="nb-tb-btn" disabled={!editor.can().undo()} onClick={() => editor.chain().focus().undo().run()}>↩</button>
                        <button className="nb-tb-btn" disabled={!editor.can().redo()} onClick={() => editor.chain().focus().redo().run()}>↪</button>
                    </div>
                    </div>
                </>
            )}
            <div className={`nb-editor-scroll${!editable ? " nb-view-content" : ""}`} onClick={handleViewClick}>
                <EditorContent editor={editor} />
            </div>
        </div>
    );
}

// ─── Card Creator Panel ───────────────────────────────────────────────────────

function CardCreatorPanel({ setToast }) {
    const [open, setOpen] = useState(false);
    const [decks, setDecks] = useState([]);
    const [deckId, setDeckId] = useState(null);

    useEffect(() => {
        loggedInvoke("get_groups")
            .then((groups) => setDecks(groups.filter((g) => g.group_type === "deck")))
            .catch(e => logError("catch", e));
    }, []);

    const deckSelector = (
        <div className="dk-new-card-row">
            <label>Deck</label>
            <select value={deckId ?? ""} onChange={(e) => setDeckId(e.target.value ? Number(e.target.value) : null)}>
                <option value="">Select a deck…</option>
                {decks.map((d) => <option key={d.id} value={d.id}>{d.name}</option>)}
            </select>
        </div>
    );

    return (
        <div className={`nb-card-creator${open ? " open" : ""}`}>
            <div className="nb-card-toggle" onClick={() => setOpen((o) => !o)}>
                <span style={{ fontSize: 13, fontWeight: 500 }}>Create a card</span>
                <span style={{ fontSize: 10, color: "var(--t-text-3)" }}>{open ? "▾" : "▸"}</span>
            </div>
            {/* Body opens below the bar; kept mounted while closed so in-progress
                input survives toggling */}
            <div className="nb-card-body">
                <NewCardForm
                    groupId={deckId}
                    setToast={setToast}
                    onCreated={() => {}}
                    deckSelector={deckSelector}
                />
            </div>
        </div>
    );
}

// ─── Editable page wrapper ────────────────────────────────────────────────────

function EditablePageEditor({ initialContent, initialAudioFile, onSave, onAudioChange }) {
    const [contentJson, setContentJson] = useState(initialContent ?? null);

    useEffect(() => {
        const handler = () => { onSave(contentJson); };
        window.addEventListener("nb-save-request", handler);
        return () => window.removeEventListener("nb-save-request", handler);
    }, [contentJson, onSave]);

    return (
        <PageEditor
            content={initialContent ?? null}
            onChange={setContentJson}
            editable={true}
            audioFile={initialAudioFile}
            onAudioChange={onAudioChange}
        />
    );
}

// ─── Page Viewer / Editor ─────────────────────────────────────────────────────

function PageView({ setToast, notebook, onBack, returnTo, onReturnToOrigin }) {
    const [allPages, setAllPages] = useState([]);
    const [query, setQuery] = useState("");
    const [filteredPages, setFilteredPages] = useState([]);
    const [pageIndex, setPageIndex] = useState(0);
    const [editing, setEditing] = useState(false);
    const [dateOn, setDateOn] = useState("");
    const [today,  setToday] = useState(null);
    const [isNew, setIsNew] = useState(false);
    const [editTitle, setEditTitle] = useState("");
    const [editDesc, setEditDesc] = useState("");
    const [editContent, setEditContent] = useState(null);
    const [editAudioFile, setEditAudioFile] = useState(null);

    useEffect(() => {
        loadPages();
        loggedInvoke("get_current_date").then(setToday).catch(e => logError("catch", e));
    }, [notebook.id]);

    async function loadPages() {
        try {
            const pages = await loggedInvoke("get_pages", { notebookId: notebook.id });
            setAllPages(pages);
        } catch (e) { logError("catch", e); setToast("Failed to load pages.", "error"); }
    }

    useEffect(() => {
        let pages = allPages;
        if (query.trim()) {
            const q = query.toLowerCase();
            pages = pages.filter(p =>
                p.title.toLowerCase().includes(q) || (p.description ?? "").toLowerCase().includes(q)
            );
        }
        if (dateOn) pages = pages.filter(p => p.created_date === dateOn);
        setFilteredPages(pages);
        setPageIndex(0);
    }, [query, dateOn, allPages]);

    useEffect(() => {
        setPageIndex((prev) => Math.min(prev, Math.max(0, filteredPages.length - 1)));
    }, [filteredPages.length]);

    const currentPage = filteredPages[pageIndex] ?? null;

    function startEdit(page) {
        setEditTitle(page.title);
        setEditDesc(page.description ?? "");
        setEditAudioFile(page.audio_file ?? null);
        try { setEditContent(rewriteContentForDisplay(JSON.parse(page.content))); }
        catch { setEditContent(null); }
        setIsNew(false); setEditing(true);
    }

    function startNew() {
        setEditTitle(""); setEditDesc(""); setEditContent(null); setEditAudioFile(null);
        setIsNew(true); setEditing(true);
    }

    async function savePage(contentJson) {
        if (!editTitle.trim()) { setToast("Title is required."); return; }
        const contentStr = JSON.stringify(rewriteContentForSave(contentJson));
        try {
            if (isNew) {
                const newPage = await loggedInvoke("create_page", {
                    page: {
                        group_id: notebook.id,
                        title: editTitle.trim(),
                        description: editDesc.trim() || null,
                        content: contentStr,
                        audio_file: editAudioFile,
                    }
                });
                setAllPages((prev) => [...prev, newPage]);
                setQuery(""); setPageIndex(allPages.length); setToast("Page created.");
            } else {
                await loggedInvoke("update_page", {
                    page: {
                        ...currentPage,
                        title: editTitle.trim(),
                        description: editDesc.trim() || null,
                        content: contentStr,
                        audio_file: editAudioFile,
                    }
                });
                setAllPages((prev) => prev.map((p) =>
                    p.id === currentPage.id
                        ? { ...currentPage, title: editTitle.trim(), description: editDesc.trim() || null, content: contentStr, audio_file: editAudioFile }
                        : p
                ));
                setToast("Page saved.");
            }
            setEditing(false); setIsNew(false);
        } catch (e) { logError("catch", e); setToast("Failed to save page.", "error"); }
    }

    async function deletePage() {
        if (!currentPage) return;
        try {
            await loggedInvoke("delete_page", { id: currentPage.id });
            const remaining = allPages.filter((p) => p.id !== currentPage.id);
            setAllPages(remaining);
            setPageIndex((prev) => Math.min(prev, Math.max(0, remaining.length - 1)));
            setToast("Page deleted.");
        } catch (e) { logError("catch", e); setToast("Failed to delete page.", "error"); }
    }

    function getDisplayContent(page) {
        try { return rewriteContentForDisplay(JSON.parse(page.content)); }
        catch { return null; }
    }

    function formatDate(dateStr) {
        if (!dateStr) return "";
        try {
            const [year, month, day] = dateStr.split("-").map(Number);
            return new Date(year, month - 1, day).toLocaleDateString("en-US", {
                year: "numeric", month: "long", day: "numeric"
            });
        } catch { return dateStr; }
    }

    async function onCancel() {
        setEditing(false);
        setIsNew(false);
        await loggedInvoke("cleanup_orphaned_media");
    }

    return (
        <div className="nb-pages-root">
            <div className="nb-pages-header">
                {returnTo ? (
                    <button className="quiet" onClick={onReturnToOrigin}>← Back to {returnTo.label}</button>
                ) : (
                    <button className="quiet" onClick={onBack}>← Notebooks</button>
                )}
                <h2>{notebook.name}</h2>
                <span style={{ fontSize: 12, color: "var(--t-text-3)" }}>{allPages.length} page{allPages.length !== 1 ? "s" : ""}</span>
                {!editing && <button onClick={startNew}>+ New Page</button>}
            </div>

            {!editing && allPages.length > 1 && (
                <div className="nb-search">
                    <input type="text" placeholder="Search pages by title or description…" value={query}
                        onChange={(e) => { setQuery(e.target.value); }} />
                    {today && (
                        <div className="nb-date-filter">
                            <span style={{ fontSize: 12, color: "var(--t-text-3)", fontWeight: 500 }}>Created:</span>
                            <input type="date" value={dateOn} onChange={e => setDateOn(e.target.value)} />
                            {dateOn && <button style={{ fontSize: 11, padding: "1px 5px" }} onClick={() => setDateOn("")}>Clear</button>}
                        </div>
                    )}
                </div>
            )}

            <div className="nb-page-area">
                {editing ? (
                    <>
                        <div className="nb-edit-body">
                            <div className="nb-page-meta">
                                <div className="nb-page-meta-edit">
                                    <input className="nb-page-title-input" placeholder="Title…" value={editTitle} onChange={(e) => setEditTitle(e.target.value)}
                                        onKeyDown={(e) => { if (e.key === "Enter") window.dispatchEvent(new CustomEvent("nb-save-request")); if (e.key === "Escape") onCancel(); }} />
                                    <input className="nb-page-desc-input" placeholder="Description (optional)…" value={editDesc} onChange={(e) => setEditDesc(e.target.value)}
                                        onKeyDown={(e) => { if (e.key === "Enter") window.dispatchEvent(new CustomEvent("nb-save-request")); if (e.key === "Escape") onCancel(); }} />
                                    <button className="primary" onClick={() => window.dispatchEvent(new CustomEvent("nb-save-request"))}>Save</button>
                                    <button onClick={onCancel}>Cancel</button>
                                </div>
                                <hr className="nb-page-divider" />
                            </div>
                            <EditablePageEditor
                                initialContent={editContent}
                                initialAudioFile={editAudioFile}
                                onSave={savePage}
                                onAudioChange={setEditAudioFile}
                            />
                            <CardCreatorPanel setToast={setToast} />
                        </div>
                    </>
                ) : (
                    <>
                        {allPages.length === 0 ? (
                            <div className="nb-page-empty">No pages yet, create one above!</div>
                        ) : filteredPages.length === 0 ? (
                            <div className="nb-no-match">No pages match your filters.</div>
                        ) : currentPage ? (
                            <>
                                <div className="nb-page-meta">
                                    <div className="nb-page-title-row">
                                        <div className="nb-page-title">{currentPage.title}</div>
                                        <button onClick={() => startEdit(currentPage)}>Edit</button>
                                        <ConfirmDelete onConfirm={deletePage} />
                                    </div>
                                    {(currentPage.description || currentPage.created_date) && (
                                        <div className="nb-page-meta-row">
                                            {currentPage.description && (
                                                <span className="nb-page-description">{currentPage.description}</span>
                                            )}
                                            {currentPage.created_date && (
                                                <span className="nb-page-date">Created {formatDate(currentPage.created_date)}</span>
                                            )}
                                        </div>
                                    )}
                                    <hr className="nb-page-divider" />
                                </div>
                                <PageEditor
                                    content={getDisplayContent(currentPage)}
                                    onChange={() => {}}
                                    editable={false}
                                    audioFile={currentPage.audio_file ?? null}
                                />
                            </>
                        ) : null}
                        {filteredPages.length > 0 && (
                            <div className="nb-page-nav">
                                <button onClick={() => setPageIndex((i) => i === 0 ? filteredPages.length - 1 : i - 1)}>{"‹"}</button>
                                <span className="nb-page-nav-indicator">
                                    <input
                                        type="number"
                                        inputMode="numeric"
                                        className="nb-page-jump"
                                        min={1}
                                        max={filteredPages.length}
                                        key={pageIndex}
                                        defaultValue={pageIndex + 1}
                                        onClick={(e) => e.target.select()}
                                        onKeyDown={(e) => {
                                            if (e.key === "Enter") { e.target.blur(); return; }
                                            if (e.key === "Escape") { e.target.value = pageIndex + 1; e.target.blur(); return; }
                                            // digits only — block e/+/-/. and any other characters
                                            if (e.key.length === 1 && !/[0-9]/.test(e.key)) e.preventDefault();
                                        }}
                                        onPaste={(e) => {
                                            if (!/^[0-9]+$/.test(e.clipboardData.getData("text"))) e.preventDefault();
                                        }}
                                        onBlur={(e) => {
                                            const n = parseInt(e.target.value, 10);
                                            if (!isNaN(n)) {
                                                // out-of-range clamps: too high → last page, below 1 → page 1
                                                const idx = Math.min(filteredPages.length - 1, Math.max(0, n - 1));
                                                setPageIndex(idx);
                                                e.target.value = idx + 1;
                                            } else {
                                                e.target.value = pageIndex + 1;
                                            }
                                        }}
                                    />
                                    <span>/ {filteredPages.length}</span>
                                </span>
                                <button onClick={() => setPageIndex((i) => i === filteredPages.length - 1 ? 0 : i + 1)}>{"›"}</button>
                            </div>
                        )}
                    </>
                )}
            </div>
        </div>
    );
}

// ─── Root ─────────────────────────────────────────────────────────────────────

export default function Notebooks({ setToast, initialNotebook, onClearInitial, returnTo, onReturnToOrigin }) {
    const [view, setView] = useState(initialNotebook ? VIEW_PAGES : VIEW_NOTEBOOKS);
    const [activeNotebook, setActiveNotebook] = useState(initialNotebook ?? null);

    useEffect(() => {
        if (initialNotebook) onClearInitial?.();
    }, []);

    return (
        <>
            <div className="nb-root">
                {view === VIEW_NOTEBOOKS && (
                    <NotebookList setToast={setToast} onOpenNotebook={(nb) => { setActiveNotebook(nb); setView(VIEW_PAGES); }} />
                )}
                {view === VIEW_PAGES && activeNotebook && (
                    <PageView
                        setToast={setToast}
                        notebook={activeNotebook}
                        onBack={() => { setActiveNotebook(null); setView(VIEW_NOTEBOOKS); }}
                        returnTo={returnTo}
                        onReturnToOrigin={onReturnToOrigin}
                    />
                )}
            </div>
        </>
    );
}