import { invoke } from "@tauri-apps/api/core";
import { error as writeToLog } from "@tauri-apps/plugin-log";

const MAX = 30;
const crumbs = [];

export async function loggedInvoke(command, args) {
    crumbs.push({ ts: Date.now(), command });
    if (crumbs.length > MAX) crumbs.shift();
    return invoke(command, args);
}

export async function logError(context, error) {
    const trail = crumbs
        .map(b => `${new Date(b.ts).toISOString()}  ${b.command}`)
        .join("\n");
    const msg = `[${context}] ${error}\n\nRecent actions:\n${trail}`;
    console.error(msg);
    try { await writeToLog(msg); } catch { /* plugin unavailable in pure web dev mode */ }
}
