import { useState, useRef, useEffect, useLayoutEffect } from "react";
import { createPortal } from "react-dom";
import { openUrl } from "@tauri-apps/plugin-opener";
import { loggedInvoke, logError } from "./logger";

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

// Disc and letter live in one SVG so they rasterise as a single unit, since a CSS circle
// and a small glyph snap to the pixel grid by different rules and the letter drifts
// These are glyph outlines lifted out of the body font and placed dead centre on the disc,
// so nothing is measured or hinted at draw time, and they need regenerating if it changes
const DISC_GLYPHS = {
  A: "M4.002 10.006 6.276 3.994H7.726L9.998 10.006H8.571L8.191 8.889H5.809L5.429 10.006ZM6.164 7.817H7.829L6.993 5.375Z",
  D: "M4.319 10.006V3.994H6.235Q7.004 3.994 7.637 4.155Q8.27 4.316 8.728 4.671Q9.185 5.026 9.433 5.598Q9.681 6.17 9.681 6.998Q9.681 7.84 9.431 8.417Q9.18 8.994 8.717 9.344Q8.254 9.694 7.627 9.85Q6.999 10.006 6.235 10.006ZM5.633 8.9H6.254Q6.699 8.9 7.079 8.822Q7.458 8.744 7.737 8.541Q8.015 8.338 8.171 7.964Q8.327 7.59 8.327 6.998Q8.327 6.408 8.171 6.031Q8.015 5.654 7.733 5.448Q7.451 5.242 7.075 5.163Q6.699 5.085 6.254 5.085H5.633Z",
  N: "M4.313 10.006V3.994H6.017L8.417 8.433V3.994H9.687V10.006H7.984L5.583 5.562V10.006Z",
  M: "M3.68 10.006V3.994H5.683L7.0 8.418L8.331 3.994H10.32V10.006H9.036V5.205L7.547 10.006H6.44L4.957 5.205V10.006Z",
};

// How far the letters sit off dead centre, in the same units the paths are drawn in, where
// DISC_DROP moves all three together and a letter can take its own sideways nudge on top
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

const DECK_STATE = {
  merged:   { letter: "M", fill: "var(--t-plum)",   title: "Merged into another deck" },
  deleted:  { letter: "D", fill: "var(--t-red-2)",  title: "Deck deleted" },
  archived: { letter: "A", fill: "var(--t-text-3)", title: "Every session in this deck is archived, so none of it counts toward your totals" },
};

// The state of a whole deck's stats, shown as a colored disc beside its name
export function DeckStateBadge({ state }) {
  const s = DECK_STATE[state];
  if (!s) return null;
  return (
    <span title={s.title} style={{ display: "inline-flex", flexShrink: 0 }}>
      <LetterDisc letter={s.letter} fill={s.fill} />
    </span>
  );
}

const URL_PATTERN = /(https?:\/\/[^\s]+|www\.[^\s]+)/gi;

