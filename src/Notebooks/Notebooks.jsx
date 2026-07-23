import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import { NewCardForm } from "../Decks/Decks";
import { ConfirmDelete } from "../UIUtils";
import { mediaSrc } from "../mediaPaths";
import { AudioPlayer } from "../Decks/CardFace";
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
import { TextStyle, FontSize } from "@tiptap/extension-text-style";
import { Highlight } from "@tiptap/extension-highlight";
import { RevealBlock } from "./CustomExtentions";
import { rewriteContentForDisplay, rewriteContentForSave, stripPastedFonts } from "./NotebookUtils";
import "./Notebooks.css";

const VIEW_NOTEBOOKS = "notebooks";
const VIEW_PAGES     = "pages";

// Choosing DEFAULT_FONT_SIZE clears the mark instead of setting one, matching the .ProseMirror base size.
const FONT_SIZES = [10, 11, 12, 13, 14, 16, 18, 20, 24, 28, 32];
const DEFAULT_FONT_SIZE = 13;

// The mark stores a concrete hex value, so CSS variables can't be used here.
const FONT_COLORS = [
    { label: "Default",    value: null },
    { label: "Red",        value: "#C0392B" },
    { label: "Orange",     value: "#C2702A" },
    { label: "Gold",       value: "#B08A1F" },
    { label: "Green",      value: "#4A8C5E" },
    { label: "Blue",       value: "#3E6E96" },
    { label: "Purple",     value: "#7A5E8A" },
    { label: "Brown",      value: "#8A6E55" },
    { label: "Terracotta", value: "#C2705A" },
    { label: "Grey",       value: "#6B5458" },
];

async function pickFile(extensions) {
    try {
        const path = await open({ multiple: false, filters: [{ name: "File", extensions }] });
        return path ?? null;
    } catch { return null; }
}

// Matches the backend's ORDER BY name COLLATE NOCASE
const byName = (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });

// Empty paragraphs don't count. Any other node type does.
function isContentEmpty(json) {
    const hasContent = (node) => {
        if (!node) return false;
        if (node.type === "text") return (node.text ?? "").trim().length > 0;
        if (node.type !== "paragraph" && node.type !== "doc") return true;
        return (node.content ?? []).some(hasContent);
    };
    return !json || !hasContent(json);
}

// Notebook List

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
        if (!name) {setToast("Please enter a valid name.", "warn"); return; };
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
        if (!name) { setEditingId(null); setToast("Please enter a valid name.", "warn"); return; }
        try {
            await loggedInvoke("update_notebook", { notebook: { id, plan_id: null, name, group_type: "notebook" } });
            setNotebooks((prev) => prev.map((n) => n.id === id ? { ...n, name } : n).sort(byName));
            setToast(`${editingName} successfully updated.`);
        } catch (e) { logError("catch", e); setToast("Failed to update notebook.", "error"); }
        setEditingId(null);
    };

    const duplicateNotebook = async (nb) => {
        const existing = new Set(notebooks.map((n) => n.name));
        let name = `${nb.name} (copy)`;
        let n = 2;
        while (existing.has(name)) { name = `${nb.name} (copy ${n})`; n++; }
        try {
            const copy = await loggedInvoke("duplicate_notebook", { notebookId: nb.id, newName: name });
            const [updated, counts] = await Promise.all([
                loggedInvoke("get_notebooks"),
                loggedInvoke("get_notebook_page_counts"),
            ]);
            setNotebooks(updated);
            setPageCounts(Object.fromEntries(counts));
            setToast(`${copy.name} created.`);
        } catch (e) { logError("catch", e); setToast("Failed to duplicate notebook.", "error"); }
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
        if (!mergeNotebookA || !mergeNotebookB) { setToast("Please select two notebooks.", "warn"); return; }
        if (mergeNotebookA === mergeNotebookB) { setToast("Please select two different notebooks.", "warn"); return; }
        if (!mergeName.trim()) { setToast("Please enter a name for the merged notebook.", "warn"); return; }
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
                    <div style={{ fontSize: 11, color: "var(--t-text-3)", fontStyle: "italic" }}>
                        The two source notebooks will be deleted after their pages move into the new notebook. Pages are ordered by date created. If either notebook is linked to a todo, that link will be removed.
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
                                            <b>{pageCounts[nb.id] ?? 0}</b>
                                            <span>{(pageCounts[nb.id] ?? 0) === 1 ? "page" : "pages"}</span>
                                        </span>
                                        <span className="landing-stat-divider" />
                                        <button style={{ alignSelf: "center" }} onClick={(e) => { e.stopPropagation(); onOpenNotebook(nb, true); }}>+ New Page</button>
                                    </div>
                                </>
                            )}
                        </div>
                        <div className="landing-card-actions" onClick={(e) => e.stopPropagation()}>
                            {editingId === nb.id ? (
                                <>
                                    <button className="primary" onMouseDown={(e) => e.preventDefault()} onClick={() => confirmEdit(nb.id)}>Save</button>
                                    <button onMouseDown={(e) => e.preventDefault()} onClick={() => setEditingId(null)}>Cancel</button>
                                </>
                            ) : (
                                <>
                                    <button onClick={(e) => startEdit(nb, e)}>Edit</button>
                                    <button onClick={(e) => { e.stopPropagation(); duplicateNotebook(nb); }}>Duplicate</button>
                                    <ConfirmDelete onConfirm={() => deleteNotebook(nb.id)} small />
                                </>
                            )}
                        </div>
                    </div>
                ))}
            </div>
            <div className="nb-new-notebook">
                <input type="text" placeholder="New notebook name..." value={newName}
                    onChange={(e) => setNewName(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && createNotebook()} />
                <button className="primary" onClick={createNotebook}>+ Create</button>
            </div>
        </>
    );
}

