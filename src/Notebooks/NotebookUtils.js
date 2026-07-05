import { convertFileSrc } from "@tauri-apps/api/core";

// ─── Rewrite TipTap JSON for display ─────────────────────────────────────────
// Walks the TipTap JSON and converts all image srcs from absolute disk
// paths to asset:// URLs that the Tauri webview can load.

export function rewriteContentForDisplay(json) {
    if (!json || typeof json !== "object") return json;
    const node = { ...json };

    if (node.type === "image" && node.attrs) {
        const raw = node.attrs.rawPath || node.attrs.src;
        node.attrs = { ...node.attrs, src: convertFileSrc(raw), rawPath: raw };
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