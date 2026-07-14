import { Limiter } from "./limiter.js";

// Toast to Go — one package slot per instance UUID; push overwrites, pull reads.
// Uploads are multipart because a Worker request body is capped at 100 MB.

const UUID_RE =
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

const MAX_PART_BYTES = 55 * 1024 * 1024;
const MAX_PARTS = 20;
const MAX_TOTAL_BYTES = 1024 * 1024 * 1024;

const json = (body, status = 200) =>
    new Response(JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
    });

const err = (status, message) => json({ error: message }, status);

const tooMany = (message = "Too many requests. Try again in a minute.", retryAfter = 60) =>
    new Response(JSON.stringify({ error: message }), {
        status: 429,
        headers: { "content-type": "application/json", "retry-after": String(retryAfter) },
    });

export default {
    async fetch(request, env) {
        const { pathname } = new URL(request.url);
        const seg = pathname.split("/").filter(Boolean); // ["p", uuid, ...]

        if (seg[0] !== "p" || !seg[1]) return err(404, "Not found");

        // Key validation must stay first: it's what stops the bucket being open storage.
        const key = seg[1];
        if (!UUID_RE.test(key)) return err(400, "Malformed ID");

        const ip = request.headers.get("cf-connecting-ip") ?? "unknown";
        if (!(await env.IP_LIMIT.limit({ key: ip })).success) return tooMany();

        const rest = seg.slice(2);

        if (rest.length === 0) return handleObject(request, env, key);
        if (rest[0] === "mpu") {
            if (rest.length === 1 && request.method === "POST") {
                const soft = await env.UPLOAD_IP_LIMIT.limit({ key: ip });
                if (!soft.success) return tooMany();

                const fresh = !(await env.PACKAGES.head(key));
                const stub = env.LIMITER.get(env.LIMITER.idFromName("global"));
                const verdict = await stub.fetch("https://limiter/check", {
                    method: "POST",
                    body: JSON.stringify({ ip, slot: key, fresh }),
                });
                const v = await verdict.json();
                if (!v.ok) {
                    return v.reason === "quota"
                        ? tooMany("Your network has set up too many new Toast to Go IDs today. Try again tomorrow.", 86400)
                        : tooMany();
                }
            }
            return handleMultipart(request, env, key, rest.slice(1));
        }
        return err(404, "Not found");
    },
};

async function handleObject(request, env, key) {
    if (request.method === "HEAD") {
        const head = await env.PACKAGES.head(key);
        if (!head) return new Response(null, { status: 404 });
        return new Response(null, {
            status: 200,
            headers: {
                "content-length": String(head.size),
                "x-package-size": String(head.size),
                "x-package-uploaded": head.uploaded.toISOString(),
            },
        });
    }

    if (request.method === "GET") {
        const obj = await env.PACKAGES.get(key);
        if (!obj) return err(404, "No package found for that ID");

        const headers = new Headers();
        obj.writeHttpMetadata(headers);
        headers.set("content-type", "application/zip");
        headers.set("content-length", String(obj.size));
        headers.set("etag", obj.httpEtag);
        return new Response(obj.body, { status: 200, headers });
    }

    return err(405, "Method not allowed");
}

async function handleMultipart(request, env, key, rest) {
    if (rest.length === 0) {
        if (request.method !== "POST") return err(405, "Method not allowed");
        const mpu = await env.PACKAGES.createMultipartUpload(key);
        return json({ uploadId: mpu.uploadId });
    }

    const uploadId = rest[0];
    const mpu = env.PACKAGES.resumeMultipartUpload(key, uploadId);

    if (rest.length === 1) {
        if (request.method !== "DELETE") return err(405, "Method not allowed");
        try {
            await mpu.abort();
        } catch {}
        return json({});
    }

    if (rest[1] === "complete") {
        if (request.method !== "POST") return err(405, "Method not allowed");

        let body;
        try {
            body = await request.json();
        } catch {
            return err(400, "Malformed completion body");
        }

        const parts = body?.parts;
        if (!Array.isArray(parts) || parts.length === 0) {
            return err(400, "Completion requires a non-empty parts list");
        }
        if (parts.length > MAX_PARTS) {
            return err(413, "Package is too large");
        }
        if (!parts.every((p) => Number.isInteger(p?.partNumber) && typeof p?.etag === "string")) {
            return err(400, "Malformed parts list");
        }

        let obj;
        try {
            obj = await mpu.complete(parts);
        } catch (e) {
            return err(400, `Could not complete upload: ${e.message}`);
        }
        // Per-part checks trust content-length; this doesn't.
        if (obj.size > MAX_TOTAL_BYTES) {
            await env.PACKAGES.delete(key);
            return err(413, "Package is too large");
        }
        return json({});
    }

    // PUT a single part.
    const partNumber = Number(rest[1]);
    if (!Number.isInteger(partNumber) || partNumber < 1 || partNumber > MAX_PARTS) {
        return err(413, "Package is too large");
    }
    if (request.method !== "PUT") return err(405, "Method not allowed");
    if (!request.body) return err(400, "Empty part");

    const declared = Number(request.headers.get("content-length") ?? 0);
    if (declared > MAX_PART_BYTES) return err(413, "Part is too large");

    try {
        const part = await mpu.uploadPart(partNumber, request.body);
        return json({ partNumber: part.partNumber, etag: part.etag });
    } catch (e) {
        return err(400, `Could not upload part: ${e.message}`);
    }
}

export { Limiter };
