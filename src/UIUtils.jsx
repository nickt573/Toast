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
          borderRadius: "var(--t-r-lg)",
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

// Small deck/notebook type badge shown inside group pills (kin to the "?" Tip badge).
export function GroupTypeBadge({ type }) {
  return (
    <span style={{
      marginLeft: 4, fontSize: 9, fontWeight: 700, padding: "0 5px",
      borderRadius: "var(--t-r-pill)", border: "1px solid currentColor",
      opacity: 0.6, lineHeight: 1.8, letterSpacing: "0.03em",
    }}>
      {type === "notebook" ? "nb" : "dk"}
    </span>
  );
}

// Full-info resource card (name / type / url / notes) — shared by Stats and the study page.
export function ResourceCard({ res }) {
  const openResource = () => res.url && openUrl(res.url.startsWith("http") ? res.url : `https://${res.url}`);
  return (
    <div className="st-resource-card">
      <div className="st-resource-card-head">
        <span className="st-resource-card-name">{res.name}</span>
        {res.resource_type && <span className="st-resource-card-type">{res.resource_type}</span>}
      </div>
      {res.url && (
        <span className="st-resource-card-url" onClick={openResource}>
          Link<span style={{ marginLeft: 3, fontSize: 9 }}>↗</span>
        </span>
      )}
      {res.notes && <div className="st-resource-card-notes">{res.notes}</div>}
    </div>
  );
}

export function ConfirmDelete({ onConfirm, label = "Delete", small = false }) {
  const [confirming, setConfirming] = useState(false);
  const s = small ? { fontSize: 12, padding: "2px 7px" } : {};
  if (confirming) return (
    <span style={{ display: "inline-flex", gap: 4, alignItems: "center", whiteSpace: "nowrap", flexShrink: 0 }}>
      <button className="danger" style={s} onClick={() => { onConfirm(); setConfirming(false); }}>Yes</button>
      <button className="quiet" style={s} onClick={() => setConfirming(false)}>No</button>
    </span>
  );
  return <button className="danger" style={s} onClick={() => setConfirming(true)}>{label}</button>;
}