// Audio Recorder

// 16 kHz is plenty for voice and keeps the WAV (and IPC payload) about 3x smaller than the mic's normal rate.
const WAV_RATE = 16000;

function encodeWav(pcm, srcRate) {
    let samples = pcm;
    let rate = srcRate;
    if (srcRate > WAV_RATE) {
        const n = Math.floor(pcm.length * WAV_RATE / srcRate);
        samples = new Float32Array(n);
        const step = srcRate / WAV_RATE;
        for (let i = 0; i < n; i++) {
            const pos = i * step, j = Math.floor(pos), frac = pos - j;
            samples[i] = pcm[j] + (pcm[Math.min(j + 1, pcm.length - 1)] - pcm[j]) * frac;
        }
        rate = WAV_RATE;
    }
    const buf = new ArrayBuffer(44 + samples.length * 2);
    const v = new DataView(buf);
    const ws = (o, s) => { for (let i = 0; i < s.length; i++) v.setUint8(o + i, s.charCodeAt(i)); };
    ws(0, "RIFF"); v.setUint32(4, 36 + samples.length * 2, true); ws(8, "WAVE");
    ws(12, "fmt "); v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true);
    v.setUint32(24, rate, true); v.setUint32(28, rate * 2, true); v.setUint16(32, 2, true); v.setUint16(34, 16, true);
    ws(36, "data"); v.setUint32(40, samples.length * 2, true);
    for (let i = 0; i < samples.length; i++) {
        const s = Math.max(-1, Math.min(1, samples[i]));
        v.setInt16(44 + i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true);
    }
    return new Uint8Array(buf);
}

