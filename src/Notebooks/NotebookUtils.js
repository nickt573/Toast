import { mediaSrc } from "../mediaPaths";

// ─── Legacy heading migration ────────────────────────────────────────────────
// The editor used to offer H1/H2/H3; it now offers numeric font sizes instead,
// and the heading node is gone from the schema. Pages saved before that change
// still hold heading nodes, which ProseMirror would silently drop on load — so
// they are rewritten into paragraphs of bold, sized text that read the same.

const HEADING_FONT_SIZES = { 1: "24px", 2: "18px", 3: "16px" };

// Applies a font size to a text node without clobbering a size it already carries.
function applyHeadingMarks(node, fontSize) {
    if (node.type !== "text") return node;
    const marks = node.marks ?? [];
    const existing = marks.find((m) => m.type === "textStyle");
    if (existing?.attrs?.fontSize) return node;

    const textStyle = { type: "textStyle", attrs: { ...(existing?.attrs ?? {}), fontSize } };
    const rest = marks.filter((m) => m.type !== "textStyle");
    const bold = rest.some((m) => m.type === "bold") ? [] : [{ type: "bold" }];
    return { ...node, marks: [...rest, ...bold, textStyle] };
}

function migrateHeading(node) {
    const { level, ...attrs } = node.attrs ?? {};
    const fontSize = HEADING_FONT_SIZES[level] ?? HEADING_FONT_SIZES[3];
    // attrs carries textAlign, which paragraphs support too
    const paragraph = { type: "paragraph", attrs };
    if (Array.isArray(node.content) && node.content.length > 0) {
        paragraph.content = node.content.map((child) => applyHeadingMarks(child, fontSize));
    }
    return paragraph;
}

// ─── Rewrite TipTap JSON for display ─────────────────────────────────────────
// Walks the TipTap JSON and converts all image srcs from absolute disk
// paths to asset:// URLs that the Tauri webview can load.

export function rewriteContentForDisplay(json) {
    if (!json || typeof json !== "object") return json;
    let node = { ...json };

    if (node.type === "heading") node = migrateHeading(node);

    if (node.type === "image" && node.attrs) {
        const raw = node.attrs.rawPath || node.attrs.src;
        node.attrs = { ...node.attrs, src: mediaSrc(raw), rawPath: raw };
    }

    if (Array.isArray(node.content)) {
        node.content = node.content.map(rewriteContentForDisplay);
    }

    return node;
}

export function rewriteContentForSave(json) {
    if (!json || typeof json !== "object") return json;
    const node = { ...json };

    if (node.type === "image" && node.attrs) {
        const raw = node.attrs.rawPath || node.attrs.src;
        node.attrs = { ...node.attrs, src: raw };
    }

    if (Array.isArray(node.content)) {
        node.content = node.content.map(rewriteContentForSave);
    }

    return node;
}

// ─── Paste normalization ─────────────────────────────────────────────────────
// Every page renders in the app's body font. Font sizes and colors survive a
// paste (both are editable here); the source's font family never does.

export function stripPastedFonts(html) {
    const doc = new DOMParser().parseFromString(html, "text/html");

    doc.body.querySelectorAll("[style]").forEach((el) => {
        el.style.removeProperty("font-family");
        el.style.removeProperty("font");   // shorthand also sets font-family
        if (!el.getAttribute("style")) el.removeAttribute("style");
    });
    doc.body.querySelectorAll("font[face]").forEach((el) => el.removeAttribute("face"));

    return doc.body.innerHTML;
}
