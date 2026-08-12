export function timestampMilliseconds(value) {
  if (value === null || value === undefined || value === "") return Number.NaN;
  const text = String(value);
  const numeric = /^\d+$/.test(text) ? Number(text) : Number.NaN;
  const time = Number.isFinite(numeric)
    ? new Date(numeric < 1_000_000_000_000 ? numeric * 1000 : numeric)
    : new Date(text);
  return time.valueOf();
}

function hasUsage(snapshot) {
  return (
    snapshot?.status === "ok" &&
    Boolean(snapshot?.primary || snapshot?.secondary)
  );
}

export function shouldReplaceSnapshot(current, candidate, { force = false } = {}) {
  if (force || !current) return true;

  const currentHasUsage = hasUsage(current);
  const candidateHasUsage = hasUsage(candidate);
  if (!candidateHasUsage) return !currentHasUsage;
  if (!currentHasUsage) return true;

  const currentTime = timestampMilliseconds(current.observedAt);
  const candidateTime = timestampMilliseconds(candidate.observedAt);
  if (!Number.isFinite(candidateTime)) return !Number.isFinite(currentTime);
  if (!Number.isFinite(currentTime)) return true;
  if (candidateTime > currentTime) return true;
  if (candidateTime < currentTime) return false;

  return !(current.source === "online" && candidate.source !== "online");
}

export function isRecentSnapshot(
  snapshot,
  { now = Date.now(), maxAgeMs = 2 * 60_000 } = {},
) {
  const observedAt = timestampMilliseconds(snapshot?.observedAt);
  return (
    snapshot?.status === "ok" &&
    Number.isFinite(observedAt) &&
    observedAt <= now + 30_000 &&
    now - observedAt <= maxAgeMs
  );
}
