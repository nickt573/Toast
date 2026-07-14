// Single in-flight guard for Toast to Go transfers. The window close handler
// checks it so a push started anywhere can't be interrupted by (or stacked
// with) a close-triggered push.
export const togoLock = { active: false };
