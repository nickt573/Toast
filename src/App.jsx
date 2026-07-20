
import { useState, useEffect, useRef } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { loggedInvoke, logError } from "./logger";
import { togoLock } from "./togoLock";
import { BusyOverlay } from "./UIUtils";
import "./App.css";

import Toast from "./Toast";
import Plans from "./Plans/Plans";
import Decks from "./Decks/Decks";
import Notebooks from "./Notebooks/Notebooks";
import Homepage from "./Homepage";
import Stats from "./Stats/Stats";
import ToGo from "./ToGo/ToGo";
import HowTo, { HELP_PAGES } from "./HowTo";

const TABS = [
  { key: "home",      label: "Home",      subtitle: "Today's Dashboard",   color: "var(--t-pink)"   },
  { key: "plans",     label: "Plans",     subtitle: "Study Plans & Todos", color: "#E8935F" }, /* peachy orange, caramel reads gold on the dark nav */
  { key: "decks",     label: "Decks",     subtitle: "SRS Flashcard Decks",     color: "var(--t-blue)"   },
  { key: "notebooks", label: "Notebooks", subtitle: "Notes & Journals",       color: "var(--t-plum)"   },
  { key: "stats",     label: "Stats",     subtitle: "Progress & Streaks",  color: "var(--t-stat)"   },
  { key: "togo",      label: "Toast to Go", subtitle: "Study Anywhere",    color: "var(--t-silver)" },
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
  const [dateReady, setDateReady] = useState(false);
  const [closePush, setClosePush] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [helpPage, setHelpPage] = useState(0);
  const [firstLaunch, setFirstLaunch] = useState(false);
  const dateChecked = useRef(false);

  function openHelp() {
    setHelpPage(0);
    setHelpOpen(true);
  }

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

  // Check GitHub Releases for a newer version on launch. If the user accepts,
  // download, verify signature, install, and relaunch. Silently no-ops in dev.
  useEffect(() => {
    (async () => {
      try {
        const update = await check();
        if (!update) return;
        const yes = await ask(
          `Toast ${update.version} is available (currently ${update.currentVersion}). Changes can be found in the release notes on GitHub. Update now?
          Note: A newer push to Toast to Go cannot be pulled on an older version.`,
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

  // Toast to Go: push on close, per the user's setting. A failed push must never
  // trap the user in a window that won't shut, log it and close anyway.
  useEffect(() => {
    let unlisten;
    let gone = false;
    (async () => {
      const win = getCurrentWindow();
      const fn = await win.onCloseRequested(async (event) => {
        // A push or pull is already in flight (here or in the ToGo tab):
        // swallow the repeat close so it can't stack dialogs or double-push.
        if (togoLock.active) return event.preventDefault();

        let behavior = "ask";
        try {
          ({ close_behavior: behavior } = await loggedInvoke("get_togo_config"));
        } catch (e) {
          logError("get_togo_config on close", e);
          return;
        }
        if (behavior === "never") return;

        event.preventDefault();
        togoLock.active = true;
        try {
          if (behavior === "ask") {
            const yes = await ask("Push your changes to Toast to Go before closing?", {
              title: "Toast to Go",
              kind: "info",
              okLabel: "Push",
              cancelLabel: "Close without pushing",
            });
            if (!yes) return await win.destroy();
          }
          setClosePush(true);
          try {
            await loggedInvoke("push_package", { force: true });
          } catch (e) {
            logError("push on close", e);
          }
          await win.destroy(); // not close(): that re-fires this handler
        } finally {
          togoLock.active = false;
          setClosePush(false);
        }
      });
      // registration is async, cleanup may have already run
      if (gone) fn();
      else unlisten = fn;
    })();
    return () => { gone = true; unlisten?.(); };
  }, []);

  // Roll the day over before anything renders: child effects run before parent
  // effects, so a page mounted alongside this one would read the pre-rollover date.
  useEffect(() => {
    loggedInvoke("cleanup_orphaned_media").catch(e => logError("cleanup_orphaned_media", e));
    if (dateChecked.current) return;
    dateChecked.current = true;
    (async () => {
      let firstLaunch = false;
      try {
        // Must run before update_date, which inserts the app_date row
        firstLaunch = await loggedInvoke("is_first_launch");
      } catch (e) {
        logError("is_first_launch", e);
      }
      try {
        await loggedInvoke("update_date");
      } catch (e) {
        logError("update_date", e);
      } finally {
        setDateReady(true);
        if (firstLaunch) {
          setFirstLaunch(true);
          openHelp();
        }
      }
    })();
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
          onOpenHelp={openHelp}
          firstLaunch={firstLaunch}
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
    case "togo":
      menuComp = <ToGo setToast={showToast} />;
      break;
    default:
      menuComp = <div style={{ padding: 16 }}>Error</div>;
  }

  const helpTab = helpOpen ? HELP_PAGES[helpPage]?.tab : null;
  const navTab = ({ key, label, subtitle, color }) => (
    <button
      key={key}
      className={`app-nav-tab${key === "home" ? " app-nav-tab--home" : ""}${menu === key ? " active" : ""}${helpTab === key ? " help-spot" : ""}`}
      style={{ "--tab-underline": color }}
      onClick={() => navigate(key)}
    >
      <span className="app-nav-tab-label">{label}</span>
      <span className="app-nav-tab-sub">{subtitle}</span>
    </button>
  );

  return (
    <div className="app-shell">
      <nav className="app-nav">
        <div className="app-nav-logo" title="Toast">
          <img src="/toast-icon.png" alt="Toast" draggable={false} />
        </div>

        {navTab(TABS[0])}
        <div className="app-nav-spacer" />
        <div className="app-nav-center">{TABS.slice(1).map(navTab)}</div>
        <div className="app-nav-spacer app-nav-spacer--right" />
      </nav>

      <div className="app-content">{dateReady ? menuComp : null}</div>
      {helpOpen && (
        <HowTo page={helpPage} setPage={setHelpPage} onClose={() => setHelpOpen(false)} />
      )}
      {closePush && (
        <BusyOverlay
          title="Pushing to Toast to Go…"
          note="Toast will close when this finishes. Please don't shut down your computer."
        />
      )}
      <Toast message={toast.message} type={toast.type} />
    </div>
  );
}