// MediaRecorder deliberately not used: WebKitGTK advertises audio/mp4 but emits zero bytes on Linux.
function useAudioRecorder({ audioFile, onAudioChange }) {
    const [recording, setRecording] = useState(false);
    const [paused, setPaused] = useState(false);
    const ctxRef = useRef(null);
    const nodesRef = useRef([]);
    const chunksRef = useRef([]);
    const pausedRef = useRef(false);
    const streamRef = useRef(null);

    function teardown() {
        streamRef.current?.getTracks().forEach(t => t.stop());
        streamRef.current = null;
        nodesRef.current.forEach(n => { try { n.disconnect(); } catch { /* already gone */ } });
        nodesRef.current = [];
        ctxRef.current?.close().catch(() => {});
        ctxRef.current = null;
    }

    useEffect(() => teardown, []);

    async function startRecording() {
        try {
            const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
            streamRef.current = stream;
            const ctx = new AudioContext();
            ctxRef.current = ctx;
            const source = ctx.createMediaStreamSource(stream);
            const proc = ctx.createScriptProcessor(4096, 1, 1);
            // Zero-gain sink keeps the mic from echoing while keeping the processor wired to the destination.
            const sink = ctx.createGain();
            sink.gain.value = 0;
            chunksRef.current = [];
            pausedRef.current = false;
            proc.onaudioprocess = (e) => {
                if (!pausedRef.current) chunksRef.current.push(new Float32Array(e.inputBuffer.getChannelData(0)));
            };
            source.connect(proc);
            proc.connect(sink);
            sink.connect(ctx.destination);
            nodesRef.current = [source, proc, sink];
            setRecording(true);
            setPaused(false);
        } catch (e) { logError("microphone_access", e); teardown(); }
    }

    function pauseRecording() { pausedRef.current = true; setPaused(true); }
    function resumeRecording() { pausedRef.current = false; setPaused(false); }

    async function stopRecording() {
        const rate = ctxRef.current?.sampleRate ?? 44100;
        teardown();
        setRecording(false);
        setPaused(false);
        const chunks = chunksRef.current;
        chunksRef.current = [];
        const total = chunks.reduce((n, c) => n + c.length, 0);
        if (total === 0) return;
        const pcm = new Float32Array(total);
        let off = 0;
        for (const c of chunks) { pcm.set(c, off); off += c.length; }
        try {
            const path = await loggedInvoke("save_page_audio", {
                data: Array.from(encodeWav(pcm, rate)),
                mime: "audio/wav",
            });
            onAudioChange(path);
        } catch (e) { logError("save_page_audio", e); }
    }

    // Just clears the reference. Deleting from disk here broke Cancel: the DB kept pointing at a gone file.
    // Actual cleanup happens in update_page on save and cleanup_orphaned_media on cancel.
    function deleteAudio() { onAudioChange(null); }

    return { recording, paused, startRecording, pauseRecording, resumeRecording, stopRecording, deleteAudio };
}

// With a clip present, the play button takes the record slot. Re-record by deleting then recording again.
function AudioControls({ audioFile, audio }) {
    if (audio.recording) {
        return (
            <>
                {!audio.paused ? (
                    <button className="nb-tb-btn record recording" onClick={audio.pauseRecording}>⏸ Pause</button>
                ) : (
                    <button className="nb-tb-btn record recording" onClick={audio.resumeRecording}>▶ Resume</button>
                )}
                <button className="nb-tb-btn" onClick={audio.stopRecording}>■ Stop</button>
            </>
        );
    }
    if (audioFile) {
        return (
            <>
                <AudioPlayer path={audioFile} buttonClassName="audio-btn sm" />
                <button className="nb-tb-btn danger" onClick={audio.deleteAudio}>Delete Audio</button>
            </>
        );
    }
    return (
        <button className="nb-tb-btn record" onClick={audio.startRecording}>● Record</button>
    );
}

// Page Editor (TipTap)

