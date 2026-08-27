const FIVE_HOUR_MAX_MINUTES = 360;

function windowMinutes(window) {
  const value = Number(window?.windowMinutes);
  return Number.isFinite(value) && value > 0 ? value : null;
}

export function isFiveHourWindow(window) {
  const minutes = windowMinutes(window);
  return minutes !== null && minutes <= FIVE_HOUR_MAX_MINUTES;
}

export function orderedQuotaWindows(windows) {
  return [...windows].sort((left, right) => {
    const leftIsFiveHour = isFiveHourWindow(left);
    const rightIsFiveHour = isFiveHourWindow(right);
    if (leftIsFiveHour !== rightIsFiveHour) {
      return leftIsFiveHour ? -1 : 1;
    }

    return (
      (windowMinutes(left) ?? Number.POSITIVE_INFINITY) -
      (windowMinutes(right) ?? Number.POSITIVE_INFINITY)
    );
  });
}

export function selectPrimaryQuotaWindow(windows) {
  return orderedQuotaWindows(windows).find(isFiveHourWindow) ?? windows[0] ?? null;
}
