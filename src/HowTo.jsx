import { useEffect } from "react";
import "./HowTo.css";

export const HELP_PAGES = [
    {
        tab: null,
        title: "Welcome to Toast!",
        body: "Toast is a home for managing and tracking your language learning. Set up plans to work through each day, and Toast keeps track of the rest.",
    },
    {
        tab: "plans",
        title: "Plans",
        body: "A plan is one subject or goal, like a language or a class. Fill it with todos that repeat on the days you pick, each with categories describing the language skills it targets (like reading or listening). Link your decks to a plan for daily SRS review, and add outside resources like books and websites. You can also tag a todo with the decks, notebooks, and resources it uses, so they're always one click away.",
    },
    {
        tab: "decks",
        title: "Decks",
        body: "Decks hold your flashcards. Make your own cards with images and audio, or import your favorite Anki decks. Link a deck to a plan and set how many new cards and reviews you want per day. Adaptive ease makes sure the cards you find hard come back sooner than the cards you find easy.",
    },
    {
        tab: "notebooks",
        title: "Notebooks",
        body: "Notebooks are for writing things down. Keep pages of notes with images, record audio right onto a page, and format your pages however you'd like. Tag a notebook on a todo to refer to it while you study.",
    },
    {
        tab: "stats",
        title: "Stats",
        body: "Stats shows your study statistics plan by plan. Charts break down your card results and where your hours went, and the summaries track retention, cards learned, todos done, study time, and your streaks. Every deck session and completed todo is logged at the bottom, where you can edit or delete entries. Keep in mind stats persist even if the data they relate to is deleted.",
    },
    {
        tab: "togo",
        title: "Toast to Go",
        body: "Toast to Go carries your data between computers. Push a copy when you leave one machine, then pull it down on another using your key. You can save the keys you pull from often, and choose whether Toast offers to push whenever you close the app. Keep in mind that you cannot pull packages with a different version number or with a date from the future.",
    },
    {
        tab: "home",
        title: "Home",
        body: "The Home page brings it all together. Each day your plans show how many todos and cards are due, and opening one lets you check todos off, study your decks, and time your sessions. Anything extra you did can be logged too. Happy studying!",
    },
];

export default function HowTo({ page, setPage, onClose }) {
    const p = HELP_PAGES[page];
    const last = page === HELP_PAGES.length - 1;

    useEffect(() => {
        function onKey(e) {
            if (e.key === "ArrowRight" && !last) setPage(page + 1);
            if (e.key === "ArrowLeft" && page > 0) setPage(page - 1);
            if (e.key === "Escape") onClose();
        }
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [page, last]);

    return (
        <div className="howto-overlay">
            <div className="howto-card">
                <button className="howto-close" onClick={onClose} title="Close">×</button>
                <div className="howto-title">{p.title}</div>
                <div className="howto-body">{p.body}</div>
                <div className="howto-nav">
                    <button className="howto-arrow" onClick={() => setPage(page - 1)} disabled={page === 0}>‹</button>
                    <span className="howto-count">{page + 1} / {HELP_PAGES.length}</span>
                    {last
                        ? <button className="primary howto-done" onClick={onClose}>Done</button>
                        : <button className="howto-arrow" onClick={() => setPage(page + 1)}>›</button>}
                </div>
            </div>
        </div>
    );
}