// Renders free text with any pasted links turned into clickable links
export function Linkify({ text }) {
  if (!text) return null;
  return (
    <>
      {text.split(URL_PATTERN).map((part, i) => {
        if (i % 2 === 0) return part;
        // Trailing punctuation reads as sentence punctuation, not as part of the link
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

// Full-info resource card, shared by Stats and the study page
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

// One striped bar for an entry's resources, decks and notebooks, where the family shows in
// the stripe alone and the arrow appears only while the item is still reachable. A tagged
// notebook page rides along as a small number, and the arrow then opens straight to it
export function ItemBar({ name, family, url, onOpen, pageNumber = null, dead = false }) {
  const clickable = !!url || !!onOpen;
  const open = (e) => {
    e.stopPropagation();
    if (url) openUrl(url.startsWith("http") ? url : `https://${url}`);
    else onOpen?.();
  };
  return (
    <div className={`st-item-bar st-item-bar--${family}${dead ? " st-item-bar--dead" : ""}`}>
      <span className="st-item-bar-name">{name}</span>
      {clickable && <span className="t-open-arrow st-item-bar-arrow" onClick={open}>↗</span>}
      {pageNumber != null && <span className="nbtag-badge">p.{pageNumber}</span>}
    </div>
  );
}

// The lowest a portaled picker menu may open, just under the app's top nav. A menu whose trigger
// has scrolled up behind the bar returns null so the caller drops it rather than let it ride over
// the nav, and it opens again once the trigger scrolls back into view
function navClipTop() {
  const nav = document.querySelector(".app-nav");
  return nav ? nav.getBoundingClientRect().bottom : 0;
}

// Picker for tagging one page of a notebook on a logged todo, shared by the three logging
// flows. Pages are numbered by the same order the notebook shows them, so the number a user
// picks is the number they see. Empty page_id means no tag
export function NotebookPageTag({ notebookId, pageId, onChange }) {
  const [pages, setPages] = useState([]);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [pos, setPos] = useState(null);
  const wrapRef = useRef(null);
  const menuRef = useRef(null);

  useEffect(() => {
    let alive = true;
    loggedInvoke("get_pages", { notebookId })
      .then(rows => { if (alive) setPages(rows); })
      .catch(e => logError("get_pages", e));
    return () => { alive = false; };
  }, [notebookId]);

  // Pin the menu just under the trigger's on-screen spot. The 264 matches the menu width, so it
  // shifts left when the trigger sits too near the right edge to open flush
  const place = () => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const r = wrap.getBoundingClientRect();
    const top = r.bottom + 6;
    if (top < navClipTop()) { setPos(null); return; }
    const left = Math.max(8, Math.min(r.left, window.innerWidth - 264 - 8));
    setPos({ top, left });
  };

  useLayoutEffect(() => { if (open) place(); }, [open]);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e) => {
      if (wrapRef.current && wrapRef.current.contains(e.target)) return;
      if (menuRef.current && menuRef.current.contains(e.target)) return;
      setOpen(false);
    };
    // Freeze the page behind the menu while it is open, so an open menu can never travel up into
    // a header as its trigger scrolls. Scrolling inside the menu's own list still passes through
    const blockScroll = (e) => {
      if (menuRef.current && menuRef.current.contains(e.target)) return;
      e.preventDefault();
    };
    // A scroll that slips past the block anyway re-pins the menu to its trigger instead of
    // leaving it stranded
    const onScroll = (e) => {
      if (menuRef.current && menuRef.current.contains(e.target)) return;
      place();
    };
    document.addEventListener("mousedown", onDoc);
    window.addEventListener("wheel", blockScroll, { passive: false, capture: true });
    window.addEventListener("touchmove", blockScroll, { passive: false, capture: true });
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", place);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      window.removeEventListener("wheel", blockScroll, { capture: true });
      window.removeEventListener("touchmove", blockScroll, { capture: true });
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", place);
    };
  }, [open]);

  const idx = pages.findIndex(p => p.id === pageId);
  const current = idx !== -1 ? pages[idx] : null;
  const q = query.trim().toLowerCase();
  const filtered = q ? pages.filter(p => (p.title || "").toLowerCase().includes(q)) : pages;

  return (
    <div className="unit-picker page-tag-picker" ref={wrapRef}>
      <div className="unit-picker-control">
        <button type="button" className={`unit-picker-select${current ? " has-unit" : ""}`}
          onClick={() => setOpen(o => !o)}>
          <span className="unit-picker-select-label">
            {current ? `p.${idx + 1} ${current.title || "Untitled"}` : "Tag a page"}
          </span>
          {current
            ? <span className="unit-picker-clear" title="Clear page"
                onClick={(e) => { e.stopPropagation(); onChange(null); }}>×</span>
            : <span className="unit-picker-caret">▾</span>}
        </button>
      </div>

      {open && pos && createPortal(
        <div className="unit-picker-menu page-tag-menu" ref={menuRef} style={{ top: pos.top, left: pos.left }}>
          {pages.length > 8 && (
            <input type="text" className="unit-picker-search" autoFocus placeholder="Find a page"
              value={query} onChange={e => setQuery(e.target.value)} />
          )}
          <div className="unit-picker-list">
            {filtered.length === 0 && <div className="unit-picker-hint">No pages</div>}
            {filtered.map(p => (
              <div key={p.id} className={`unit-picker-row${p.id === pageId ? " active" : ""}`}>
                <button type="button" className={`unit-picker-opt${p.id === pageId ? " active" : ""}`}
                  onClick={() => { onChange(p.id); setOpen(false); setQuery(""); }}>
                  <span className="unit-picker-count">p.{pages.indexOf(p) + 1}</span>
                  <span className="unit-picker-optname">{p.title || "Untitled"}</span>
                </button>
              </div>
            ))}
          </div>
        </div>,
        document.body)}
    </div>
  );
}

