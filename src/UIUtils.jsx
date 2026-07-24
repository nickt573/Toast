import { useState, useRef, useLayoutEffect } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

export function Tip({ text }) {
  const [visible, setVisible] = useState(false);
  const [tipStyle, setTipStyle] = useState({ top: 0, left: 0, opacity: 0 });
  const badgeRef = useRef(null);
  const tipRef = useRef(null);

  useLayoutEffect(() => {
    if (!visible || !badgeRef.current || !tipRef.current) return;
    const badge = badgeRef.current.getBoundingClientRect();
    const tip = tipRef.current.getBoundingClientRect();
    const GAP = 6;
    const PAD = 8;

    let top = badge.top - tip.height - GAP;
    let left = badge.left + badge.width / 2 - tip.width / 2;

    if (top < PAD) top = badge.bottom + GAP;
    if (left < PAD) left = PAD;
    if (left + tip.width > window.innerWidth - PAD) left = window.innerWidth - tip.width - PAD;

    setTipStyle({ top, left, opacity: 1 });
  }, [visible]);

  return (
    <span
      ref={badgeRef}
      onMouseEnter={() => { setTipStyle(s => ({ ...s, opacity: 0 })); setVisible(true); }}
      onMouseLeave={() => setVisible(false)}
      style={{
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        width: 14, height: 14, borderRadius: "50%", border: "1px solid var(--t-btn-bdr)",
        background: "var(--t-btn)",
        fontSize: 10, color: "var(--t-btn-fg)", cursor: "help", flexShrink: 0,
        marginLeft: 0, userSelect: "none",
      }}
    >
      ?
      {visible && (
        <div ref={tipRef} style={{
          position: "fixed",
          top: tipStyle.top,
          left: tipStyle.left,
          opacity: tipStyle.opacity,
          background: "var(--t-text)",
          color: "var(--t-bg)",
          fontSize: 12,
          padding: "6px 10px",
          borderRadius: "var(--t-r)",
          maxWidth: 240,
          zIndex: 9999,
          pointerEvents: "none",
          lineHeight: 1.45,
          boxShadow: "0 2px 10px rgba(0,0,0,0.20)",
          whiteSpace: "normal",
          wordBreak: "break-word",
        }}>
          {text}
        </div>
      )}
    </span>
  );
}

// Disc and letter live in one SVG so they rasterise as a single unit. A CSS circle and a
// 9px glyph snap to the pixel grid by different rules, and on the WebKit webview that
// disagreement lands a different way depending on where the badge sits, so the letter
// drifts off the disc from one place to the next.
//
// So these are not letters at all. They are the outlines of Atkinson Hyperlegible Next at
// weight 700, lifted out of the font and placed so each one's ink is dead centre on the
// disc. Nothing here is measured, rounded or hinted at draw time, so all three discs read
// the same wherever they land. Regenerate them if the body font ever changes.
const DISC_GLYPHS = {
  A: "M4.002 10.006 6.276 3.994H7.726L9.998 10.006H8.571L8.191 8.889H5.809L5.429 10.006ZM6.164 7.817H7.829L6.993 5.375Z",
  D: "M4.319 10.006V3.994H6.235Q7.004 3.994 7.637 4.155Q8.27 4.316 8.728 4.671Q9.185 5.026 9.433 5.598Q9.681 6.17 9.681 6.998Q9.681 7.84 9.431 8.417Q9.18 8.994 8.717 9.344Q8.254 9.694 7.627 9.85Q6.999 10.006 6.235 10.006ZM5.633 8.9H6.254Q6.699 8.9 7.079 8.822Q7.458 8.744 7.737 8.541Q8.015 8.338 8.171 7.964Q8.327 7.59 8.327 6.998Q8.327 6.408 8.171 6.031Q8.015 5.654 7.733 5.448Q7.451 5.242 7.075 5.163Q6.699 5.085 6.254 5.085H5.633Z",
  N: "M4.313 10.006V3.994H6.017L8.417 8.433V3.994H9.687V10.006H7.984L5.583 5.562V10.006Z",
};

// How far the letters sit off dead centre, in the same units the paths are drawn in.
// DISC_DROP moves all three together, and a letter can take its own sideways nudge on top:
// D has a flat stem down its left and all its weight in the bowl, so centring it by its
// outline leaves it looking a touch left of where the others sit.
const DISC_DROP = 0;
const DISC_SHIFT = { D: 0.15 };

