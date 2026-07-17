import { Node, mergeAttributes } from "@tiptap/core";
import { ReactNodeViewRenderer, NodeViewWrapper } from "@tiptap/react";
import { useState } from "react";

// Reveal Block: prompt + answer, in read-only mode the answer is hidden behind a toggle.

// Fixed-basis label column so the Prompt and Answer cells (and their right
// borders) always line up regardless of content width.
const revealLabelCell = {
    flex: "0 0 76px",
    padding: "6px 10px",
    background: "var(--t-surface-3)",
    borderRight: "1px solid var(--t-border-2)",
    fontSize: 11,
    fontWeight: 700,
    color: "var(--t-text-2)",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    userSelect: "none",
};

function RevealBlockView({ node, updateAttributes, editor }) {
    const [revealed, setRevealed] = useState(false);
    const isEditable = editor.isEditable;

    return (
        <NodeViewWrapper>
            <div style={{
                border: "1px solid var(--t-border-2)",
                borderRadius: "var(--t-r-lg)",
                overflow: "hidden",
                margin: "8px 0",
                fontSize: 14,
                background: "var(--t-surface)",
            }}>
                {/* Prompt row */}
                <div style={{
                    display: "flex",
                    alignItems: "stretch",
                    borderBottom: "1px solid var(--t-border-2)",
                }}>
                    <div style={revealLabelCell}>
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
                                background: "var(--t-surface)",
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
                        ...revealLabelCell,
                        flexDirection: "column",
                        justifyContent: "center",
                        gap: 5,
                    }}>
                        <span>Answer</span>
                        {!isEditable && (
                            <button
                                onClick={() => setRevealed((r) => !r)}
                                style={{
                                    fontSize: 10,
                                    fontWeight: 700,
                                    fontFamily: "inherit",
                                    padding: "2px 9px",
                                    border: "1px solid var(--t-accent-bdr)",
                                    borderRadius: "var(--t-r)",
                                    background: revealed ? "var(--t-accent)" : "var(--t-surface)",
                                    color: revealed ? "var(--t-accent-fg)" : "var(--t-accent-h)",
                                    cursor: "pointer",
                                    whiteSpace: "nowrap",
                                    transition: "background var(--t-ease), color var(--t-ease)",
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
                                background: "var(--t-surface)",
                            }}
                        />
                    ) : revealed ? (
                        <div style={{
                            flex: 1,
                            padding: "8px 10px",
                            whiteSpace: "pre-wrap",
                        }}>
                            {node.attrs.answer || <em style={{ color: "var(--t-text-3)" }}>No answer</em>}
                        </div>
                    ) : (
                        <div style={{
                            flex: 1,
                            padding: "8px 10px",
                            display: "flex",
                            alignItems: "center",
                            color: "var(--t-text-3)",
                            letterSpacing: "0.2em",
                            userSelect: "none",
                        }}>
                            •••
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