// Single-choice dropdown that wears the same chrome as the unit and page pickers, so every menu
// in the app matches instead of falling back to a native select. Pass a flat `options` list or
// named `groups`, each option being { value, label }. `emptyOption` adds a leading row for the
// unselected choice, and the trigger falls back to `placeholder` when nothing is picked
export function SelectMenu({
  value, onChange, options, groups, placeholder = "Select…", emptyOption,
  menuWidth = 264, searchThreshold = 8, searchPlaceholder = "Search",
  emptyHint = "Nothing to pick", className = "",
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [pos, setPos] = useState(null);
  const wrapRef = useRef(null);
  const menuRef = useRef(null);

  const groupList = groups ?? (options ? [{ label: null, options }] : []);
  const allOptions = groupList.flatMap(g => g.options);
  const selected = allOptions.find(o => o.value === value) || null;
  const atEmpty = emptyOption && value === emptyOption.value;

  // Pin the menu just under the trigger's on-screen spot, shifting left when the trigger sits
  // too near the right edge to open flush, exactly as the page tag menu does
  const place = () => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const r = wrap.getBoundingClientRect();
    const top = r.bottom + 6;
    if (top < navClipTop()) { setPos(null); return; }
    const left = Math.max(8, Math.min(r.left, window.innerWidth - menuWidth - 8));
    setPos({ top, left });
  };
  useLayoutEffect(() => { if (open) place(); }, [open]);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e) => {
      if (wrapRef.current && wrapRef.current.contains(e.target)) return;
      if (menuRef.current && menuRef.current.contains(e.target)) return;
      setOpen(false);
    };
    // Freeze the page behind the menu while it is open, so an open menu can never travel up into
    // a header as its trigger scrolls. Scrolling inside the menu's own list still passes through
    const blockScroll = (e) => {
      if (menuRef.current && menuRef.current.contains(e.target)) return;
      e.preventDefault();
    };
    // A scroll that slips past the block anyway re-pins the menu to its trigger instead of
    // leaving it stranded
    const onScroll = (e) => {
      if (menuRef.current && menuRef.current.contains(e.target)) return;
      place();
    };
    document.addEventListener("mousedown", onDoc);
    window.addEventListener("wheel", blockScroll, { passive: false, capture: true });
    window.addEventListener("touchmove", blockScroll, { passive: false, capture: true });
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", place);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      window.removeEventListener("wheel", blockScroll, { capture: true });
      window.removeEventListener("touchmove", blockScroll, { capture: true });
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", place);
    };
  }, [open]);

  const q = query.trim().toLowerCase();
  const shownGroups = q
    ? groupList
        .map(g => ({ ...g, options: g.options.filter(o => String(o.label).toLowerCase().includes(q)) }))
        .filter(g => g.options.length > 0)
    : groupList;

  const choose = (v) => { onChange(v); setOpen(false); setQuery(""); };
  const triggerLabel = selected ? selected.label : (emptyOption ? emptyOption.label : placeholder);

  return (
    <div className={`unit-picker page-tag-picker${className ? " " + className : ""}`} ref={wrapRef}>
      <div className="unit-picker-control">
        <button type="button" className={`unit-picker-select${selected ? " has-unit" : ""}`}
          onClick={() => setOpen(o => !o)}>
          <span className="unit-picker-select-label">{triggerLabel}</span>
          <span className="unit-picker-caret">▾</span>
        </button>
      </div>

      {open && pos && createPortal(
        <div className="unit-picker-menu page-tag-menu" ref={menuRef} style={{ top: pos.top, left: pos.left, width: menuWidth }}>
          {allOptions.length > searchThreshold && (
            <input type="text" className="unit-picker-search" autoFocus placeholder={searchPlaceholder}
              value={query} onChange={e => setQuery(e.target.value)} />
          )}
          <div className="unit-picker-list">
            {emptyOption && !q && (
              <div className={`unit-picker-row${atEmpty ? " active" : ""}`}>
                <button type="button" className={`unit-picker-opt${atEmpty ? " active" : ""}`}
                  onClick={() => choose(emptyOption.value)}>
                  <span className="unit-picker-optname">{emptyOption.label}</span>
                </button>
              </div>
            )}
            {shownGroups.map((g, gi) => (
              <div key={g.label ?? gi} className="unit-picker-optgroup">
                {g.label != null && <div className="unit-picker-grouphead">{g.label}</div>}
                {g.options.map(o => (
                  <div key={o.value} className={`unit-picker-row${o.value === value ? " active" : ""}`}>
                    <button type="button" className={`unit-picker-opt${o.value === value ? " active" : ""}`}
                      onClick={() => choose(o.value)}>
                      <span className="unit-picker-optname">{o.label}</span>
                    </button>
                  </div>
                ))}
              </div>
            ))}
            {allOptions.length === 0 && <div className="unit-picker-hint">{emptyHint}</div>}
            {allOptions.length > 0 && shownGroups.length === 0 && <div className="unit-picker-hint">No matches</div>}
          </div>
        </div>,
        document.body)}
    </div>
  );
}

// Full-screen blocker for operations that must not be interrupted, sitting under the toast
// layer so error toasts stay readable
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

// The round add button in a landing header and its create popup, where the parent owns the
// open state so this and the other header buttons can cancel each other
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
