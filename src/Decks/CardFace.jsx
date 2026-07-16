import { useState, useEffect, useRef } from "react";
import { mediaSrc } from "../mediaPaths";
import { loggedInvoke, logError } from "../logger";
import { openUrl } from "@tauri-apps/plugin-opener";

// Helpers

export function rewriteAnkiSrcs(html) {
    if (!html) return "";
    return html
        .replace(/src=["']([^"']+)["']/g, (match, src) => {
            if (src.startsWith("http") || src.startsWith("asset://")) return match;
            try { return `src="${mediaSrc(src)}"`; } catch { return match; }
        })
        .replace(/<img\b([^>]*?)>/gi, (_, attrs) => {
            const cleaned = attrs
                .replace(/\s*width=["'][^"']*["']/gi, "")
                .replace(/\s*height=["'][^"']*["']/gi, "")
                .replace(/\s*style=["'][^"']*["']/gi, "");
            return `<img${cleaned} style="max-width:100%;max-height:280px;object-fit:contain;border-radius:6px;display:block;margin:10px auto;">`;
        });
}

export function cleanAnkiHtml(html) {
    if (!html) return "";
    return html
        .replace(/\{\{[^}]*\}\}/g, "")
        .replace(/<div[^>]*>\s*<\/div>/gi, "")
        .replace(/<span>\s*<\/span>/gi, "")
        // Anki wraps each line in a div. Every div boundary becomes a comma separator.
        .replace(/<div[^>]*>/gi, ", ")
        .replace(/<\/div>/gi, "")
        .replace(/(\s*<br\s*\/?>\s*){3,}/gi, "<br/><br/>")
        .replace(/(\s*,){2,}\s*/g, ", ")
        .replace(/(<(?:br|hr)[^>]*>\s*)(,\s*)+/gi, "$1")
        .replace(/^\s*(,\s*)+/, "")
        // Empty fields joined during import leave stray dividers: collapse
        // consecutive <hr>s and drop any at the very start or end
        .replace(/(?:<hr[^>]*>\s*){2,}/gi, "<hr/>")
        .replace(/^(?:\s*<hr[^>]*>)+/i, "")
        .replace(/(?:<hr[^>]*>\s*)+$/i, "")
        // Field separators show as a vertical gap, not a visible bar
        .replace(/<hr[^>]*>/gi, '<div style="height:10px"></div>')
        .trim();
}

export function renderAnkiHtml(html) {
    return cleanAnkiHtml(rewriteAnkiSrcs(html));
}

// Normalize so search queries and card text always compare equal.
// Strips invisible chars, straightens curly quotes, collapses all whitespace (including &nbsp;).
export function normalizeSearchText(str) {
    if (!str) return "";
    return str
        .replace(/[\u00AD\u200B-\u200D\uFEFF]/g, "")
        .replace(/[\u2018\u2019]/g, "'")
        .replace(/[\u201C\u201D]/g, '"')
        .replace(/\s+/g, " ")
        .trim();
}

export function stripHtml(str) {
    if (!str) return "";
    const stripped = str
        .replace(/<(?:hr|br)[^>]*>/gi, ", ")
        .replace(/<div[^>]*>/gi, ", ")
        .replace(/<[^>]*>/g, "");
    const txt = document.createElement("textarea");
    txt.innerHTML = stripped;
    return normalizeSearchText(txt.value)
        .replace(/(\s*,\s*){2,}/g, ", ")
        .replace(/^\s*(,\s*)+/, "")
        .replace(/(,\s*)+$/, "")
        .trim();
}

export function LinkifiedText({ text }) {
    const urlRegex = /(https?:\/\/[^\s<>"]+|(?<!\w)(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}(?:\/[^\s<>"]*)?)/g;
    const linked = (text ?? "").replace(urlRegex, (url) => {
        const href = url.startsWith("http") ? url : `https://${url}`;
        return `<a href="${href}" rel="noopener noreferrer">${url}</a>`;
    });
    const handleClick = (e) => {
        const a = e.target.closest("a");
        if (a) { e.preventDefault(); openUrl(a.href); }
    };
    return (
        <div
            style={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}
            dangerouslySetInnerHTML={{ __html: linked }}
            onClick={handleClick}
        />
    );
}

