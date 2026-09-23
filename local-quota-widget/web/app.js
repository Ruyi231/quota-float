import {
  isRecentSnapshot,
  shouldReplaceSnapshot,
  timestampMilliseconds,
} from "./snapshot-order.mjs";
import {
  isFiveHourWindow,
  orderedQuotaWindows,
  selectPrimaryQuotaWindow,
} from "./quota-display.mjs";
import {
  autoRefreshIntervalLabel,
  normalizeAutoRefreshInterval,
} from "./refresh-settings.mjs";

const invoke = window.__TAURI__?.core?.invoke;

const elements = {
  widget: document.querySelector("#widget"),
  orb: document.querySelector("#orb"),
  orbValue: document.querySelector("#orb-value"),
  orbStatus: document.querySelector("#orb-status"),
  panel: document.querySelector("#panel"),
  planLine: document.querySelector("#plan-line"),
  summaryLabel: document.querySelector("#summary-label"),
  summaryValue: document.querySelector("#summary-value"),
  freshness: document.querySelector("#freshness"),
  windows: document.querySelector("#windows"),
  emptyState: document.querySelector("#empty-state"),
  emptyMessage: document.querySelector("#empty-message"),
  skin: document.querySelector("#skin"),
  skinMenu: document.querySelector("#skin-menu"),
  skinToast: document.querySelector("#skin-toast"),
  skinChoices: [...document.querySelectorAll("[data-skin-choice]")],
  customSkinChoice: document.querySelector("#custom-skin-choice"),
  photoPick: document.querySelector("#photo-pick"),
  removePhoto: document.querySelector("#remove-photo"),
  refresh: document.querySelector("#refresh"),
  refreshInterval: document.querySelector("#refresh-interval"),
  refreshNote: document.querySelector("#refresh-note"),
  collapse: document.querySelector("#collapse"),
  quit: document.querySelector("#quit"),
  dragHandle: document.querySelector("#drag-handle"),
  resetQuery: document.querySelector("#reset-query"),
  resetCount: document.querySelector("#reset-count"),
  resetExpiry: document.querySelector("#reset-expiry"),
};

const skins = [
  { id: "midnight", name: "深海" },
  { id: "aurora", name: "极光" },
  { id: "porcelain", name: "暖白" },
  { id: "sakura", name: "樱粉" },
];

let expanded = false;
let hoverTimer;
let collapseTimer;
let latestSnapshot;
let skinToastTimer;
let customSkinUrl;
let orbPointer;
let refreshInFlight;
let lastRefreshAt = 0;
let latestResetCredits;
let resetCreditBlockedUntil = 0;
let quotaOnlineBlockedUntil = 0;
let autoRefreshTimer;

const AUTO_REFRESH_INTERVAL_KEY =
  "local-quota-widget-auto-refresh-interval-minutes-v1";
const RESET_CREDIT_MIN_INTERVAL_MS = 60_000;
const RESET_CREDIT_RATE_LIMIT_FALLBACK_MS = 5 * 60_000;
const QUOTA_ONLINE_RATE_LIMIT_FALLBACK_MS = 5 * 60_000;
const ACTIVITY_REFRESH_MIN_INTERVAL_MS = 10_000;

function showSkinToast(message) {
  elements.skinToast.textContent = message;
  elements.skinToast.classList.add("is-visible");
  window.clearTimeout(skinToastTimer);
  skinToastTimer = window.setTimeout(
    () => elements.skinToast.classList.remove("is-visible"),
    1400,
  );
}

function setSkinMenuOpen(open) {
  elements.skinMenu.hidden = !open;
  elements.skin.setAttribute("aria-expanded", String(open));
}

function applySkin(skinId, announce = false) {
  const builtIn = skins.find((skin) => skin.id === skinId);
  const isCustom = skinId === "custom" && Boolean(customSkinUrl);
  const skin = isCustom ? { id: "custom", name: "我的照片" } : builtIn || skins[0];
  elements.widget.dataset.skin = skin.id;
  elements.skin.title = `一键换肤 · 当前：${skin.name}`;
  elements.skin.setAttribute("aria-label", `切换皮肤，当前为${skin.name}`);
  try {
    window.localStorage.setItem("local-quota-widget-skin", skin.id);
  } catch {
    // A blocked localStorage must not prevent theme switching.
  }
  if (announce) showSkinToast(`已切换：${skin.name}`);
}

