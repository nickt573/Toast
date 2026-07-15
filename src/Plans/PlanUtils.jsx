const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

// Rainbow categories (muted to fit the cozy theme, each works as a white-text fill
// and as colored text on a light tint). Reading-red is kept brighter than the
// deep-maroon resource color so the two reds don't collide.
export const CATEGORIES = [
    { label: "Reading",    bit: 1,  color: "#C0392B" },  // red
    { label: "Writing",    bit: 2,  color: "#C2702A" },  // orange
    { label: "Speaking",   bit: 4,  color: "#B08A1F" },  // dark gold (not pale yellow: legibility)
    { label: "Listening",  bit: 8,  color: "#4A8C5E" },  // green
    { label: "Vocabulary", bit: 16, color: "#3E6E96" },  // blue
    { label: "Grammar",    bit: 32, color: "#7A5E8A" },  // purple
    { label: "Culture",    bit: 64, color: "#8A6E55" },  // brown
];

// Single source of truth for category colors, themed to the cozy palette.
export const CATEGORY_COLOR_BY_LABEL = Object.fromEntries(CATEGORIES.map(c => [c.label, c.color]));

// Frequency helpers

export function computeFrequency(frequency) {
    let mask = 0;
    for (let i = 0; i < 7; i++) {
        if (frequency[i]) mask |= (1 << i);
    }
    return mask;
}

export function maskToArray(mask) {
    return Array.from({ length: 7 }, (_, i) => (mask & (1 << i)) !== 0);
}

// Category helpers

export function computeCategory(categoryArray) {
    let mask = 0;
    for (const { bit } of CATEGORIES) {
        if (categoryArray[bit]) mask |= bit;
    }
    return mask;
}

export function maskToCategories(mask) {
    const result = {};
    for (const { bit } of CATEGORIES) {
        result[bit] = (mask & bit) !== 0;
    }
    return result;
}

// Shared components: both pickers render the shared .picker-pill control (App.css).

export function FrequencyPicker({ frequency, onChange }) {
    return (
        <div style={{ display: "flex", gap: 5, flexWrap: "wrap" }}>
            {DAYS.map((d, i) => (
                <label key={d} className={`picker-pill${frequency[i] ? " active-accent" : ""}`}>
                    <input
                        type="checkbox"
                        checked={frequency[i]}
                        onChange={() => onChange(i)}
                        style={{ margin: 0 }}
                    />
                    {d}
                </label>
            ))}
        </div>
    );
}

// Solid colored category pills, matching the todo stats table header.
export function CategoryPills({ mask, style }) {
    const cats = CATEGORIES.filter(({ bit }) => mask & bit);
    if (cats.length === 0) return null;
    return (
        <span style={{ display: "inline-flex", gap: 5, flexWrap: "wrap", ...style }}>
            {cats.map(({ label, color }) => (
                <span key={label} style={{
                    display: "inline-block", padding: "2px 8px", borderRadius: "var(--t-r-pill)",
                    fontSize: 10, fontWeight: 500, background: color, color: "var(--t-btn-fg)",
                }}>
                    {label}
                </span>
            ))}
        </span>
    );
}

export function CategoryPicker({ categoryMap, onChange }) {
    return (
        <div style={{ display: "flex", gap: 5, flexWrap: "wrap" }}>
            {CATEGORIES.map(({ label, bit, color }) => {
                const active = !!categoryMap[bit];
                // Category colors are per-item, so the active state stays inline
                const style = active
                    ? { borderColor: color, background: `${color}1F`, color }
                    : undefined;
                return (
                    <label key={bit} className="picker-pill" style={style}>
                        <input
                            type="checkbox"
                            checked={active}
                            onChange={() => onChange(bit)}
                            style={{ margin: 0 }}
                        />
                        {label}
                    </label>
                );
            })}
        </div>
    );
}