// Audio helpers

export function extractRawAudioSrcs(html) {
    if (!html) return [];
    const srcs = [];
    const re = /<audio\b([^>]*)>/gi;
    let m;
    while ((m = re.exec(html)) !== null) {
        const s = /src=["']([^"']+)["']/i.exec(m[1]);
        if (s) srcs.push(s[1]);
    }
    return srcs;
}

export function stripAudioTags(html) {
    if (!html) return "";
    return html
        .replace(/<audio\b[^>]*>[\s\S]*?<\/audio>/gi, "")
        .replace(/<audio\b[^>]*\/>/gi, "");
}

// AudioPlayer

export function AudioPlayer({ path, style, buttonClassName = "audio-btn" }) {
    const [src, setSrc] = useState(null);
    const [playing, setPlaying] = useState(false);
    const [loading, setLoading] = useState(false);
    const [failed, setFailed] = useState(false);
    const audioRef = useRef(null);

    useEffect(() => {
        if (!path) return;
        const ext = (path.split(".").pop() ?? "").toLowerCase();
        const mime = {
            mp3: "audio/mpeg", wav: "audio/wav", ogg: "audio/ogg", opus: "audio/ogg",
            m4a: "audio/mp4", mp4: "audio/mp4", webm: "audio/webm",
        }[ext] ?? "audio/mpeg";
        setLoading(true);
        setFailed(false);
        // Played via a blob URL: data: URIs never reach webkit's media
        // pipeline on Linux, and blob URLs work everywhere
        let objectUrl = null;
        let cancelled = false;
        loggedInvoke("read_audio_b64", { path })
            .then(b64 => {
                if (cancelled) return;
                if (b64.length === 0) {
                    // e.g. recordings from builds whose recorder emitted no data
                    setFailed(true);
                    setLoading(false);
                    return;
                }
                const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
                objectUrl = URL.createObjectURL(new Blob([bytes], { type: mime }));
                setSrc(objectUrl);
                setLoading(false);
            })
            .catch(e => {
                logError("read_audio_b64", e);
                if (!cancelled) { setFailed(true); setLoading(false); }
            });
        return () => {
            cancelled = true;
            if (objectUrl) URL.revokeObjectURL(objectUrl);
            setSrc(null);
            setPlaying(false);
        };
    }, [path]);

    function handleToggle() {
        const audio = audioRef.current;
        if (!audio) return;
        if (playing) {
            audio.pause();
        } else {
            audio.play().catch(e => { logError("audio_play", e); setFailed(true); });
        }
    }

    if (failed) {
        return (
            <div style={{ fontSize: 12, color: "var(--t-text-3)", textAlign: "center", ...style }}>
                Audio unavailable
            </div>
        );
    }
    if (!src && !loading) return null;

    return (
        <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 8, ...style }}>
            {src && (
                <audio
                    ref={audioRef}
                    src={src}
                    onPlay={() => setPlaying(true)}
                    onPause={() => setPlaying(false)}
                    onEnded={() => setPlaying(false)}
                    onError={() => setFailed(true)}
                />
            )}
            <button onClick={handleToggle} disabled={!src || loading} className={buttonClassName}>
                <span style={{ fontSize: "1.1em" }}>{playing ? "⏸" : "▶"}</span>
                <span>{loading ? "Loading..." : playing ? "Pause" : "Play"}</span>
            </button>
        </div>
    );
}

// CardFace

const imgStyle = {
    maxWidth: "100%",
    maxHeight: 280,
    objectFit: "contain",
    borderRadius: 6,
    display: "block",
    margin: "10px auto 0",
    border: "1px solid var(--t-border)",
};

