import { appDataDir } from "@tauri-apps/api/path";
import { convertFileSrc } from "@tauri-apps/api/core";

// Media paths are stored relative to the app data dir ("cards/images/x.png").
// mediaSrc must stay synchronous (it runs inside render paths), so the app
// dir is fetched once before the first render, see main.jsx.

let appDir = "";

export async function initMediaPaths() {
    const d = await appDataDir();
    appDir = /[\\/]$/.test(d) ? d : d + "/";
}

const isUrl = (p) => /^(https?:|asset:|data:|blob:)/i.test(p);
const isAbsolute = (p) => p.startsWith("/") || /^[A-Za-z]:[\\/]/.test(p);

// Converts a stored media path (or a freshly picked absolute file) to a URL
// the webview can load. Absolute paths still resolve directly so databases
// that predate relative storage keep working.
export function mediaSrc(p) {
    if (!p || isUrl(p)) return p;
    if (isAbsolute(p)) return convertFileSrc(p);
    return convertFileSrc(appDir + p);
}