function LetterDisc({ letter, fill, style }) {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" role="img" aria-hidden="true"
      style={{ flexShrink: 0, verticalAlign: "middle", display: "inline-block", ...style }}>
      <circle cx="7" cy="7" r="7" fill={fill} />
      <path d={DISC_GLYPHS[letter]} fill="var(--t-accent-fg)"
        transform={`translate(${DISC_SHIFT[letter] ?? 0} ${DISC_DROP})`} />
    </svg>
  );
}

export function GroupTypeBadge({ type }) {
  const nb = type === "notebook";
  return (
    <LetterDisc
      letter={nb ? "N" : "D"}
      fill={nb ? "color-mix(in srgb, var(--t-plum) 75%, #000)" : "color-mix(in srgb, var(--t-blue) 75%, #000)"}
      style={{ marginLeft: 4 }}
    />
  );
}

// The archived marker on a session row
export function ArchivedBadge() {
  return <LetterDisc letter="A" fill="var(--t-text-3)" />;
}

const URL_PATTERN = /(https?:\/\/[^\s]+|www\.[^\s]+)/gi;

// Renders free text with any pasted links turned into clickable links
export function Linkify({ text }) {
  if (!text) return null;
  return (
    <>
      {text.split(URL_PATTERN).map((part, i) => {
        if (i % 2 === 0) return part;
        // Trailing punctuation reads as sentence punctuation, not part of the link
        const trimmed = part.replace(/[.,;:!?)\]]+$/, "");
        const tail = part.slice(trimmed.length);
        return (
          <span key={i}>
            <a
              href={trimmed}
              className="t-inline-link"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                openUrl(trimmed.startsWith("http") ? trimmed : `https://${trimmed}`);
              }}>
              {trimmed}
            </a>
            {tail}
          </span>
        );
      })}
    </>
  );
}

// Full-info resource card, shared by Stats and the study page.
export function ResourceCard({ res }) {
  const openResource = () => res.url && openUrl(res.url.startsWith("http") ? res.url : `https://${res.url}`);
  return (
    <div className="st-resource-card">
      <div className="st-resource-card-head">
        <span className="st-resource-card-name">{res.name}</span>
        {res.url && <span className="t-open-arrow st-resource-card-url" onClick={openResource}>↗</span>}
        {res.resource_type && <span className="st-resource-card-type">{res.resource_type}</span>}
      </div>
      {res.notes && <div className="st-resource-card-notes">{res.notes}</div>}
    </div>
  );
}

// Full-screen blocker for operations that must not be interrupted (Toast to Go
// transfers). Sits under .toast (z 1000) so error toasts stay readable.
export function BusyOverlay({ title, note }) {
  return (
    <div className="busy-overlay">
      <div className="busy-overlay-card">
        <div className="busy-spinner" />
        <div className="busy-overlay-title">{title}</div>
        {note && <div className="busy-overlay-note">{note}</div>}
      </div>
    </div>
  );
}

// The round "+" in a landing header and its create popup. The parent owns `open` so the
// other header buttons can close it and it can close them, the way those buttons already
// cancel each other.
export function CreateMenu({ open, onToggle, value, onChange, onCreate, title, placeholder }) {
  return (
    <span className="t-add-wrap">
      <button className={`t-add-btn${open ? " open" : ""}`} title={title} aria-label={title}
        onClick={onToggle}>+</button>
      {open && (
        <div className="t-add-menu">
          <div className="t-add-menu-title">{title}</div>
          <input autoFocus value={value} placeholder={placeholder}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") onCreate(); if (e.key === "Escape") onToggle(); }} />
          <div className="t-add-menu-actions">
            <button className="primary" onClick={onCreate}>+ Create</button>
            <button onClick={onToggle}>Cancel</button>
          </div>
        </div>
      )}
    </span>
  );
}

export function ConfirmDelete({ onConfirm, label = "Delete", small = false }) {
  const [confirming, setConfirming] = useState(false);
  const s = small ? { fontSize: 12, padding: "4px 9px" } : {};
  if (confirming) return (
    <span style={{ display: "inline-flex", gap: 4, alignItems: "center", whiteSpace: "nowrap", flexShrink: 0 }}>
      <button className="danger" style={s} onClick={() => { onConfirm(); setConfirming(false); }}>Yes</button>
      <button className="quiet" style={s} onClick={() => setConfirming(false)}>No</button>
    </span>
  );
  return <button className="danger" style={s} onClick={() => setConfirming(true)}>{label}</button>;
}