const audioWrapStyle = {
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    gap: 6,
    marginTop: 10,
};

const audioStyle = { width: "100%", maxWidth: 320 };

export function CardFace({ card, showBack }) {
    const frontAudioSrcs = card.is_uploaded ? extractRawAudioSrcs(card.front) : [];
    const backAudioSrcs  = card.is_uploaded ? extractRawAudioSrcs(card.back)  : [];
    const supportAudioSrcs = card.is_uploaded && card.imported_support ? extractRawAudioSrcs(card.imported_support) : [];

    // Anki-embedded audio first, then any audio added in the editor
    const frontAudio = [
        ...frontAudioSrcs.map((src, i) => <AudioPlayer key={`anki-${i}`} path={src} style={audioStyle} />),
        ...(card.front_audio ? [<AudioPlayer key="own" path={card.front_audio} style={audioStyle} />] : []),
    ];

    const backAudio = [
        ...backAudioSrcs.map((src, i) => <AudioPlayer key={`anki-${i}`} path={src} style={audioStyle} />),
        ...(card.back_audio ? [<AudioPlayer key="own" path={card.back_audio} style={audioStyle} />] : []),
    ];

    return (
        <div style={{ width: "100%" }}>
            <div style={{ textAlign: "center" }}>
                {card.is_uploaded ? (
                    <div style={{ fontSize: 16, fontWeight: 600, color: "var(--t-text)", fontFamily: "inherit" }}
                        dangerouslySetInnerHTML={{ __html: renderAnkiHtml(stripAudioTags(card.front)) }} />
                ) : (
                    <div style={{ fontSize: 16, fontWeight: 600, whiteSpace: "pre-wrap", wordBreak: "break-word", color: "var(--t-text)" }}>
                        <LinkifiedText text={card.front} />
                    </div>
                )}
            </div>

            {card.front_image && (
                <img src={mediaSrc(card.front_image)} alt="" style={imgStyle} />
            )}

            {frontAudio.length > 0 && (
                <div style={audioWrapStyle}>{frontAudio}</div>
            )}

            {showBack && (
                <>
                    <hr style={{ border: "none", borderTop: "1px solid var(--t-border)", margin: "14px 0" }} />
                    <div style={{ textAlign: "center" }}>
                        {card.is_uploaded ? (
                            <div style={{ fontSize: 15, color: "var(--t-text)", fontFamily: "inherit" }}
                                dangerouslySetInnerHTML={{ __html: renderAnkiHtml(stripAudioTags(card.back)) }} />
                        ) : (
                            <div style={{ fontSize: 15, whiteSpace: "pre-wrap", wordBreak: "break-word", color: "var(--t-text)" }}>
                                <LinkifiedText text={card.back} />
                            </div>
                        )}
                        {(card.imported_support || card.support) && (
                            <div style={{ fontSize: 13, color: "var(--t-text-2)", marginTop: 10, whiteSpace: "pre-wrap", borderTop: "1px solid var(--t-border)", paddingTop: 10 }}>
                                {card.imported_support && (
                                    <div dangerouslySetInnerHTML={{ __html: renderAnkiHtml(stripAudioTags(card.imported_support)) }} />
                                )}
                                {supportAudioSrcs.length > 0 && (
                                    <div style={audioWrapStyle}>
                                        {supportAudioSrcs.map((src, i) => <AudioPlayer key={i} path={src} style={audioStyle} />)}
                                    </div>
                                )}
                                {card.support && (
                                    <div style={{ marginTop: card.imported_support ? 8 : 0 }}>
                                        <LinkifiedText text={card.support} />
                                    </div>
                                )}
                            </div>
                        )}
                    </div>

                    {card.back_image && (
                        <img src={mediaSrc(card.back_image)} alt="" style={imgStyle} />
                    )}

                    {backAudio.length > 0 && (
                        <div style={audioWrapStyle}>{backAudio}</div>
                    )}
                </>
            )}
        </div>
    );
}