export function PageEditor({ content, onChange, editable, audioFile, onAudioChange }) {
    const [linkPrompt, setLinkPrompt] = useState(false);
    const [linkText, setLinkText] = useState("");
    const [linkUrl, setLinkUrl] = useState("");
    const [colorPrompt, setColorPrompt] = useState(false);
    const audio = useAudioRecorder({ audioFile, onAudioChange: onAudioChange ?? (() => {}) });

    const toolbarRef = useRef(null);
    const [tbScroll, setTbScroll] = useState({ l: false, r: false });
    const updateTbScroll = useCallback(() => {
        const el = toolbarRef.current;
        if (!el) return;
        const l = el.scrollLeft > 1;
        const r = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
        setTbScroll((prev) => (prev.l === l && prev.r === r ? prev : { l, r }));
    }, []);
    // Mouse wheel only sends vertical delta; redirect it horizontal since the toolbar has no vertical overflow.
    const handleTbWheel = useCallback((e) => {
        const el = toolbarRef.current;
        if (!el || el.scrollWidth <= el.clientWidth) return;
        if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
        el.scrollLeft += e.deltaY;
        e.preventDefault();
    }, []);

    const editor = useEditor({
        extensions: [
            // heading is off: sizing is a numeric font size, not a document level.
            // Legacy heading nodes are migrated on load (see NotebookUtils).
            StarterKit.configure({ code: false, codeBlock: false, link: false, heading: false }),
            TextStyle,
            Color,
            FontSize,
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
            TextAlign.configure({ types: ["paragraph"] }),
            RevealBlock,
        ],
        content: content ?? null,
        editable,
        editorProps: { transformPastedHTML: stripPastedFonts },
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
            editor.chain().focus().setImage({ src: mediaSrc(path), rawPath: path }).run();
        }
    }, [editor]);

    const closeLinkPrompt = useCallback(() => {
        setLinkPrompt(false);
        setLinkText("");
        setLinkUrl("");
    }, []);

    const confirmLink = useCallback(() => {
        const url = linkUrl.trim();
        if (!url) { closeLinkPrompt(); return; }
        const text = linkText.trim() || url;
        const href = url.startsWith("http") ? url : `https://${url}`;
        editor.chain().focus()
            .insertContent({ type: "text", marks: [{ type: "link", attrs: { href } }], text })
            .insertContent({ type: "text", text: " " })
            .run();
        closeLinkPrompt();
    }, [editor, linkText, linkUrl, closeLinkPrompt]);

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

    const applyColor = useCallback((value) => {
        setColorPrompt(false);
        const chain = editor.chain().focus();
        (value ? chain.setColor(value) : chain.unsetColor()).run();
    }, [editor]);

    if (!editor) return null;

    const textStyle = editor.getAttributes("textStyle");
    const activeFontSize = parseInt(textStyle.fontSize, 10) || DEFAULT_FONT_SIZE;
    const activeColor = textStyle.color ?? null;

    return (
        <div className="nb-editor-wrap">
            {editable && (
                <>
                    {linkPrompt && (
                        <div className="nb-inline-prompt">
                            <input type="text" autoFocus
                                placeholder="Display text (optional)"
                                value={linkText}
                                onChange={(e) => setLinkText(e.target.value)}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter") confirmLink();
                                    if (e.key === "Escape") closeLinkPrompt();
                                }} />
                            <input type="text"
                                placeholder="URL"
                                value={linkUrl}
                                onChange={(e) => setLinkUrl(e.target.value)}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter") confirmLink();
                                    if (e.key === "Escape") closeLinkPrompt();
                                }} />
                            <button className="nb-tb-btn" onClick={confirmLink}>Insert</button>
                            <button className="nb-tb-btn" onClick={closeLinkPrompt}>Cancel</button>
                        </div>
                    )}
                    {/* Swatches on their own row; the toolbar would clip an anchored popover when scrolled. */}
                    {colorPrompt && (
                        <div className="nb-inline-prompt nb-color-prompt">
                            {FONT_COLORS.map(({ label, value }) => (
                                <button key={label} title={label}
                                    className={`nb-color-swatch${value === null ? " nb-color-swatch--default" : ""}${activeColor === value ? " active" : ""}`}
                                    style={value ? { background: value } : undefined}
                                    onClick={() => applyColor(value)}>
                                    {value === null ? "Default" : ""}
                                </button>
                            ))}
                            <button className="nb-tb-btn" onClick={() => setColorPrompt(false)}>Cancel</button>
                        </div>
                    )}
                    <div className={`nb-toolbar-wrap${tbScroll.l ? " scroll-l" : ""}${tbScroll.r ? " scroll-r" : ""}`}>
                    <div className="nb-toolbar" ref={toolbarRef} onScroll={updateTbScroll} onWheel={handleTbWheel}>
                        <AudioControls audioFile={audioFile} audio={audio} />
                        <div className="nb-tb-sep" />
                        <button className={`nb-tb-btn${editor.isActive("bold") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleBold().run()}><b>B</b></button>
                        <button className={`nb-tb-btn${editor.isActive("italic") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleItalic().run()}><i>I</i></button>
                        <button className={`nb-tb-btn${editor.isActive("underline") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleUnderline().run()}><u>U</u></button>
                        <button className={`nb-tb-btn${editor.isActive("strike") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleStrike().run()}><s>S</s></button>
                        <button className={`nb-tb-btn${editor.isActive("highlight") ? " active" : ""}`} onClick={() => editor.chain().focus().toggleHighlight().run()}><mark>H</mark></button>
                        <div className="nb-tb-sep" />
                        <select className="nb-tb-select" title="Font size"
                            value={activeFontSize}
                            onChange={(e) => {
                                const v = Number(e.target.value);
                                if (v === DEFAULT_FONT_SIZE) editor.chain().focus().unsetFontSize().run();
                                else editor.chain().focus().setFontSize(`${v}px`).run();
                            }}>
                            {FONT_SIZES.map((s) => <option key={s} value={s}>{s}</option>)}
                        </select>
                        <button className={`nb-tb-btn nb-tb-color-btn${colorPrompt ? " active" : ""}`} title="Font color"
                            onClick={() => setColorPrompt((p) => !p)}>
                            A<span className="nb-tb-color-chip" style={{ background: activeColor ?? "var(--t-text)" }} />
                        </button>
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

// Card Creator Panel

function CardCreatorPanel({ setToast }) {
    const [open, setOpen] = useState(false);
    const [decks, setDecks] = useState([]);
    const [deckId, setDeckId] = useState(null);
    // Dragged panel height, null means size to fit the form
    const [creatorHeight, setCreatorHeight] = useState(null);
    const creatorRef = useRef(null);
    const dragMovedRef = useRef(false);

    // Panel can't shrink below this while dragging
    const CREATOR_MIN_PX = 90;

    // The open header doubles as a drag handle: clicks still toggle, drags resize
    const startDrag = (e) => {
        if (e.button !== 0 || !open) return;
        const panel = creatorRef.current;
        const frame = panel?.offsetParent;
        if (!panel || !frame) return;
        const maxHeight = frame.getBoundingClientRect().height - 12;
        const startHeight = panel.offsetHeight;
        const startY = e.clientY;
        let moved = false;
        dragMovedRef.current = false;

        const heightAt = (ev) => Math.min(maxHeight, Math.max(CREATOR_MIN_PX, startHeight + (startY - ev.clientY)));
        const onMove = (ev) => {
            if (!moved && Math.abs(ev.clientY - startY) < 4) return;
            moved = true;
            dragMovedRef.current = true;
            setCreatorHeight(heightAt(ev));
        };
        const onUp = () => {
            document.removeEventListener("mousemove", onMove);
            document.removeEventListener("mouseup", onUp);
        };
        document.addEventListener("mousemove", onMove);
        document.addEventListener("mouseup", onUp);
        e.preventDefault();
    };

    const toggle = () => {
        if (dragMovedRef.current) { dragMovedRef.current = false; return; }
        setOpen((o) => !o);
    };

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
        <div
            ref={creatorRef}
            className={`nb-card-creator${open ? " open" : ""}`}
            style={open && creatorHeight != null ? { height: creatorHeight } : undefined}
        >
            <div className="nb-card-toggle" onMouseDown={startDrag} onClick={toggle}>
                <span style={{ fontSize: 13, fontWeight: 700 }}>Create a card</span>
                <span style={{ fontSize: 10, color: "var(--t-text-3)" }}>{open ? "▾" : "▸"}</span>
            </div>
            {/* Kept mounted while closed so in-progress input survives toggling. */}
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

// Editable page wrapper

function EditablePageEditor({ initialContent, initialAudioFile, onSave, onAudioChange, onDraft }) {
    const [contentJson, setContentJson] = useState(initialContent ?? null);

    useEffect(() => {
        const handler = () => { onSave(contentJson); };
        window.addEventListener("nb-save-request", handler);
        return () => window.removeEventListener("nb-save-request", handler);
    }, [contentJson, onSave]);

    return (
        <PageEditor
            content={initialContent ?? null}
            onChange={(json) => { setContentJson(json); onDraft?.(json); }}
            editable={true}
            audioFile={initialAudioFile}
            onAudioChange={onAudioChange}
        />
    );
}

// Page Viewer / Editor

function PageView({ setToast, notebook, onBack, returnTo, onReturnToOrigin, startNewOnOpen }) {
    const [allPages, setAllPages] = useState([]);
    const [query, setQuery] = useState("");
    const [pageIndex, setPageIndex] = useState(0);
    // After a save, the page may have moved or not existed yet; this holds the id to land on once the list settles.
    const [pendingPageId, setPendingPageId] = useState(null);
    const [editing, setEditing] = useState(false);
    const [dateOn, setDateOn] = useState("");
    const [today,  setToday] = useState(null);
    const [isNew, setIsNew] = useState(false);
    const [editTitle, setEditTitle] = useState("");
    const [editDesc, setEditDesc] = useState("");
    const [editContent, setEditContent] = useState(null);
    const [editAudioFile, setEditAudioFile] = useState(null);
    const liveContentRef = useRef(null);

    useEffect(() => {
        loadPages();
        loggedInvoke("get_current_date").then(setToday).catch(e => logError("catch", e));
    }, [notebook.id]);

    useEffect(() => { if (startNewOnOpen) startNew(); }, []);

    async function loadPages() {
        try {
            const pages = await loggedInvoke("get_pages", { notebookId: notebook.id });
            setAllPages(pages);
        } catch (e) { logError("catch", e); setToast("Failed to load pages.", "error"); }
    }

    const filteredPages = useMemo(() => {
        let pages = allPages;
        if (query.trim()) {
            const q = query.toLowerCase();
            pages = pages.filter(p =>
                p.title.toLowerCase().includes(q) || (p.description ?? "").toLowerCase().includes(q)
            );
        }
        if (dateOn) pages = pages.filter(p => p.created_date === dateOn);
        return pages;
    }, [allPages, query, dateOn]);

    // Filter changes start over at page one. Editing must NOT reset the index: that's how you stay on the saved page.
    useEffect(() => { setPageIndex(0); }, [query, dateOn]);

    useEffect(() => {
        setPageIndex((prev) => Math.min(prev, Math.max(0, filteredPages.length - 1)));
    }, [filteredPages.length]);

    // Runs after the reset above, so landing on a saved page wins over the filter reset
    useEffect(() => {
        if (pendingPageId === null) return;
        const idx = filteredPages.findIndex(p => p.id === pendingPageId);
        if (idx !== -1) setPageIndex(idx);
        setPendingPageId(null);
    }, [filteredPages, pendingPageId]);

    const currentPage = filteredPages[pageIndex] ?? null;

    function startEdit(page) {
        setEditTitle(page.title);
        setEditDesc(page.description ?? "");
        setEditAudioFile(page.audio_file ?? null);
        let parsed = null;
        try { parsed = rewriteContentForDisplay(JSON.parse(page.content)); }
        catch { /* unreadable content edits as blank */ }
        setEditContent(parsed);
        liveContentRef.current = parsed;
        setIsNew(false); setEditing(true);
    }

    function startNew() {
        setEditTitle(""); setEditDesc(""); setEditContent(null); setEditAudioFile(null);
        liveContentRef.current = null;
        setIsNew(true); setEditing(true);
    }

    async function savePage(contentJson) {
        const title = editTitle.trim() || "Untitled";
        const contentStr = JSON.stringify(rewriteContentForSave(contentJson));
        try {
            if (isNew) {
                const newPage = await loggedInvoke("create_page", {
                    page: {
                        group_id: notebook.id,
                        title,
                        description: editDesc.trim() || null,
                        content: contentStr,
                        audio_file: editAudioFile,
                    }
                });
                setAllPages((prev) => [...prev, newPage]);
                // Clear both filters so the new page can't be filtered out from under us
                setQuery(""); setDateOn(""); setPendingPageId(newPage.id); setToast("Page created.");
            } else {
                await loggedInvoke("update_page", {
                    page: {
                        ...currentPage,
                        title,
                        description: editDesc.trim() || null,
                        content: contentStr,
                        audio_file: editAudioFile,
                    }
                });
                setAllPages((prev) => prev.map((p) =>
                    p.id === currentPage.id
                        ? { ...currentPage, title, description: editDesc.trim() || null, content: contentStr, audio_file: editAudioFile }
                        : p
                ));
                // Follow the page if a retitle moved it within the current filter
                setPendingPageId(currentPage.id);
                setToast("Page saved.");
            }
            setEditing(false); setIsNew(false);
            // Recordings replaced mid-edit stay on disk until now. DB is authoritative so orphans are safe to sweep.
            loggedInvoke("cleanup_orphaned_media").catch(e => logError("cleanup_orphaned_media", e));
            return true;
        } catch (e) { logError("catch", e); setToast("Failed to save page.", "error"); return false; }
    }

    // Back autosaves. An entirely empty untitled page is discarded instead of saved.
    async function handleBack(navigate) {
        if (editing) {
            const draft = liveContentRef.current;
            const hasContent = !isContentEmpty(draft) || editDesc.trim() || editAudioFile;
            if (!editTitle.trim() && !hasContent) {
                await onCancel();
            } else if (!(await savePage(draft))) {
                return;
            }
        }
        navigate();
    }

    async function deletePage() {
        if (!currentPage) return;
        try {
            await loggedInvoke("delete_page", { id: currentPage.id });
            setAllPages((prev) => prev.filter((p) => p.id !== currentPage.id));
            // Land on the previous page. Deleting the first one leaves index 0, now the page that followed it.
            setPageIndex((prev) => Math.max(0, prev - 1));
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
                    <button className="quiet" onClick={() => handleBack(onReturnToOrigin)}>← Back to {returnTo.label}</button>
                ) : (
                    <button className="quiet" onClick={() => handleBack(onBack)}>← Back</button>
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
                            <span>Created:</span>
                            <input type="date" value={dateOn} onChange={e => setDateOn(e.target.value)} />
                            {dateOn && <button className="nb-date-clear" title="Clear" onClick={() => setDateOn("")}>×</button>}
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
                                onDraft={(json) => { liveContentRef.current = json; }}
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
                                        {currentPage.audio_file && (
                                            <AudioPlayer path={currentPage.audio_file} buttonClassName="audio-btn sm" />
                                        )}
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
                                            // block e/+/-/. and other non-numeric keys
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

// Root

export default function Notebooks({ setToast, initialNotebook, onClearInitial, returnTo, onReturnToOrigin, homeSignal }) {
    const [view, setView] = useState(initialNotebook ? VIEW_PAGES : VIEW_NOTEBOOKS);
    const [activeNotebook, setActiveNotebook] = useState(initialNotebook ?? null);
    const [startNewOnOpen, setStartNewOnOpen] = useState(false);

    // Re-clicking the Notebooks tab comes back here. Compared against the count this
    // mount started on: the effect runs on mount as well, and the count stays above zero
    // once anything has been re-clicked, so a notebook opened from a plan would be
    // closed again the moment it opened.
    const signalAtMount = useRef(homeSignal);
    useEffect(() => {
        if (homeSignal === signalAtMount.current) return;
        setActiveNotebook(null);
        setStartNewOnOpen(false);
        setView(VIEW_NOTEBOOKS);
    }, [homeSignal]);

    useEffect(() => {
        if (initialNotebook) onClearInitial?.();
    }, []);

    return (
        <>
            <div className="nb-root">
                {view === VIEW_NOTEBOOKS && (
                    <NotebookList setToast={setToast} onOpenNotebook={(nb, startNew = false) => { setActiveNotebook(nb); setStartNewOnOpen(startNew); setView(VIEW_PAGES); }} />
                )}
                {view === VIEW_PAGES && activeNotebook && (
                    <PageView
                        setToast={setToast}
                        notebook={activeNotebook}
                        onBack={() => { setActiveNotebook(null); setStartNewOnOpen(false); setView(VIEW_NOTEBOOKS); }}
                        returnTo={returnTo}
                        onReturnToOrigin={onReturnToOrigin}
                        startNewOnOpen={startNewOnOpen}
                    />
                )}
            </div>
        </>
    );
}
