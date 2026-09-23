export const AUTO_REFRESH_INTERVALS_MINUTES = [1, 2, 5, 10, 30];
export const DEFAULT_AUTO_REFRESH_INTERVAL_MINUTES = 1;

export function normalizeAutoRefreshInterval(value) {
  const minutes = Number(value);
  return AUTO_REFRESH_INTERVALS_MINUTES.includes(minutes)
    ? minutes
    : DEFAULT_AUTO_REFRESH_INTERVAL_MINUTES;
}

export function autoRefreshIntervalLabel(value) {
  return `每 ${normalizeAutoRefreshInterval(value)} 分钟`;
}