function installCustomPhoto(dataUrl) {
  customSkinUrl = dataUrl;
  elements.widget.style.setProperty(
    "--custom-skin-image",
    `url("${customSkinUrl}")`,
  );
  elements.customSkinChoice.hidden = false;
  elements.removePhoto.hidden = false;
}

async function importPhoto() {
  if (!invoke) return;
  elements.photoPick.disabled = true;
  try {
    const dataUrl = await invoke("choose_custom_skin");
    if (!dataUrl) return;
    installCustomPhoto(dataUrl);
    applySkin("custom", true);
    setSkinMenuOpen(false);
  } catch (error) {
    showSkinToast(String(error || "照片保存失败"));
  } finally {
    elements.photoPick.disabled = false;
  }
}

async function clearCustomPhoto() {
  if (!invoke) return;
  try {
    await invoke("clear_custom_skin");
    customSkinUrl = undefined;
    elements.widget.style.removeProperty("--custom-skin-image");
    elements.customSkinChoice.hidden = true;
    elements.removePhoto.hidden = true;
    applySkin("midnight", true);
    setSkinMenuOpen(false);
  } catch {
    showSkinToast("照片删除失败");
  }
}

async function initializeSkin() {
  let saved;
  try {
    saved = window.localStorage.getItem("local-quota-widget-skin");
  } catch {
    saved = null;
  }
  if (invoke) {
    try {
      const dataUrl = await invoke("get_custom_skin");
      if (dataUrl) installCustomPhoto(dataUrl);
    } catch {
      // Built-in skins remain available when the saved photo is unavailable.
    }
  }
  applySkin(saved || "midnight");
}

