import { useState, useEffect, useCallback } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { loggedInvoke, logError } from "../logger";
import { togoLock } from "../togoLock";
import { BusyOverlay } from "../UIUtils";
import { TIMER_STORE_KEY } from "../Homepage";
import "./ToGo.css";

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const DOTS = "••••••••-••••-••••-••••-••••••••••••";

const CLOSE_OPTIONS = [
  { value: "always", label: "Always push",   hint: "Push automatically when Toast closes" },
  { value: "ask",    label: "Ask me",        hint: "Offer a push each time Toast closes" },
  { value: "never",  label: "Never push",         hint: "Only push when I press the button" },
];

// Clipboard API is unreliable in WebKitGTK; fall back to a scratch textarea.
async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(ta);
      return ok;
    } catch {
      return false;
    }
  }
}

function timeAgo(iso) {
  if (!iso) return null;
  const secs = (Date.now() - new Date(iso).getTime()) / 1000;
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  const days = Math.floor(secs / 86400);
  return days === 1 ? "yesterday" : `${days}d ago`;
}

function fmtSize(bytes) {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 ** 2) return `${Math.round(bytes / 1024 ** 2)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

function EyeIcon({ open }) {
  return (
    <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" strokeWidth="2">
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7z" />
      <circle cx="12" cy="12" r="3" />
      {open && <line x1="3" y1="21" x2="21" y2="3" />}
    </svg>
  );
}

// IDs are bearer credentials, masked by default and copyable without revealing.
function MaskedId({ id, onCopied }) {
  const [shown, setShown] = useState(false);

  return (
    <div className="togo-id">
      <code className={`togo-id-value${shown ? "" : " masked"}`}>{shown ? id : DOTS}</code>
      <button
        className="togo-icon-btn"
        onClick={() => setShown(s => !s)}
        aria-label={shown ? "Hide ID" : "Show ID"}
        title={shown ? "Hide ID" : "Show ID"}
      >
        <EyeIcon open={shown} />
      </button>
      <button
        className="togo-icon-btn"
        onClick={async () => onCopied(await copyText(id))}
        aria-label="Copy ID"
        title="Copy ID"
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" strokeWidth="2">
          <rect x="9" y="9" width="12" height="12" rx="2" />
          <path d="M5 15V5a2 2 0 0 1 2-2h10" />
        </svg>
      </button>
    </div>
  );
}

export default function ToGo({ setToast }) {
  const [cfg, setCfg] = useState(null);
  const [pullId, setPullId] = useState("");
  const [pullShown, setPullShown] = useState(false);
  const [busy, setBusy] = useState("");
  const [editingId, setEditingId] = useState(null);
  const [editingLabel, setEditingLabel] = useState("");

  const load = useCallback(async () => {
    try {
      setCfg(await loggedInvoke("get_togo_config"));
    } catch (e) {
      logError("get_togo_config", e);
      setToast("Couldn't load Toast to Go.", "error");
    }
  }, [setToast]);

  useEffect(() => { load(); }, [load]);

  async function push() {
    if (busy) return;
    setBusy("Pushing…");
    togoLock.active = true;
    try {
      await loggedInvoke("push_package");
      await load();
      setToast("Pushed.");
    } catch (e) {
      logError("push_package", e);
      setToast(String(e), "error");
    } finally {
      togoLock.active = false;
      setBusy("");
    }
  }

  async function pull(raw) {
    if (busy) return;
    const id = raw.trim();
    if (!UUID_RE.test(id)) {
      setToast("That doesn't look like a Toast to Go ID.", "error");
      return;
    }

    setBusy("Pulling…");
    togoLock.active = true;
    try {
      const slot = await loggedInvoke("slot_exists", { id });
      if (!slot) {
        setToast("No package found for that ID.", "error");
        return;
      }
      const pushed = timeAgo(slot.uploaded);
      const what = `${fmtSize(slot.size)} package${pushed ? ` pushed ${pushed}` : ""}`;

      const ok = await ask(
        `This erases everything in this copy of Toast (every deck, note, plan, and stat) and replaces it with the ${what} at that ID. This cannot be undone.`,
        { title: "Replace all local data?", kind: "warning", okLabel: "Replace everything", cancelLabel: "Cancel" }
      );
      if (!ok) return;

      await loggedInvoke("pull_package", { id });
      // Study timers are keyed by plan id; the pulled db has its own.
      try { localStorage.removeItem(TIMER_STORE_KEY); } catch { /* non-fatal */ }
      // Reload, not relaunch(): the Rust side already swapped the db, and a
      // process restart kills the vite server under `tauri dev`.
      window.location.reload();
      return;
    } catch (e) {
      logError("pull_package", e);
      setToast(String(e), "error");
    } finally {
      togoLock.active = false;
      setBusy("");
    }
  }

  async function setClose(value) {
    try {
      setCfg(await loggedInvoke("set_close_behavior", { behavior: value }));
    } catch (e) {
      logError("set_close_behavior", e);
      setToast("Couldn't save that setting.", "error");
    }
  }

  function startRename(r) {
    setEditingId(r.id);
    setEditingLabel(r.label ?? "");
  }

  async function confirmRename(id) {
    setEditingId(null);
    try {
      setCfg(await loggedInvoke("label_recent_pull", { id, label: editingLabel }));
    } catch (e) {
      logError("label_recent_pull", e);
    }
  }

  async function forget(id) {
    try {
      setCfg(await loggedInvoke("forget_recent_pull", { id }));
    } catch (e) {
      logError("forget_recent_pull", e);
    }
  }

  const copied = ok => setToast(ok ? "ID copied." : "Couldn't copy.", ok ? "info" : "error");

  if (!cfg) return null;

  return (
    <div className="togo-root">
      {busy && (
        <BusyOverlay
          title={busy === "Pulling…" ? "Pulling from Toast to Go…" : "Pushing to Toast to Go…"}
          note="This can take a bit, hang tight. Please don't shut down your computer."
        />
      )}
      <header className="togo-header">
        <h2>Toast to Go</h2>
      </header>

      <div className="togo-body">
        <div className="togo-inner">
          <section className="togo-card">
            <h3>Push</h3>
            <p className="togo-blurb">
              Push saves everything (decks, notebooks, plans, and stats) to the ID below. 
              Keep it safe!
            </p>
            <MaskedId id={cfg.instance_id} onCopied={copied} />
            <div className="togo-row">
              <button className="primary" onClick={push} disabled={!!busy}>
                {busy === "Pushing…" ? busy : "Push"}
              </button>
              <span className="togo-meta">
                {cfg.last_push ? `Last pushed ${timeAgo(cfg.last_push)}` : "Never pushed"}
              </span>
            </div>
          </section>

          <section className="togo-card">
            <h3>Pull</h3>
            <p className="togo-blurb">
              Paste an ID from another machine to bring its data here.
            </p>
            <div className="togo-id">
              <input
                className="togo-input"
                type={pullShown ? "text" : "password"}
                placeholder="Toast to Go ID"
                value={pullId}
                spellCheck={false}
                autoComplete="off"
                onChange={e => setPullId(e.target.value)}
                onKeyDown={e => e.key === "Enter" && pull(pullId)}
              />
              <button
                className="togo-icon-btn"
                onClick={() => setPullShown(s => !s)}
                aria-label={pullShown ? "Hide ID" : "Show ID"}
                title={pullShown ? "Hide ID" : "Show ID"}
              >
                <EyeIcon open={pullShown} />
              </button>
              <button className="danger" onClick={() => pull(pullId)} disabled={!!busy || !pullId}>
                {busy === "Pulling…" ? busy : "Pull"}
              </button>
            </div>
            <p className="togo-note togo-warn">
              Pulling replaces everything in this copy of Toast, so is highly suggested that you push before pulling.
            </p>
          </section>

          <section className="togo-card">
            <h3>Recent IDs</h3>
            {cfg.recent_pulls.length === 0 ? (
              <p className="togo-empty">IDs you pull from are remembered here.</p>
            ) : (
              <ul className="togo-recents">
                {cfg.recent_pulls.map(r => (
                  <li key={r.id}>
                    {editingId === r.id ? (
                      <input
                        className="togo-label-input"
                        value={editingLabel}
                        autoFocus
                        placeholder="Name this ID"
                        onChange={e => setEditingLabel(e.target.value)}
                        onKeyDown={e => {
                          if (e.key === "Enter") confirmRename(r.id);
                          if (e.key === "Escape") setEditingId(null);
                        }}
                        onBlur={() => confirmRename(r.id)}
                      />
                    ) : (
                      <button className="togo-label" onClick={() => startRename(r)} title="Rename">
                        {r.label || "Unnamed"}
                      </button>
                    )}
                    <MaskedId id={r.id} onCopied={copied} />
                    <span className="togo-meta">pulled {timeAgo(r.pulled_at)}</span>
                    <button className="danger togo-btn-sm" onClick={() => pull(r.id)} disabled={!!busy}>
                      Pull
                    </button>
                    <button
                      className="togo-icon-btn"
                      onClick={() => forget(r.id)}
                      aria-label="Forget this ID"
                      title="Forget"
                    >
                      ×
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="togo-card">
            <h3>When Toast closes...</h3>
            <div className="togo-choices">
              {CLOSE_OPTIONS.map(o => (
                <button
                  key={o.value}
                  className={`togo-choice${cfg.close_behavior === o.value ? " active" : ""}`}
                  onClick={() => setClose(o.value)}
                >
                  <span className="togo-choice-label">{o.label}</span>
                  <span className="togo-choice-hint">{o.hint}</span>
                </button>
              ))}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
