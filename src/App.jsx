
import { useState, useEffect, useRef } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask } from "@tauri-apps/plugin-dialog";
import { loggedInvoke, logError } from "./logger";
import "./App.css";

import Toast from "./Toast";
import Plans from "./Plans/Plans";
import Decks from "./Decks/Decks";
import Notebooks from "./Notebooks/Notebooks";
import Homepage from "./Homepage";
import Stats from "./Stats/Stats";

const TABS = [
  { key: "home",      label: "Home",      subtitle: "Today's Dashboard",   color: "var(--t-pink)"   },
  { key: "plans",     label: "Plans",     subtitle: "Study Plans & Todos", color: "#E8935F" }, /* peachy orange — caramel reads gold on the dark nav */
  { key: "decks",     label: "Decks",     subtitle: "SRS Flashcard Decks",     color: "var(--t-blue)"   },
  { key: "notebooks", label: "Notebooks", subtitle: "Notes & Journals",       color: "var(--t-plum)"   },
  { key: "stats",     label: "Stats",     subtitle: "Progress & Streaks",  color: "var(--t-stat)"   },
];

export default function App() {
  const [menu, setMenu] = useState("home");
  const [toast, setToast] = useState({ message: "", type: "info" });
  const [initialDeck, setInitialDeck] = useState(null);
  const [initialNotebook, setInitialNotebook] = useState(null);
  const [returnTo, setReturnTo] = useState(null);
  const [homeReturnContext, setHomeReturnContext] = useState(null);
  const [plansReturnContext, setPlansReturnContext] = useState(null);
  const [statsReturnContext, setStatsReturnContext] = useState(null);
  const [refreshDayCount, setRefreshDayCount] = useState(0);
  const dateChecked = useRef(false);

  function showToast(msg, type = "info") {
    setToast({ message: msg, type });
    setTimeout(() => setToast({ message: "", type: "info" }), 2000);
  }

  async function refreshDay() {
    try {
      await loggedInvoke("update_date");
      setRefreshDayCount(c => c + 1);
      showToast("Day refreshed.");
    } catch (e) {
      logError("refreshDay", e);
      showToast("Failed to refresh day.", "error");
    }
  }

  function navigateToGroup(group, origin) {
    if (origin) setReturnTo(origin);
    if (group.group_type === "deck") {
      setInitialDeck(group);
      setInitialNotebook(null);
      setMenu("decks");
    } else if (group.group_type === "notebook") {
      setInitialNotebook(group);
      setInitialDeck(null);
      setMenu("notebooks");
    }
  }

  function returnToOrigin() {
    if (!returnTo) return;
    if (returnTo.homeContext)  setHomeReturnContext(returnTo.homeContext);
    if (returnTo.plansContext) setPlansReturnContext(returnTo.plansContext);
    if (returnTo.statsContext) setStatsReturnContext(returnTo.statsContext);
    setMenu(returnTo.menu);
    setReturnTo(null);
  }

  function navigate(key) {
    setMenu(key);
    setReturnTo(null);
    setHomeReturnContext(null);
    setPlansReturnContext(null);
    setStatsReturnContext(null);
  }

  // Check GitHub Releases for a newer version on launch; if the user accepts,
  // download, verify signature, install, and relaunch. Silently no-ops in dev.
  useEffect(() => {
    (async () => {
      try {
        const update = await check();
        if (!update) return;
        const yes = await ask(
          `Toast ${update.version} is available (currently ${update.currentVersion}). Update now?`,
          { title: "Update Available", kind: "info", okLabel: "Update", cancelLabel: "Later" }
        );
        if (!yes) return;
        showToast("Downloading update…");
        await update.downloadAndInstall();
        await relaunch();
      } catch (e) {
        logError("updater", e);
      }
    })();
  }, []);

  useEffect(() => {
    loggedInvoke("cleanup_orphaned_media").catch(e => logError("cleanup_orphaned_media", e));
    if (!dateChecked.current) {
      dateChecked.current = true;
      loggedInvoke("update_date").catch(e => logError("update_date", e));
    }
  }, []);

  let menuComp;
  switch (menu) {
    case "home":
      menuComp = (
        <Homepage
          setToast={showToast}
          onNavigateToGroup={navigateToGroup}
          returnContext={homeReturnContext}
          onConsumeReturnContext={() => setHomeReturnContext(null)}
          refreshDayCount={refreshDayCount}
          onRefreshDay={refreshDay}
        />
      );
      break;
    case "plans":
      menuComp = (
        <Plans
          setToast={showToast}
          onNavigateToGroup={navigateToGroup}
          returnContext={plansReturnContext}
          onConsumeReturnContext={() => setPlansReturnContext(null)}
        />
      );
      break;
    case "decks":
      menuComp = (
        <Decks
          setToast={showToast}
          initialDeck={initialDeck}
          onClearInitial={() => setInitialDeck(null)}
          returnTo={returnTo}
          onReturnToOrigin={returnToOrigin}
        />
      );
      break;
    case "notebooks":
      menuComp = (
        <Notebooks
          setToast={showToast}
          initialNotebook={initialNotebook}
          onClearInitial={() => setInitialNotebook(null)}
          returnTo={returnTo}
          onReturnToOrigin={returnToOrigin}
        />
      );
      break;
    case "stats":
      menuComp = (
        <Stats
          setToast={showToast}
          onNavigateToGroup={navigateToGroup}
          returnContext={statsReturnContext}
          onConsumeReturnContext={() => setStatsReturnContext(null)}
        />
      );
      break;
    default:
      menuComp = <div style={{ padding: 16 }}>Error</div>;
  }

  return (
    <div className="app-shell">
      <nav className="app-nav">
        <div className="app-nav-logo" title="Toast">
          <img src="/toast-icon.png" alt="Toast" draggable={false} />
        </div>

        {TABS.map(({ key, label, subtitle, color }) => (
          <button
            key={key}
            className={`app-nav-tab${key === "home" ? " app-nav-tab--home" : ""}${menu === key ? " active" : ""}`}
            style={{ "--tab-underline": color }}
            onClick={() => navigate(key)}
          >
            <span className="app-nav-tab-label">{label}</span>
            <span className="app-nav-tab-sub">{subtitle}</span>
          </button>
        ))}

        <div className="app-nav-spacer" />
      </nav>

      <div className="app-content">{menuComp}</div>
      <Toast message={toast.message} type={toast.type} />
    </div>
  );
}
