import { mediaSrc } from "../mediaPaths";

// Legacy heading migration
// H1/H2/H3 was replaced with numeric font sizes. Old pages still hold heading nodes that ProseMirror
// would silently drop, so they are rewritten into bold, sized paragraphs that read the same.

const HEADING_FONT_SIZES = { 1: "24px", 2: "18px", 3: "16px" };

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

// Rewrite TipTap JSON for display

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

// Paste normalization
// Font sizes and colors survive a paste (both are editable here), but the source's font family never does.

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