function creditExpiryText(value) {
  if (value === null || value === undefined || value === "") return "到期时间未知";
  const text = String(value);
  const date = /^\d+$/.test(text)
    ? new Date(Number(text) * 1000)
    : new Date(text);
  if (Number.isNaN(date.valueOf())) return text;
  return date.toLocaleString("zh-CN", {
    year: "numeric",
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function renderResetCredits(snapshot) {
  latestResetCredits = snapshot;
  const hasCount =
    snapshot?.count !== null &&
    snapshot?.count !== undefined &&
    Number.isFinite(Number(snapshot.count));
  elements.resetCount.textContent = hasCount
    ? `${Number(snapshot.count)} 次`
    : "次数未返回";
  const expirations = Array.isArray(snapshot?.expiresAt) ? snapshot.expiresAt : [];
  elements.resetExpiry.textContent = expirations.length
    ? expirations.map(creditExpiryText).join("；")
    : "接口未返回到期时间";
}

async function queryResetCredits() {
  if (!invoke) return;
  const remainingSeconds = Math.ceil(
    (resetCreditBlockedUntil - Date.now()) / 1000,
  );
  if (remainingSeconds > 0) {
    showSkinToast(`重置卡查询冷却中，请 ${remainingSeconds} 秒后重试`);
    return;
  }
  const approved = window.confirm(
    "在线查询会在本次点击后读取 Codex 登录凭据，并访问 ChatGPT 的重置卡接口。不会发送对话，也不会消耗模型额度。是否继续？",
  );
  if (!approved) return;
  resetCreditBlockedUntil = Date.now() + RESET_CREDIT_MIN_INTERVAL_MS;
  elements.resetQuery.disabled = true;
  elements.resetQuery.textContent = "查询中…";
  if (!latestResetCredits) {
    elements.resetCount.textContent = "正在查询";
    elements.resetExpiry.textContent = "请稍候";
  }
  try {
    renderResetCredits(await invoke("get_reset_credits_online"));
    elements.resetQuery.textContent = "重新查询";
  } catch (error) {
    const message = String(error || "重置卡服务暂不可用");
    const retrySeconds = message.match(/(\d+)\s*秒后重试/)?.[1];
    if (message.includes("过于频繁")) {
      resetCreditBlockedUntil =
        Date.now() +
        (retrySeconds
          ? Number(retrySeconds) * 1000
          : RESET_CREDIT_RATE_LIMIT_FALLBACK_MS);
    }
    if (!latestResetCredits) {
      elements.resetCount.textContent = "查询受限";
      elements.resetExpiry.textContent = message;
    }
    elements.resetQuery.textContent = "稍后重试";
    showSkinToast(message);
  } finally {
    elements.resetQuery.disabled = false;
  }
}

function clampPercent(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return 0;
  return Math.max(0, Math.min(100, number));
}

function percentText(value) {
  const number = clampPercent(value);
  return `${number >= 10 ? Math.round(number) : number.toFixed(1)}%`;
}

function planName(value) {
  if (!value) return "本机会话日志";
  const normalized = String(value).toLowerCase();
  const names = {
    plus: "ChatGPT Plus",
    pro: "ChatGPT Pro",
    team: "ChatGPT Team",
    business: "ChatGPT Business",
    enterprise: "ChatGPT Enterprise",
  };
  return names[normalized] ?? `Codex ${value}`;
}

function resetText(epochSeconds) {
  if (!Number.isFinite(Number(epochSeconds))) return "重置时间未知";
  const date = new Date(Number(epochSeconds) * 1000);
  if (Number.isNaN(date.valueOf())) return "重置时间未知";
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  const options = sameDay
    ? { hour: "2-digit", minute: "2-digit" }
    : { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" };
  return `${sameDay ? "今天" : ""}${date.toLocaleString("zh-CN", options)} 重置`;
}

function freshnessText(timestamp) {
  if (!timestamp) return "等待数据";
  const time = new Date(timestampMilliseconds(timestamp));
  if (Number.isNaN(time.valueOf())) return "已读取本地记录";
  const minutes = Math.max(0, Math.floor((Date.now() - time.valueOf()) / 60_000));
  if (minutes < 1) return "刚刚记录";
  if (minutes < 60) return `${minutes} 分钟前记录`;
  if (minutes < 24 * 60) return `${Math.floor(minutes / 60)} 小时前记录`;
  return `${Math.floor(minutes / (24 * 60))} 天前记录`;
}

function updateFreshness() {
  elements.freshness.textContent = freshnessText(latestSnapshot?.observedAt);
}

function renderWindow(item) {
  const remaining = clampPercent(item.remainingPercent);
  const row = document.createElement("article");
  row.className = "window-row";

  const label = document.createElement("span");
  label.className = "label";
  label.textContent = item.label;

  const percent = document.createElement("span");
  percent.className = "percent";
  percent.textContent = `${percentText(remaining)} 剩余`;

  const reset = document.createElement("span");
  reset.className = "reset";
  reset.textContent = resetText(item.resetsAt);

  const spacer = document.createElement("span");
  spacer.setAttribute("aria-hidden", "true");

  const progress = document.createElement("div");
  progress.className = "progress";
  progress.setAttribute("role", "progressbar");
  progress.setAttribute("aria-label", `${item.label}剩余额度`);
  progress.setAttribute("aria-valuemin", "0");
  progress.setAttribute("aria-valuemax", "100");
  progress.setAttribute("aria-valuenow", String(remaining));
  const fill = document.createElement("span");
  progress.append(fill);

  row.append(label, percent, reset, spacer, progress);
  requestAnimationFrame(() => {
    fill.style.width = `${remaining}%`;
  });
  return row;
}

function render(snapshot) {
  latestSnapshot = snapshot;
  const windows = orderedQuotaWindows(
    [snapshot?.primary, snapshot?.secondary].filter(Boolean),
  );
  const ok = snapshot?.status === "ok" && windows.length > 0;
  const primaryWindow = ok ? selectPrimaryQuotaWindow(windows) : null;
  const remaining = primaryWindow
    ? clampPercent(primaryWindow.remainingPercent)
    : 0;

  elements.orbValue.textContent = ok ? percentText(remaining) : "--";
  elements.orb.style.setProperty("--remaining", `${remaining * 3.6}deg`);
  elements.orbStatus.className = `status-dot ${ok ? "ok" : "error"}`;
  const sourceLabel =
    snapshot?.source === "online" ? "在线额度服务" : "本地记录";
  elements.planLine.textContent = ok
    ? `${planName(snapshot.planType)} · ${sourceLabel}`
    : "未找到可用的本地额度记录";
  elements.summaryLabel.textContent = isFiveHourWindow(primaryWindow)
    ? "5 小时可用额度"
    : primaryWindow?.label || "可用额度";
  elements.summaryValue.textContent = ok ? percentText(remaining) : "--%";
  updateFreshness();
  elements.windows.replaceChildren(...windows.map(renderWindow));
  elements.windows.hidden = !ok;
  elements.emptyState.hidden = ok;
  elements.emptyMessage.textContent =
    snapshot?.message || "请先在 Codex 中运行一次任务，然后再刷新。";
}

async function executeRefresh({
  online = false,
  announce = false,
  fallbackLocal = false,
} = {}) {
  if (!invoke) {
    render({
      status: "missing",
      message: "当前页面未运行在桌面应用中。",
    });
    return;
  }
  elements.refresh.classList.add("is-spinning");
  elements.refresh.disabled = true;
  try {
    let snapshot;
    let usedFallback = false;
    let onlineError;
    try {
      if (online) {
        snapshot = await invoke("get_quota_snapshot_online");
        if (snapshot?.resetCredits) {
          renderResetCredits(snapshot.resetCredits);
          elements.resetQuery.textContent = "重新查询";
        }
      } else {
        snapshot = await invoke("get_quota_snapshot");
      }
    } catch (error) {
      if (!online || !fallbackLocal) throw error;
      usedFallback = true;
      onlineError = String(error || "在线额度服务暂不可用");
      if (onlineError.includes("过于频繁")) {
        const retrySeconds = onlineError.match(/(\d+)\s*秒后重试/)?.[1];
        quotaOnlineBlockedUntil =
          Date.now() +
          (retrySeconds
            ? Number(retrySeconds) * 1000
            : QUOTA_ONLINE_RATE_LIMIT_FALLBACK_MS);
      }
      snapshot = await invoke("get_quota_snapshot");
    }
    const accepted = shouldReplaceSnapshot(latestSnapshot, snapshot, {
      force: online && !usedFallback,
    });
    if (accepted) {
      render(snapshot);
    } else {
      updateFreshness();
    }
    lastRefreshAt = Date.now();
    if (announce) {
      showSkinToast(
        usedFallback
          ? accepted && isRecentSnapshot(snapshot)
            ? "已刷新：本地最新额度"
            : accepted
              ? "已读取本地记录，在线服务暂不可用"
              : "已保留较新的额度"
          : online
            ? "已在线刷新额度"
            : accepted
              ? "已读取本地最新记录"
              : "本地记录较旧，已保留当前额度",
      );
    }
    return accepted ? snapshot : latestSnapshot;
  } catch (error) {
    if (!latestSnapshot) {
      render({
        status: "error",
        source: online ? "online" : "local",
        message: String(error || "额度刷新失败。"),
      });
    }
    if (announce) showSkinToast(String(error || "额度刷新失败"));
    return undefined;
  } finally {
    elements.refresh.disabled = false;
    window.setTimeout(() => elements.refresh.classList.remove("is-spinning"), 240);
  }
}

async function refresh(options = {}) {
  if (refreshInFlight) return refreshInFlight;
  const task = executeRefresh(options);
  refreshInFlight = task;
  try {
    return await task;
  } finally {
    if (refreshInFlight === task) refreshInFlight = undefined;
  }
}

function readAutoRefreshInterval() {
  try {
    return normalizeAutoRefreshInterval(
      window.localStorage.getItem(AUTO_REFRESH_INTERVAL_KEY),
    );
  } catch {
    return normalizeAutoRefreshInterval();
  }
}

function saveAutoRefreshInterval(minutes) {
  try {
    window.localStorage.setItem(AUTO_REFRESH_INTERVAL_KEY, String(minutes));
  } catch {
    // The default interval remains available when localStorage is unavailable.
  }
}

async function scheduledOnlineRefresh({ announce = false } = {}) {
  const cooldownSeconds = Math.ceil(
    (quotaOnlineBlockedUntil - Date.now()) / 1000,
  );
  if (cooldownSeconds > 0) {
    const snapshot = await refresh({ announce: false });
    if (announce) {
      showSkinToast(
        isRecentSnapshot(snapshot)
          ? "已刷新：本地最新额度"
          : `在线刷新冷却中，请 ${cooldownSeconds} 秒后重试`,
      );
    }
    return snapshot;
  }
  return refresh({
    online: true,
    announce,
    fallbackLocal: true,
  });
}

function configureAutoRefresh(minutes = readAutoRefreshInterval()) {
  const normalized = normalizeAutoRefreshInterval(minutes);
  window.clearInterval(autoRefreshTimer);
  elements.refreshInterval.value = String(normalized);
  const label = autoRefreshIntervalLabel(normalized);
  elements.refreshNote.textContent = `${label}自动在线刷新`;
  autoRefreshTimer = window.setInterval(
    () => scheduledOnlineRefresh(),
    normalized * 60_000,
  );
  return normalized;
}

async function refreshOnlineWhenDue() {
  if (refreshInFlight) return refreshInFlight;
  if (Date.now() - lastRefreshAt < ACTIVITY_REFRESH_MIN_INTERVAL_MS) return;
  await scheduledOnlineRefresh();
}

async function manualRefresh() {
  await scheduledOnlineRefresh({ announce: true });
}

async function setExpanded(next) {
  window.clearTimeout(hoverTimer);
  window.clearTimeout(collapseTimer);
  if (expanded === next) return;
  expanded = next;
  if (!next) setSkinMenuOpen(false);
  elements.widget.classList.toggle("is-expanded", next);
  elements.widget.classList.toggle("is-collapsed", !next);
  if (invoke) {
    try {
      await invoke("resize_widget", { expanded: next });
    } catch (error) {
      console.error("窗口缩放失败", error);
    }
  }
  if (next) {
    updateFreshness();
    refreshOnlineWhenDue();
  }
}

elements.orb.addEventListener("mouseenter", () => {
  window.clearTimeout(collapseTimer);
  hoverTimer = window.setTimeout(() => setExpanded(true), 110);
});

elements.orb.addEventListener("mouseleave", () => {
  window.clearTimeout(hoverTimer);
});

elements.panel.addEventListener("mouseenter", () => {
  window.clearTimeout(collapseTimer);
});

elements.panel.addEventListener("mouseleave", () => {
  collapseTimer = window.setTimeout(() => setExpanded(false), 420);
});

elements.collapse.addEventListener("click", () => setExpanded(false));
elements.skin.addEventListener("click", (event) => {
  event.stopPropagation();
  setSkinMenuOpen(elements.skinMenu.hidden);
});
for (const choice of elements.skinChoices) {
  choice.addEventListener("click", () => {
    applySkin(choice.dataset.skinChoice, true);
    setSkinMenuOpen(false);
  });
}
elements.photoPick.addEventListener("click", importPhoto);
elements.removePhoto.addEventListener("click", clearCustomPhoto);
document.addEventListener("pointerdown", (event) => {
  if (
    !elements.skinMenu.hidden &&
    !elements.skinMenu.contains(event.target) &&
    !elements.skin.contains(event.target)
  ) {
    setSkinMenuOpen(false);
  }
});
elements.refresh.addEventListener("click", manualRefresh);
elements.refreshInterval.addEventListener("change", () => {
  const interval = configureAutoRefresh(elements.refreshInterval.value);
  saveAutoRefreshInterval(interval);
  scheduledOnlineRefresh();
  showSkinToast(`已设为${autoRefreshIntervalLabel(interval)}自动在线刷新`);
});
elements.resetQuery.addEventListener("click", queryResetCredits);
elements.quit.addEventListener("click", async () => {
  if (invoke) await invoke("hide_widget");
});

function clearOrbPointer(event) {
  if (!orbPointer || orbPointer.id !== event.pointerId) return;
  const shouldExpand = !orbPointer.dragging;
  orbPointer = undefined;
  if (shouldExpand) setExpanded(true);
}

elements.orb.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  window.clearTimeout(hoverTimer);
  window.clearTimeout(collapseTimer);
  orbPointer = {
    id: event.pointerId,
    startX: event.screenX,
    startY: event.screenY,
    dragging: false,
  };
  elements.orb.setPointerCapture?.(event.pointerId);
});

elements.orb.addEventListener("pointermove", async (event) => {
  if (!orbPointer || orbPointer.id !== event.pointerId || orbPointer.dragging) {
    return;
  }
  const distance = Math.hypot(
    event.screenX - orbPointer.startX,
    event.screenY - orbPointer.startY,
  );
  if (distance < 5 || !invoke) return;
  orbPointer.dragging = true;
  elements.orb.releasePointerCapture?.(event.pointerId);
  try {
    await invoke("start_dragging", { expanded: false });
  } catch (error) {
    console.error("窗口拖动失败", error);
  }
});

elements.orb.addEventListener("pointerup", clearOrbPointer);
elements.orb.addEventListener("pointercancel", (event) => {
  if (orbPointer?.id === event.pointerId) orbPointer = undefined;
});

elements.dragHandle.addEventListener("pointerdown", async (event) => {
  if (event.button !== 0 || event.target.closest("button") || !invoke) return;
  window.clearTimeout(hoverTimer);
  window.clearTimeout(collapseTimer);
  try {
    await invoke("start_dragging", { expanded: true });
  } catch (error) {
    console.error("窗口拖动失败", error);
  }
});

window.addEventListener("focus", refreshOnlineWhenDue);
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) {
    updateFreshness();
    refreshOnlineWhenDue();
  }
});
window.setInterval(updateFreshness, 30_000);
configureAutoRefresh();
Promise.all([initializeSkin(), scheduledOnlineRefresh()]).finally(async () => {
  if (invoke) await invoke("show_widget");
});
