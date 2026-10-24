/** Paths come back with either slash style; compare them in one normal form. The one `norm` for the whole app. */
export const norm = (p?: string | null) => (p ?? "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
