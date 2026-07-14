const HOUR = 3600_000;
const DAY = 86400_000;

const MAX_UPLOADS_PER_IP_PER_HOUR = 20;
const MAX_UPLOADS_PER_SLOT_PER_HOUR = 20;
const MAX_NEW_SLOTS_PER_IP_PER_DAY = 10;

export class Limiter {
    constructor(ctx) {
        this.sql = ctx.storage.sql;
        this.sql.exec(
            "CREATE TABLE IF NOT EXISTS uploads (ip TEXT, slot TEXT, ts INTEGER)"
        );
        this.sql.exec("CREATE INDEX IF NOT EXISTS uploads_ts ON uploads (ts)");
    }

    count(query, ...args) {
        return this.sql.exec(query, ...args).one().c;
    }

    async fetch(request) {
        // `fresh` = slot doesn't exist in R2 yet. Re-pushing an existing slot
        // never touches the new-slot quota, so shared IPs (CGNAT) only compete
        // on genuinely-first pushes.
        const { ip, slot, fresh } = await request.json();
        const now = Date.now();

        this.sql.exec("DELETE FROM uploads WHERE ts < ?", now - DAY);

        const ipHour = this.count(
            "SELECT COUNT(*) AS c FROM uploads WHERE ip = ? AND ts > ?",
            ip,
            now - HOUR
        );
        if (ipHour >= MAX_UPLOADS_PER_IP_PER_HOUR) {
            return Response.json({ ok: false, reason: "rate" });
        }

        const slotHour = this.count(
            "SELECT COUNT(*) AS c FROM uploads WHERE slot = ? AND ts > ?",
            slot,
            now - HOUR
        );
        if (slotHour >= MAX_UPLOADS_PER_SLOT_PER_HOUR) {
            return Response.json({ ok: false, reason: "rate" });
        }

        // Bounds storage growth: minting UUIDs is free, bringing new slots into
        // existence isn't.
        if (fresh) {
            const newSlots = this.count(
                "SELECT COUNT(DISTINCT slot) AS c FROM uploads WHERE ip = ? AND ts > ?",
                ip,
                now - DAY
            );
            if (newSlots >= MAX_NEW_SLOTS_PER_IP_PER_DAY) {
                return Response.json({ ok: false, reason: "quota" });
            }
        }

        this.sql.exec("INSERT INTO uploads (ip, slot, ts) VALUES (?, ?, ?)", ip, slot, now);
        return Response.json({ ok: true });
    }
}
