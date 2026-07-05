import { Node, mergeAttributes } from "@tiptap/core";
import { ReactNodeViewRenderer, NodeViewWrapper } from "@tiptap/react";
import { useState } from "react";

// ─── Reveal Block ─────────────────────────────────────────────────────────────
// Plain-text prompt + answer. In edit mode both fields are visible and editable.
// In read-only mode the answer is hidden behind a Reveal toggle.

function RevealBlockView({ node, updateAttributes, editor }) {
    const [revealed, setRevealed] = useState(false);
    const isEditable = editor.isEditable;

    return (
        <NodeViewWrapper>
            <div style={{
                border: "1px solid var(--t-border-2)",
                borderRadius: "6px",
                overflow: "hidden",
                margin: "8px 0",
                fontSize: 14,
            }}>
                {/* Prompt row */}
                <div style={{
                    display: "flex",
                    alignItems: "stretch",
                    borderBottom: "1px solid var(--t-border)",
                }}>
                    <div style={{
                        padding: "6px 10px",
                        background: "var(--t-surface-2)",
                        borderRight: "1px solid var(--t-border)",
                        fontSize: 11,
                        fontWeight: 700,
                        letterSpacing: "0.02em",
                        color: "var(--t-text-3)",
                        display: "flex",
                        alignItems: "center",
                        minWidth: 64,
                        userSelect: "none",
                    }}>
                        Prompt
                    </div>
                    {isEditable ? (
                        <input
                            type="text"
                            value={node.attrs.prompt}
                            placeholder="Write your prompt here…"
                            onChange={(e) => updateAttributes({ prompt: e.target.value })}
                            style={{
                                flex: 1,
                                border: "none",
                                outline: "none",
                                padding: "8px 10px",
                                fontSize: 14,
                                fontFamily: "inherit",
                                background: "#fff",
                            }}
                        />
                    ) : (
                        <div style={{ flex: 1, padding: "8px 10px" }}>
                            {node.attrs.prompt || <em style={{ color: "var(--t-text-3)" }}>No prompt</em>}
                        </div>
                    )}
                </div>

                {/* Answer row */}
                <div style={{ display: "flex", alignItems: "stretch" }}>
                    <div style={{
                        padding: "6px 10px",
                        background: "var(--t-surface-2)",
                        borderRight: "1px solid var(--t-border)",
                        fontSize: 11,
                        fontWeight: 700,
                        letterSpacing: "0.02em",
                        color: "var(--t-text-3)",
                        display: "flex",
                        alignItems: "center",
                        justifyContent: "space-between",
                        minWidth: 64,
                        userSelect: "none",
                        flexDirection: "column",
                        gap: 4,
                    }}>
                        <span>Answer</span>
                        {!isEditable && (
                            <button
                                onClick={() => setRevealed((r) => !r)}
                                style={{
                                    fontSize: 10,
                                    padding: "2px 6px",
                                    border: "1px solid var(--t-border-2)",
                                    borderRadius: 3,
                                    background: "#fff",
                                    cursor: "pointer",
                                    whiteSpace: "nowrap",
                                }}
                            >
                                {revealed ? "Hide" : "Show"}
                            </button>
                        )}
                    </div>
                    {isEditable ? (
                        <textarea
                            value={node.attrs.answer}
                            placeholder="Write the answer here…"
                            onChange={(e) => updateAttributes({ answer: e.target.value })}
                            rows={3}
                            style={{
                                flex: 1,
                                border: "none",
                                outline: "none",
                                padding: "8px 10px",
                                fontSize: 14,
                                fontFamily: "inherit",
                                resize: "vertical",
                                background: "#fff",
                            }}
                        />
                    ) : (
                        <div style={{
                            flex: 1,
                            padding: "8px 10px",
                            display: revealed ? "block" : "none",
                            whiteSpace: "pre-wrap",
                        }}>
                            {node.attrs.answer || <em style={{ color: "var(--t-text-3)" }}>No answer</em>}
                        </div>
                    )}
                </div>
            </div>
        </NodeViewWrapper>
    );
}

export const RevealBlock = Node.create({
    name: "revealBlock",
    group: "block",
    atom: true,

    addAttributes() {
        return {
            prompt: { default: "" },
            answer: { default: "" },
        };
    },

    parseHTML() {
        return [{ tag: "div[data-type='reveal-block']" }];
    },

    renderHTML({ HTMLAttributes }) {
        return ["div", mergeAttributes(HTMLAttributes, { "data-type": "reveal-block" })];
    },

    addNodeView() {
        return ReactNodeViewRenderer(RevealBlockView);
    },
});